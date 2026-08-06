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

/// The host-authored notice `resume_session` broadcasts when the user clicks
/// Resume on a paused session. Travels the normal `broadcast` path, so the
/// reconcile directive (if the pause came from a Stop) is prepended wire-only,
/// and any held peer-forwards / OOB answers flush in behind it.
const RESUME_NOTICE: &str = "▶ Resumed. Continue exactly where you left off — \
     finish your in-flight task. Any peer messages or question answers held \
     during the pause follow this notice; fold them in before proceeding.";

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

/// Outcome of a cancel's interrupt→SIGKILL escalation, decided AFTER the
/// interrupt window. Pure (see [`AppState::escalation_outcome`]) so the
/// honored > superseded > sigkill precedence is unit-tested without a live duo.
#[derive(Debug, PartialEq, Eq)]
enum EscalationOutcome {
    /// Both agents went idle in time — the interrupt was honored, process kept.
    InterruptHonored,
    /// A user message arrived during the window — it already aborted the stuck
    /// turn, so skip the SIGKILL (don't kill the user's fresh turn + warm cache).
    SupersededByUser,
    /// Interrupt not honored and not superseded — force-kill as the fallback.
    Sigkill,
}

/// One step of `broadcast`'s auto-heal respawn loop. Pure (see
/// [`AppState::broadcast_deliver_step`]) so the bounded-retry + branch logic is
/// unit-tested without a live session.
#[derive(Debug, PartialEq, Eq)]
enum DeliverStep {
    /// Healthy (or absent — errors at the `get` after the loop) → deliver under
    /// the current lock hold.
    Deliver,
    /// Present but stale and under the retry cap → respawn, then re-check.
    Respawn,
    /// Present but stale and the retry cap is hit → deliver best-effort (the send
    /// logs on failure) rather than loop forever.
    GiveUpBestEffort,
}

/// What to do with an out-of-band tray answer once the bridge has persisted it.
/// Pure (see [`AppState::tray_wake_step`]) so the paused-vs-preempt branch is
/// unit-tested without a live session.
#[derive(Debug, PartialEq, Eq)]
enum TrayWakeStep {
    /// The duo is paused: stash the wire for the next `broadcast` (Send /
    /// Resume) instead of delivering. No interrupt — nothing is being
    /// delivered, and a tray answer must not release a deliberate pause.
    StashForResume,
    /// Live duo: interrupt the in-flight turn, THEN deliver the answer.
    PreemptAndDeliver,
}

/// Max respawn attempts in `broadcast`'s auto-heal loop before delivering
/// best-effort. Bounds a pathological respawn→stale→respawn cycle.
const BROADCAST_MAX_RESPAWNS: u32 = 3;

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
    /// Out-of-band wakes (answered tray questions) that arrived while the
    /// session was PAUSED — an answer must not restart a paused duo, but it
    /// must not be lost either. `resolve_choice` stashes the wire body here
    /// instead of waking stdin; the next `broadcast` (a user Send / Resume)
    /// drains it to both agents after the user's message.
    pending_paused_wakes: Mutex<std::collections::HashMap<String, Vec<String>>>,
    /// Per-session PTY terminals (Terminal subtab). Lazily spawned on first
    /// `terminal_open`, killed on `close_session`. Shared as an `Arc` so the
    /// signaling bridge's MCP handlers can reach the same PTYs.
    pub terminals: Arc<crate::core::TerminalRegistry>,
}

