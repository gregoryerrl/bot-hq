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
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use tauri::Emitter;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

/// Wire-only directive prepended to the first user message after a cancel
/// (interrupt redesign, Batch 3.1) so the `--resume`d agent reconciles any
/// partial state the force-interrupted turn left behind before acting.
const RECONCILE_DIRECTIVE: &str = "[System: your previous turn was force-interrupted (Stop). \
     Before acting on the message below, run `git status` to check the workspace and clear any \
     stale lock files or partial writes the interrupted operation may have left (e.g. a leftover \
     .git/index.lock).]";

/// How long to wait for an interrupted agent to honor a `control_request` and go
/// idle before escalating to a SIGKILL. The interrupt keeps the process alive
/// (warm cache, no respawn); the SIGKILL fallback covers a dropped interrupt or a
/// wedged agent.
const INTERRUPT_ESCALATION: std::time::Duration = std::time::Duration::from_secs(2);

/// Outcome of initiating a cancel (`AppState::cancel_session_turn`).
pub enum CancelOutcome {
    /// The session wasn't live (no-op). Nothing more to do.
    Done,
    /// HANDS was mid an atomic op (git commit/push/migration); the cancel is
    /// DEFERRED. The caller polls this flag lock-free until it clears (or a
    /// timeout), THEN runs `interrupt_then_escalate` so the working tree isn't left
    /// half-written. `Cancelling` is already set, so the UI shows "Cancelling…"
    /// for the whole window.
    Deferred(Arc<std::sync::atomic::AtomicBool>),
    /// The common path: the caller spawns a detached task that runs
    /// `interrupt_then_escalate` — a `control_request` interrupt (abort the turn,
    /// keep the process: warm cache, no respawn) with a ~2s SIGKILL fallback.
    /// `Cancelling` is already set.
    Interrupting,
}

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
    /// Sessions awaiting a post-cancel reconciliation nudge (interrupt redesign,
    /// Batch 3.1). `cancel_session_turn` inserts; the next `broadcast` consumes
    /// it, prepending a wire-only directive so the resumed agent verifies the
    /// workspace (lock files / partial writes) before acting on the new message.
    pending_reconcile: Mutex<HashSet<String>>,
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
            pending_reconcile: Mutex::new(HashSet::new()),
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

    /// Hard-cancel a session's in-flight turn — the Stop button (interrupt
    /// redesign, Batch 3 + 3.1 Part 1). Sets `Cancelling` (the UI shows
    /// "Cancelling…" + keeps the input locked for the whole kill window), then
    /// decides:
    /// - **immediate** kill of both agents' current incarnation (today's path)
    ///   when HANDS is not mid an atomic op, returning [`CancelOutcome::Done`];
    /// - **deferred** kill ([`CancelOutcome::Deferred`]) when HANDS is mid a
    ///   `git commit`/`git push`/migration — the caller polls the returned flag
    ///   and calls [`cancel_kill_now`](Self::cancel_kill_now) once it clears, so
    ///   the working tree is never left half-written.
    ///
    /// On a kill, each supervisor tears down, its pump's event channel closes,
    /// and the pump's post-loop activity clear flips that agent to idle — so once
    /// both clear, the session returns to `Idle` and the chat input unlocks. The
    /// handle is left in the map but goes stale (`input_tx` closed); the next
    /// user message respawns each agent via `--resume`, restoring prior context.
    /// No-op (`Done`) if the session isn't live.
    pub async fn cancel_session_turn(&self, session_id: &str) -> Result<CancelOutcome> {
        let deferred = {
            let mut sessions = self.sessions.lock().await;
            let Some(handle) = sessions.get_mut(session_id) else {
                return Ok(CancelOutcome::Done); // not live → no-op
            };
            // Mark Cancelling FIRST → the UI shows "Cancelling…" + keeps the
            // input locked for the whole kill window (immediate or deferred). It
            // auto-clears to Idle in the tracker once both pumps go idle.
            handle.activity.set_cancelling(true);
            // HANDS mid an atomic op (git commit/push/migration)? Defer: hand the
            // shared flag to the caller to poll, and do NOT kill yet.
            handle
                .in_atomic_tool
                .load(Ordering::Acquire)
                .then(|| Arc::clone(&handle.in_atomic_tool))
        };
        match deferred {
            Some(flag) => {
                tracing::info!(session_id, "cancel: deferring interrupt — mid atomic tool");
                Ok(CancelOutcome::Deferred(flag))
            }
            None => Ok(CancelOutcome::Interrupting),
        }
    }

    /// The interrupt half of a cancel: send a `control_request` interrupt to both
    /// live agents (abort the in-flight turn, keep the process — warm cache, no
    /// `--resume` respawn), wait up to `INTERRUPT_ESCALATION` for them to go idle,
    /// and SIGKILL-escalate via [`cancel_kill_now`](Self::cancel_kill_now) only if
    /// they don't honor it in time. Queues the post-cancel reconciliation nudge in
    /// EITHER outcome. Driven by a detached task from the Tauri command — the
    /// non-atomic path immediately, the atomic-deferred path once the op completes.
    pub async fn interrupt_then_escalate(&self, session_id: &str) {
        let activity = {
            let sessions = self.sessions.lock().await;
            let Some(handle) = sessions.get(session_id) else {
                return; // session gone → nothing to cancel
            };
            // EYES (Rain) first (review-only, side-effect-safe), then HANDS —
            // mirrors cancel_kill_now. `interrupt` is best-effort (&self try_send);
            // a full/closed control channel returns false and the idle-watch below
            // times out into the SIGKILL fallback.
            if let Some(rain) = handle.rain.as_ref() {
                rain.interrupt("cancel");
            }
            handle.brian.interrupt("cancel");
            Arc::clone(&handle.activity)
        };

        let deadline = tokio::time::Instant::now() + INTERRUPT_ESCALATION;
        if activity.await_both_idle(deadline).await {
            // Interrupt honored: process alive at a turn boundary (Cancelling has
            // auto-cleared to Idle). Queue the nudge so the next user message
            // reconciles the workspace before acting.
            self.pending_reconcile
                .lock()
                .await
                .insert(session_id.to_string());
            tracing::info!(
                session_id,
                "cancel: interrupt honored — process kept alive (warm cache)"
            );
        } else {
            tracing::warn!(
                session_id,
                secs = INTERRUPT_ESCALATION.as_secs(),
                "cancel: interrupt not honored in time — SIGKILL fallback"
            );
            // cancel_kill_now kills the process group AND queues the nudge.
            self.cancel_kill_now(session_id).await;
        }
    }

    /// The kill half of a cancel: tear down both agents NOW and queue the
    /// post-cancel reconciliation nudge. The SIGKILL fallback for
    /// [`interrupt_then_escalate`](Self::interrupt_then_escalate) when an agent
    /// doesn't honor the interrupt in time. Re-acquires `sessions`; a no-op if the
    /// session is already gone.
    pub async fn cancel_kill_now(&self, session_id: &str) {
        let killed = {
            let mut sessions = self.sessions.lock().await;
            if let Some(handle) = sessions.get_mut(session_id) {
                // EYES (Rain) is review-only → side-effect-safe; cancel it first.
                // HANDS (Brian) may be mid-tool, so kill it last.
                if let Some(rain) = handle.rain.as_mut() {
                    rain.kill();
                }
                handle.brian.kill();
                true
            } else {
                false
            }
        };
        if killed {
            // Queue a post-cancel reconciliation nudge for the next user message
            // (consumed in `broadcast`) — separate lock, acquired after releasing
            // `sessions`, so there's no nested lock ordering to deadlock on.
            self.pending_reconcile
                .lock()
                .await
                .insert(session_id.to_string());
            tracing::info!(session_id, "cancel: killed in-flight turn(s)");
        }
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
        // Drop any queued post-cancel reconciliation flag (a session cancelled
        // then closed without a follow-up message would otherwise linger).
        self.pending_reconcile.lock().await.remove(id);
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
        // Reset the L2 volley hard-cap: the user just spoke, so the consecutive
        // peer-forward counter (`duo::flush_buffer`) starts fresh. Deliberately
        // here and not in `clear_awaiting` — `advance_phase` calls that too, and
        // a phase self-advance is not a user message.
        handle
            .user_silent_forwards
            .store(0, std::sync::atomic::Ordering::Release);
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
        // Consume any queued post-cancel reconciliation directive for this
        // session (set by cancel_session_turn) — prepended wire-only so the
        // resumed agent reconciles partial state before acting on this message.
        let reconcile = self
            .pending_reconcile
            .lock()
            .await
            .remove(session_id)
            .then_some(RECONCILE_DIRECTIVE);
        // Human preemption (the always-typeable unblock's spine): the user's
        // message must take effect NOW, not queue behind a turn-in-flight (or an
        // idle agent-to-agent volley). Fire a warm control_request interrupt at
        // both agents BEFORE delivering. Verified harmless when idle
        // (control_response{success}, process survives, next message still
        // processed), and it aborts the in-flight turn when busy — so we don't
        // gate on the flaky activity `busy` signal. The pump's biased control
        // channel writes this ahead of the message on stdin, so each agent aborts
        // then reads the new message. No SIGKILL escalation (unlike cancel) — the
        // message IS the next work, and the process stays warm (no --resume).
        handle.brian.interrupt("user-preempt");
        if let Some(rain) = handle.rain.as_ref() {
            rain.interrupt("user-preempt");
        }
        let id = broadcast_user_message(
            &self.storage,
            session_id,
            text,
            phase,
            reconcile,
            &handle.brian.input_tx,
            handle.rain.as_ref().map(|r| &r.input_tx),
        )
        .await?;
        // The user's message was dispatched to both agents → they're now busy
        // (the duo's turn-start). The awaiting flag was cleared just above, so
        // this recompute moves the session AwaitingUser/Idle → Busy.
        handle.activity.set_busy(Author::Brian, true);
        if handle.rain.is_some() {
            handle.activity.set_busy(Author::Rain, true);
        }
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
