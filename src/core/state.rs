//! `AppState`: top-level handle the UI layer holds.

use crate::agents::OutgoingUserMessage;
use crate::core::broadcast::{broadcast_user_message, with_phase_envelope};
use crate::core::ipav::IpavPhase;
use crate::core::session::{
    open_session, spawn_existing_session, OpenSessionRequest, SessionHandle,
};
use crate::paths::Paths;
use crate::signaling::{ExternalServer, SignalingBridge, SignalingEvent, SignalingServer};
use crate::storage::{Author, MessageKind, Session, Storage};
use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use tauri::Emitter;
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
    /// External MCP server handle. None when disabled or port-busy at startup;
    /// the binary stays usable in that case (internal MCP keeps working).
    pub external_server: Mutex<Option<ExternalServer>>,
    /// Populated from Tauri's `setup()` once the AppHandle exists. The
    /// external MCP starts BEFORE Tauri setup (see main.rs ordering), so
    /// any MCP tool that needs the webview (screenshot, click, scroll, etc.)
    /// has to wait for this to be filled. `OnceCell` because it's write-once
    /// at startup; no contention.
    pub app_handle: std::sync::OnceLock<tauri::AppHandle>,
    /// Populated from Tauri's `setup()` once the filesystem watcher is up. The
    /// session spawn/close paths register + unregister working repos here so each
    /// session's Apply-tab diff updates live. `OnceLock` — write-once at startup,
    /// like `app_handle`.
    pub fs_watcher: std::sync::OnceLock<crate::tauri_events::WatcherHandle>,
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
            external_server: Mutex::new(None),
            app_handle: std::sync::OnceLock::new(),
            fs_watcher: std::sync::OnceLock::new(),
        }
    }

    pub async fn open_session(
        &self,
        title: impl Into<String>,
        working_repo_path: Option<std::path::PathBuf>,
    ) -> Result<String> {
        // External-driver entry: models from agent config, solo/duo from the
        // user's `rain_disabled_default` setting (no create dialog on this
        // path). The UI create path persists per-agent model + Rain toggle on
        // the row, then spawns via spawn_existing_session.
        let mut req = OpenSessionRequest::duo(title, working_repo_path);
        req.rain_enabled = self.storage.default_rain_enabled().await;
        let handle = open_session(
            req,
            &self.paths,
            self.storage.clone(),
            Arc::clone(&self.bridge),
            self.signaling_addr,
        )
        .await?;
        let id = handle.id.clone();
        self.watch_session_repo(&id, &handle);
        self.sessions.lock().await.insert(id.clone(), handle);
        // Tell the frontend a session was created. This covers the external
        // driver path (UI create paths already self-invalidate list_sessions);
        // no-op until the AppHandle is set in setup.
        if let Some(app) = self.app_handle.get() {
            let _ = app.emit(
                crate::tauri_events::types::SESSION_CREATED,
                serde_json::json!({ "session_id": id }),
            );
        }
        Ok(id)
    }

    /// Spawn subprocesses for an existing session row if not already running.
    /// Idempotent — safe to call repeatedly.
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
                    if let Some(rain) = stale.rain.as_mut() {
                        rain.kill();
                    }
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
        self.watch_session_repo(session_id, &handle);
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
                if let Some(rain) = handle.rain.as_mut() {
                    rain.kill();
                }
                tracing::info!(session_id, "restarting session to apply config change");
            }
        }
        // Handle now absent → ensure_session_started re-spawns from scratch
        // (re-running build_command, which re-reads claude-overrides.json + the
        // per-agent mcp-config).
        self.ensure_session_started(session_id).await
    }

    /// Register a session's working repo with the filesystem watcher so its
    /// Apply-tab diff updates live on file changes. No-op if the watcher isn't up
    /// yet or the session has no working repo.
    fn watch_session_repo(&self, id: &str, handle: &SessionHandle) {
        if let (Some(watcher), Some(repo)) =
            (self.fs_watcher.get(), handle.working_repo_path.as_ref())
        {
            watcher.add_repo(id, repo.clone());
        }
    }

    pub async fn close_session(&self, id: &str, archive: bool) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        if let Some(mut handle) = sessions.remove(id) {
            handle.brian.kill();
            if let Some(rain) = handle.rain.as_mut() {
                rain.kill();
            }
        }
        // Stop live-watching this session's working repo.
        if let Some(watcher) = self.fs_watcher.get() {
            watcher.remove_repo(id);
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
        // Drop the bridge's in-memory per-session state (project map + awaiting
        // flag) so closed sessions don't leak map entries for the process life.
        self.bridge.unregister_session(id).await;
        // Worktree-isolated session: remove its worktree if (and only if) it
        // is clean. Never forced — a dirty worktree outlives the session so
        // uncommitted work is recoverable; the session branch always survives.
        if let Ok(Some(row)) = self.storage.get_session(id).await {
            if let (Some(base), Some(wt)) = (row.base_repo_path, row.working_repo_path) {
                let sid = id.to_string();
                let outcome = tokio::task::spawn_blocking(move || {
                    crate::core::worktree::remove_worktree_if_clean(
                        std::path::Path::new(&base),
                        std::path::Path::new(&wt),
                    )
                })
                .await;
                use crate::core::worktree::RemoveOutcome;
                match outcome {
                    Ok(RemoveOutcome::Removed) => {
                        tracing::info!(session_id = %sid, "session worktree removed (clean)");
                    }
                    Ok(RemoveOutcome::Kept(reason)) => {
                        tracing::warn!(session_id = %sid, %reason, "session worktree KEPT (dirty) — recover or remove it manually");
                    }
                    Ok(RemoveOutcome::Gone) => {
                        tracing::debug!(session_id = %sid, "session worktree already gone");
                    }
                    Err(e) => {
                        tracing::warn!(?e, session_id = %sid, "worktree removal task failed");
                    }
                }
            }
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
        // Auto-heal: if the duo went stale (e.g. an agent's stdin pump died,
        // closing the public input channel — a now-deaf agent that would silently
        // drop this message), evict + respawn it before delivering so the user's
        // message isn't lost. `ensure_session_started` is a no-op on a healthy
        // session.
        let stale = {
            let sessions = self.sessions.lock().await;
            sessions.get(session_id).is_some_and(|h| h.is_stale())
        };
        if stale {
            tracing::info!(
                session_id,
                "session stale on broadcast; respawning before delivery"
            );
            self.ensure_session_started(session_id).await?;
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
        // until the user actually picks an option. Emit HaltsCleared only when
        // rows actually flipped, so the UI refetches the tray + clears the
        // "needs input" bell (a DB-only clear leaves list_pending_tray stale).
        // The guard matters: broadcast() runs on every user message.
        match self.storage.clear_pending_halts(session_id).await {
            Ok(cleared) if cleared > 0 => {
                self.bridge.notify_halts_cleared(session_id.to_string());
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(?e, session_id, "clear_pending_halts failed"),
        }
        let phase = handle.ipav.lock().await.current_phase;
        let id = broadcast_user_message(
            &self.storage,
            session_id,
            text,
            phase,
            &handle.brian.input_tx,
            handle.rain.as_ref().map(|r| &r.input_tx),
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
        // A2 (adherence): remember the phase we're leaving, to detect Plan→Apply.
        let prev_phase = handle.ipav.lock().await.current_phase;

        self.clear_awaiting(handle, session_id).await;
        match self.storage.clear_pending_halts(session_id).await {
            Ok(cleared) if cleared > 0 => {
                self.bridge.notify_halts_cleared(session_id.to_string());
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(?e, session_id, "clear_pending_halts (advance_phase) failed");
            }
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
        handle.send_to_both(OutgoingUserMessage::text(notice)).await;

        // A2 (adherence): the peer-ack the prompts don't mechanically enforce.
        // On the Plan→Apply boundary in a duo session, remind Brian (HANDS) to
        // confirm Rain's plan review before mutating. Brian-only; no-op solo;
        // gated by the adherence_nudges setting.
        if Self::should_peer_ack_nudge(prev_phase, target, handle.rain.is_some())
            && self.storage.adherence_nudges_enabled().await
        {
            let _ = handle
                .brian
                .input_tx
                .send(OutgoingUserMessage::text(
                    "🔔 Entering Apply. Before you mutate: confirm Rain reviewed the plan — \
                     pull session_doc_search(phase=\"plan\") and check her pushback landed. If \
                     she hasn't reviewed yet, wait for it (mark_awaiting_user) rather than \
                     applying unreviewed."
                        .to_string(),
                ))
                .await;
        }
        Ok(())
    }

    /// A2 (adherence): whether the Plan→Apply boundary in a duo session warrants
    /// the peer-ack nudge to Brian. Pure for testing; the caller additionally
    /// AND-gates the `adherence_nudges` setting.
    fn should_peer_ack_nudge(prev: IpavPhase, target: IpavPhase, has_rain: bool) -> bool {
        has_rain && prev == IpavPhase::Plan && target == IpavPhase::Apply
    }

    pub async fn resolve_choice(
        &self,
        choice_id: &str,
        picked: String,
        confirm_stale: bool,
    ) -> Result<crate::signaling::ResolveOutcome> {
        use crate::signaling::ResolveOutcome;
        let outcome = self
            .bridge
            .resolve_choice_confirmable(choice_id, picked, confirm_stale)
            .await?;
        // Only the timed-out fallback needs us to wake the duo subprocess. The
        // OOB message is already in storage (bridge wrote it). To actually wake
        // the duo so they read + act on it, also: (1) clear the awaiting-user
        // halt so the duo pump resumes peer-forwarding, (2) push the body
        // through both agents' input_tx so their stdin receives a wake message.
        // We deliberately do NOT call broadcast_user_message (which re-inserts)
        // — the storage row already exists. Delivered + StaleGateNeedsConfirm
        // need no wake (the agent is live, or nothing ran).
        if let ResolveOutcome::AgentReceiverDroppedFellBack { session_id, body } = &outcome {
            let sessions = self.sessions.lock().await;
            if let Some(handle) = sessions.get(session_id) {
                self.clear_awaiting(handle, session_id).await;
                let phase = handle.ipav.lock().await.current_phase;
                let wire = with_phase_envelope(phase, body);
                handle
                    .send_to_both(crate::agents::OutgoingUserMessage::text(wire))
                    .await;
            }
            // else: session closed in the gap between resolve and wake — the OOB
            // message persists in storage, so a future reopen still sees it.
        }
        Ok(outcome)
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
    use super::*;

    // Live session tests require RUN_LIVE_TESTS=1 (subprocesses spawn).
    // We unit-test the static pieces here.

    #[test]
    fn smoke() {
        // Module compiles.
    }

    #[test]
    fn peer_ack_nudge_only_on_plan_to_apply_duo() {
        // A2: fires only when crossing Plan→Apply in a duo session.
        assert!(AppState::should_peer_ack_nudge(
            IpavPhase::Plan,
            IpavPhase::Apply,
            true
        ));
        // Solo (no Rain) → no peer to ack.
        assert!(!AppState::should_peer_ack_nudge(
            IpavPhase::Plan,
            IpavPhase::Apply,
            false
        ));
        // Other transitions don't nudge.
        assert!(!AppState::should_peer_ack_nudge(
            IpavPhase::Investigate,
            IpavPhase::Plan,
            true
        ));
        // Re-entering Apply from Verify isn't the plan-review boundary.
        assert!(!AppState::should_peer_ack_nudge(
            IpavPhase::Verify,
            IpavPhase::Apply,
            true
        ));
    }
}