impl AppState {
    pub async fn new(paths: Paths, storage: Storage, server: SignalingServer) -> Self {
        let bridge = Arc::clone(&server.bridge);
        let addr = server.local_addr;
        // Sweep native conversations orphaned by a force-quit or a session
        // deleted without a clean close — `close_session` clears its own, but
        // nothing else ever did, so the directory accumulated one full
        // transcript per unclean end. Skipped entirely (never fail-open into
        // deleting live histories) if the session list can't be read.
        match storage.list_active_sessions().await {
            Ok(sessions) => {
                let keep: std::collections::HashSet<String> =
                    sessions.into_iter().map(|s| s.id).collect();
                let removed =
                    crate::agents::native::history::sweep_orphans(&paths.data_dir, &keep);
                if removed > 0 {
                    tracing::info!(removed, "swept orphaned native-history conversations");
                }
            }
            Err(e) => {
                tracing::warn!(?e, "could not list sessions; skipping native-history sweep")
            }
        }
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
            pending_paused_wakes: Mutex::new(std::collections::HashMap::new()),
            terminals: Arc::new(crate::core::TerminalRegistry::new()),
        }
    }

    /// Open a session from the external driver.
    ///
    /// `brian_model_id` / `rain_model_id` are saved-model ids; `None` falls back
    /// to the per-agent config, which is the historical behaviour. Pass them when
    /// the caller wants a SPECIFIC model for this session; since 0038 the
    /// per-agent fallback carries `native` / `context_window` itself, so omitting
    /// them still reaches the native loop whenever the Agents tab assigned a
    /// native model. Solo/duo still comes from the user's `rain_disabled_default`
    /// setting; there is no create dialog here.
    pub async fn open_session(
        &self,
        title: impl Into<String>,
        working_repo_path: Option<std::path::PathBuf>,
        brian_model_id: Option<String>,
        rain_model_id: Option<String>,
    ) -> Result<String> {
        let mut req = OpenSessionRequest::duo(title, working_repo_path);
        req.rain_enabled = self.storage.default_rain_enabled().await;
        req.brian_model_id = brian_model_id;
        req.rain_model_id = rain_model_id;
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
        // Seed the roster before spawning. 0044's backfill was a one-shot over
        // the sessions that existed when it applied, so anything created since
        // starts with an empty roster and every message it writes resolves
        // `participant_id` to NULL. This is the one choke point every creation
        // path funnels through, and it's idempotent — two no-op inserts on a
        // healthy respawn. A failure must NOT block the spawn: `author` still
        // carries attribution, so a missing roster degrades the channel rather
        // than the session.
        if let Err(e) = self.storage.ensure_session_roster(session_id).await {
            tracing::warn!(session_id, error = ?e, "seeding session roster failed");
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
            // input locked for the whole kill window (immediate or deferred).
            // Then latch the pause (that order — see set_paused's ORDERING
            // note): once both pumps go idle the tracker auto-clears
            // cancelling and the session lands in Paused, not Idle — input
            // enabled, duo held until the user steers, resumes, or closes.
            handle.activity.set_cancelling(true);
            handle.activity.set_paused(true);
            // A fresh cancel begins un-superseded; `broadcast` flips this true if a
            // user message arrives during the escalation window (then the SIGKILL
            // is skipped). Reset here so a prior supersede can't suppress THIS kill.
            handle
                .cancel_superseded
                .store(false, Ordering::Release);
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

    /// Resume a paused session (the Paused bar's Resume button). Releases the
    /// latch by broadcasting a host-authored resume notice through the normal
    /// [`broadcast`](Self::broadcast) path — which clears `paused`, consumes
    /// any pending post-cancel reconciliation directive, delivers OOB wakes
    /// held during the pause, and flushes the router's held forwards behind
    /// the notice. Auto-heals a SIGKILLed (stale) duo via broadcast's respawn
    /// loop. No-op when the session isn't live or isn't paused (stale click).
    pub async fn resume_session(&self, session_id: &str) -> Result<()> {
        {
            let sessions = self.sessions.lock().await;
            let Some(handle) = sessions.get(session_id) else {
                return Ok(()); // not live → nothing to resume
            };
            if !handle.activity.is_paused() {
                return Ok(()); // not paused → stale click; don't nudge the duo
            }
        }
        self.broadcast(session_id, RESUME_NOTICE).await
    }

    /// The interrupt half of a cancel: send a `control_request` interrupt to both
    /// live agents (abort the in-flight turn, keep the process — warm cache, no
    /// `--resume` respawn), wait up to `INTERRUPT_ESCALATION` for them to go idle,
    /// and SIGKILL-escalate via [`cancel_kill_now`](Self::cancel_kill_now) only if
    /// they don't honor it in time. Queues the post-cancel reconciliation nudge in
    /// EITHER outcome. Driven by a detached task from the Tauri command — the
    /// non-atomic path immediately, the atomic-deferred path once the op completes.
    pub async fn interrupt_then_escalate(
        &self,
        session_id: &str,
        pressed_at: &str,
        deferred_ms: u64,
        deferral_capped: bool,
    ) {
        let (activity, cancel_superseded, brian_queued, rain_queued) = {
            let sessions = self.sessions.lock().await;
            let Some(handle) = sessions.get(session_id) else {
                return; // session gone → nothing to cancel
            };
            // EYES (Rain) first (review-only, side-effect-safe), then HANDS —
            // mirrors cancel_kill_now. `interrupt` is best-effort (&self try_send);
            // a full/closed control channel returns false and the idle-watch below
            // times out into the SIGKILL fallback.
            //
            // KEEP the booleans. They used to be discarded, which made a DROPPED
            // interrupt indistinguishable from one the agent received and ignored
            // — two different bugs with the same symptom, and no way to tell them
            // apart after the fact.
            let rain_queued = handle.rain.as_ref().map(|rain| rain.interrupt("cancel"));
            let brian_queued = handle.brian.interrupt("cancel");
            if !brian_queued {
                tracing::warn!(session_id, "cancel: HANDS interrupt was NOT queued");
            }
            if rain_queued == Some(false) {
                tracing::warn!(session_id, "cancel: EYES interrupt was NOT queued");
            }
            (
                Arc::clone(&handle.activity),
                Arc::clone(&handle.cancel_superseded),
                brian_queued,
                rain_queued,
            )
        };

        let deadline = tokio::time::Instant::now() + INTERRUPT_ESCALATION;
        let both_idle = activity.await_both_idle(deadline).await;
        let superseded = cancel_superseded.load(Ordering::Acquire);
        let idled = activity.idled_since_cancel();
        let outcome = Self::escalation_outcome(both_idle, superseded, idled);

        // Record BEFORE acting: a SIGKILL tears the session down, and telemetry
        // written after that is telemetry you don't get for the case you most
        // need it. Best-effort — losing a row must never block a cancel.
        let record = crate::storage::CancelEventRecord {
            session_id: session_id.to_string(),
            pressed_at: pressed_at.to_string(),
            settled_at: crate::storage::now_utc(),
            deferred_ms: deferred_ms as i64,
            deferral_capped,
            brian_interrupt_queued: Some(brian_queued),
            rain_interrupt_queued: rain_queued,
            both_idle,
            cancel_superseded: superseded,
            idled_since_cancel: idled,
            outcome: match outcome {
                EscalationOutcome::InterruptHonored => "honored",
                EscalationOutcome::SupersededByUser => "superseded",
                EscalationOutcome::Sigkill => "sigkill",
            }
            .to_string(),
        };
        if let Err(e) = self.storage.insert_cancel_event(&record).await {
            tracing::warn!(?e, session_id, "cancel: could not record cancel event");
        }

        match outcome {
            EscalationOutcome::InterruptHonored => {
                // Process alive at a turn boundary (Cancelling auto-cleared to Idle).
                // Queue the nudge so the next user message reconciles the workspace.
                self.pending_reconcile
                    .lock()
                    .await
                    .insert(session_id.to_string());
                tracing::info!(
                    session_id,
                    "cancel: interrupt honored — process kept alive (warm cache)"
                );
            }
            EscalationOutcome::SupersededByUser => {
                // The user's message (with its own preempt interrupt in `broadcast`)
                // already aborted the stuck turn — a SIGKILL would needlessly kill
                // the fresh turn + warm cache. Skip it; clear any lingering
                // Cancelling AND the pause latch (the user already steered —
                // landing in Paused after their message would re-halt the duo
                // they just woke). `broadcast` also clears it; this covers the
                // escalation racing ahead of that clear.
                activity.set_cancelling(false);
                activity.set_paused(false);
                tracing::info!(
                    session_id,
                    "cancel: superseded by a user message — skipping SIGKILL fallback"
                );
            }
            EscalationOutcome::Sigkill => {
                tracing::warn!(
                    session_id,
                    secs = INTERRUPT_ESCALATION.as_secs(),
                    "cancel: interrupt not honored in time — SIGKILL fallback"
                );
                // cancel_kill_now kills the process group AND queues the nudge.
                self.cancel_kill_now(session_id).await;
            }
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
        // Reap the session's PTY terminal alongside the agent subprocesses.
        self.terminals.kill_and_remove(id).await;
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
        // Drop any persisted native-agent conversations. These exist so a native
        // agent survives an app restart (a CLI agent comes back via `--resume`),
        // which is meaningless once the session is closed — and without this the
        // directory accumulates one file per native session forever, each holding
        // the full transcript.
        for agent in crate::agents::AgentRole::NAMES {
            crate::agents::native::history::clear(&crate::agents::native::history::history_path(
                &self.paths.data_dir,
                id,
                agent,
            ));
        }
        // Drop any queued post-cancel reconciliation flag (a session cancelled
        // then closed without a follow-up message would otherwise linger).
        self.pending_reconcile.lock().await.remove(id);
        // Same for wakes held during a pause — moot once the session closes.
        self.pending_paused_wakes.lock().await.remove(id);
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
        // message isn't lost. The check and the respawn can't be atomic
        // (`ensure_session_started` needs the lock, so we must drop it), so the
        // session could go stale again in the window between them — re-check under
        // the SAME lock hold we deliver under, respawning up to a few times. The
        // healthy `break sessions` keeps that hold through delivery (no TOCTOU);
        // an absent session breaks too → the `ok_or` below errors as before.
        let mut attempts = 0u32;
        let sessions = loop {
            let sessions = self.sessions.lock().await;
            let present_and_stale = match sessions.get(session_id) {
                Some(h) => h.is_stale(),
                None => false, // absent → don't respawn here; error after the loop
            };
            match Self::broadcast_deliver_step(present_and_stale, attempts) {
                // Healthy (deliver under this hold) or absent (→ ok_or err below).
                DeliverStep::Deliver => break sessions,
                DeliverStep::GiveUpBestEffort => {
                    tracing::warn!(
                        session_id,
                        attempts,
                        "session still stale after respawns; delivering best-effort"
                    );
                    break sessions;
                }
                DeliverStep::Respawn => {
                    drop(sessions); // release before ensure_session_started (it locks)
                    attempts += 1;
                    tracing::info!(
                        session_id,
                        attempt = attempts,
                        "session stale on broadcast; respawning before delivery"
                    );
                    self.ensure_session_started(session_id).await?;
                }
            }
        };
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("no live session {session_id}"))?;
        // Clear the awaiting halt BEFORE forwarding the user's reply so the
        // duo pumps see chunks again.
        self.clear_awaiting(handle, session_id).await;
        // A user message supersedes any in-flight cancel escalation: set this so
        // `interrupt_then_escalate` skips its SIGKILL (the message + its own
        // preempt-interrupt below already abort the stuck turn — a kill would
        // needlessly drop the fresh turn + warm cache). Set as EARLY as possible —
        // ahead of the awaits and the preempt interrupt — to close the window
        // where the ~2s escalation timer could fire first. Clear any lingering
        // Cancelling so the input/UI doesn't stick.
        handle
            .cancel_superseded
            .store(true, std::sync::atomic::Ordering::Release);
        handle.activity.set_cancelling(false);
        // A user message is the steer: release the pause latch so the dispatch
        // below runs the duo normally (a Send while Paused = clarify/steer; the
        // Resume button routes here too, as a resume-notice broadcast).
        handle.activity.set_paused(false);
        // Reset the L2 volley hard-cap: the user just spoke, so the consecutive
        // peer-forward counter (`router::route_forward`) starts fresh. Deliberately
        // here and not in `clear_awaiting` — `advance_phase` calls that too, and
        // a phase self-advance is not a user message.
        handle
            .user_silent_forwards
            .store(0, std::sync::atomic::Ordering::Release);
        // Same hard boundary for the router's convergence streak: clear it so a
        // pre-message streak (surviving an honored interrupt) can't suppress the
        // first post-message peer-forward. Router consumes the flag at its
        // convergence stage; no-op for a solo session (no router).
        if let Some(router) = &handle.router {
            router
                .convergence_reset
                .store(true, std::sync::atomic::Ordering::Release);
        }
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
        // A user prompt also re-arms the idle-unflagged watchdog's
        // once-per-window nudge (and its >0 count marks the session as
        // having a task at all).
        handle
            .user_broadcasts
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        // A user message supersedes any declared background work: the agents
        // wake for it, so the WORKING badge would be stale the moment they
        // settle into the new task. (Expiry and this are the only clears —
        // never activity transitions.)
        self.bridge.clear_session_working(session_id).await;
        handle.activity.set_busy(Author::Brian, true);
        if handle.rain.is_some() {
            handle.activity.set_busy(Author::Rain, true);
        }
        self.bridge
            .notify_message_persisted(Arc::from(session_id), id);
        // Release everything the pause held, BEHIND the user's message (their
        // steer preempts; the held context follows). (1) OOB answer wakes
        // stashed by `resolve_choice` while paused → deliver to both stdins
        // now. (2) Tell the router to flush held peer-forwards — through its
        // command channel, so they land in order behind anything in flight.
        let held_wakes = self
            .pending_paused_wakes
            .lock()
            .await
            .remove(session_id)
            .unwrap_or_default();
        for wire in held_wakes {
            handle
                .send_to_both(crate::agents::OutgoingUserMessage::text(wire))
                .await;
        }
        flush_held(handle.router.as_ref().map(|r| &r.tx), session_id);
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

        handle.ipav.lock().await.advance(target);
        let notice = target.transition_notice().to_string();

        // Synthetic phase-change message in storage.
        let id = self
            .storage
            .insert_message(session_id, Author::User, MessageKind::PhaseChange, &notice)
            .await?;
        self.bridge
            .notify_message_persisted(Arc::from(session_id), id);
        // Fed to HANDS's stdin so it lands as a natural prompt.
        //
        // NOT to EYES (issues.md #8). Waking the reviewer on a phase transition
        // buys nothing and costs a turn: she has no new content to review, so
        // the turn is a "holding for Brian's plan" acknowledgment — and each one
        // burns a slot of the `VOLLEY_HARD_CAP` budget that #24 showed was being
        // exhausted before substantive reviews could get through. Measured in
        // this very session: filler turns landing 7-45 s after each phase
        // change, 40-116 chars apiece.
        //
        // She loses no information. Every peer forward carries the current phase
        // in its envelope (`peer_forward_message(from, body, phase, …)`), so she
        // reads the new phase on the next message that actually has something in
        // it. Provider-limit peer notices still wake her deliberately — that is a
        // different path and stays.
        let _ = handle
            .brian
            .input_tx
            .send(OutgoingUserMessage::text(notice))
            .await;

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
                .send(OutgoingUserMessage::text(Self::APPLY_ENTRY_NUDGE.to_string()))
                .await;
        }
        // A phase advance clears `awaiting` (above), so it must also release what
        // the router held during that halt — otherwise the held forward waits for
        // the user to type, which a phase click is not.
        flush_held(handle.router.as_ref().map(|r| &r.tx), session_id);
        Ok(())
    }

    /// The Plan→Apply nudge to HANDS.
    ///
    /// It must NOT tell the agent to `mark_awaiting_user` while it waits on its
    /// peer — that call is hard-REFUSED for a peer-shaped reason
    /// (`jsonrpc.rs::peer_shaped_reason`, shipped `3282708` to end a 100-minute
    /// mutual-deferral deadlock), so the old wording ordered something the tool
    /// rejects, and the rejection text then told the agent to do the opposite.
    /// Observed live three times in one session before it was reconciled.
    ///
    /// Waiting on a peer is not waiting on the user: a turn's output forwards to
    /// the peer automatically, so SAYING SO is the wake mechanism. Pinned by
    /// `apply_nudge_never_tells_hands_to_park_on_the_user`.
    const APPLY_ENTRY_NUDGE: &'static str =
        "🔔 Entering Apply. Before you mutate: confirm Rain reviewed the plan — pull \
         session_doc_search(phase=\"plan\") and check her pushback landed. If it hasn't, \
         say so in chat (your turn output is forwarded to her automatically, which wakes \
         her) and do non-mutating prep meanwhile. Don't park on the USER for a peer wait.";

    /// A2 (adherence): whether the Plan→Apply boundary in a duo session warrants
    /// the peer-ack nudge to Brian. Pure for testing; the caller additionally
    /// AND-gates the `adherence_nudges` setting.
    fn should_peer_ack_nudge(prev: IpavPhase, target: IpavPhase, has_rain: bool) -> bool {
        has_rain && prev == IpavPhase::Plan && target == IpavPhase::Apply
    }

    /// Decide a cancel escalation's outcome after the interrupt window. Pure for
    /// testing; precedence is honored > superseded > sigkill (a user message that
    /// arrives can't resurrect a turn that already went idle, so `both_idle` wins).
    ///
    /// `idled_since_cancel` is what makes the superseded branch safe. A user
    /// message alone is NOT evidence the stuck turn aborted — it only proves the
    /// USER did something. If the agent never honored the interrupt, skipping
    /// the SIGKILL on that basis leaves it running after Stop.
    ///
    /// So the skip requires proof the turn actually stopped: the agents reached
    /// idle at some point since the cancel. That still protects the case the
    /// skip exists for — interrupt honored, then the user's message started a
    /// fresh turn that is busy again by the deadline — while an agent that never
    /// reached idle gets force-killed as the user asked.
    ///
    /// This is a SAFETY NET against any agent that doesn't honor a
    /// `control_request` in time (a native-loop model, a future tool that blocks
    /// on stdin, a dropped interrupt), NOT a fix for a confirmed claude-code
    /// behavior. An earlier version of this comment asserted that claude-code
    /// cannot see interrupts while blocked on a synchronous Bash call; a live
    /// test on 2026-07-29 disproved that — it aborted a running `cargo build`
    /// ~2s after Stop and the process survived (PID unchanged), i.e.
    /// `InterruptHonored`. Which path actually causes a Stop not to hold is
    /// still unknown; `cancel_events` exists to answer that from data next time.
    fn escalation_outcome(
        both_idle: bool,
        cancel_superseded: bool,
        idled_since_cancel: bool,
    ) -> EscalationOutcome {
        if both_idle {
            EscalationOutcome::InterruptHonored
        } else if cancel_superseded && idled_since_cancel {
            EscalationOutcome::SupersededByUser
        } else {
            EscalationOutcome::Sigkill
        }
    }

    /// Decide one step of `broadcast`'s auto-heal loop. Pure for testing. A
    /// healthy OR absent handle delivers (absent then errors at the `get`); a
    /// present-but-stale handle respawns until the cap, then delivers best-effort.
    fn broadcast_deliver_step(present_and_stale: bool, attempts: u32) -> DeliverStep {
        if !present_and_stale {
            DeliverStep::Deliver
        } else if attempts >= BROADCAST_MAX_RESPAWNS {
            DeliverStep::GiveUpBestEffort
        } else {
            DeliverStep::Respawn
        }
    }

    /// Decide what an out-of-band tray answer does to a live session. Pure for
    /// testing; `holds_wakes` is the pause latch (`activity.holds_wakes()`).
    fn tray_wake_step(holds_wakes: bool) -> TrayWakeStep {
        if holds_wakes {
            TrayWakeStep::StashForResume
        } else {
            TrayWakeStep::PreemptAndDeliver
        }
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
        // A resolved tray answer is user engagement — the same contract as a
        // typed message for the idle-unflagged watchdog: it marks the session
        // as having a task and re-arms the once-per-window nudge. The d61d277
        // live smoke found tray-only input left the watchdog disarmed (only
        // `broadcast` bumped the counter). StaleGateNeedsConfirm is excluded:
        // nothing was flipped or delivered.
        if matches!(
            outcome,
            ResolveOutcome::Delivered | ResolveOutcome::AgentReceiverDroppedFellBack { .. }
        ) {
            if let Ok(Some(entry)) = self.storage.get_tray_entry(choice_id).await {
                let sessions = self.sessions.lock().await;
                if let Some(handle) = sessions.get(&entry.session_id) {
                    handle
                        .user_broadcasts
                        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                }
            }
        }
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
                // PAUSED gate: an answered tray question must not restart a
                // paused duo (the user may be triaging their tray while this
                // session stays parked). Stash the wire; the next `broadcast`
                // (Send / Resume) delivers it behind the user's message.
                match Self::tray_wake_step(handle.activity.holds_wakes()) {
                    TrayWakeStep::StashForResume => {
                        self.pending_paused_wakes
                            .lock()
                            .await
                            .entry(session_id.clone())
                            .or_default()
                            .push(wire);
                    }
                    TrayWakeStep::PreemptAndDeliver => {
                        // Human preemption, same spine as `broadcast` (issues.md
                        // #27): an answered tray card is the user speaking, so it
                        // must take effect at the next tool boundary instead of
                        // waiting out the agent's whole current turn — two
                        // same-day races in s-b69a5c01 had an agent building on
                        // premises the parked answer had already overturned.
                        // BEFORE the send, mirroring `broadcast`: the pump's
                        // biased control channel writes the interrupt ahead of
                        // stdin, so each agent aborts and then reads the answer.
                        // Verified idle-harmless there (control_response{success},
                        // process survives, next message still processed) — so no
                        // gate on the flaky `busy` signal, and no SIGKILL
                        // escalation (the answer IS the next work).
                        handle.brian.interrupt("tray-answer-preempt");
                        if let Some(rain) = handle.rain.as_ref() {
                            rain.interrupt("tray-answer-preempt");
                        }
                        handle
                            .send_to_both(crate::agents::OutgoingUserMessage::text(wire))
                            .await;
                        // Answering a tray card ends the halt just as a typed
                        // message does, so release what the router held behind
                        // it — AFTER the answer itself, so the peer's held
                        // chatter lands behind it. Not in the stash arm above:
                        // there the wire is deliberately held for the next
                        // broadcast, and flushing would defeat the pause latch.
                        flush_held(handle.router.as_ref().map(|r| &r.tx), session_id);
                    }
                }
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

/// Release any peer-forwards the router is holding.
///
/// The router holds forwards while the duo is halted on the user. Something has
/// to tell it the halt is over, and for a long time only `broadcast` did — so a
/// forward parked behind a question stayed parked when the user ANSWERED that
/// question from the tray, or when the phase advanced. It surfaced only on the
/// next typed message, leaving the peer half-deaf in between (observed live,
/// 2026-08-04). Every path that clears `awaiting` now ends here.
///
/// **Call it at the END of the path, never inside `clear_awaiting`.** Clearing
/// happens BEFORE the user's own message is delivered; flushing there would
/// release held peer chatter AHEAD of what the user just said. `broadcast`'s
/// ordering — message, then held paused-wakes, then this — is the contract.
///
/// Never blocks and never fails a caller: a full/closed channel just means the
/// forwards stay held until the next flush. `None` (solo session) is a no-op.
fn flush_held(
    router_tx: Option<&tokio::sync::mpsc::Sender<crate::core::router::RouterCommand>>,
    session_id: &str,
) {
    let Some(tx) = router_tx else { return };
    if tx
        .try_send(crate::core::router::RouterCommand::FlushHeld)
        .is_err()
    {
        tracing::warn!(session_id, "router FlushHeld not sent (channel full/closed)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Live session tests require RUN_LIVE_TESTS=1 (subprocesses spawn).
    // We unit-test the static pieces here.

    #[test]
    fn apply_nudge_never_tells_hands_to_park_on_the_user() {
        // The old wording said "wait for it (mark_awaiting_user)" — a call the
        // tool HARD-REFUSES for a peer-shaped reason
        // (jsonrpc.rs::peer_shaped_reason, shipped 3282708). A compliant agent
        // got refused and told to do the opposite of the nudge; it fired three
        // times in one session before this was reconciled.
        let nudge = AppState::APPLY_ENTRY_NUDGE;
        assert!(
            !nudge.contains("mark_awaiting_user"),
            "the Apply nudge must not order a call that mark_awaiting_user refuses"
        );
        // The refusal keys on these words anywhere in the reason, so the nudge
        // must not push HANDS toward parking at all.
        assert!(
            !nudge.contains("ask_user_choice"),
            "a peer wait is not a user decision — don't route it to the tray either"
        );
        // …while still carrying its actual job.
        assert!(nudge.contains("session_doc_search(phase=\"plan\")"));
        assert!(
            nudge.contains("forwarded"),
            "must name the real wake mechanism: turn output forwards to the peer"
        );
    }

    #[tokio::test]
    async fn flush_held_sends_on_a_live_router_and_no_ops_without_one() {
        use crate::core::router::RouterCommand;

        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        super::flush_held(Some(&tx), "s1");
        assert!(
            matches!(rx.try_recv(), Ok(RouterCommand::FlushHeld)),
            "a live router must receive FlushHeld"
        );

        // Solo session: no router, no panic, nothing sent.
        super::flush_held(None, "s1");
        assert!(rx.try_recv().is_err(), "None router must be a silent no-op");
    }

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

    // --- Batch 6: cancel escalation decision (cancel/new-message race fix) ---

    #[test]
    fn escalation_honored_when_both_idle_regardless_of_supersede() {
        // `both_idle` wins over supersede: a turn that already ended can't be
        // "saved" by a later user message, and there's nothing left to kill.
        assert_eq!(
            AppState::escalation_outcome(true, false, false),
            EscalationOutcome::InterruptHonored
        );
        assert_eq!(
            AppState::escalation_outcome(true, true, true),
            EscalationOutcome::InterruptHonored
        );
    }

    #[test]
    fn escalation_superseded_skips_sigkill_only_with_proof_the_turn_stopped() {
        // Busy at the deadline, user message arrived, AND the agents reached
        // idle in between → the interrupt WAS honored and this busy is the
        // user's fresh turn. Skipping the kill protects that turn.
        assert_eq!(
            AppState::escalation_outcome(false, true, true),
            EscalationOutcome::SupersededByUser
        );
    }

    #[test]
    fn escalation_sigkills_a_wedged_agent_even_after_a_user_message() {
        // The "Stop doesn't hold" bug. claude-code reads control_request from
        // stdin BETWEEN turns, so an agent blocked on a synchronous Bash call
        // never sees the cancel interrupt nor the message's preempt interrupt.
        // It never reached idle, so the user message is not evidence the turn
        // stopped — and skipping the kill on it alone left the agent running
        // after Stop. Force-kill instead.
        assert_eq!(
            AppState::escalation_outcome(false, true, false),
            EscalationOutcome::Sigkill
        );
    }

    #[test]
    fn escalation_sigkill_when_not_honored_and_not_superseded() {
        // Not idle, no user message → the interrupt was dropped/wedged → SIGKILL
        // fallback so a hung turn can't leave the working tree half-written.
        assert_eq!(
            AppState::escalation_outcome(false, false, false),
            EscalationOutcome::Sigkill
        );
    }

    // --- Batch 7: broadcast auto-heal respawn loop decision (TOCTOU fix) ---

    #[test]
    fn broadcast_delivers_when_healthy_or_absent() {
        // present_and_stale=false covers BOTH a healthy handle (deliver under the
        // current lock hold — no TOCTOU) and an absent one (delivers to the loop
        // exit, which then `ok_or`-errors). Either way: no respawn.
        assert_eq!(
            AppState::broadcast_deliver_step(false, 0),
            DeliverStep::Deliver
        );
        assert_eq!(
            AppState::broadcast_deliver_step(false, BROADCAST_MAX_RESPAWNS),
            DeliverStep::Deliver
        );
    }

    #[test]
    fn broadcast_respawns_stale_under_cap_then_gives_up() {
        // Present + stale → respawn each attempt up to the cap, then best-effort.
        for attempts in 0..BROADCAST_MAX_RESPAWNS {
            assert_eq!(
                AppState::broadcast_deliver_step(true, attempts),
                DeliverStep::Respawn,
                "attempt {attempts} under the cap must respawn"
            );
        }
        assert_eq!(
            AppState::broadcast_deliver_step(true, BROADCAST_MAX_RESPAWNS),
            DeliverStep::GiveUpBestEffort
        );
        assert_eq!(
            AppState::broadcast_deliver_step(true, BROADCAST_MAX_RESPAWNS + 5),
            DeliverStep::GiveUpBestEffort
        );
    }

    // --- issues.md #27: an OOB tray answer preempts the running turn ---

    #[test]
    fn tray_answer_preempts_a_live_duo_but_not_a_paused_one() {
        // Live: the answer is the user speaking, so it aborts the in-flight turn
        // instead of waiting it out (the s-b69a5c01 races — an agent finishing a
        // deliverable while its superseding answer sat unread on stdin).
        assert_eq!(
            AppState::tray_wake_step(false),
            TrayWakeStep::PreemptAndDeliver
        );
        // Paused: nothing is delivered (the wire is stashed for the next
        // broadcast), so there is nothing to preempt — and interrupting here
        // would half-release a pause the user deliberately set.
        assert_eq!(
            AppState::tray_wake_step(true),
            TrayWakeStep::StashForResume
        );
    }

    #[test]
    fn tray_preempt_fires_before_the_answer_is_sent() {
        // Ordering contract, mirroring `broadcast`: the pump's biased control
        // channel writes the interrupt ahead of stdin, so the agent aborts and
        // THEN reads the answer. Sending first would let the whole turn run out
        // before the abort landed — the bug this fixes. Guarded as source order
        // because the delivery arm needs a live subprocess to exercise.
        let src = include_str!("state.rs");
        let arm = src
            .split("TrayWakeStep::PreemptAndDeliver => {")
            .nth(1)
            .expect("the preempt arm must exist");
        let interrupt = arm.find("brian.interrupt(\"tray-answer-preempt\")");
        let send = arm.find("send_to_both");
        assert!(
            interrupt.is_some() && send.is_some() && interrupt < send,
            "the tray-answer interrupt must fire BEFORE send_to_both"
        );
    }
}
