//! `AppState`: top-level handle the UI layer holds.

use crate::agents::OutgoingUserMessage;
use crate::core::broadcast::{broadcast_user_message, with_phase_envelope};
use crate::core::ipav::IpavPhase;
use crate::core::session::{
    open_session, spawn_emma_handle, spawn_existing_session, EmmaHandle, OpenSessionRequest,
    SessionHandle,
};
use crate::paths::Paths;
use crate::signaling::{ExternalServer, SignalingBridge, SignalingEvent, SignalingServer};
use crate::storage::{Author, MessageKind, Session, Storage};
use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

pub struct AppState {
    pub paths: Paths,
    pub storage: Storage,
    pub bridge: Arc<SignalingBridge>,
    pub signaling_addr: SocketAddr,
    pub signaling_server: Mutex<Option<SignalingServer>>,
    pub sessions: Mutex<HashMap<String, SessionHandle>>,
    pub emma: Mutex<Option<EmmaHandle>>,
    /// External MCP server handle. None when disabled or port-busy at startup;
    /// the binary stays usable in that case (internal MCP keeps working).
    pub external_server: Mutex<Option<ExternalServer>>,
    /// Populated from Tauri's `setup()` once the AppHandle exists. The
    /// external MCP starts BEFORE Tauri setup (see main.rs ordering), so
    /// any MCP tool that needs the webview (screenshot, click, scroll, etc.)
    /// has to wait for this to be filled. `OnceCell` because it's write-once
    /// at startup; no contention.
    pub app_handle: once_cell::sync::OnceCell<tauri::AppHandle>,
}

impl AppState {
    pub async fn new(paths: Paths, storage: Storage, server: SignalingServer) -> Self {
        let bridge = Arc::clone(&server.bridge);
        let addr = server.local_addr;
        Self {
            paths,
            storage,
            bridge,
            signaling_addr: addr,
            signaling_server: Mutex::new(Some(server)),
            sessions: Mutex::new(HashMap::new()),
            emma: Mutex::new(None),
            external_server: Mutex::new(None),
            app_handle: once_cell::sync::OnceCell::new(),
        }
    }

    /// Spawn Emma's solo agent if not already running. Idempotent. Logs +
    /// returns Err on spawn failure (e.g., missing `claude` CLI), does NOT
    /// crash startup.
    pub async fn ensure_emma_started(&self) -> Result<()> {
        let mut emma = self.emma.lock().await;
        if emma.is_some() {
            return Ok(());
        }
        let handle = spawn_emma_handle(
            &self.paths,
            self.storage.clone(),
            Arc::clone(&self.bridge),
            self.signaling_addr,
        )
        .await?;
        *emma = Some(handle);
        Ok(())
    }

    /// Kill Emma's current subprocess and respawn with the latest agent_configs
    /// row. Used when the user swaps Emma's model — env vars are baked in at
    /// spawn time, so the only way to apply a config change is to restart the
    /// subprocess. Brian/Rain don't need this path because their model swaps
    /// apply on next session spawn.
    pub async fn restart_emma(&self) -> Result<()> {
        let mut emma = self.emma.lock().await;
        if let Some(mut old) = emma.take() {
            old.agent.kill();
        }
        let handle = spawn_emma_handle(
            &self.paths,
            self.storage.clone(),
            Arc::clone(&self.bridge),
            self.signaling_addr,
        )
        .await?;
        *emma = Some(handle);
        Ok(())
    }

    pub async fn open_session(
        &self,
        title: impl Into<String>,
        working_repo_path: Option<std::path::PathBuf>,
    ) -> Result<String> {
        let req = OpenSessionRequest {
            title: title.into(),
            working_repo_path,
        };
        let handle = open_session(
            req,
            &self.paths,
            self.storage.clone(),
            Arc::clone(&self.bridge),
            self.signaling_addr,
        )
        .await?;
        let id = handle.id.clone();
        self.sessions.lock().await.insert(id.clone(), handle);
        Ok(id)
    }

