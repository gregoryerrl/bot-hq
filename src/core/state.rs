//! `AppState`: top-level handle the UI layer holds.

use crate::agents::OutgoingUserMessage;
use crate::core::broadcast::broadcast_user_message;
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
        let id = broadcast_user_message(
            &self.storage,
            session_id,
            text,
            &handle.brian.input_tx,
            &handle.rain.input_tx,
        )
        .await?;
        self.bridge
            .notify_message_persisted(session_id.to_string(), id);
        Ok(())
    }

    /// Set IPAV phase + emit a synthetic user "phase advanced to X" message so
    /// both agents see the transition naturally.
    pub async fn advance_phase(&self, session_id: &str, target: IpavPhase) -> Result<()> {
        let sessions = self.sessions.lock().await;
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("no live session {session_id}"))?;

        let ts = chrono::Utc::now().to_rfc3339();
        handle.ipav.lock().await.advance(target, ts);
        let notice = format!("phase advanced to {}", target.name());

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
        self.bridge.resolve_choice(choice_id, picked).await
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
