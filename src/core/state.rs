//! `AppState`: top-level handle the UI layer holds.

use crate::core::broadcast::broadcast_user_message;
use crate::core::close_learnings;
use crate::core::ipav::IpavPhase;
use crate::core::session::{
    open_session, spawn_existing_session, OpenSessionRequest, SessionAgent, SessionHandle,
};
use crate::paths::Paths;
use crate::signaling::{ExternalServer, SignalingBridge, SignalingEvent, SignalingServer};
use crate::storage::{Author, MessageKind, PersistedMessage, Session, Storage};
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

/// Everything an out-of-band tray answer does to a live session, decided in one
/// place from the only two facts that bear on it: whether the session holds
/// wakes (the pause latch) and whether the bridge managed to record the answer.
/// Pure (see [`AppState::tray_wake`]) so all four combinations are unit-tested
/// without a live session.
///
/// FOUR independent flags rather than an enum of named cases, because the bug
/// this replaced was exactly a case that bundled them. Gating the whole block on
/// a recorded row skipped the halt clear and the router flush along with the
/// send, leaving the session parked on a question the user had already answered
/// with the peer's messages queued behind that halt. As an enum the two are one
/// variant and nothing can tell them apart; as fields, `(live, unrecorded)` is a
/// value you can compare against, which is what the test does.
#[derive(Debug, PartialEq, Eq)]
struct TrayWake {
    /// Lift the awaiting-user halt. Always — the user answered, and that stays
    /// true whatever happened to the row afterwards.
    clear_halt: bool,
    /// Hold the receipt for the next `broadcast` (Send / Resume) instead of
    /// delivering: the pause latch. A tray answer must not release a pause the
    /// user deliberately set. Needs a receipt, since a receipt is the thing held.
    stash: bool,
    /// Interrupt the in-flight turn and write the answer to every agent. Needs a
    /// receipt, and the interrupt is PAIRED with it: firing it with nothing
    /// behind it would abort a live turn and give the agent nothing to read in
    /// its place, which is worse than leaving the turn alone.
    deliver: bool,
    /// Release what the router held behind the halt. Follows `clear_halt`, NOT
    /// the receipt — otherwise the peer's queued messages sit behind a halt that
    /// is already gone. Never in the stash case: there the hold is the point.
    flush: bool,
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
    /// must not be lost either. `resolve_choice` stashes the RECEIPT here
    /// instead of waking stdin; the next `broadcast` (a user Send / Resume)
    /// drains it to both agents after the user's message.
    ///
    /// Receipts rather than wire strings (B5 Task 2): `send_to_all` takes one,
    /// and the row is already written by the time an answer is stashed. The
    /// drain produces the same bytes the old code froze here — same body, same
    /// envelope, both fixed at post time — but re-derives them from the row
    /// instead of carrying a copy, so a held wake cannot drift from what the
    /// chat shows.
    pending_paused_wakes: Mutex<std::collections::HashMap<String, Vec<PersistedMessage>>>,
    /// Per-session PTY terminals (Terminal subtab). Lazily spawned on first
    /// `terminal_open`, killed on `close_session`. Shared as an `Arc` so the
    /// signaling bridge's MCP handlers can reach the same PTYs.
    pub terminals: Arc<crate::core::TerminalRegistry>,
    /// Sessions currently running rc3 D15's close-out learnings turn.
    ///
    /// The epilogue leaves the session handle ALIVE while it runs, so a second
    /// `close_session` in that window still finds a live session and would
    /// otherwise start a second epilogue — a double-clicked Close button, or
    /// the epilogue's own agent calling the `close_session` MCP tool on the
    /// turn we just gave it, which is the likely one. A session in this set
    /// skips straight to teardown.
    epilogue_in_flight: Mutex<HashSet<String>>,
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
            pending_paused_wakes: Mutex::new(std::collections::HashMap::new()),
            terminals: Arc::new(crate::core::TerminalRegistry::new()),
            epilogue_in_flight: Mutex::new(HashSet::new()),
        }
    }

    /// Open a session from the external driver.
    ///
    /// `brian_model_id` / `rain_model_id` are saved-model ids; `None` falls back
    /// to the per-agent config, which is the historical behaviour. Pass them when
    /// the caller wants a SPECIFIC model for this session. The two parameter
    /// NAMES are the driver's wire contract and are left alone; what they mean
    /// here is slot 0 and slot 1 (rc3 D10).
    ///
    /// **This path has no dialog, so it takes the product default: ONE
    /// participant** (rc3 D13 — the `rain_disabled_default` setting it used to
    /// read is deleted, and design §1 puts the default at one agent). See
    /// [`Storage::ensure_session_roster`], which seeds it. A driver that wants a
    /// second participant has to add the role to the session it creates; passing
    /// `rain_model_id` alone does NOT add one, and there is no roster slot for
    /// it to land on.
    pub async fn open_session(
        &self,
        title: impl Into<String>,
        working_repo_path: Option<std::path::PathBuf>,
        brian_model_id: Option<String>,
        rain_model_id: Option<String>,
    ) -> Result<String> {
        let mut req = OpenSessionRequest::full(title, working_repo_path);
        // rc3 D13: the product default with no UI behind it. One participant.
        req.solo = true;
        // Positional over the default roster's turn order. The two parameter
        // NAMES are the external driver's wire contract and are left alone; what
        // they mean here is slot 0 and slot 1 (rc3 D10).
        req.models = vec![brian_model_id, rain_model_id];
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
                    for agent in stale.agents_mut() {
                        agent.handle.kill();
                    }
                    tracing::info!(session_id, "evicted stale session handle; re-spawning");
                }
            }
        }
        // The roster seed moved into `spawn_session_handle` (B4b.2) — it is the
        // choke point BOTH creation paths share, and this one is not: the
        // external driver's `open_session` never reaches here.
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
                for agent in handle.agents_mut() {
                    agent.handle.kill();
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
            // Participants that CANNOT edit files go first (side-effect-safe), then
            // the ones that can —
            // mirrors cancel_kill_now. `interrupt` is best-effort (&self try_send);
            // a full/closed control channel returns false and the idle-watch below
            // times out into the SIGKILL fallback.
            //
            // KEEP the booleans. They used to be discarded, which made a DROPPED
            // interrupt indistinguishable from one the agent received and ignored
            // — two different bugs with the same symptom, and no way to tell them
            // apart after the fact.
            // Mutation-capable agents LAST — the ordering is the point, so iterate
            // by capability rather than by turn position (which puts the
            // executor at 0). rc3 D10/D11: was `a.slug != "brian"`, and with
            // role-derived slugs that predicate matched EVERY agent, so the
            // executor would have been interrupted first, mid-tool. `None` =
            // this session has no non-mutating peer.
            let mut rain_queued: Option<bool> = None;
            for agent in handle.agents().filter(|a| !a.edits_files()) {
                let queued = agent.handle.interrupt("cancel");
                if !queued {
                    tracing::warn!(session_id, slug = %agent.slug, "cancel: peer interrupt was NOT queued");
                }
                rain_queued = Some(rain_queued.unwrap_or(true) && queued);
            }
            let brian_queued = handle
                .hands()
                .is_some_and(|h| h.handle.interrupt("cancel"));
            if !brian_queued {
                tracing::warn!(session_id, "cancel: HANDS interrupt was NOT queued");
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
                // Agents that cannot edit files are side-effect-safe; kill them
                // first. A mutation-capable agent may be mid-tool, so it goes
                // last. Capability, not turn position — same rule as
                // `interrupt_then_escalate`.
                let mut hands: Option<&mut SessionAgent> = None;
                for agent in handle.agents_mut() {
                    if agent.edits_files() {
                        hands = Some(agent);
                    } else {
                        agent.handle.kill();
                    }
                }
                if let Some(h) = hands {
                    h.handle.kill();
                }
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

    /// Close a session — both the user's Close button (`tauri_cmd::sessions`)
    /// and the agent's own `close_session` MCP tool (via the control-event
    /// worker in `main.rs`) land here.
    ///
    /// **rc3 D15 splits this.** When the session qualifies for a close-out
    /// learnings turn ([`close_learnings::decide`]), the row is closed and the
    /// UI is told immediately, the turn runs DETACHED, and teardown follows it.
    /// The close itself never waits: D15's *"a failed or slow write never
    /// delays the close and never leaves the session un-closable."* Every other
    /// session takes [`Self::teardown_session`] directly, which is this
    /// function's pre-D15 body unchanged.
    ///
    /// The ordering inside the epilogue arm is the load-bearing part. The row
    /// must be closed and `SessionClosed` fired BEFORE the turn, or the close
    /// blocks on it; teardown must come AFTER, or the agent's `cl_write_file`
    /// reaches a bridge that has already forgotten the session
    /// (`unregister_session` drops both the project map and the close gate).
    pub async fn close_session(
        self: &Arc<Self>,
        id: &str,
        archive: bool,
        path: close_learnings::ClosePath,
    ) -> Result<()> {
        let decision = self.close_epilogue_decision(id, path).await;
        // Claim only when the decision wants a turn. `insert` returns false
        // when the id is already there, so the claim is atomic in one short
        // hold and the decision runs outside it — no `epilogue_in_flight` →
        // `sessions` lock nesting.
        let claimed = decision == close_learnings::Epilogue::Run
            && self
                .epilogue_in_flight
                .lock()
                .await
                .insert(id.to_string());
        // Exhaustive on purpose — see `ClosePlan`. Deleting the epilogue arm
        // has to be a compile error, not a silently inert feature.
        match close_learnings::plan(decision, claimed) {
            close_learnings::ClosePlan::TearDownNow => {
                self.teardown_session(id, Some(archive)).await
            }
            close_learnings::ClosePlan::RunEpilogueFirst => {
                // Closed and off the UI's hands from here; the turn is epilogue.
                self.storage.close_session(id, archive).await?;
                self.bridge.notify_session_closed(id.to_string());
                let this = Arc::clone(self);
                let sid = id.to_string();
                tokio::spawn(async move {
                    this.run_close_epilogue(&sid).await;
                    // Release the claim BEFORE teardown, so a teardown failure
                    // can never leave a session marked mid-epilogue forever.
                    this.epilogue_in_flight.lock().await.remove(&sid);
                    // `archive` is already applied — the row closed before the
                    // turn.
                    if let Err(e) = this.teardown_session(&sid, None).await {
                        tracing::warn!(?e, session_id = %sid, "close epilogue: teardown failed");
                    }
                });
                Ok(())
            }
        }
    }

    /// Does this session get a close-out learnings turn (rc3 D15)?
    ///
    /// Reads the three live facts [`close_learnings::decide`] needs and hands
    /// them over; the policy is entirely in `decide`, so the arms are unit-
    /// tested without a session. A session that is not live at all cannot take
    /// a turn, and an unreadable roster is treated as "no writer" — the silent
    /// skip, which is the safe direction: D15 would rather nothing happen than
    /// have a session prompted into writing the library on a guess.
    async fn close_epilogue_decision(
        &self,
        id: &str,
        path: close_learnings::ClosePath,
    ) -> close_learnings::Epilogue {
        let activity = {
            let sessions = self.sessions.lock().await;
            match sessions.get(id) {
                // A STALE handle is treated as no session at all, and that is
                // the sharpest edge here: `broadcast` auto-heals a stale
                // session by RESPAWNING it, so asking one for its learnings
                // would start a fresh subprocess with none of this session's
                // context and hand it "write what you learned" — the exact
                // filler D15 exists to prevent, produced by the mechanism meant
                // to capture knowledge. If the agents are gone, so is anything
                // they learned.
                Some(handle) if !handle.is_stale() => handle.activity.current(),
                _ => return close_learnings::Epilogue::SkipNoWriter,
            }
        };
        let any_writer = match self.storage.participants_for_session(id).await {
            Ok(roster) => roster.iter().any(|p| {
                p.enabled
                    && crate::agents::CapabilitySet::from_json(&p.capabilities)
                        .is_some_and(|caps| {
                            caps.contains(crate::agents::Capability::WriteContextLibrary)
                        })
            }),
            Err(e) => {
                tracing::warn!(?e, session_id = %id, "close epilogue: roster unreadable; skipping");
                false
            }
        };
        let (cl_written, close_nudged) = self.bridge.close_gate_flags(id).await;
        let decision = close_learnings::decide(activity, any_writer, cl_written, close_nudged, path);
        // **Three of the four arms were silent, and only one of them by
        // requirement.** D15 asks for `SkipNoWriter` to make no noise TO THE
        // USER — a row — and says nothing about the log. The cost of the other
        // two being silent was paid on 2026-08-13: no session had ever run an
        // epilogue, and nothing recorded which arm had refused it, so the
        // written-down diagnosis blamed a slow ring instead. Same argument D26
        // made for agent health the same morning.
        //
        // INFO, and the inputs alongside the verdict: a close happens once per
        // session, and a verdict without what produced it only moves the guess
        // one level down.
        tracing::info!(
            session_id = %id,
            ?decision,
            ?path,
            ?activity,
            any_writer,
            cl_written,
            close_nudged,
            "close epilogue decision"
        );
        decision
    }

    /// The detached half of D15: ask, wait, and record what happened.
    ///
    /// Never returns an error — it is fire-and-forget by construction, and its
    /// only product is the row. A delivery failure and a decline both end here
    /// with a `system_notice`, saying different things, which is the D15 item
    /// this closes: *"a fire-and-forget write that FAILS looks identical to one
    /// that correctly declined."*
    async fn run_close_epilogue(&self, id: &str) {
        use close_learnings::Outcome;
        let outcome = match self
            .broadcast(id, close_learnings::CLOSE_LEARNINGS_PROMPT)
            .await
        {
            Err(e) => Outcome::Failed(e.to_string()),
            Ok(()) => {
                let activity = {
                    let sessions = self.sessions.lock().await;
                    sessions.get(id).map(|h| Arc::clone(&h.activity))
                };
                match activity {
                    // The handle vanished under us (a concurrent close). Not a
                    // failure to report — nothing was asked and nothing hung.
                    None => return,
                    Some(activity) => {
                        let deadline = tokio::time::Instant::now()
                            + close_learnings::CLOSE_EPILOGUE_TIMEOUT;
                        // No settle delay, and that is load-bearing rather than
                        // an omission: `broadcast` sets every agent's `busy`
                        // flag ITSELF, before it returns (the turn-start
                        // recompute), so by here the tracker already reads
                        // Busy. Waiting for the pump's first event instead
                        // would race — a slow first token would read the
                        // PRE-turn idle and report every epilogue as declined.
                        if activity.await_both_idle(deadline).await {
                            let (wrote, _) = self.bridge.close_gate_flags(id).await;
                            if wrote {
                                Outcome::Wrote
                            } else {
                                Outcome::Declined
                            }
                        } else {
                            Outcome::TimedOut
                        }
                    }
                }
            }
        };
        self.post_close_learnings_row(id, &outcome).await;
    }

    /// Post the epilogue's outcome as a `system_notice`, exactly as the D7
    /// capped-halt notice does (`sequencer::announce_round_cap`): host-authored,
    /// so `origin = 'system'` with a NULL participant.
    ///
    /// **The post is the contract; the notification is the nicety** — a missed
    /// `notify_message_persisted` costs a refetch, a missed row costs the only
    /// account of what the close decided.
    async fn post_close_learnings_row(&self, id: &str, outcome: &close_learnings::Outcome) {
        let body = close_learnings::outcome_notice(outcome);
        match self
            .storage
            .post_to_channel(
                Arc::from(id),
                "system",
                None,
                MessageKind::SystemNotice.as_str(),
                body,
                None,
            )
            .await
        {
            Ok(row) => self
                .bridge
                .notify_message_persisted(Arc::from(id), row.message_id()),
            Err(e) => tracing::warn!(
                ?e,
                session_id = %id,
                ?outcome,
                "close epilogue: outcome row not posted; the close has no on-screen account"
            ),
        }
    }

    /// Kill the session's processes and drop every trace of it from memory.
    ///
    /// `archive` is `Some(_)` on the direct path (this call closes the row too)
    /// and `None` when [`Self::close_session`]'s D15 epilogue arm already
    /// closed it — the row is closed once, before the epilogue, so the UI never
    /// waits on a learnings turn.
    async fn teardown_session(&self, id: &str, archive: Option<bool>) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        if let Some(mut handle) = sessions.remove(id) {
            for agent in handle.agents_mut() {
                agent.handle.kill();
            }
        }
        // Stop live-watching this session's working repo.
        if let Some(watcher) = self.fs_watcher.get() {
            watcher.remove_repo(id);
        }
        // Reap the session's PTY terminal alongside the agent subprocesses.
        self.terminals.kill_and_remove(id).await;
        if let Some(archive) = archive {
            self.storage.close_session(id, archive).await?;
        }
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
    /// `SessionHandle` is dropped).
    ///
    /// Does NOT touch pending-halt ROWS. That is [`Self::user_responded`]'s
    /// job, and it is the only thing that does it — this doc used to say
    /// "callers that also answer those call `clear_pending_halts` separately",
    /// which is the arrangement that let one caller forget for 52 occurrences
    /// (rc3 D28).
    async fn clear_awaiting(&self, handle: &SessionHandle, session_id: &str) {
        handle
            .awaiting
            .store(false, std::sync::atomic::Ordering::Release);
        self.bridge.clear_session_awaiting(session_id).await;
    }

    /// **The user responded.** Release the ring and clear the halt they answered
    /// — one call, because they are one event (rc3 **D28**).
    ///
    /// # Why this is a function and not two lines at each call site
    ///
    /// It was two lines at each call site, and they drifted. Three paths mean
    /// "the user responded" — a typed message, an answered tray card, a phase
    /// advance — and each did a different subset:
    ///
    /// | | releases the ring | clears the halt row |
    /// |---|---|---|
    /// | typed message | yes | yes |
    /// | **answered tray card** | yes | **no** |
    /// | phase advance | no | yes |
    ///
    /// So answering a question released the cycle and left its halt row pending
    /// for ever. The bell stayed lit, the user answered again, and the agent —
    /// legitimately, having a new question — parked another. **Measured across
    /// the session archive: 52 occasions where a second tray row opened while
    /// the first was still unanswered**, the worst a single row that sat
    /// unanswered while six more stacked behind it over 53 minutes.
    ///
    /// The user reported it as "answering a question didn't clear a halt, so
    /// they parked another halt after some time, now there are 2 halts in
    /// tray". Every half was individually correct.
    ///
    /// This is the third bug of exactly this shape: a halt shipped with no
    /// release (D19), a health verdict that reached the UI but no record (D26),
    /// and now a release without a clear. The pattern is one event wired through
    /// two halves at N call sites, where nothing makes the halves travel
    /// together. A function is what makes them travel together.
    ///
    /// `mentions` rides along because the ring release already carries it (D17);
    /// a tray answer passes none.
    async fn user_responded(&self, session_id: &str, mentions: Vec<i64>, release_ring: bool) {
        // The halt row FIRST. If the ring release panicked or the process died
        // between the two, a cleared row and a halted ring is a session the next
        // message fixes; a released ring and a pending row is the bug above, and
        // it is invisible.
        match self.storage.clear_pending_halts(session_id).await {
            Ok(cleared) if cleared > 0 => {
                self.bridge.notify_halts_cleared(session_id.to_string());
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(?e, session_id, "clear_pending_halts failed"),
        }
        // `advance_phase` is the one caller that does NOT release: a phase
        // self-advance answers the halt without being a message anyone reads,
        // so waking the ring on it would hand out a turn over an empty backlog.
        if release_ring {
            self.bridge
                .notify_ring_user_message(session_id, mentions)
                .await;
        }
    }

    /// The participants a user message summons, in the order they were named
    /// (rc3 **D17**).
    ///
    /// **An `@word` that names nobody is ordinary prose, never an error** (D1).
    /// The `@` picker in the composer makes the case rare by construction — it
    /// offers this session's participants and nothing else — but text also
    /// arrives from the external driver, and a message REFUSED for naming a
    /// participant that has since left the roster would be a far worse failure
    /// than a word that did nothing. Same for a read error: the summons is
    /// dropped and the ring carries on, because the user's message landing one
    /// turn later beats it not landing.
    ///
    /// A disabled participant is dropped too. There is no process behind one, so
    /// handing it the turn would stop the cycle on a participant that cannot
    /// complete it — the frozen-cycle case the sequencer's module doc describes,
    /// reached deliberately instead of by accident.
    async fn resolve_mentions(&self, session_id: &str, text: &str) -> Vec<i64> {
        let slugs = crate::core::mentions::parse_mention_slugs(text);
        let mut ids = Vec::with_capacity(slugs.len());
        for slug in slugs {
            // Slug OR the user's label (rc3 D20, migration 0053). Without this
            // the label would break the property `speaker_of`'s doc rests on:
            // a participant reading `[skeptic]` must be reading the string the
            // user would type to summon it. The slug still resolves, so a label
            // that is not mention-shaped costs the alias, never the participant.
            match self.storage.participant_by_mention(session_id, &slug).await {
                Ok(Some(p)) if p.enabled => ids.push(p.id),
                Ok(Some(_)) => tracing::debug!(
                    session_id,
                    %slug,
                    "a mention named a disabled participant; ignored"
                ),
                Ok(None) => tracing::debug!(
                    session_id,
                    %slug,
                    "a mention named nobody in this session; it is ordinary prose"
                ),
                Err(e) => tracing::warn!(
                    session_id,
                    %slug,
                    ?e,
                    "resolving a mention failed; the summons is dropped"
                ),
            }
        }
        ids
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
        // pumps see chunks again.
        self.clear_awaiting(handle, session_id).await;
        // The ring's RELEASE is not here — it rides the notify below, AFTER the
        // row is posted. `clear_awaiting` only lowers a flag; the sequencer
        // halted on `QuestionParked` and a user message is the only thing that
        // un-halts it, so releasing it here would hand out a turn over an empty
        // backlog and land the message a turn late.
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
        for agent in handle.agents() {
            agent.handle.interrupt("user-preempt");
        }
        // No recipient list: the row IS the delivery (rc3 D19). The ring hands
        // the turn to the front of the rotation and that participant drains the
        // row off its cursor; everyone else reads it when their turn comes.
        let id = broadcast_user_message(&self.storage, session_id, text, phase, reconcile).await?;
        // **Who did the user name?** (rc3 D17.) Resolved HERE, on the one path
        // that writes an `origin = "user"` row — which is what makes "only the
        // user may mention" structural rather than a rule agents are asked to
        // follow. A participant that types `@advisor` writes text, and there is
        // no code anywhere that could act on it.
        let mentions = self.resolve_mentions(session_id, text).await;
        // Told AFTER the row exists, so the participant it wakes has something
        // to drain. Reversing these two hands out a turn over an empty backlog
        // and the message lands a turn late. Both halves of "the user responded"
        // ride this one call (rc3 D28).
        self.user_responded(session_id, mentions, true).await;
        // The user's message reaches the front of the rotation → busy. The
        // awaiting flag was cleared just above, so this recompute moves the
        // session AwaitingUser/Idle → Busy.
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
        for agent in handle.agents() {
            handle.activity.set_busy_slug(&agent.slug, true);
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
        // The rows exist; the ring replays them off each cursor (rc3 D19).
        // Fanning them into every stdin here woke participants outside their
        // turn, which is what stranded their epochs.
        let _ = &held_wakes;
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
        // Clears the halt without releasing the ring: a phase self-advance
        // answers the question but is not a message anyone reads, so waking the
        // ring on it would hand out a turn over an empty backlog (rc3 D28).
        self.user_responded(session_id, Vec::new(), false).await;

        handle.ipav.lock().await.advance(target);

        // Synthetic phase-change message in storage. No envelope: the wire is
        // the notice byte for byte, because `transition_notice()` already
        // carries its own `[PHASE: X]` and a phase envelope would double-tag it.
        // The one host-authored site where the row needed no reordering to
        // become the wire — the receipt goes straight to HANDS below.
        //
        // The `&'static str` goes in directly; it used to be pre-`to_string`d
        // because the wire moved the owned copy, and that consumer is gone.
        let persisted = self
            .storage
            .insert_message(
                session_id,
                Author::User,
                MessageKind::PhaseChange,
                target.transition_notice(),
            )
            .await?;
        self.bridge
            .notify_message_persisted(Arc::from(session_id), persisted.message_id());
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
        if let Some(hands) = handle.hands() {
            hands.deliver(&persisted).await;
        }

        // A2 (adherence): the peer-ack the prompts don't mechanically enforce.
        // On the Plan→Apply boundary in a duo session, remind Brian (HANDS) to
        // confirm Rain's plan review before mutating. Brian-only; no-op solo;
        // gated by the adherence_nudges setting.
        if Self::should_peer_ack_nudge(prev_phase, target, handle.agent_count() > 1)
            && self.storage.adherence_nudges_enabled().await
        {
            if let Some(hands) = handle.hands() {
                // Its own `system` row (0044: host injections, NULL participant).
                // It cannot ride the phase-change row above — that one is the
                // user-visible "advanced to Apply" notice and this is a separate
                // instruction to one agent, so two messages means two rows.
                match self
                    .storage
                    .post_to_channel(
                        session_id,
                        "system",
                        None,
                        MessageKind::SystemNotice.as_str(),
                        Self::APPLY_ENTRY_NUDGE,
                        None,
                    )
                    .await
                {
                    Ok(nudge) => {
                        self.bridge
                            .notify_message_persisted(Arc::from(session_id), nudge.message_id());
                        hands.deliver(&nudge).await;
                    }
                    // A missed nudge is a softer failure than a failed phase
                    // advance, so it warns rather than aborting the transition.
                    Err(e) => tracing::warn!(?e, session_id, "apply-entry nudge not persisted"),
                }
            }
        }
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
        "🔔 Entering Apply. Before you mutate: confirm your reviewer reviewed the plan — \
         pull session_doc_search(phase=\"plan\") and check their pushback landed. If it \
         hasn't, say so in chat (your turn output is forwarded to them automatically, \
         which wakes them) and do non-mutating prep meanwhile. Don't park on the USER \
         for a peer wait.";

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
    /// `control_request` in time (a future tool that blocks on stdin, a dropped
    /// interrupt), NOT a fix for a confirmed claude-code behavior. An earlier version of this comment asserted that claude-code
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
    /// testing; `holds_wakes` is the pause latch (`activity.holds_wakes()`) and
    /// `recorded` is whether the bridge got a receipt back for the answer.
    ///
    /// Read the four lines as "what does each consequence actually depend on":
    /// the halt clear on neither input, the flush on the pause latch alone, and
    /// only the two that move a receipt around depend on there being one.
    fn tray_wake(holds_wakes: bool, recorded: bool) -> TrayWake {
        TrayWake {
            clear_halt: true,
            stash: holds_wakes && recorded,
            deliver: !holds_wakes && recorded,
            flush: !holds_wakes,
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
        // OOB message is already in storage (bridge wrote it, envelope and all).
        // To actually wake the duo so they read + act on it, also: (1) clear the
        // awaiting-user halt so the duo pump resumes peer-forwarding, (2) deliver
        // the receipt so their stdin receives a wake message. We deliberately do
        // NOT call broadcast_user_message (which re-inserts) — the storage row
        // already exists. Delivered + StaleGateNeedsConfirm need no wake (the
        // agent is live, or nothing ran).
        //
        // `receipt: None` means the answer was never recorded (no storage wired,
        // or the insert failed). Which of the four steps below that suppresses —
        // and which it must NOT — is [`AppState::tray_wake`]'s decision, kept
        // pure so every combination is a value a test can compare rather than a
        // shape a test has to guess at.
        if let ResolveOutcome::AgentReceiverDroppedFellBack {
            session_id, receipt, ..
        } = &outcome
        {
            let sessions = self.sessions.lock().await;
            if let Some(handle) = sessions.get(session_id) {
                let step = Self::tray_wake(handle.activity.holds_wakes(), receipt.is_some());
                if step.clear_halt {
                    self.clear_awaiting(handle, session_id).await;
                }
                if let Some(receipt) = receipt {
                    // PAUSED gate: an answered tray question must not restart a
                    // paused duo (the user may be triaging their tray while this
                    // session stays parked). Stash it; the next `broadcast`
                    // (Send / Resume) delivers it behind the user's message.
                    if step.stash {
                        self.pending_paused_wakes
                            .lock()
                            .await
                            .entry(session_id.clone())
                            .or_default()
                            .push(receipt.clone());
                    }
                    // Human preemption, same spine as `broadcast` (issues.md
                    // #27): an answered tray card is the user speaking, so it
                    // must take effect at the next tool boundary instead of
                    // waiting out the agent's whole current turn — two same-day
                    // races in s-b69a5c01 had an agent building on premises the
                    // parked answer had already overturned. The interrupt goes
                    // BEFORE the send, mirroring `broadcast`: the pump's biased
                    // control channel writes it ahead of stdin, so each agent
                    // aborts and then reads the answer. Verified idle-harmless
                    // there (control_response{success}, process survives, next
                    // message still processed) — so no gate on the flaky `busy`
                    // signal, and no SIGKILL escalation (the answer IS the next
                    // work).
                    if step.deliver {
                        for agent in handle.agents() {
                            agent.handle.interrupt("tray-answer-preempt");
                        }
                        // Answering a tray card ends the halt exactly as a typed
                        // message does, so it goes through the ring for the same
                        // reason (rc3 D19): the receipt row is already persisted,
                        // and this releases the cycle and hands the turn to the
                        // front rather than waking everyone at once.
                        //
                        // No mentions: an answer is a pick from a list of
                        // options, not prose the user composed, so there is no
                        // `@` in it to honour (rc3 D17).
                        //
                        // **This path used to release the ring and nothing else**
                        // (rc3 D28), so the halt it answered stayed pending and
                        // the bell never cleared. It goes through the one entry
                        // point now, like every other way of responding.
                        self.user_responded(session_id, Vec::new(), true).await;
                    }
                }
                // Answering a tray card ends the halt just as a typed message
                // does, so release what the router held behind it — after the
                // answer, so the peer's held chatter lands behind it.
                if step.flush {
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
    fn tray_wake_covers_all_four_pause_and_record_combinations() {
        // Live + recorded: the answer is the user speaking, so it aborts the
        // in-flight turn instead of waiting it out (the s-b69a5c01 races — an
        // agent finishing a deliverable while its superseding answer sat unread
        // on stdin), and the router releases what the halt held.
        assert_eq!(
            AppState::tray_wake(false, true),
            TrayWake {
                clear_halt: true,
                stash: false,
                deliver: true,
                flush: true
            }
        );
        // Paused + recorded: nothing is delivered (the receipt is stashed for
        // the next broadcast), so there is nothing to preempt — and interrupting
        // or flushing here would half-release a pause the user deliberately set.
        assert_eq!(
            AppState::tray_wake(true, true),
            TrayWake {
                clear_halt: true,
                stash: true,
                deliver: false,
                flush: false
            }
        );
        // Live + UNRECORDED — the regression. The insert failing gates the send
        // and nothing else: the halt still lifts and the router still flushes,
        // because both follow from the user having answered rather than from the
        // row. Getting this wrong parked the session on a question already
        // answered, with the peer's messages queued behind the dead halt and the
        // user's own pick visible in the tray the whole time.
        assert_eq!(
            AppState::tray_wake(false, false),
            TrayWake {
                clear_halt: true,
                stash: false,
                deliver: false,
                flush: true
            }
        );
        // Paused + unrecorded: nothing to hold and nothing to send, so the halt
        // clear is the whole of what this case owes. The pause latch is left
        // exactly as the user set it.
        assert_eq!(
            AppState::tray_wake(true, false),
            TrayWake {
                clear_halt: true,
                stash: false,
                deliver: false,
                flush: false
            }
        );
    }

    #[test]
    fn tray_preempt_fires_before_the_answer_is_sent() {
        // Ordering contract, mirroring `broadcast`: the pump's biased control
        // channel writes the interrupt ahead of stdin, so the agent aborts and
        // THEN reads the answer. Sending first would let the whole turn run out
        // before the abort landed — the bug this fixes.
        //
        // Source order, because this is the ordering of two side effects inside
        // one branch and the branch needs a live subprocess to exercise —
        // `tray_wake` decides WHETHER to deliver, which is a value, but not the
        // order of the two calls that carry it out. Scoped to the `deliver`
        // block so the haystack is the code being described.
        let src = include_str!("state.rs");
        let arm = src
            .split("if step.deliver {")
            .nth(1)
            .expect("the preempt block must exist");
        // Bounded at the step that follows it, so the haystack is the block and
        // not the rest of the file — an unbounded tail would happily match an
        // `interrupt` and a `send_to_all` that live nowhere near each other.
        let arm = &arm[..arm.find("if step.flush").expect("the flush step follows it")];
        let interrupt = arm.find("interrupt(\"tray-answer-preempt\")");
        // The delivery step is the ring notify since rc3 D19 — `send_to_all`
        // fanned the receipt into every stdin, which woke participants outside
        // their turn — and since rc3 D28 it is `user_responded`, which releases
        // the ring and clears the halt as one event. The ORDERING property is
        // unchanged and is what this pins: the preempt must fire before the
        // answer is handed on, whichever call hands it on.
        let send = arm.find("user_responded(");
        assert!(
            interrupt.is_some() && send.is_some() && interrupt < send,
            "the tray-answer interrupt must fire BEFORE the answer is delivered"
        );
    }

    /// rc3 **D28**: every way of responding clears the halt AND releases the
    /// ring — and there is exactly one function that can do either.
    ///
    /// The bug this pins is not hypothetical arithmetic. Answering a tray card
    /// released the ring and left the halt row pending, so the bell never
    /// cleared; the user answered again, the agent parked another question, and
    /// the tray grew. **52 occasions in the session archive where a second tray
    /// row opened while the first was still unanswered** — the worst, one row
    /// unanswered while six stacked behind it across 53 minutes.
    ///
    /// Asserted over the SOURCE because the alternative needs a live session
    /// with a real subprocess for each of three paths. What it checks is the
    /// property that actually failed: not "does this path clear the halt" — each
    /// path looked fine on its own — but "is there more than one place that
    /// can". A fourth path added tomorrow inherits both halves or fails here.
    #[test]
    fn responding_to_the_user_is_one_function_with_no_second_way_to_do_half_of_it() {
        let src = include_str!("state.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");

        // The dotted CALL form, not the bare name: these docs name both
        // functions in prose, and a test that counted prose would measure its
        // own explanation. (It did, on the first run.)
        assert_eq!(
            prod.matches(".clear_pending_halts(").count(),
            1,
            "the halt row is cleared in exactly one place — a second call site is \
             a path that can forget the other half"
        );
        // The ring release: the bridge method, called once, from the same place.
        assert_eq!(
            prod.matches(".notify_ring_user_message(").count(),
            1,
            "the ring is released in exactly one place, for the same reason"
        );

        // And every way of responding goes through it. Three today; the count is
        // deliberately not asserted, because a fourth is fine — routing around
        // it is not.
        let callers = prod.matches("user_responded(").count();
        assert!(
            callers >= 4,
            "expected the definition plus every response path to call it, found {callers}"
        );
    }

    /// rc3 **D16**: the UI's Close button is not capability-gated, and must
    /// never become so.
    ///
    /// `close_session` gates on `Capability::CloseSession` for AGENTS as of D16,
    /// which makes "a roster where nobody holds it" a legal configuration — it
    /// means the session ends when the user says so. That configuration is only
    /// usable because the button takes a different path:
    /// `tauri_cmd::sessions::close_session` → `CoreAppState::close_session`,
    /// which consults no capability set at all. Route it through the agent gate
    /// and a session whose roster ticks nothing becomes a session nobody can
    /// end, including its owner.
    ///
    /// Asserted over both files rather than one function body, because the
    /// hazard is a check appearing ANYWHERE on the path, not in one place.
    #[test]
    fn the_users_close_button_is_not_capability_gated() {
        for (name, src) in [
            ("core::state", include_str!("state.rs")),
            ("tauri_cmd::sessions", include_str!("../tauri_cmd/sessions.rs")),
        ] {
            let prod = src
                .split("mod tests {")
                .next()
                .expect("a split always yields a first part");
            for marker in [
                "allows_tool",
                "required_for",
                "ResolvedCapabilities",
                "capability_gated",
            ] {
                assert!(
                    !prod.contains(marker),
                    "{name} consults `{marker}` — the user's close path must not read \
                     an agent capability, or a roster that ticks nobody becomes a \
                     session its owner cannot end"
                );
            }
        }
    }

    /// rc3 **D17**: only a USER message is parsed for mentions.
    ///
    /// Enforced by construction rather than by asking agents not to: the parse
    /// lives behind `resolve_mentions`, which is PRIVATE — so no module outside
    /// `core::state` can reach it, and the compiler is what says so — and inside
    /// this file it is called from exactly one place, the user's own broadcast.
    /// A participant that writes `@advisor` writes text; there is no code that
    /// could act on it.
    ///
    /// The count is what this test is for. Adding a second call site is how the
    /// rule would be lost, and it would be lost SILENTLY — every existing test
    /// would still pass, because honouring a peer's mention breaks nothing that
    /// is currently asserted. It would instead compose into the summon loop D17
    /// describes: every turn substantive, so the tally never completes, spin
    /// detection never fires, and only the 500-lap round cap ends it.
    #[test]
    fn the_mention_parse_has_exactly_one_call_site_and_it_is_the_user_path() {
        let src = include_str!("state.rs");
        // Production code only. This test names the function in its own
        // assertions, and a self-counting test is a test that measures itself.
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        // The definition, plus its one call. Anything above two is a second
        // path into the summons.
        assert_eq!(
            prod.matches("resolve_mentions(").count(),
            2,
            "mentions are parsed on the user's message and nowhere else — a \
             second call site would let a participant summon another"
        );
        // …and that call is inside `broadcast`, between the row being persisted
        // and the ring being told. Bounded at the notify so the haystack is the
        // step being described.
        let body = prod
            .split("pub async fn broadcast(")
            .nth(1)
            .expect("the user's broadcast path must exist");
        let body = &body[..body
            .find("user_responded(")
            .expect("the broadcast tells the ring, via the one response path (D28)")];
        assert!(
            body.contains("resolve_mentions("),
            "the parse belongs to the user's own message path"
        );
    }
}