    /// Spawn subprocesses for an existing session row (e.g., the seeded Emma
    /// singleton) if not already running. Idempotent — safe to call repeatedly.
    /// Logs and returns Err if spawn fails, but does NOT poison the AppState.
    pub async fn ensure_session_started(&self, session_id: &str) -> Result<()> {
        if self.sessions.lock().await.contains_key(session_id) {
            return Ok(());
        }
        let handle = spawn_existing_session(
            session_id,
            &self.paths,
            self.storage.clone(),
            Arc::clone(&self.bridge),
            self.signaling_addr,
        )
        .await?;
        self.sessions
            .lock()
            .await
            .insert(session_id.to_string(), handle);
        Ok(())
    }

    pub async fn close_session(&self, id: &str, archive: bool) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        if let Some(mut handle) = sessions.remove(id) {
            handle.brian.kill();
            handle.rain.kill();
        }
        self.storage.close_session(id, archive).await?;
        // Wipe any session-level permission grants. Grants don't carry into
        // the next session this user opens — they have to be re-issued.
        if let Err(e) = self.bridge.cleanup_session_permissions(id).await {
            tracing::warn!(?e, session_id = %id, "cleanup_session_permissions failed");
        }
        Ok(())
    }

    pub async fn list_active_sessions(&self) -> Result<Vec<Session>> {
        self.storage.list_active_sessions().await
    }

    pub async fn broadcast(&self, session_id: &str, text: &str) -> Result<()> {
        // Emma is a solo singleton — route to her single agent, not the duo path.
        if session_id == "emma" {
            let emma = self.emma.lock().await;
            let handle = emma
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("emma not started"))?;
            let id = self
                .storage
                .insert_message(
                    "emma",
                    crate::storage::Author::User,
                    crate::storage::MessageKind::Text,
                    text,
                )
                .await?;
            self.bridge
                .notify_message_persisted("emma".into(), id);
            let msg = crate::agents::OutgoingUserMessage::text(text);
            let _ = handle.agent.input_tx.send(msg).await;
            return Ok(());
        }
        let sessions = self.sessions.lock().await;
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("no live session {session_id}"))?;
        // Clear the awaiting halt BEFORE forwarding the user's reply so the
        // duo pumps see chunks again. The bridge map is also cleared (kept
        // in sync) — both point at the same Arc<AtomicBool>, but the bridge
        // call is what survives if/when SessionHandle is dropped.
        handle
            .awaiting
            .store(false, std::sync::atomic::Ordering::Release);
        self.bridge.clear_session_awaiting(session_id).await;
        // Flip every pending `mark_awaiting_user` row to 'answered' — the
        // user's reply IS the answer to a halt. `choice` rows stay pending
        // until the user actually picks an option.
        if let Err(e) = self.storage.clear_pending_halts(session_id).await {
            tracing::warn!(?e, session_id, "clear_pending_halts failed");
        }
        let phase = handle.ipav.lock().await.current_phase;
        let id = broadcast_user_message(
            &self.storage,
            session_id,
            text,
            phase,
            &handle.brian.input_tx,
            &handle.rain.input_tx,
        )
        .await?;
        self.bridge
            .notify_message_persisted(session_id.to_string(), id);
        Ok(())
    }

    /// Set IPAV phase + emit a synthetic user "phase advanced to X" message so
    /// both agents see the transition naturally. Also clears any awaiting-user
    /// halt — an agent that fired `request_phase_advance` has effectively been
    /// answered by the chip click, so the duo should resume.
    pub async fn advance_phase(&self, session_id: &str, target: IpavPhase) -> Result<()> {
        let sessions = self.sessions.lock().await;
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("no live session {session_id}"))?;

        handle
            .awaiting
            .store(false, std::sync::atomic::Ordering::Release);
        self.bridge.clear_session_awaiting(session_id).await;
        if let Err(e) = self.storage.clear_pending_halts(session_id).await {
            tracing::warn!(?e, session_id, "clear_pending_halts (advance_phase) failed");
        }

        let ts = chrono::Utc::now().to_rfc3339();
        handle.ipav.lock().await.advance(target, ts);
        let notice = target.transition_notice().to_string();

        // Synthetic phase-change message in storage.
        let id = self
            .storage
            .insert_message(session_id, Author::User, MessageKind::PhaseChange, &notice)
            .await?;
        self.bridge
            .notify_message_persisted(session_id.to_string(), id);
        // And fed to both agents' stdin so they pick it up as a natural prompt.
        let msg = OutgoingUserMessage::text(notice);
        let _ = handle.brian.input_tx.send(msg.clone()).await;
        let _ = handle.rain.input_tx.send(msg).await;
        Ok(())
    }

    pub async fn resolve_choice(&self, choice_id: &str, picked: String) -> Result<()> {
        use crate::signaling::ResolveOutcome;
        match self.bridge.resolve_choice(choice_id, picked).await? {
            ResolveOutcome::Delivered => Ok(()),
            ResolveOutcome::AgentReceiverDroppedFellBack { session_id, body } => {
                // The OOB message is already in storage (bridge wrote it). To
                // actually wake the duo subprocess so they read + act on it,
                // also: (1) clear the awaiting-user halt so the duo pump
                // resumes peer-forwarding, (2) push the body through both
                // agents' input_tx so their stdin receives a wake message.
                // We deliberately do NOT call broadcast_user_message (which
                // re-inserts) — the storage row already exists.
                if session_id == "emma" {
                    let emma = self.emma.lock().await;
                    if let Some(handle) = emma.as_ref() {
                        // Emma is solo — no IPAV phase tracked; send raw.
                        let msg = crate::agents::OutgoingUserMessage::text(&body);
                        let _ = handle.agent.input_tx.send(msg).await;
                    }
                    return Ok(());
                }
                let sessions = self.sessions.lock().await;
                let Some(handle) = sessions.get(&session_id) else {
                    // Session closed in the gap between resolve and wake —
                    // the OOB message persists in storage either way, so
                    // future re-opens of the session view will still see it.
                    return Ok(());
                };
                handle
                    .awaiting
                    .store(false, std::sync::atomic::Ordering::Release);
                self.bridge.clear_session_awaiting(&session_id).await;
                let phase = handle.ipav.lock().await.current_phase;
                let wire = with_phase_envelope(phase, &body);
                let msg = crate::agents::OutgoingUserMessage::text(wire);
                let _ = handle.brian.input_tx.send(msg.clone()).await;
                let _ = handle.rain.input_tx.send(msg).await;
                Ok(())
            }
        }
    }

    pub fn subscribe_signaling(&self) -> broadcast::Receiver<SignalingEvent> {
        self.bridge.subscribe()
    }

    pub async fn current_phase(&self, session_id: &str) -> Option<IpavPhase> {
        let sessions = self.sessions.lock().await;
        let handle = sessions.get(session_id)?;
        let phase = handle.ipav.lock().await.current_phase;
        Some(phase)
    }

    /// HEAD SHA captured when this session was spawned, used by the session
    /// view's Apply tab to diff "everything applied this session". Returns
    /// None when no working repo, no `.git/`, the spawn-time capture failed,
    /// or the session has already closed.
    pub async fn session_start_sha(&self, session_id: &str) -> Option<String> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .and_then(|h| h.session_start_sha.clone())
    }

    /// Working-repo path for a live session, or None if no repo / not running.
    /// Pairs with `session_start_sha` for the Apply-tab `git diff` invocation.
    pub async fn working_repo_path(&self, session_id: &str) -> Option<std::path::PathBuf> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .and_then(|h| h.working_repo_path.clone())
    }
}

#[cfg(test)]
mod tests {
    // Live session tests require RUN_LIVE_TESTS=1 (subprocesses spawn).
    // We unit-test the static pieces here.

    #[test]
    fn smoke() {
        // Module compiles.
    }
}
