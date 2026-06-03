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
    /// Serializes the duo-spawn path in `ensure_session_started` so two
    /// concurrent calls for the same session (e.g. a double-mount of the
    /// session view firing `respawn_session` twice) can't both pass the
    /// contains_key check and spawn two Brian+Rain pairs — the second insert
    /// would overwrite the first handle and orphan its subprocesses (untracked,
    /// so close_session can't reap them). Only the spawn path takes this; the
    /// fast already-running check short-circuits before acquiring it.
    spawn_gate: Mutex<()>,
    pub emma: Mutex<Option<EmmaHandle>>,
    /// External MCP server handle. None when disabled or port-busy at startup;
    /// the binary stays usable in that case (internal MCP keeps working).
    pub external_server: Mutex<Option<ExternalServer>>,
    /// Populated from Tauri's `setup()` once the AppHandle exists. The
    /// external MCP starts BEFORE Tauri setup (see main.rs ordering), so
    /// any MCP tool that needs the webview (screenshot, click, scroll, etc.)
    /// has to wait for this to be filled. `OnceCell` because it's write-once
    /// at startup; no contention.
    pub app_handle: std::sync::OnceLock<tauri::AppHandle>,
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
            spawn_gate: Mutex::new(()),
            emma: Mutex::new(None),
            external_server: Mutex::new(None),
            app_handle: std::sync::OnceLock::new(),
        }
    }

    /// Spawn Emma's solo agent if not already running. Idempotent. Logs +
    /// returns Err on spawn failure (e.g., missing `claude` CLI), does NOT
    /// crash startup.
    pub async fn ensure_emma_started(&self) -> Result<()> {
        let mut emma = self.emma.lock().await;
        // Healthy + running → done. A stale handle (supervisor terminated on a
        // permanent error / exhausted retries) is dropped so we re-spawn.
        if let Some(handle) = emma.as_ref() {
            if !handle.is_stale() {
                return Ok(());
            }
            if let Some(mut old) = emma.take() {
                old.agent.kill();
                tracing::info!("evicted stale Emma handle; re-spawning");
            }
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
        // Fast path: already running AND healthy. A handle whose supervisor has
        // terminated (permanent API error / exhausted retry budget) lingers in
        // the map but is stale — fall through to evict + re-spawn so the
        // session recovers on the next interaction without an app restart.
        if let Some(handle) = self.sessions.lock().await.get(session_id) {
            if !handle.is_stale() {
                return Ok(());
            }
        }
        // Slow path: take the spawn gate so concurrent callers serialize, then
        // re-check under the gate — a racing call may have spawned while we
        // waited. Without this double-check two callers both pass the fast
        // check and spawn duplicate duos (one gets orphaned).
        let _gate = self.spawn_gate.lock().await;
        {
            let mut sessions = self.sessions.lock().await;
            if let Some(handle) = sessions.get(session_id) {
                if !handle.is_stale() {
                    return Ok(());
                }
                // Evict the stale (crashed) handle before re-spawning. Killing
                // already-dead agents is a no-op.
                if let Some(mut stale) = sessions.remove(session_id) {
                    stale.brian.kill();
                    stale.rain.kill();
                    tracing::info!(session_id, "evicted stale session handle; re-spawning");
                }
            }
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

    /// Force-restart a session's duo: evict the live handle (killing both
    /// agents) and re-spawn from the CURRENT config. Agent overrides + the
    /// inherited Claude config are read at spawn, so this is how a running
    /// session picks up a Claude-config change made in Settings. Each agent
    /// resumes its prior claude-code conversation via `--resume`, so context
    /// is preserved. Unlike `close_session`, the session row stays open.
    pub async fn restart_session(&self, session_id: &str) -> Result<()> {
        {
            let mut sessions = self.sessions.lock().await;
            if let Some(mut handle) = sessions.remove(session_id) {
                handle.brian.kill();
                handle.rain.kill();
                tracing::info!(session_id, "restarting session to apply config change");
            }
        }
        // Handle now absent → ensure_session_started re-spawns from scratch
        // (re-running build_command, which re-reads claude-overrides.json + the
        // per-agent mcp-config).
        self.ensure_session_started(session_id).await
    }

    pub async fn close_session(&self, id: &str, archive: bool) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        if let Some(mut handle) = sessions.remove(id) {
            handle.brian.kill();
            handle.rain.kill();
        }
        self.storage.close_session(id, archive).await?;
        // The session's pending tray items are moot now the agents are gone —
        // withdraw them so a closed session doesn't leave dead `pending` rows.
        if let Err(e) = self.storage.withdraw_pending_tray_for_session(id).await {
            tracing::warn!(?e, session_id = %id, "withdraw_pending_tray_for_session failed");
        }
        // Drop the canonical session-policy snapshot. It does not carry into
        // the next session this user opens — that session re-seeds from the
        // current general+project blueprints at spawn.
        if let Err(e) = self.bridge.cleanup_session_policy(id).await {
            tracing::warn!(?e, session_id = %id, "cleanup_session_policy failed");
        }
        // Tell the UI the session is closed so it can navigate away from the
        // (now-closed) session view + refresh its session lists.
        self.bridge.notify_session_closed(id.to_string());
        Ok(())
    }

    pub async fn list_active_sessions(&self) -> Result<Vec<Session>> {
        self.storage.list_active_sessions().await
    }

    /// Clear the awaiting-user halt for a live session: flip the handle's
    /// atomic AND the bridge's mirror (kept in sync — both point at the same
    /// `Arc<AtomicBool>`, but the bridge copy is what survives if the
    /// `SessionHandle` is dropped). Does NOT touch pending-halt rows; callers
    /// that also answer those call `clear_pending_halts` separately.
    async fn clear_awaiting(&self, handle: &SessionHandle, session_id: &str) {
        handle
            .awaiting
            .store(false, std::sync::atomic::Ordering::Release);
        self.bridge.clear_session_awaiting(session_id).await;
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
        // duo pumps see chunks again.
        self.clear_awaiting(handle, session_id).await;
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

        self.clear_awaiting(handle, session_id).await;
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
        handle
            .send_to_both(OutgoingUserMessage::text(notice))
            .await;
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
                self.clear_awaiting(handle, &session_id).await;
                let phase = handle.ipav.lock().await.current_phase;
                let wire = with_phase_envelope(phase, &body);
                handle
                    .send_to_both(crate::agents::OutgoingUserMessage::text(wire))
                    .await;
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
