//! `AppState`: top-level handle the UI layer holds.

use crate::core::activity::ActivityTracker;
use crate::core::broadcast::broadcast_user_message;
use crate::core::close_learnings;
use crate::core::ipav::IpavPhase;
use crate::core::session::{spawn_existing_session, SessionAgent, SessionHandle};
use crate::paths::Paths;
use crate::signaling::{SignalingBridge, SignalingEvent, SignalingServer};
use crate::storage::{MessageKind, Session, Storage};
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
/// reconcile directive (if the pause came from a Stop) is prepended wire-only.
/// Nothing is "held" during a pause any more (rc3 D19 — every message is a
/// row, delivered off each participant's cursor): rows posted while paused
/// simply precede this notice in the participant's next batch, which is what
/// the text now says instead of promising a flush that does not exist.
const RESUME_NOTICE: &str = "▶ Resumed. Continue exactly where you left off — \
     finish your in-flight task. Anything posted while the session was paused \
     (peer messages, question answers) is in this same batch, above this line; \
     fold it in before proceeding.";

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

/// How long a cancel waits for an in-flight atomic op (git commit / push /
/// migration) to clear before interrupting anyway. A hung op still gets
/// cancelled — the SIGKILL fallback reaps it — but the working tree is given
/// this long to not be left half-written.
pub const ATOMIC_OP_DEFERRAL_CAP: std::time::Duration = std::time::Duration::from_secs(8);

/// Wait for `flag` (the in-atomic-tool marker) to clear, polling lock-free every
/// 100 ms, up to `cap`. Returns `(waited_ms, capped)` — `capped` when the op was
/// still running at the deadline. Pure enough to test with a flag a task
/// clears; the wait used to be inlined in the Tauri command (round 11).
pub async fn await_atomic_op_or_cap(
    flag: &std::sync::atomic::AtomicBool,
    cap: std::time::Duration,
) -> (u64, bool) {
    let started = tokio::time::Instant::now();
    let deadline = started + cap;
    let mut capped = false;
    while flag.load(std::sync::atomic::Ordering::Acquire) {
        if tokio::time::Instant::now() >= deadline {
            capped = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    (started.elapsed().as_millis() as u64, capped)
}

/// Outcome of a cancel's interrupt→SIGKILL escalation, decided AFTER the
/// interrupt window. Pure (see [`AppState::escalation_outcome`]) so the
/// honored > superseded > sigkill precedence is unit-tested without a live session.
#[derive(Debug, PartialEq, Eq)]
enum EscalationOutcome {
    /// Every agent went idle in time — the interrupt was honored, process kept.
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
/// place from the three facts that bear on it: whether the session holds wakes
/// (the pause latch), whether the bridge managed to record the answer, and
/// whether any participant is mid-turn. Pure (see [`AppState::tray_wake`]) so
/// every combination is unit-tested without a live session.
///
/// Independent flags rather than an enum of named cases, because the bug this
/// replaced was exactly a case that bundled them. Gating the whole block on a
/// recorded row skipped the halt clear along with the send, leaving the session
/// parked on a question the user had already answered.
///
/// **The `deliver` flag this struct used to carry is gone, and that is rc3
/// D34.** It interrupted every agent (`tray-answer-preempt`, issues.md #27) and
/// reset the ring — so answering a parked question threw away the holder's
/// in-flight turn: the epoch moved and its completion was discarded on arrival.
/// #27's evidence was real (s-b69a5c01: an agent finishing a deliverable while
/// its superseding answer sat unread), but the cure predates the decree that
/// **Pause is the only real interrupt**. The answer row is already persisted by
/// the time this table is consulted, and delivery is a PULL — the next handover
/// drains it — so a running ring needs nothing from us: the answer frames the
/// next turn instead of aborting the current one. Only an IDLE ring needs a
/// wake, or the answer sits unread forever.
#[derive(Debug, PartialEq, Eq)]
struct TrayWake {
    /// Lift the awaiting-user halt. Always — the user answered, and that stays
    /// true whatever happened to the row afterwards.
    clear_halt: bool,
    /// Wake an idle ring so the answer is read (`user_responded`, the D28 single
    /// entry point). Needs a receipt — waking with nothing behind it hands out a
    /// turn over an empty backlog — and needs the ring NOT running: waking a
    /// running ring is the interrupt D34 removed.
    release: bool,
}

/// Max respawn attempts in `broadcast`'s auto-heal loop before delivering
/// best-effort. Bounds a pathological respawn→stale→respawn cycle.
const BROADCAST_MAX_RESPAWNS: u32 = 3;

/// One staged tray answer: `(choice_id, the option the user picked)`. The pair
/// `send_user_response` destructures at its resolve loop.
type StagedPick = (String, String);

/// What the Stage toggle holds for one session: the typed message, and the tray
/// answers staged alongside it.
///
/// An alias rather than a struct, on purpose — this shape crosses
/// `send_user_response`, `deliver_staged` and the rehydrate read as a plain
/// tuple, and a struct would change all three signatures plus the frontend's
/// hand-written mirror for a naming gain. What was actually wrong was that
/// `Vec<(String, String)>` said nothing at any of those call sites; now the two
/// names do, and `staged_responses`' type is readable in one line.
type StagedResponse = (String, Vec<StagedPick>);

/// Boundaries in a row a stage may fail to deliver before the ring stops being
/// re-armed for it (see `AppState::staged_attempts`).
pub const STAGED_DELIVERY_MAX_ATTEMPTS: u8 = 3;

/// **How a user message arrived — the one axis that decides whether it aborts an
/// in-flight turn.**
///
/// The user's design, and the reason the Stage feature exists at all: *"staged
/// messages should never interrupt the agents. It squeezes itself in the flow
/// without interrupting anything. The Pause button is the only real interrupt."*
///
/// A typed Send is the always-typeable unblock's spine — it must take effect NOW
/// rather than queue behind a turn in flight, so it fires a warm
/// `control_request` interrupt at every agent first. A STAGED message is the
/// opposite instrument: the user chose to queue it, and `deliver_staged` already
/// waits for a turn boundary to release it. Sharing `broadcast` meant it also
/// shared the preempt, so the queued message detonated on arrival — aborting the
/// very turn it had politely waited for. The wait made it worse, not better:
/// the boundary notification is async, so by delivery time the ring has usually
/// dealt the NEXT turn, and that fresh turn is what got cut.
///
/// This is rc3 D34's decree ("Pause is the only real interrupt") reaching the
/// one path D34 did not cover. D34 deleted the `tray-answer-preempt` outright
/// because a tray click is never urgent; the preempt here cannot be deleted,
/// because a typed Send legitimately needs it — so the decision is named instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserSend {
    /// The user pressed Send (or an equivalent immediate action). Preempts.
    Typed,
    /// A staged message released at a turn boundary. Never preempts.
    Staged,
}

impl UserSend {
    /// Whether this delivery may abort an in-flight turn.
    ///
    /// Extracted so the decision is a value a test can assert rather than a
    /// branch buried in `broadcast_as` — conventions.md's "pin the WIRE" rule.
    /// The two call sites that choose a variant are pinned separately.
    pub fn preempts(self) -> bool {
        matches!(self, UserSend::Typed)
    }
}

pub struct AppState {
    pub paths: Paths,
    pub storage: Storage,
    pub bridge: Arc<SignalingBridge>,
    pub signaling_addr: SocketAddr,
    pub signaling_server: Mutex<Option<SignalingServer>>,
    pub sessions: Mutex<HashMap<String, SessionHandle>>,
    /// Serializes the spawn path in `ensure_session_started` so two
    /// concurrent calls for the same session (e.g. a double-mount of the
    /// session view firing `respawn_session` twice) can't both pass the
    /// contains_key check and spawn two rosters — the second insert
    /// would overwrite the first handle and orphan its subprocesses (untracked,
    /// so close_session can't reap them). Only the spawn path takes this; the
    /// fast already-running check short-circuits before acquiring it.
    spawn_gate: Mutex<()>,
    /// Populated from Tauri's `setup()` once the AppHandle exists. The
    /// signaling MCP server starts BEFORE Tauri setup (see main.rs ordering), so
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
    /// How many boundaries in a row `deliver_staged` has FAILED to deliver a
    /// session's stage. Reset by a delivery or an unstage. At
    /// [`STAGED_DELIVERY_MAX_ATTEMPTS`] the stage stays (content + row) but the
    /// ring is no longer re-armed for it and a system row says so — a
    /// persistent send failure otherwise re-fires at every idle boundary
    /// forever (round 8, T2-4; never observed live, closed on principle).
    staged_attempts: std::sync::Mutex<HashMap<String, u8>>,
    /// The Stage toggle's content (2026-08-15): session_id → (text, staged
    /// tray picks), written when the user toggles Stage while the ring runs,
    /// taken by [`deliver_staged`](Self::deliver_staged) when the ring
    /// reaches a boundary. The SEQUENCER holds only a flag; the content
    /// lives here so a reloaded frontend can rehydrate its toggle and an
    /// unstage is a plain remove.
    staged_responses: Mutex<std::collections::HashMap<String, StagedResponse>>,
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
    /// turn we just gave it, which is the likely one. A close for a session in
    /// this set JOINS the running epilogue (`ClosePlan::JoinInFlight` — it
    /// tears nothing down; the winner does, after the turn) and applies only
    /// the archive half of its own request (round 11).
    epilogue_in_flight: Mutex<HashSet<String>>,
    /// Sessions whose [`teardown_session`](Self::teardown_session) is between
    /// its guard block and the end of its cleanup tail.
    ///
    /// Round 7 narrowed the `sessions` lock in `teardown_session` to the
    /// remove + kill + row close, so the tail (PTY reap, tray withdrawal,
    /// policy-snapshot cleanup, bridge unregister, worktree removal) runs
    /// unlocked — and at the time `ensure_session_started` consulted neither
    /// `closed_at` (a closed session's respawn was the Archive panel's
    /// "reopen for review") nor any other marker. A `respawn_session` landing
    /// in that gap (SessionView fires one on mount) got a fresh handle whose
    /// bridge registration, policy file and PTY the tail then tore down under
    /// it. (Since round 10 `ensure_session_started` DOES refuse a closed row —
    /// reopening is the explicit `reopen_session` — but the closed marker is
    /// still what covers the tail: the row is closed under the guard, the
    /// tail runs after, and this set is what a spawn in between reads.)
    /// This set is that marker: inserted under the same guard that removes
    /// the handle, read by `ensure_session_started` under the same lock,
    /// removed once the tail has finished. Nothing is held across I/O — the
    /// alternative, holding `spawn_gate` for the whole teardown, would sit
    /// across the worktree `spawn_blocking`, which is round 7's complaint
    /// with a narrower victim set. `std::sync::Mutex`: every hold is a
    /// non-awaiting insert / contains / remove.
    closing: std::sync::Mutex<HashSet<String>>,
}

/// **The closed-row refusal** `ensure_session_started` applies (round 10, B4),
/// pure so the rule has a test seat: a row with `closed_at` set is not
/// spawnable; reopening is `AppState::reopen_session`, which clears the column
/// before it spawns.
fn refuse_closed(session_id: &str, closed_at: Option<&str>) -> Result<()> {
    match closed_at {
        Some(when) => anyhow::bail!(
            "session {session_id} is closed (since {when}); reopen it first — viewing a \
             closed session no longer respawns its participants"
        ),
        None => Ok(()),
    }
}

/// Who moved the phase — the one thing `advance_phase` must branch on.
///
/// A bool would do it and did not: the ring-release decision reads as an
/// incidental `false` at the call site, and the second caller inherited it
/// silently. The variants name the two cases so a third has to say which it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseAdvanceSource {
    /// A participant's own `advance_phase` tool call. Does NOT release the ring:
    /// the caller is mid-turn, so a release would deal a turn over an empty
    /// backlog (rc3 D28).
    Agent,
    /// The user picking a phase in the session header. RELEASES the ring: the
    /// session is halted because nobody is mid-turn, and this is the response
    /// that resumes it.
    ///
    /// Since the phase-advance vote landed (D37), its D36 escape valve — the user
    /// forcing past a stalled tally — is this same variant, not a third one: the
    /// round cap names the deadlock and points at the header control, and the
    /// transition it commits clears the stuck votes, so "who advanced" stays a
    /// single question.
    User,
}

impl PhaseAdvanceSource {
    /// Whether this advance is also a user response that should deal a turn.
    fn releases_ring(self) -> bool {
        matches!(self, Self::User)
    }
}

/// **The phase write: the in-memory move and the epoch bump, as one unit.**
///
/// ## Why this is a function and not two lines in `advance_phase`
///
/// Migration 0062 gave the phase-advance vote an epoch to close its TIME axis,
/// and justified the design by its call-site count — *"exactly ONE production
/// call site, `AppState`'s phase writer"*. That call site was never written.
/// `bump_phase_epoch` shipped with a definition, five calls across three tests in
/// its own file, and nothing else, so the column sat at 0, no vote was ever
/// invalidated by a transition, and no vote row was ever cleared (round 5, E1).
///
/// Nothing in this crate can construct an `AppState`, so a guard on
/// `advance_phase` itself could only read source text. Extracting the join gives
/// a function a test can actually call with a real `Storage` — conventions.md's
/// remedy for exactly this shape.
///
/// **The visibility is load-bearing.** This is deliberately not `pub`: 0062's
/// claim is frozen in an applied migration and can never be corrected, so the
/// code has to keep it true instead. One caller, `advance_phase`, which is the
/// phase writer 0062 names — the seam IS that writer, factored out so it can be
/// tested. Making this `pub` so an integration test can reach it would let a
/// second caller in and silently falsify an immutable artifact.
///
/// ## Ordering
///
/// The bump precedes the caller's `?`-fallible `insert_message`, and that is
/// deliberate for one specific path: if the message insert fails, `advance_phase`
/// returns `Err` with the in-memory phase already moved, and a bump placed after
/// it would leave the votes standing at the old epoch — the exact stale-vote
/// state this closes. Bumping first costs a re-vote there and never a phantom
/// advance.
///
/// That claim is scoped to the `insert_message` path and no further. A bump that
/// fails on its own is swallowed with a warning below, so the phase still moves
/// with stale votes — fail-open to precisely the behaviour that shipped before
/// this function existed, which is the right direction for a transition that
/// must not be blocked by a bookkeeping write.
///
/// Safe against eating the tally that authorized this very transition: that
/// tally is read and consumed in `signaling/bridge` before the event which
/// reaches `advance_phase` is sent. An ordering constraint, not an accident.
async fn commit_phase_transition(
    storage: &Storage,
    session_id: &str,
    ipav: &Mutex<crate::core::ipav::IpavState>,
    target: IpavPhase,
) {
    ipav.lock().await.advance(target);
    // Survives a restart (migration 0063). Before this, a session resumed at
    // Investigate whatever it had been doing, and every participant was handed
    // "Gather facts only. No Edit, Write, or mutating Bash" mid-Apply.
    if let Err(e) = storage
        .set_persisted_ipav_phase(session_id, target.tag())
        .await
    {
        tracing::warn!(?e, session_id, "persisting the IPAV phase");
    }
    // Clears every vote for this session as well as moving the epoch — both
    // halves of what a transition owes the tally. See bump_phase_epoch.
    if let Err(e) = storage.bump_phase_epoch(session_id).await {
        tracing::warn!(?e, session_id, "bumping the phase epoch");
    }
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
            app_handle: std::sync::OnceLock::new(),
            fs_watcher: std::sync::OnceLock::new(),
            pending_reconcile: Mutex::new(HashSet::new()),
            staged_responses: Mutex::new(std::collections::HashMap::new()),
            staged_attempts: std::sync::Mutex::new(HashMap::new()),
            terminals: Arc::new(crate::core::TerminalRegistry::new()),
            epilogue_in_flight: Mutex::new(HashSet::new()),
            closing: std::sync::Mutex::new(HashSet::new()),
        }
    }

    /// Is this session between its teardown's guard block and the end of its
    /// cleanup tail? See the `closing` field. A poisoned lock reads as "not
    /// closing" — fail-OPEN, the opposite of a refusal guard's usual
    /// direction, accepted because the critical sections are a `HashSet`
    /// insert / contains / remove and cannot realistically panic.
    fn is_closing(&self, session_id: &str) -> bool {
        self.closing
            .lock()
            .map(|c| c.contains(session_id))
            .unwrap_or(false)
    }

    /// Set (`true`) or clear (`false`) the closing mark. Returns whether the
    /// call CHANGED the set — so `mark_closing(id, true)` answers "am I the
    /// teardown that owns this session's tail?", which is what makes
    /// `teardown_session` idempotent under two overlapping closes.
    fn mark_closing(&self, session_id: &str, closing: bool) -> bool {
        match self.closing.lock() {
            Ok(mut set) => {
                if closing {
                    set.insert(session_id.to_string())
                } else {
                    set.remove(session_id)
                }
            }
            Err(_) => false,
        }
    }

    /// Tell the frontend — and, through it, plugins holding `list_sessions`
    /// (`PluginHost` relays it as `sessions_changed`) — that a session row now
    /// exists and its handle is registered. Called by BOTH create paths
    /// (`tauri_cmd::sessions::create_session` and `dispatch_session_inner`)
    /// once `ensure_session_started` has run, so the invalidate never races the
    /// insert. Until round 7 the only emitter was `open_session`, the external
    /// driver's entry point, which had had no caller since the driver's
    /// removal — the event was never emitted in production while two live
    /// subscribers waited for it. No-op until the `AppHandle` is set in setup.
    pub fn notify_session_created(&self, session_id: &str) {
        if let Some(app) = self.app_handle.get() {
            let _ = app.emit(
                crate::tauri_events::types::SESSION_CREATED,
                serde_json::json!({ "session_id": session_id }),
            );
        }
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
        // check and spawn duplicate rosters (one gets orphaned).
        let _gate = self.spawn_gate.lock().await;
        {
            let mut sessions = self.sessions.lock().await;
            // Read under the SAME lock `teardown_session` inserts under: a
            // teardown in progress has removed the handle (so the fast path
            // above fell through) and its tail is still unregistering the
            // session's bridge state — a spawn now would be torn down under
            // itself. Once the tail is done the marker is gone and a respawn
            // (the Archive panel's reopen-for-review) proceeds normally.
            if self.is_closing(session_id) {
                anyhow::bail!("session {session_id} is closing; retry once it has closed");
            }
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
        // **A closed row does not respawn** (round 10, B4 — the user's pick: a
        // closed session reopens on a Reopen button, not on a view). Until
        // this, viewing an archived session revived its whole roster — the
        // Archive tab's "reopen for review": four such rosters were alive at
        // once on 2026-08-18 from clicks made to copy session ids, one of them
        // idle-NUDGED post-close into burning a turn to re-close itself. The
        // one path that may spawn a roster for a closed row is
        // `reopen_session`, which clears `closed_at` FIRST and then lands here.
        // Read here rather than in the Tauri command so the auto-heal in
        // `broadcast_as` and `restart_session` inherit the refusal — a plugin
        // broadcast into a closed session must not revive it either.
        if let Ok(Some(row)) = self.storage.get_session(session_id).await {
            if let Err(refusal) = refuse_closed(session_id, row.closed_at.as_deref()) {
                return Err(refusal);
            }
        }
        // The roster seed moved into `spawn_session_handle` (B4b.2) — it is the
        // choke point every creation path shares, and this one was not: the
        // external driver's `open_session` (deleted 2026-08-17) never reached here.
        let mut handle = spawn_existing_session(
            session_id,
            &self.paths,
            self.storage.clone(),
            Arc::clone(&self.bridge),
            self.signaling_addr,
        )
        .await?;
        {
            let mut sessions = self.sessions.lock().await;
            // The spawn above ran outside the map lock (it takes seconds). A
            // teardown that started meanwhile has already closed the row and
            // is unregistering the session's bridge state: registering this
            // handle would keep subprocesses alive for a session that no
            // longer exists. Kill them and report, rather than insert.
            if self.is_closing(session_id) {
                for agent in handle.agents_mut() {
                    agent.handle.kill();
                }
                anyhow::bail!("session {session_id} closed while its agents were spawning");
            }
            self.watch_session_repo(session_id, &handle);
            sessions.insert(session_id.to_string(), handle);
        }
        // The path a RELAUNCH takes: this is where a session that was mid-stage
        // gets its message back.
        self.rehydrate_stage(session_id).await;
        Ok(())
    }

    /// **Reopen a closed session** (round 10, B4): clear the row's `closed_at`
    /// / `archived` / halt slot, spawn the roster (`--resume` off each
    /// participant's own claude session id, as any respawn does), and tell the
    /// frontend. `notify_session_created` is the right event by its own
    /// documented meaning — "a session row now exists" for `list_sessions` and
    /// the plugins' `sessions_changed` — which is exactly what a reopen changes.
    /// A row that was not closed reopens nothing and spawns nothing: the
    /// SessionView's mount respawn already covers a live session.
    pub async fn reopen_session(&self, session_id: &str) -> Result<()> {
        let moved = self.storage.reopen_session(session_id).await?;
        if !moved {
            // Not closed (or unknown): a second click that beat the view's
            // refetch, or a stale view of a row somebody else reopened. There
            // is nothing to reopen and nothing to fail — the storage half
            // returns `false` precisely so that this is harmless. Round 11:
            // this used to return an error, and the bar rendered the user's
            // double click as "Reopen failed: … not closed; nothing to reopen".
            tracing::debug!(session_id, "reopen: the row is not closed; nothing to do");
            return Ok(());
        }
        tracing::info!(session_id, "session reopened on the user's button; respawning its roster");
        let started = self.ensure_session_started(session_id).await;
        // The row moved either way, so the frontend is told BEFORE the spawn
        // result is returned: the dashboard lists the session again, and a
        // failed spawn still surfaces — as the bar's inline error here, and as
        // the SessionView's own retry banner once its `get_session` refetches.
        self.notify_session_created(session_id);
        started
    }

    /// Force-restart a session's roster: evict the live handle (killing every
    /// agent) and re-spawn from the CURRENT config. Agent overrides + the
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
    /// - **immediate** kill of every agent's current incarnation (today's path)
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
        // The pause command goes to the ring OUTSIDE the `sessions` lock (see
        // below); reaching this far past the early return is what decides it,
        // so no flag is needed (round 10 dropped a `pause_ring` that could only
        // ever be `true`).
        let deferred = {
            let mut sessions = self.sessions.lock().await;
            let Some(handle) = sessions.get_mut(session_id) else {
                return Ok(CancelOutcome::Done); // not live → no-op
            };
            // Mark Cancelling FIRST → the UI shows "Cancelling…" + keeps the
            // input locked for the whole kill window (immediate or deferred).
            // Then latch the pause (that order — see set_paused's ORDERING
            // note): once every pump goes idle the tracker auto-clears
            // cancelling and the session lands in Paused, not Idle — input
            // enabled, session held until the user steers, resumes, or closes.
            handle.activity.set_cancelling(true);
            handle.activity.set_paused(true);
            // **And the RING, which until now was never told** (B1-F8, the
            // user's call 2026-08-16). The latch above is the UI's; without this
            // the interrupt ends the holder's turn, its `result` arrives as an
            // ordinary completion, and the ring deals the NEXT participant under
            // the Paused banner — a fresh turn started by a Stop.
            //
            // Sent BEFORE the interrupt goes out (below, past this block),
            // which is what makes the ordering safe: the completion the
            // interrupt causes cannot reach the ring ahead of a command queued
            // before the agent was even signalled.
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
        // Outside the `sessions` lock: `notify_ring_pause` takes the bridge's
        // own, and holding two is how a lock-order hazard starts. (The one
        // deliberate exception to that rule is `broadcast_as`, which keeps the
        // guard through delivery so the stale-check and the deliver cannot be
        // split by a respawn — it says so at its `break sessions`.)
        self.bridge.notify_ring_pause(session_id).await;
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
    /// [`broadcast`](Self::broadcast) path — which clears `paused` and consumes
    /// any pending post-cancel reconciliation directive; rows posted during the
    /// pause are already on the channel and precede the notice in each
    /// participant's next batch (nothing is held or flushed since rc3 D19).
    /// Auto-heals a SIGKILLed (stale) session via broadcast's respawn loop.
    /// No-op when the session isn't live or isn't paused (stale click).
    pub async fn resume_session(&self, session_id: &str) -> Result<()> {
        {
            let sessions = self.sessions.lock().await;
            let Some(handle) = sessions.get(session_id) else {
                return Ok(()); // not live → nothing to resume
            };
            if !handle.activity.is_paused() {
                return Ok(()); // not paused → stale click; don't nudge the session
            }
        }
        self.broadcast(session_id, RESUME_NOTICE).await
    }

    /// **The Stop button, end to end** (round 11): decide with
    /// [`cancel_session_turn`](Self::cancel_session_turn), then drive the
    /// interrupt → SIGKILL escalation OFF-THREAD — including the atomic-op
    /// deferral, which used to live in the Tauri command: the 8 s cap, the
    /// 100 ms poll and the `deferred_ms` telemetry were policy no test could
    /// reach and no non-Tauri caller could get right. Returns as soon as the
    /// escalation is detached, so the command (and any other caller) returns
    /// immediately and the UI keeps showing "Pausing…" for the window.
    ///
    /// `pressed_at` is stamped by the caller at the top, so `cancel_events.
    /// pressed_at` is when the USER acted, not when the escalation finished —
    /// the gap between the two is precisely what a user experiences as "Stop
    /// didn't do anything".
    pub async fn cancel_and_escalate(self: &Arc<Self>, session_id: &str, pressed_at: String) -> Result<()> {
        match self.cancel_session_turn(session_id).await? {
            CancelOutcome::Done => {}
            CancelOutcome::Interrupting => {
                // The common path: interrupt every agent and drive the ~2s
                // SIGKILL escalation off-thread. Detached so this returns at
                // once. An `Arc<Self>` (not `&self`) so the task can re-acquire
                // `sessions` without holding it across the wait.
                let this = Arc::clone(self);
                let sid = session_id.to_string();
                tokio::spawn(async move {
                    this.interrupt_then_escalate(&sid, &pressed_at, 0, false).await;
                });
            }
            CancelOutcome::Deferred(flag) => {
                // An edit-capable participant is mid an atomic op. Poll the flag
                // lock-free until it clears, THEN interrupt+escalate — with a
                // hard cap so a hung op still gets cancelled (the SIGKILL
                // fallback reaps it). Detached, like the common path.
                let this = Arc::clone(self);
                let sid = session_id.to_string();
                tokio::spawn(async move {
                    let (deferred_ms, capped) =
                        await_atomic_op_or_cap(&flag, ATOMIC_OP_DEFERRAL_CAP).await;
                    if capped {
                        tracing::warn!(
                            session_id = %sid,
                            "cancel: atomic-op deferral hit the cap — interrupting now"
                        );
                    }
                    // Recorded because this window is the leading candidate for
                    // "Stop kept working": it delays the interrupt by up to the
                    // cap before anything is even sent.
                    this.interrupt_then_escalate(&sid, &pressed_at, deferred_ms, capped)
                        .await;
                });
            }
        }
        Ok(())
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
        let (activity, cancel_superseded, hands_queued, eyes_queued) = {
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
            let mut eyes_queued: Option<bool> = None;
            for agent in handle.agents().filter(|a| !a.edits_files()) {
                let queued = agent.interrupt("cancel");
                if !queued {
                    tracing::warn!(session_id, slug = %agent.slug, "cancel: peer interrupt was NOT queued");
                }
                eyes_queued = Some(eyes_queued.unwrap_or(true) && queued);
            }
            let hands_queued = handle
                .hands()
                .is_some_and(|h| h.interrupt("cancel"));
            if !hands_queued {
                tracing::warn!(session_id, "cancel: HANDS interrupt was NOT queued");
            }
            (
                Arc::clone(&handle.activity),
                Arc::clone(&handle.cancel_superseded),
                hands_queued,
                eyes_queued,
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
            slot0_interrupt_queued: Some(hands_queued),
            slot1_interrupt_queued: eyes_queued,
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
                // landing in Paused after their message would re-halt the session
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

    /// The kill half of a cancel: tear down every agent NOW and queue the
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
        // One short hold decides both facts: is an epilogue ALREADY in flight
        // for this session (round 11 — asked for every decision, because the
        // epilogue's own agent closing on the turn it was given decides
        // SkipBusy/SkipAlreadyHandled, never Run), and — only when the decision
        // wants a turn — did this call win the claim. `insert` returns false
        // when the id is already there, so the claim is atomic, and the
        // decision runs outside the hold — no `epilogue_in_flight` →
        // `sessions` lock nesting.
        let (claimed, in_flight) = {
            let mut set = self.epilogue_in_flight.lock().await;
            if set.contains(id) {
                (false, true)
            } else if decision == close_learnings::Epilogue::Run {
                (set.insert(id.to_string()), false)
            } else {
                (false, false)
            }
        };
        // Exhaustive on purpose — see `ClosePlan`. Deleting the epilogue arm
        // has to be a compile error, not a silently inert feature.
        match close_learnings::plan(decision, claimed, in_flight) {
            close_learnings::ClosePlan::TearDownNow => {
                self.teardown_session(id, Some(archive)).await
            }
            close_learnings::ClosePlan::JoinInFlight => {
                // The other close owns the teardown; doing it here kills the
                // learnings turn it just started (B2-5). The row is already
                // closed by that path, so the user's Close has taken effect —
                // except the ARCHIVE half (round 11): the winner may have closed
                // unarchived (the agent's tool passes `archive = false`), and
                // `close_session` cannot re-apply the flag on a closed row, so
                // a "close and archive" that joins applies it here.
                if archive {
                    match self.storage.archive_session(id).await {
                        Ok(true) => {
                            self.bridge.notify_session_closed(id.to_string());
                        }
                        Ok(false) => {}
                        Err(e) => {
                            tracing::warn!(?e, session_id = %id, "close: archive on join failed");
                        }
                    }
                }
                tracing::debug!(
                    session_id = %id,
                    archive,
                    "close: an epilogue is already in flight; leaving the teardown to it"
                );
                Ok(())
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
                        // WAIT FOR THE TURN, THEN FOR THE LAP — two separate
                        // ways to answer before the agents have said anything,
                        // and D15 shipped with both.
                        //
                        // 1. This used to wait `await_both_idle` immediately,
                        //    on a comment claiming `broadcast` marks every agent
                        //    busy before returning. It has not since `519cbba` —
                        //    the ring is the one busy-true writer, and
                        //    `broadcast_marks_nobody_busy` pins exactly that. So
                        //    the wait was answered by the idle state the
                        //    broadcast itself left behind, in 7ms, and every
                        //    close since recorded `Declined` while
                        //    `teardown_session` killed the agents as the ring
                        //    dealt the turn. No epilogue had ever run.
                        // 2. Arming on the busy edge is necessary and not
                        //    sufficient: `await_both_idle` returns on the FIRST
                        //    idle poll, and `hand_turn_to` deliberately leaves
                        //    "the sub-second gap between a completion and the
                        //    next handover" — so on an N≥2 roster the armed wait
                        //    could return in the gap after the first turn and
                        //    kill the participant about to write.
                        //
                        // Hence: arm on the turn starting, then wait for the LAP
                        // to end (the halt slot refilling, or sustained idle
                        // past the handover gap). The CL-write flag is read once
                        // afterwards — never as an early exit, which would tear
                        // down mid-lap and reintroduce (2) for whoever holds the
                        // turn at that moment.
                        let arm_deadline = tokio::time::Instant::now()
                            + close_learnings::CLOSE_EPILOGUE_ARM_TIMEOUT;
                        if !activity.await_turn_started(arm_deadline).await {
                            Outcome::NeverStarted
                        } else {
                            let deadline = tokio::time::Instant::now()
                                + close_learnings::CLOSE_EPILOGUE_TIMEOUT;
                            if activity
                                .await_lap_end(deadline, ActivityTracker::LAP_QUIET_WINDOW)
                                .await
                            {
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
        if crate::core::post_system_notice(
            &self.storage,
            Some(&self.bridge),
            id,
            MessageKind::SystemNotice,
            body,
            None,
        )
        .await
        .is_none()
        {
            tracing::warn!(
                session_id = %id,
                ?outcome,
                "close epilogue: outcome row not posted; the close has no on-screen account"
            );
        }
    }

    /// Kill the session's processes and drop every trace of it from memory.
    ///
    /// `archive` is `Some(_)` on the direct path (this call closes the row too)
    /// and `None` when [`Self::close_session`]'s D15 epilogue arm already
    /// closed it — the row is closed once, before the epilogue, so the UI never
    /// waits on a learnings turn.
    async fn teardown_session(&self, id: &str, archive: Option<bool>) -> Result<()> {
        // **The global `sessions` lock is held for the remove + kill only.**
        // It used to be taken at the top and held to the end — across the PTY
        // reap, the DB close, the tray withdrawal, the policy-file cleanup, the
        // bridge unregister and a `spawn_blocking(git worktree remove)` — so
        // for the whole of one session's close every other session's
        // `broadcast_as`, `halt_declared`, `resolve_choice` wake,
        // `current_phase` and `get_session_runtime` blocked behind it (round 7).
        // Nothing below needs the map: the handle is out of it, the kills are
        // queued, and the rest is per-session I/O — EXCEPT the row close, which
        // stays under the guard on purpose: "removed from the map" and "closed
        // in storage" must be one step (one UPDATE; the slow work stays
        // outside). What CAN respawn a session in the gap after this block is
        // not `broadcast` (its `Deliver` step errors on a missing handle) but
        // `ensure_session_started` — `respawn_session` fires it on every
        // SessionView mount — so the guard also marks the session `closing`,
        // under this same lock, and `ensure_session_started` refuses the id
        // until the tail below has finished (round 8). The marker is cleared
        // at the end of this function on every path — and it is what makes
        // this function idempotent: two overlapping closes (the epilogue's
        // detached teardown and a user's Close in the same instant; two
        // `TearDownNow` plans) both reach here, and only the one that SET the
        // mark owns the tail. The other returns at once — the handle is
        // already out of the map, the row is being closed, and a second tail
        // would clear the mark while the first is still unregistering
        // (review, round 8).
        {
            let mut sessions = self.sessions.lock().await;
            if !self.mark_closing(id, true) {
                tracing::debug!(session_id = %id, "teardown already in progress; joining it");
                return Ok(());
            }
            if let Some(mut handle) = sessions.remove(id) {
                for agent in handle.agents_mut() {
                    agent.handle.kill();
                }
            }
            if let Some(archive) = archive {
                if let Err(e) = self.storage.close_session(id, archive).await {
                    // Not torn down after all: leave the session respawnable.
                    self.mark_closing(id, false);
                    return Err(e);
                }
            }
        }
        // A stage the user left on this session can never deliver now — drop
        // the content and clear the durable slot, so a relaunch does not
        // rehydrate a message for a session that no longer exists.
        self.staged_responses.lock().await.remove(id);
        if let Err(e) = self.storage.set_staged_message(id, None).await {
            tracing::warn!(?e, session_id = %id, "clearing the closed session's staged message failed");
        }
        // Stop live-watching this session's working repo.
        if let Some(watcher) = self.fs_watcher.get() {
            watcher.remove_repo(id);
        }
        // Reap the session's PTY terminal alongside the agent subprocesses.
        self.terminals.kill_and_remove(id).await;
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
        // Tail done: the session may be respawned again (Archive panel,
        // "reopen for review").
        self.mark_closing(id, false);
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
    /// "callers that also answer those call the halt-clear separately",
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
    async fn user_responded(
        &self,
        session_id: &str,
        mentions: Vec<i64>,
        release_ring: bool,
        clear_halt: bool,
    ) {
        // The halt row FIRST. If the ring release panicked or the process died
        // between the two, a cleared row and a halted ring is a session the next
        // message fixes; a released ring and a pending row is the bug above, and
        // it is invisible.
        // `clear_halt = false` is the GATE-ANSWER case (rc3 D35, found live in
        // `s-86a81478`): approving a parked command is answering THAT approval,
        // not the session's halt — the first cut cleared the slot on any
        // resolve, so the one HALT the user saw was wiped by them approving an
        // unrelated gate, and the ring released straight back into work.
        if clear_halt {
            match self.storage.clear_session_halt(session_id).await {
                Ok(true) => {
                    self.bridge.notify_halts_cleared(session_id.to_string());
                }
                Ok(false) => {}
                Err(e) => tracing::warn!(?e, session_id, "clear_session_halt failed"),
            }
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
    /// arrives from the plugin proxy, and a message REFUSED for naming a
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

    /// The user-message paste gate: `Some(refusal)` when `len` exceeds the
    /// wire clamp, `None` when the message may pass.
    ///
    /// The threshold IS [`crate::storage::WIRE_BODY_CLAMP_BYTES`] — one
    /// constant, two layers. This gate refuses at the door with guidance the
    /// user can act on; the wire clamp truncates whatever gets past every
    /// door (agent dumps, rows already on record). Sharing the constant means
    /// an ACCEPTED user message is never truncated on delivery.
    fn oversized_message_refusal(len: usize) -> Option<String> {
        const CAP: usize = crate::storage::WIRE_BODY_CLAMP_BYTES;
        (len > CAP).then(|| {
            format!(
                "message is {len} bytes — the per-message cap is {CAP} (~50k \
                 tokens). A paste this size wedges the participants' context \
                 windows unrecoverably (s-f6a441ff: a 2.9 MB paste ended the \
                 session). Save the bulk to a file in the working repo and send \
                 the path instead — participants read files selectively."
            )
        })
    }

    pub async fn broadcast(&self, session_id: &str, text: &str) -> Result<()> {
        self.broadcast_as(session_id, text, UserSend::Typed).await
    }

    /// [`broadcast`](Self::broadcast) with the arrival mode named explicitly.
    /// See [`UserSend`] for why the mode exists and what it decides.
    pub async fn broadcast_as(
        &self,
        session_id: &str,
        text: &str,
        send: UserSend,
    ) -> Result<()> {
        // The paste gate, FIRST — before any respawn work is spent on a
        // message that will be refused. s-f6a441ff: a 2,977,078-byte paste of
        // prod logs was accepted, delivered, and lodged in both participants'
        // subprocess transcripts; every prompt after it exceeded even the 1M
        // window and the session died volleying "Prompt is too long". The
        // refusal carries the fix the user reached for themselves that day
        // ("i've put it in temp.md") — a file, referenced by path.
        if let Some(refusal) = Self::oversized_message_refusal(text.len()) {
            anyhow::bail!(refusal);
        }
        // Auto-heal: if the session went stale (e.g. an agent's stdin pump died,
        // closing the public input channel — a now-deaf agent that would silently
        // drop this message), evict + respawn it before delivering so the user's
        // message isn't lost. The check and the respawn can't be atomic
        // (`ensure_session_started` needs the lock, so we must drop it), so the
        // session could go stale again in the window between them — re-check under
        // the SAME lock hold we deliver under, respawning up to a few times. The
        // healthy `break sessions` keeps that hold through delivery (no TOCTOU);
        // an absent session breaks too → the `ok_or` below errors as before.
        // **This is the one deliberate exception to "never hold `sessions`
        // across a bridge/storage await"** (`cancel_session_turn` states the
        // rule): the hold spans the row insert, the mention resolve and the
        // ring release below, and it is kept because a respawn slipping between
        // the stale-check and the delivery is exactly the deaf-agent loss the
        // check exists to prevent (round 10 recorded the trade-off; the cost is
        // that other AppState callers wait a few DB round-trips per user send).
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
        // Clear the awaiting flag BEFORE posting the user's reply so the
        // activity state leaves `AwaitingUser`.
        self.clear_awaiting(handle, session_id).await;
        // The ring's RELEASE is not here — it rides the notify below, AFTER the
        // row is posted. `clear_awaiting` only lowers a flag; the sequencer
        // halted on `HaltDeclared` and a user message is the only thing that
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
        // below runs the session normally (a Send while Paused = clarify/steer; the
        // Resume button routes here too, as a resume-notice broadcast).
        handle.activity.set_paused(false);
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
        // idle agent-to-agent lap). Fire a warm control_request interrupt at
        // every agent BEFORE delivering. Verified harmless when idle
        // (control_response{success}, process survives, next message still
        // processed), and it aborts the in-flight turn when busy — so we don't
        // gate on the flaky activity `busy` signal. The pump's biased control
        // channel writes this ahead of the message on stdin, so each agent aborts
        // then reads the new message. No SIGKILL escalation (unlike cancel) — the
        // message IS the next work, and the process stays warm (no --resume).
        //
        // **A STAGED message takes none of this.** It is a queued message by the
        // user's own choice, released at a turn boundary — preempting it aborts
        // the turn it waited for. See [`UserSend`].
        if send.preempts() {
            for agent in handle.agents() {
                agent.interrupt("user-preempt");
            }
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
        self.user_responded(session_id, mentions, true, true).await;
        // A user prompt re-arms the idle-unflagged watchdog's once-per-window
        // nudge (and its >0 count marks the session as having a task at all).
        handle
            .user_broadcasts
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        // NOBODY is marked busy here. The ring hands the turn to the front of
        // the rotation and `hand_turn_to` marks THAT participant — the one
        // busy-true writer (rc3 D19b). This used to pre-mark every agent, which
        // read fine while the ring rotated (each pre-mark was laundered into a
        // real turn when its holder's deal came) and lied the moment the ring
        // stopped early: in s-ff729daa the second participant was never dealt a
        // turn before a halt, no turn end ever cleared its pre-mark, and the
        // input stayed locked under the HALT banner until the user force-paused
        // — three times in four minutes. The stale flag also read busy+silent to
        // the stall watchdog, which called a rightfully quiet participant
        // "stalled". A flag only the ring sets is a flag a stopped ring has
        // always cleared; `broadcast_marks_nobody_busy` pins this loop deleted.
        self.bridge
            .notify_message_persisted(Arc::from(session_id), id);
        Ok(())
    }

    /// **One Send, one event: the typed message plus every staged tray answer
    /// (rc3 D34).**
    ///
    /// The user's design, verbatim: *"remove the send button on tray items. On
    /// Halt, sending a message will also send all of the answers on all tray
    /// items."* Agreed for choices and not for approvals — an approval is
    /// synchronously blocked and answers on the spot in the gate; a choice is
    /// parkable, so its answer can travel with the message.
    ///
    /// What this replaces: each tray pick resolved on click, and the first one
    /// fired `user_responded` — so answering the first of three questions while
    /// halted released the ring and locked the box (D33) before the other two
    /// could be answered or a message added. Under this, picks stage in the UI
    /// and arrive here together; the answers are recorded FIRST and the typed
    /// message posts LAST, so it is the freshest row in the batch the released
    /// ring drains — the buried-user-message work (37 of 44 arrived buried)
    /// established that the last line frames the turn.
    ///
    /// Exactly one release fires, whichever branch runs: `broadcast` when there
    /// is text (it releases via its notify), else one `user_responded`. A pick
    /// that fails to resolve stays pending in the tray — visible, answerable
    /// again — rather than failing the whole Send.
    pub async fn send_user_response(
        &self,
        session_id: &str,
        text: &str,
        picks: Vec<StagedPick>,
        send: UserSend,
    ) -> Result<()> {
        // Same paste gate as `broadcast`, checked BEFORE the picks resolve —
        // a Send refused halfway would consume the staged answers and drop the
        // message, leaving the user unsure what landed. Refuse whole instead:
        // picks stay staged, the message stays in the box, the error says why.
        if let Some(refusal) = Self::oversized_message_refusal(text.len()) {
            anyhow::bail!(refusal);
        }
        let mut answered = 0usize;
        for (choice_id, picked) in picks {
            // Straight to the bridge: record the answer, skip the wake. The
            // per-pick wake machinery (`resolve_choice`) is exactly what made
            // the first answer release the ring ahead of the rest.
            match self
                .bridge
                .resolve_choice_confirmable(&choice_id, picked, false)
                .await
            {
                Ok(crate::signaling::ResolveOutcome::StaleGateNeedsConfirm { .. }) => {
                    // Staged picks are never approvals — the gate owns those and
                    // answers them on the spot — so hitting the stale gate means
                    // the row changed shape under us. Leave it pending rather
                    // than approve anything by side effect.
                    tracing::warn!(
                        session_id,
                        choice_id = %choice_id,
                        "staged pick hit the stale gate; left pending"
                    );
                }
                Ok(_) => answered += 1,
                Err(e) => tracing::warn!(
                    ?e,
                    session_id,
                    choice_id = %choice_id,
                    "staged pick failed to resolve; left pending"
                ),
            }
        }
        if !text.trim().is_empty() {
            // The message is the release, and it posts after the answers — the
            // last row of the batch, framing the turn. `send` rides through:
            // this is the branch a staged delivery takes, and it is where the
            // preempt lived.
            return self.broadcast_as(session_id, text, send).await;
        }
        if answered == 0 {
            return Err(anyhow::anyhow!(
                "nothing to send: empty message and no answers resolved"
            ));
        }
        // Answers alone. Mirror the slice of `broadcast` a release needs: the
        // engagement bump (the idle watchdog re-arms on user input) and the
        // pause latch down (a Send while paused is the steer). Nothing to drop
        // — the rows are persisted and the ring replays them off cursors.
        {
            let sessions = self.sessions.lock().await;
            if let Some(handle) = sessions.get(session_id) {
                handle
                    .user_broadcasts
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                handle.activity.set_paused(false);
            }
        }
        self.user_responded(session_id, Vec::new(), true, true).await;
        Ok(())
    }

    /// Put a session's staged message back in the slot after a relaunch, and
    /// tell the fresh ring it is there.
    ///
    /// Both halves are needed and they are separate: the CONTENT lives in
    /// `staged_responses` (what `deliver_staged` sends) and the FLAG lives in
    /// the ring (what makes a boundary park and emit `StagedDeliveryDue`). A
    /// restart rebuilds neither, so before this the durable row would have sat
    /// there with nothing watching it.
    ///
    /// The picks are deliberately not restored: they are durable tray rows, and
    /// `send_user_response` re-derives them at delivery — which is also what
    /// keeps a pick staged after the message from being lost.
    async fn rehydrate_stage(&self, session_id: &str) {
        let text = match self.storage.staged_message(session_id).await {
            Ok(Some(text)) => text,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(?e, session_id, "reading the staged message failed");
                return;
            }
        };
        tracing::debug!(
            session_id,
            bytes = text.len(),
            "restoring a staged message across the restart"
        );
        self.staged_responses
            .lock()
            .await
            .insert(session_id.to_string(), (text, Vec::new()));
        self.bridge.notify_ring_stage(session_id, true).await;
    }

    /// **Stage a user response** (the Stage toggle, 2026-08-15): hold the
    /// typed message + the staged tray picks for delivery at the ring's next
    /// turn boundary. Staging changes WHEN the user may compose, not when a
    /// message may land — delivery arrives as an ordinary user message
    /// between turns, and Pause stays the only interrupt. Re-staging
    /// replaces the previous stage (one slot, like the halt).
    pub async fn stage_user_response(
        &self,
        session_id: &str,
        text: &str,
        picks: Vec<StagedPick>,
    ) -> Result<()> {
        // The paste gate applies at STAGE time so the user hears the refusal
        // immediately, not minutes later at a boundary they aren't watching.
        if let Some(refusal) = Self::oversized_message_refusal(text.len()) {
            anyhow::bail!(refusal);
        }
        if text.trim().is_empty() && picks.is_empty() {
            anyhow::bail!("nothing to stage: empty message and no staged answers");
        }
        self.staged_responses
            .lock()
            .await
            .insert(session_id.to_string(), (text.to_string(), picks));
        // A fresh stage is a fresh set of delivery attempts.
        if let Ok(mut a) = self.staged_attempts.lock() {
            a.remove(session_id);
        }
        // **And durably.** The slot used to be process memory only, so a
        // relaunch mid-stage dropped what the user had typed — silently, while
        // the composer rehydrated to "Staged ✓" from the same empty map. The
        // picks are not persisted with it: they are already durable tray rows,
        // and re-deriving them at delivery is what keeps the snapshot equal to
        // the tray.
        if let Err(e) = self.storage.set_staged_message(session_id, Some(text)).await {
            tracing::warn!(
                ?e,
                session_id,
                "the staged message was not persisted; it will be lost if the app restarts"
            );
        }
        self.bridge.notify_ring_stage(session_id, true).await;
        Ok(())
    }

    /// The user un-toggled Stage to edit: drop the content and lower the
    /// ring's flag. A boundary that already fired finds nothing to deliver
    /// and the ring simply yields — an open box over an idle ring, which is
    /// exactly what an editing user wants.
    pub async fn unstage_user_response(&self, session_id: &str) {
        self.staged_responses.lock().await.remove(session_id);
        if let Ok(mut a) = self.staged_attempts.lock() {
            a.remove(session_id);
        }
        if let Err(e) = self.storage.set_staged_message(session_id, None).await {
            tracing::warn!(?e, session_id, "the staged message was not cleared");
        }
        self.bridge.notify_ring_stage(session_id, false).await;
    }

    /// The staged content, if any — the frontend rehydrates its toggle from
    /// this after a reload.
    pub async fn staged_response(
        &self,
        session_id: &str,
    ) -> Option<StagedResponse> {
        self.staged_responses.lock().await.get(session_id).cloned()
    }

    /// The ring reached a boundary with a stage pending
    /// (`SignalingEvent::StagedDeliveryDue`, routed here by main.rs): take
    /// the content and deliver it through [`send_user_response`] — the ONE
    /// send path, so a staged send and a typed send are the same event
    /// (answers first, message last, one release; rc3 D34/D28). On failure
    /// the content is restored and the ring re-flagged, so the user's
    /// message survives to the next boundary instead of vanishing.
    pub async fn deliver_staged(&self, session_id: &str) {
        let Some((text, picks)) = self.staged_responses.lock().await.remove(session_id) else {
            return;
        };
        // `Staged` is the whole point of this path: the user queued this message
        // so it would NOT cut a turn. See [`UserSend`].
        match self
            .send_user_response(session_id, &text, picks.clone(), UserSend::Staged)
            .await
        {
            Ok(()) => {
                // Delivered: the slot is empty again, in memory and on the row.
                if let Err(e) = self.storage.set_staged_message(session_id, None).await {
                    tracing::warn!(
                        ?e,
                        session_id,
                        "the delivered stage was not cleared; a restart would re-stage it"
                    );
                }
                if let Ok(mut a) = self.staged_attempts.lock() {
                    a.remove(session_id);
                }
                self.bridge.notify_stage_delivered(session_id);
            }
            Err(e) => {
                let attempts = match self.staged_attempts.lock() {
                    Ok(mut a) => {
                        let n = a.entry(session_id.to_string()).or_insert(0);
                        *n = n.saturating_add(1);
                        *n
                    }
                    Err(_) => 1,
                };
                // The content survives either way — the user's message must not
                // vanish on a send failure — so the next boundary can retry…
                self.staged_responses
                    .lock()
                    .await
                    .insert(session_id.to_string(), (text, picks));
                if attempts < STAGED_DELIVERY_MAX_ATTEMPTS {
                    tracing::warn!(
                        ?e,
                        session_id,
                        attempts,
                        "staged delivery failed; restoring the stage for the next boundary"
                    );
                    self.bridge.notify_ring_stage(session_id, true).await;
                } else {
                    // …but not forever: an idle ring re-fires the boundary at
                    // once, so a persistent failure would spin. Stop re-arming,
                    // say so in the channel; an unstage or a resend re-arms.
                    tracing::warn!(
                        ?e,
                        session_id,
                        attempts,
                        "staged delivery failed repeatedly; the stage stays but the ring is \
                         no longer re-armed for it"
                    );
                    crate::core::post_system_notice(
                        &self.storage,
                        Some(&self.bridge),
                        session_id,
                        MessageKind::SystemNotice,
                        format!(
                            "[System: your staged message could not be delivered \
                             ({attempts} tries; last error: {e}). It is still staged — \
                             unstage and send it again to retry.]"
                        ),
                        None,
                    )
                    .await;
                }
            }
        }
    }

    /// Set IPAV phase + post a host "phase advanced to X" row so every
    /// participant sees the transition naturally. Also clears any awaiting-user
    /// halt — an agent that fired `request_phase_advance` has effectively been
    /// answered by the chip click, so the session should resume.
    pub async fn advance_phase(
        &self,
        session_id: &str,
        target: IpavPhase,
        source: PhaseAdvanceSource,
    ) -> Result<()> {
        let sessions = self.sessions.lock().await;
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("no live session {session_id}"))?;
        // A2 (adherence): remember the phase we're leaving, to detect Plan→Apply.
        let prev_phase = handle.ipav.lock().await.current_phase;

        self.clear_awaiting(handle, session_id).await;
        // **Whether this releases the ring depends on WHO advanced**, and that
        // is the whole reason `source` exists.
        //
        // An AGENT self-advance must not release: the participant is mid-turn —
        // it just called `advance_phase` — so waking the ring would hand out a
        // turn over an empty backlog (rc3 D28).
        //
        // A USER advance is the opposite case by construction. The session is
        // halted precisely because nobody is mid-turn, and the user picking a
        // phase IS the response that resumes it. Reusing the agent's `false`
        // here cleared the halt row, cleared the awaiting badge, advanced the
        // phase, persisted the notice — and dealt no turn, leaving a session
        // that looks answered and does nothing until the idle watchdog's grace
        // elapses. Found in review before it shipped; the premise in the
        // paragraph above is stated as a fact about the CALLER, and this path
        // is a caller it was never true for.
        self.user_responded(session_id, Vec::new(), source.releases_ring(), true)
            .await;

        // The in-memory move AND the epoch bump, together. Both callers of this
        // function reach it — main.rs's agent path and the user's phase picker —
        // and they are the only two, so one seam here is complete rather than
        // one-of-N. That matters: "reset on the right set of events" is the
        // predicate shape 0062's own text says this repo has shipped wrong five
        // times, and here the set is provably one. See commit_phase_transition
        // for the ordering and the visibility argument.
        commit_phase_transition(&self.storage, session_id, &handle.ipav, target).await;

        // Synthetic phase-change message in storage. No envelope: the wire is
        // the notice byte for byte, because `transition_notice()` already
        // carries its own `[PHASE: X]` and a phase envelope would double-tag it.
        //
        // Host-authored (`origin = "system"`), so participants read it as
        // `[system] [PHASE: X] …`. It went through the user-row writer until
        // round 7 and every participant read the transition as the USER's words
        // — the notice text is bot-hq's, and the transition may be an agent
        // vote's doing (D37); the user typed none of it.
        //
        // The `&'static str` goes in directly; it used to be pre-`to_string`d
        // because the wire moved the owned copy, and that consumer is gone.
        // A failed insert is not fatal here: the phase HAS moved (committed
        // above) and the participants learn it from the envelope of the next
        // row they read.
        crate::core::post_system_notice(
            &self.storage,
            Some(&self.bridge),
            session_id,
            MessageKind::PhaseChange,
            target.transition_notice(),
            None,
        )
        .await;
        // Posted as a row the executor reads at its next dealt turn.
        //
        // The reviewer is not WOKEN for it (issues.md #8). Waking the reviewer
        // on a phase transition bought nothing and cost a turn: it had no new
        // content to review, so the turn was a "holding for the executor's
        // plan" acknowledgment — and each one burned a slot of the router-era
        // `VOLLEY_HARD_CAP` budget that #24 showed was being exhausted before
        // substantive reviews could get through. Measured in this very session:
        // filler turns landing 7-45 s after each phase change, 40-116 chars
        // apiece.
        //
        // It loses no information: the phase-change row sits on the channel and
        // it reads it off its own cursor at its next dealt turn, alongside the
        // next message that actually has something in it. Provider-limit peer
        // notices are a different path (a host row plus a halt) and stay.
        // **No direct write.** The row is persisted, so every participant reads
        // it off its own cursor when the ring next deals it a turn — which is
        // also when it can act on it. Writing into a stdin here did something
        // else: the participant is mid-turn (it just called `advance_phase`), so
        // the message opened a generation the ring had not dealt, whose
        // completion carries a stale epoch and is discarded. The row was then
        // delivered a second time by the ring, off the cursor. `user_responded`
        // above deliberately passes `release_ring = false` — no wake is wanted
        // here (rc3 D28) — and this now matches that intent instead of
        // contradicting it.

        // A2 (adherence): the peer-ack the prompts don't mechanically enforce.
        // On the Plan→Apply boundary in a session with a peer, remind the executor
        // (HANDS) to confirm its reviewer's plan review before mutating.
        // Executor-only; no-op solo; gated by the adherence_nudges setting.
        if Self::should_peer_ack_nudge(prev_phase, target, handle.agent_count() > 1)
            && self.storage.adherence_nudges_enabled().await
        {
            // Still gated on the executor EXISTING — the reminder is addressed
            // to it and a session without one has nobody to remind — but the row
            // now reaches it through the ring, so no handle is needed here.
            if handle.hands().is_some() {
                // Its own `system` row (0044: host injections, NULL participant).
                // It cannot ride the phase-change row above — that one is the
                // user-visible "advanced to Apply" notice and this is a separate
                // instruction to one agent, so two messages means two rows.
                // Same as the phase row above: persisted, read off the cursor
                // at the executor's next dealt turn — which is when it can act
                // on the reminder — rather than written into a stdin mid-turn,
                // where it opened a generation outside the ring and arrived
                // twice. A missed nudge is a softer failure than a failed phase
                // advance: the helper warns, the transition stands.
                crate::core::post_system_notice(
                    &self.storage,
                    Some(&self.bridge),
                    session_id,
                    MessageKind::SystemNotice,
                    Self::APPLY_ENTRY_NUDGE,
                    None,
                )
                .await;
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
    /// Waiting on a peer is not waiting on the user: what you post is a row the
    /// ring hands your peer on their next turn, so SAYING SO in the channel is
    /// the wake mechanism (the text used to promise router-style forwarding,
    /// deleted 2026-08-13). Pinned by
    /// `apply_nudge_never_tells_hands_to_park_on_the_user`.
    const APPLY_ENTRY_NUDGE: &'static str =
        "🔔 Entering Apply. Before you mutate: confirm your reviewer reviewed the plan — \
         pull session_doc_search(phase=\"plan\") and check their pushback landed. If it \
         hasn't, say so in the channel (your peers read it on their next turn — the \
         ring, not you, wakes them) and do non-mutating prep meanwhile. Don't park on \
         the USER for a peer wait.";

    /// A2 (adherence): whether a Plan→Apply boundary warrants the peer-ack
    /// nudge to the participant crossing it. Pure for testing; the caller
    /// additionally AND-gates the `adherence_nudges` setting.
    ///
    /// `has_peer` is *any* roster above one — the call site passes
    /// `handle.agent_count() > 1`, not a test for a particular participant. It
    /// was named `has_rain` and documented as "a duo session … the nudge to
    /// Brian" until round-4 F6; both described a session shape this gate never
    /// checked.
    fn should_peer_ack_nudge(prev: IpavPhase, target: IpavPhase, has_peer: bool) -> bool {
        has_peer && prev == IpavPhase::Plan && target == IpavPhase::Apply
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
    /// testing; `holds_wakes` is the pause latch (`activity.holds_wakes()`),
    /// `recorded` is whether the bridge got a receipt back for the answer, and
    /// `ring_running` is whether any participant is mid-turn
    /// (`activity.any_busy()` — the same signal the chat-input lock reads, so
    /// the UI and this table agree on what "working" means).
    ///
    /// Read the two lines as "what does each consequence actually depend on":
    /// the halt clear on nothing, and the release on all three — a receipt to
    /// read, no pause to respect, and no running ring to interrupt (rc3 D34).
    ///
    /// There used to be a third, `stash`, for the paused case. It set a map that
    /// the broadcast then discarded: the rows are in the channel and every
    /// participant reads them off its own cursor (D19), so holding a copy bought
    /// nothing. The PAUSE still suppresses the release, which is the half that
    /// was doing the work.
    fn tray_wake(holds_wakes: bool, recorded: bool, ring_running: bool) -> TrayWake {
        TrayWake {
            clear_halt: true,
            release: !holds_wakes && recorded && !ring_running,
        }
    }

    /// Does answering THIS tray row clear the session's halt slot? Only a row
    /// that was READ and is a question (rc3 D35: a gate answers its approval,
    /// not the halt). `None` — the row could not be read, or is gone — clears
    /// nothing: fail-closed, because a halt cleared by mistake is invisible
    /// while a halt left standing is one message away from cleared.
    fn tray_answer_clears_halt(row: Option<&crate::storage::SessionTrayEntry>) -> bool {
        row.is_some_and(|r| !crate::storage::is_gate_row(&r.kind, r.options_json.as_deref()))
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
        // Was the resolved row a QUESTION? Answering a gate answers that
        // approval alone (rc3 D35): it lifts the latch and may wake an idle
        // ring, but it must not clear the session's halt slot — the defect in
        // `s-86a81478` was exactly that coupling. Fail-CLOSED on the read
        // (round 10): a row that cannot be read is not known to be a question,
        // so it clears nothing — the old `resolved_a_gate = false` default
        // turned an unreadable row into a halt clear.
        let mut answered_a_question = false;
        if matches!(
            outcome,
            ResolveOutcome::Delivered | ResolveOutcome::DeliveredOutOfBand { .. }
        ) {
            let row = self.storage.get_tray_entry(choice_id).await;
            answered_a_question = Self::tray_answer_clears_halt(row.as_ref().ok().and_then(|r| r.as_ref()));
            if let Ok(Some(entry)) = row {
                let sessions = self.sessions.lock().await;
                if let Some(handle) = sessions.get(&entry.session_id) {
                    handle
                        .user_broadcasts
                        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                }
            }
        }
        // Only the timed-out fallback needs us to wake the session. The
        // OOB message is already in storage (bridge wrote it, envelope and all).
        // To actually wake the participants so they read + act on it, also: (1)
        // clear the awaiting-user halt, (2) release the ring (an IDLE ring only)
        // so the next dealt turn drains the row. We deliberately do
        // NOT call broadcast_user_message (which re-inserts) — the storage row
        // already exists. Delivered + StaleGateNeedsConfirm need no wake (the
        // agent is live, or nothing ran).
        //
        // `receipt: None` means the answer was never recorded (no storage wired,
        // or the insert failed). Which of the four steps below that suppresses —
        // and which it must NOT — is [`AppState::tray_wake`]'s decision, kept
        // pure so every combination is a value a test can compare rather than a
        // shape a test has to guess at.
        if let ResolveOutcome::DeliveredOutOfBand {
            session_id, receipt, ..
        } = &outcome
        {
            let sessions = self.sessions.lock().await;
            if let Some(handle) = sessions.get(session_id) {
                let step = Self::tray_wake(
                    handle.activity.holds_wakes(),
                    receipt.is_some(),
                    handle.activity.any_busy(),
                );
                if step.clear_halt {
                    self.clear_awaiting(handle, session_id).await;
                }
                // `receipt.is_some()` is already folded into `step` by
                // `tray_wake`; the value itself is not needed here any more —
                // the stash that consumed it is gone.
                if receipt.is_some() {
                    // The PAUSED case needs nothing here. It used to stash the
                    // receipt for the next broadcast to deliver — and the
                    // broadcast then dropped what it collected, because the rows
                    // are in the channel already and every participant reads
                    // them off its own cursor (rc3 D19). A map written on one
                    // path and discarded on the other is not a queue; it is a
                    // leak with a comment.
                    // **No interrupt, and no release while the ring runs (rc3
                    // D34).** This block used to fire `tray-answer-preempt` at
                    // every agent and reset the ring — the issues.md #27 cure
                    // for an agent building on premises a parked answer had
                    // overturned (s-b69a5c01). But the reset threw away the
                    // holder's whole in-flight turn (the epoch moves; its
                    // completion is discarded on arrival), which makes a tray
                    // click a hidden interrupt — and the decree is that **Pause
                    // is the only real interrupt**. The answer row is already
                    // persisted; delivery is a pull; the next handover drains
                    // it, so the exposure #27 worried about is now bounded at
                    // the remainder of the current turn.
                    //
                    // An IDLE ring is the one case that still needs waking —
                    // nothing will ever drain the row otherwise. Through
                    // `user_responded`, the D28 single entry point; no mentions,
                    // because a pick from a list carries no `@` to honour (D17).
                    if step.release {
                        self.user_responded(session_id, Vec::new(), true, answered_a_question)
                            .await;
                    }
                }
            }
            // else: session closed in the gap between resolve and wake — the OOB
            // message persists in storage, so a future reopen still sees it.
        }
        Ok(outcome)
    }

    /// **A declared halt stops the DECLARER's own generation (rc3 D35,
    /// `s-86a81478`).** `mark_awaiting_user` ends the agent's turn ring-side,
    /// but the subprocess keeps generating after the tool ack — calling more
    /// tools for minutes under a ⏸ HALT banner, which is exactly the "HALT
    /// doesn't halt the agents" the user reported twice. Interrupting it here
    /// is not the user cutting anyone off: the agent DECLARED it is waiting,
    /// and this makes its own declaration true. Pause remains the only USER
    /// interrupt; peers are untouched (the ring latch already stops the next
    /// deal, and a non-declarer holding a live turn keeps it).
    ///
    /// **When it fires (round 8, A1b):** on `SignalingEvent::HaltAcked`, which
    /// the declarer's own PUMP emits when the ID-matched, non-error `ToolResult`
    /// of its `halt` / `mark_awaiting_user` call arrives in its stream — not on
    /// the `AwaitingUser` state change. Fired from the state change (rc3 D35's
    /// first cut) the interrupt reached the subprocess off the serial event
    /// worker while the tool's JSON-RPC ack was still being written, usually
    /// won, and the declarer's transcript showed its own `halt` answered with
    /// claude-code's cancellation text ("The user doesn't want to proceed with
    /// this tool use…"). By the time the result is in the stream there is
    /// nothing left to race; a halt whose result is an error took no effect
    /// and does not interrupt. `main.rs` pins the routing.
    pub async fn halt_declared(&self, session_id: &str, agent_slug: &str) {
        let sessions = self.sessions.lock().await;
        if let Some(handle) = sessions.get(session_id) {
            for agent in handle.agents() {
                if agent.slug == agent_slug {
                    agent.interrupt("halt-self-declared");
                }
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
            nudge.contains("read it on their next turn"),
            "must name the real wake mechanism: the ring hands the peer the row on \
             their next turn"
        );
        // The router-era promise must not come back: nothing forwards a turn's
        // output anywhere; the row is the delivery (rc3 D19).
        assert!(!nudge.contains("forwarded"), "no forwarding exists to promise");
    }

    #[test]
    fn smoke() {
        // Module compiles.
    }

    #[test]
    fn peer_ack_nudge_only_on_plan_to_apply_duo() {
        // A2: fires only when crossing Plan→Apply in a session with a peer.
        assert!(AppState::should_peer_ack_nudge(
            IpavPhase::Plan,
            IpavPhase::Apply,
            true
        ));
        // Solo (no peer) → no peer to ack.
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
    fn tray_wake_covers_the_pause_record_and_running_combinations() {
        // **This table changed subject at rc3 D34, and the old subject was the
        // interrupt.** The `deliver` flag used to fire `tray-answer-preempt` at
        // every agent and reset the ring — so answering a parked question threw
        // away the holder's in-flight turn. Under "Pause is the only real
        // interrupt" a running ring gets NOTHING from a tray answer: the row is
        // persisted, and the next handover drains it.

        // Running + recorded: the answer rides the next boundary. No interrupt,
        // no release — the exact case that used to abort the holder's turn.
        assert_eq!(
            AppState::tray_wake(false, true, true),
            TrayWake { clear_halt: true, release: false }
        );
        // Idle + recorded: the one case that needs waking — nothing else will
        // ever drain the row. Through `user_responded`, the D28 entry point.
        assert_eq!(
            AppState::tray_wake(false, true, false),
            TrayWake { clear_halt: true, release: true }
        );
        // Paused + recorded: no release. A tray answer must not lift a pause the
        // user deliberately set — whatever the busy flags say mid-drain. (There
        // is nothing to stash: the row is in the channel already.)
        assert_eq!(
            AppState::tray_wake(true, true, false),
            TrayWake { clear_halt: true, release: false }
        );
        assert_eq!(
            AppState::tray_wake(true, true, true),
            TrayWake { clear_halt: true, release: false }
        );
        // Unrecorded — the regression the flags exist for. The insert failing
        // gates the wake and nothing else: the halt still lifts, because that
        // follows from the user having answered rather than from the row. But
        // releasing over a receipt that never landed would hand out a turn on
        // an empty backlog.
        assert_eq!(
            AppState::tray_wake(false, false, false),
            TrayWake { clear_halt: true, release: false }
        );
        assert_eq!(
            AppState::tray_wake(true, false, true),
            TrayWake { clear_halt: true, release: false }
        );
    }

    #[test]
    fn a_tray_answer_interrupts_nobody() {
        // rc3 **D34**: the `tray-answer-preempt` interrupt is deleted, and this
        // pins the deletion. It was the issues.md #27 cure — abort every agent
        // so the answer takes effect at the next tool boundary — and it made a
        // tray click a hidden interrupt that reset the ring and discarded the
        // holder's completion. Pause is the only real interrupt; if this string
        // reappears in this file, that decree is being unwound.
        let src = include_str!("state.rs");
        // Assembled at runtime so this test's own source can never match it.
        let needle = format!("interrupt(\"{}\")", "tray-answer-preempt");
        assert!(
            !src.contains(&needle),
            "the tray-answer preempt must not come back"
        );
    }

    /// **A staged message must never abort a turn.**
    ///
    /// The user's stated reason for the Stage feature: *"staged messages should
    /// never interrupt the agents. It squeezes itself in the flow without
    /// interrupting anything. The Pause button is the only real interrupt —
    /// that's literally why I added the stage feature."*
    ///
    /// What shipped broken: `deliver_staged` → `send_user_response` →
    /// `broadcast`, which fires `interrupt("user-preempt")` at every agent. The
    /// queued message inherited the typed-Send preempt wholesale, so it aborted
    /// the turn it had politely waited for — and because the boundary
    /// notification is async, the turn it cut was usually the NEXT one, freshly
    /// dealt. Waiting made it worse rather than better.
    ///
    /// `teardown_session` holds the global `sessions` lock for the remove + kill
    /// ONLY. Held to the end of the function (as it was until round 7) it
    /// serialised every other session's broadcast, halt, tray wake and runtime
    /// read behind one session's PTY reap, DB close, policy cleanup and
    /// `git worktree remove`. No `AppState` can be built in tests, so the scope
    /// is pinned in the source: the guard is bound inside a nested block, and
    /// that block closes before the first thing that does I/O.
    #[test]
    fn teardown_holds_the_sessions_lock_for_the_kill_only() {
        let code = include_str!("state.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let at = code
            .find("async fn teardown_session(")
            .expect("teardown_session must exist");
        let body = &code[at..];
        let end = body[1..]
            .find("\n    pub async fn ")
            .map_or(body.len(), |n| n + 1);
        let body = &body[..end];
        // The guard is the first statement of an anonymous nested block: an
        // opening `{` alone on a body-level line, then the lock one indent in.
        // (A top-level `let mut sessions = …` at body indent — the shape that
        // held the lock to the end — does not match; checked by mutating it.)
        let nested = "\n        {\n            let mut sessions = self.sessions.lock().await;";
        let lock = body
            .find(nested)
            .expect("the sessions guard must be the first statement of its own nested block");
        // …and that block closes (a body-level `}`) before the terminal reap —
        // the first await after the kills — but AFTER the row close, which
        // must happen under the guard so "out of the map" and "closed in
        // storage" are one step (a broadcast in the gap would respawn).
        let reap = body
            .find("kill_and_remove(")
            .expect("teardown reaps the terminal");
        let row_close = body
            .find("close_session(id, archive)")
            .expect("teardown closes the row");
        let close = body[lock..]
            .find("\n        }\n")
            .map(|n| lock + n)
            .expect("the guard's block closes");
        assert!(
            lock < row_close && row_close < close,
            "the row close must sit INSIDE the sessions guard's block"
        );
        assert!(
            close < reap,
            "the sessions guard's block must close before the terminal reap"
        );
    }

    /// **The D35 self-interrupt is keyed on the halt tool's RESULT, not on the
    /// halt state** (round 8, A1b). `main.rs`'s control worker routes
    /// `HaltAcked` to `halt_declared`; `AwaitingUser` no longer reaches it.
    /// Pinned in the source: `halt_declared` is called from exactly one arm and
    /// that arm matches `HaltAcked`; the forwarder list carries `HaltAcked` and
    /// not `AwaitingUser`. Kill-tested: route `AwaitingUser` to `halt_declared`
    /// again → red.
    #[test]
    fn the_halt_interrupt_is_routed_from_the_halt_ack_not_the_halt_state() {
        let main = include_str!("../main.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let calls: Vec<usize> = main
            .match_indices("halt_declared(")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(calls.len(), 1, "halt_declared has exactly one caller in main.rs");
        let arm_start = main[..calls[0]]
            .rfind("SignalingEvent::")
            .expect("the call sits inside a match arm");
        let arm = &main[arm_start..calls[0]];
        assert!(
            arm.starts_with("SignalingEvent::HaltAcked {"),
            "halt_declared must be reached from the HaltAcked arm, got: {}",
            &arm[..arm.len().min(80)]
        );
        assert!(
            !main.contains("SignalingEvent::AwaitingUser { session_id, agent, .. } =>"),
            "the AwaitingUser arm must not come back into the control worker"
        );
        // The forwarder that feeds the worker carries HaltAcked.
        let fwd = main
            .find("ev @ (SignalingEvent::SessionCloseRequest")
            .expect("the control-event forwarder exists");
        let fwd_block = &main[fwd..fwd + 400];
        assert!(fwd_block.contains("SignalingEvent::HaltAcked { .. }"), "HaltAcked is forwarded");
        assert!(
            !fwd_block.contains("SignalingEvent::AwaitingUser"),
            "AwaitingUser is not a control event any more"
        );
    }

    /// **Every host-declared halt interrupts** (round 8, A1b — the reviewer's
    /// blocking finding on the first cut). The agent's own `mark_awaiting_user`
    /// gets its D35 interrupt from its pump when the tool RESULT lands; a halt
    /// the HOST declares under an agent's slug (provider limit, error streak,
    /// spin breaker, idle watchdog) has no result to wait for and must fire it
    /// at once — that is `mark_awaiting_user_for`. So the bare
    /// `mark_awaiting_user(` may be called, in production, only by the JSON-RPC
    /// handler; every other production caller uses the host verb. Kill-tested:
    /// switch one host site back → red.
    #[test]
    fn host_declared_halts_go_through_the_interrupting_verb() {
        let files: &[(&str, &str)] = &[
            ("core/pump.rs", include_str!("pump.rs")),
            ("core/sequencer.rs", include_str!("sequencer.rs")),
            ("core/watchdog.rs", include_str!("watchdog.rs")),
            ("core/session.rs", include_str!("session.rs")),
            ("core/state.rs", include_str!("state.rs")),
        ];
        for (name, src) in files {
            let prod = match src.find("\n#[cfg(test)]") {
                Some(at) => &src[..at],
                None => src,
            };
            let code = prod
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                !code.contains(".mark_awaiting_user("),
                "{name}: a host-declared halt must use mark_awaiting_user_for (it interrupts)"
            );
        }
        // And the host verb interrupts: it emits the ack alongside the halt.
        let tray = include_str!("../signaling/bridge/tray.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let at = tray
            .find("pub async fn mark_awaiting_user_for(")
            .expect("the host verb exists");
        let body = &tray[at..at + 600];
        assert!(body.contains("self.notify_halt_acked("), "the host verb fires HaltAcked");
    }

    /// **A teardown cannot be respawned into** (round 8, R1).
    ///
    /// Round 7's narrowed guard left `teardown_session`'s cleanup tail
    /// unlocked, and `ensure_session_started` (`respawn_session` on every
    /// SessionView mount) consults no marker — so a spawn landing in that gap
    /// registered a handle the tail then unregistered under it. The marker is
    /// the `closing` set; the pin is where it is written and read: set INSIDE
    /// the guard block that removes the handle, cleared AFTER the tail's last
    /// step, and read by `ensure_session_started` INSIDE its own `sessions`
    /// re-check — a read outside that hold reopens the window (the reviewer's
    /// first check). No `AppState` can be built in tests; the source is the
    /// pin. Kill-tested: deleting the `is_closing` check goes red here.
    #[test]
    fn a_closing_session_is_marked_under_the_guard_and_refused_by_the_spawn_path() {
        let code = include_str!("state.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let body_of = |name: &str| {
            let at = code
                .find(&format!("async fn {name}("))
                .unwrap_or_else(|| panic!("{name} must exist"));
            let rest = &code[at..];
            // The body ends at the next fn at impl indent, pub or not.
            let end = [
                rest[1..].find("\n    pub async fn "),
                rest[1..].find("\n    async fn "),
                rest[1..].find("\n    pub fn "),
                rest[1..].find("\n    fn "),
            ]
            .into_iter()
            .flatten()
            .min()
            .map_or(rest.len(), |n| n + 1);
            rest[..end].to_string()
        };

        // teardown: mark inside the guard block, clear after the tail.
        let td = body_of("teardown_session");
        let nested = "\n        {\n            let mut sessions = self.sessions.lock().await;";
        let lock = td.find(nested).expect("teardown's guard block");
        let close = td[lock..]
            .find("\n        }\n")
            .map(|n| lock + n)
            .expect("teardown's guard block closes");
        let mark = td
            .find("self.mark_closing(id, true)")
            .expect("teardown must mark the session closing");
        assert!(
            lock < mark && mark < close,
            "the closing mark must be set INSIDE the sessions guard block"
        );
        let notify = td
            .find("notify_session_closed(")
            .expect("teardown notifies the UI last");
        let clear = td
            .rfind("self.mark_closing(id, false)")
            .expect("teardown must clear the closing mark");
        assert!(
            notify < clear,
            "the closing mark must be cleared AFTER the tail's last step"
        );
        // …and on the ONE early return (a failed row close, nothing torn down)
        // the mark is cleared first — otherwise that session stays marked
        // forever and can never be respawned (the reviewer's nit).
        let err_return = td[mark..]
            .find("return Err(e)")
            .map(|n| mark + n)
            .expect("the failed row close returns early");
        let err_clear = td[mark..]
            .find("self.mark_closing(id, false)")
            .map(|n| mark + n)
            .expect("the failed row close clears the mark");
        assert!(
            err_clear < err_return && err_return < close,
            "a failed row close must clear the closing mark before returning"
        );
        // Idempotence: the mark is the ownership test — a second overlapping
        // teardown returns before touching anything.
        let join = td[lock..]
            .find("return Ok(())")
            .map(|n| lock + n)
            .expect("a second teardown returns early");
        assert!(
            mark < join && join < close,
            "the second teardown must return inside the guard block, right after the mark"
        );

        // ensure_session_started: read inside the re-check, under the sessions
        // lock taken beneath the spawn gate.
        let es = body_of("ensure_session_started");
        let gate = es
            .find("self.spawn_gate.lock().await")
            .expect("ensure_session_started takes the spawn gate");
        let recheck = es[gate..]
            .find("let mut sessions = self.sessions.lock().await;")
            .map(|n| gate + n)
            .expect("the re-check takes the sessions lock");
        let recheck_close = es[recheck..]
            .find("\n        }\n")
            .map(|n| recheck + n)
            .expect("the re-check block closes");
        let read = es[gate..]
            .find("self.is_closing(session_id)")
            .map(|n| gate + n)
            .expect("ensure_session_started must refuse a closing session");
        assert!(
            recheck < read && read < recheck_close,
            "the closing read must sit INSIDE the sessions re-check block, \
             not before the lock is taken and not after it is released"
        );
        // …and again after the spawn, before the insert: a teardown that ran
        // during the (seconds-long) spawn must not get a handle registered.
        let insert = es
            .find("sessions.insert(session_id.to_string(), handle)")
            .expect("the spawned handle is inserted");
        let read_after_spawn = es
            .rfind("self.is_closing(session_id)")
            .expect("the post-spawn closing read");
        assert!(
            read < read_after_spawn && read_after_spawn < insert,
            "the post-spawn closing read must precede the insert"
        );
    }

    /// The phase-change notice is HOST-authored (round 7). Nothing in this
    /// crate can build an `AppState` to drive `advance_phase` end to end (the
    /// `sessions` map is only populated by a real spawn), so the wire cannot be
    /// asserted here — this pins the source instead: the notice must go through
    /// `post_system_notice`, and the user-row writer must not appear anywhere
    /// in this file. Before the fix every participant read the transition as
    /// `[user] [PHASE: X] …` — the user "saying" the phase moved, when an
    /// agent's vote may have moved it (rc3 D37).
    #[test]
    fn the_phase_change_notice_is_a_system_row_not_a_user_row() {
        let code = include_str!("state.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let ap = code
            .find("pub async fn advance_phase(")
            .expect("advance_phase must exist");
        let body = &code[ap..];
        let end = body[1..]
            .find("\n    pub async fn ")
            .map_or(body.len(), |n| n + 1);
        let body = &body[..end];
        assert!(
            body.contains("post_system_notice(") && body.contains("MessageKind::PhaseChange"),
            "advance_phase must post its transition notice through post_system_notice \
             (origin = system): {body}"
        );
        // Assembled so the assertion cannot match its own text.
        let user_writer = format!("insert_{}_message(", "user");
        assert!(
            !code.contains(&user_writer),
            "core::state must not write any row through the user-row writer; a host \
             row that goes through it reaches every participant as [user]"
        );
    }

    /// Three assertions, because restoring the bug needs only one of them back:
    /// the decision, the GUARD that consults it, and the variant the staged path
    /// picks. The third is the one that was missing entirely.
    #[test]
    fn a_staged_message_never_preempts() {
        assert!(
            UserSend::Typed.preempts(),
            "a typed Send is the always-typeable unblock's spine and must take \
             effect now — a typed message that does NOT preempt is the opposite \
             bug, and just as real"
        );
        assert!(
            !UserSend::Staged.preempts(),
            "a staged message is queued by the user's own choice"
        );

        // Comments stripped: prose about a symbol is a record, not a wire —
        // `phase_vote_wiring_test.rs` pays for this lesson at length, and this
        // file's own doc comments quote both the call form and the guard.
        let code = include_str!("state.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        // Assembled at runtime so this assertion cannot satisfy itself.
        let needle = format!("interrupt(\"{}\")", "user-preempt");
        let at = code
            .find(&needle)
            .expect("the typed-Send preempt must still exist");
        let before = &code[..at];
        let guard = before
            .rfind("if send.preempts() {")
            .expect("the preempt must be guarded by the arrival mode, not fired unconditionally");
        assert!(
            !before[guard..].contains('}'),
            "the `if send.preempts()` block CLOSES before the preempt call, so \
             the interrupt is unconditional again and every staged message cuts \
             a turn"
        );

        // The staged path must pick the non-preempting variant. Scoped to
        // `deliver_staged`'s own body — finding `UserSend::Staged` anywhere in
        // the file proves nothing, which is the shape this codebase keeps
        // shipping.
        let ds = code
            .find("pub async fn deliver_staged")
            .expect("deliver_staged must exist");
        let body = &code[ds..];
        let end = body[1..]
            .find("\n    pub async fn ")
            .map_or(body.len(), |n| n + 1);
        assert!(
            body[..end].contains("UserSend::Staged"),
            "`deliver_staged` must deliver as Staged — without it the queued \
             message reaches `broadcast` as a typed Send and preempts, which is \
             exactly the bug this test exists for"
        );
    }

    /// **A stage that cannot deliver stops re-arming the ring** (round 8,
    /// T2-4). On a send failure `deliver_staged` restored the content and
    /// re-flagged the ring with no cap; an idle ring re-fires the boundary at
    /// once, so a persistent failure spins. The re-arm is now conditional on
    /// the attempt count and the capped path posts a system row. Source-pinned
    /// (no `AppState` in tests): the re-arm sits under the attempts check, and
    /// the capped arm posts through the helper. Kill-tested: make the re-arm
    /// unconditional → red.
    #[test]
    fn deliver_staged_stops_rearming_after_the_attempt_cap() {
        let code = include_str!("state.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let ds = code
            .find("pub async fn deliver_staged")
            .expect("deliver_staged must exist");
        let body = &code[ds..];
        let end = body[1..]
            .find("\n    pub async fn ")
            .map_or(body.len(), |n| n + 1);
        let body = &body[..end];
        let check = body
            .find("if attempts < STAGED_DELIVERY_MAX_ATTEMPTS")
            .expect("the re-arm is gated on the attempt count");
        let rearm = body
            .find("notify_ring_stage(session_id, true)")
            .expect("the failure path re-arms the ring");
        assert!(check < rearm, "the re-arm must sit under the attempts check");
        let capped = body[check..]
            .find("post_system_notice(")
            .expect("the capped arm tells the user in the channel");
        assert!(rearm < check + capped, "the notice belongs to the capped arm, after the re-arm");
        assert_eq!(
            body.matches("notify_ring_stage(session_id, true)").count(),
            1,
            "exactly one re-arm, the gated one"
        );
    }

    /// **s-ff729daa**: `broadcast` pre-marked every agent busy — duo-era
    /// delivery, redundant since D19b's `hand_turn_to` marks the participant
    /// the ring actually deals. A pre-mark on a participant the ring never
    /// reached (a halt landed two turns early) had no turn end to clear it, so
    /// `any_busy` stayed true, the input stayed locked under the HALT banner,
    /// and the user force-paused three times in four minutes to get the floor.
    /// The same stale flag read busy+silent to the stall watchdog, which called
    /// the rightfully quiet participant "stalled". The ring is the only
    /// busy-true writer; if this file marks anyone busy again, that lie is back.
    #[test]
    fn broadcast_marks_nobody_busy() {
        let src = include_str!("state.rs");
        // Production section only — this doc names the call forms in prose.
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        for form in ["set_busy_slug(", ".set_busy("] {
            assert!(
                !prod.contains(form),
                "core::state calls `{form}` — the ring's hand_turn_to is the one \
                 busy-true writer, and a flag only the ring sets is a flag a \
                 stopped ring has always cleared (s-ff729daa)"
            );
        }
    }

    /// **Stop tells the RING, not only the activity latch** (B1-F8).
    ///
    /// `cancel_session_turn` set `paused` on the tracker — a UI state — and the
    /// ring was never told, so the interrupt's completion stepped the rotation
    /// and a new turn began under the ⏸ banner. The behaviour is pinned in
    /// `core::sequencer` (`a_pause_stops_the_next_deal_not_just_the_banner`);
    /// this pins the PRODUCER, which is the half that was missing for the whole
    /// life of the feature and cannot be observed from the ring's side.
    #[test]
    fn stopping_a_session_pauses_the_ring() {
        let src = include_str!("state.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        let body = prod
            .split("pub async fn cancel_session_turn")
            .nth(1)
            .expect("cancel_session_turn exists")
            .split("\n    /// ")
            .next()
            .expect("a split always yields a first part");
        assert!(
            body.contains("notify_ring_pause("),
            "Stop must reach the ring — a paused banner over a ring that keeps \
             dealing is the state this fixed"
        );
    }

    /// **A relaunch mid-stage puts the user's words back, and tells the ring.**
    ///
    /// Stage held its content in `AppState.staged_responses` — process memory —
    /// so a relaunch dropped whatever the user had composed, while the composer
    /// rehydrated its "Staged ✓" toggle from the same empty map and showed them
    /// a stage that no longer existed. Migration 0058 gives the session row a
    /// slot; this pins that BOTH halves come back, because they are separate
    /// and either alone is useless: the CONTENT (what `deliver_staged` sends)
    /// and the ring's FLAG (what makes a boundary park and emit
    /// `StagedDeliveryDue`).
    ///
    /// Source-asserted: `rehydrate_stage` needs a live `CoreAppState` with
    /// subprocesses to exercise, the same reason its neighbours here are grep
    /// pins.
    #[test]
    fn a_restart_restores_both_halves_of_a_stage() {
        let src = include_str!("state.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        let body = prod
            .split("async fn rehydrate_stage")
            .nth(1)
            .expect("rehydrate_stage exists")
            .split("\n    /// ")
            .next()
            .expect("a split always yields a first part");
        assert!(
            body.contains("staged_message("),
            "the stage comes back from the durable slot, not from memory"
        );
        assert!(
            body.contains("staged_responses"),
            "the CONTENT half: what `deliver_staged` will send"
        );
        assert!(
            body.contains("notify_ring_stage("),
            "the FLAG half: without it the fresh ring parks for nothing and the \
             message waits for a boundary that never fires"
        );
        // Every path that puts a live handle in the map rehydrates. Since round 7
        // deleted `open_session` (the external driver's create path) that is ONE
        // site — `ensure_session_started`, which both remaining create paths (the
        // dialog and the plugin proxy's `dispatch_session_inner`) and the respawn a
        // relaunch takes all go through. A second site would be a new spawn path,
        // and it must rehydrate too; zero would be the bug this guards.
        assert_eq!(
            prod.matches("self.rehydrate_stage(").count(),
            1,
            "every path that puts a live handle in the map must restore its stage — \
             the one spawn path is `ensure_session_started`"
        );
        let started = prod
            .split("pub async fn ensure_session_started")
            .nth(1)
            .expect("ensure_session_started exists");
        assert!(
            started.contains("self.rehydrate_stage("),
            "and it is ensure_session_started that does it"
        );
    }

    /// **The D15 epilogue waits for the LAP it asked for, not for the first
    /// quiet instant.**
    ///
    /// Two shipped-and-fixed mistakes are pinned here, because they are one
    /// mistake at two depths and the second is only reachable once the first is
    /// fixed:
    ///
    /// 1. Waiting `await_both_idle` right after `broadcast`, on the premise that
    ///    the broadcast had marked everyone busy. It has not since `519cbba`
    ///    (`broadcast_marks_nobody_busy`, above, pins that), so the wait was
    ///    answered by the idle state the broadcast left — 7ms, `Declined`,
    ///    agents SIGKILLed as the ring dealt the turn. No epilogue had ever run.
    /// 2. Arming on the busy edge and then waiting `await_both_idle` anyway. The
    ///    ring clears the previous holder before it marks the next one, on
    ///    purpose (`hand_turn_to`'s doc, "the sub-second gap"), so the armed
    ///    wait can still return between two turns of the same lap.
    ///
    /// Asserted over the source: `run_close_epilogue` needs a live session with
    /// two subprocesses and a running ring to exercise, which is the same reason
    /// its sibling pins above are grep tests. What it checks is the property
    /// that failed twice — which waiter this path uses.
    #[test]
    fn the_close_epilogue_waits_for_the_lap_not_the_first_idle() {
        let src = include_str!("state.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        let body = prod
            .split("async fn run_close_epilogue")
            .nth(1)
            .expect("run_close_epilogue exists")
            .split("\n    async fn ")
            .next()
            .expect("a split always yields a first part");
        for form in ["await_turn_started(", "await_lap_end("] {
            assert!(
                body.contains(form),
                "the close epilogue must `{form}` — arming on the turn AND waiting \
                 out the lap are each load-bearing; dropping either reports a \
                 decline the agents never made"
            );
        }
        assert!(
            !body.contains("await_both_idle("),
            "the close epilogue must not use `await_both_idle` — it returns on the \
             first idle poll, which inside a lap is the handover gap the ring keeps \
             deliberately (hand_turn_to's doc)"
        );
    }

    /// **A closed session reopens on a button, not on a view** (round 10, B4).
    /// The spawn path refuses a closed row, and the reopen clears the row
    /// BEFORE it spawns — so the refusal cannot bite the reopen, and nothing
    /// but the reopen can revive a closed roster. Pure rule + source pins,
    /// because `ensure_session_started` spawns real subprocesses.
    #[test]
    fn a_closed_row_is_refused_by_the_spawn_path_and_reopened_by_the_button() {
        assert!(refuse_closed("s1", None).is_ok(), "an open row spawns");
        let err = refuse_closed("s1", Some("2026-08-18T04:50:41Z"))
            .expect_err("a closed row is refused");
        assert!(err.to_string().contains("reopen it first"), "{err}");

        let src = include_str!("state.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        let started = prod
            .split("pub async fn ensure_session_started")
            .nth(1)
            .expect("ensure_session_started exists")
            .split("\n    pub ")
            .next()
            .expect("a split always yields a first part");
        let refuse = started
            .find("refuse_closed(")
            .expect("the spawn path applies the closed-row refusal");
        let spawn = started
            .find("spawn_existing_session(")
            .expect("the spawn path still spawns");
        assert!(refuse < spawn, "the refusal is read BEFORE anything is spawned");

        let reopen = prod
            .split("pub async fn reopen_session")
            .nth(1)
            .expect("reopen_session exists")
            .split("\n    pub ")
            .next()
            .expect("a split always yields a first part");
        let clear = reopen
            .find(".reopen_session(session_id)")
            .expect("the reopen clears the row through storage");
        let start = reopen
            .find("ensure_session_started(session_id)")
            .expect("the reopen spawns through the one spawn path");
        let told = reopen
            .find("notify_session_created(session_id)")
            .expect("the reopen tells the frontend the row is live again");
        assert!(clear < start && start < told, "clear the row → spawn → tell the UI");
        // Round 11: an already-open row is a success no-op — the storage half
        // returns `false` so a double click (or a stale view) is harmless, and
        // this path used to turn that into an error the bar rendered as
        // "Reopen failed". The no-op must return BEFORE the spawn.
        assert!(
            !reopen.contains("bail!"),
            "a not-closed row is not an error: the reopen is idempotent"
        );
        let noop = reopen
            .find("return Ok(());")
            .expect("the not-closed row returns Ok without spawning");
        assert!(clear < noop && noop < start, "read the row → no-op if open → spawn");
    }

    /// **An unreadable tray row keeps the halt** (round 10): the halt clears
    /// only for a row that was read and is a question — a gate never clears
    /// it (D35), and neither does a read that failed or found nothing.
    #[test]
    fn only_a_read_question_row_clears_the_halt() {
        let row = |kind: &str, options: Option<&str>, cmd: Option<&str>| {
            crate::storage::SessionTrayEntry {
                id: 1,
                session_id: "s1".into(),
                choice_id: "c".into(),
                agent: "hands".into(),
                kind: kind.into(),
                prompt: "p".into(),
                options_json: options.map(str::to_string),
                status: "answered".into(),
                picked_option: None,
                asked_at: "2026-08-18T00:00:00Z".into(),
                answered_at: None,
                supersedes_id: None,
                command_text: cmd.map(str::to_string),
            }
        };
        let question = row("choice", Some(r#"["a","b"]"#), None);
        let gate = row("approval", Some(crate::storage::GATE_OPTIONS_JSON), Some("echo"));
        let legacy_gate = row("choice", Some(crate::storage::GATE_OPTIONS_JSON), None);
        assert!(AppState::tray_answer_clears_halt(Some(&question)));
        assert!(!AppState::tray_answer_clears_halt(Some(&gate)));
        assert!(!AppState::tray_answer_clears_halt(Some(&legacy_gate)));
        assert!(
            !AppState::tray_answer_clears_halt(None),
            "an unreadable or missing row clears nothing"
        );
    }

    /// **The epilogue's turn runs BEFORE anything is torn down.**
    ///
    /// The ordering was prose in a comment and control flow in one function, and
    /// the unregister half of teardown just became load-bearing twice over:
    /// `unregister_session` now drops the ring's `Sender`, so a teardown that
    /// ran first would leave the epilogue prompt with no ring to deal it — the
    /// turn could not start at all, and the outcome row would read
    /// `NeverStarted` for a session whose agent was perfectly willing.
    ///
    /// Asserted over the source for the same reason its siblings are: exercising
    /// it needs a live session with two subprocesses. What it pins is the ORDER
    /// inside the epilogue arm — the await of the turn, then the teardown.
    #[test]
    fn the_epilogue_turn_runs_before_the_teardown_that_unregisters_the_ring() {
        let src = include_str!("state.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        let arm = prod
            .split("ClosePlan::RunEpilogueFirst =>")
            .nth(1)
            .expect("the epilogue arm exists")
            .split("\n    /// ")
            .next()
            .expect("a split always yields a first part");
        let epilogue = arm
            .find("run_close_epilogue(")
            .expect("the epilogue arm runs the epilogue");
        let teardown = arm
            .find("teardown_session(")
            .expect("the epilogue arm tears down afterwards");
        assert!(
            epilogue < teardown,
            "teardown runs before the epilogue's turn — `unregister_session` drops \
             the ring's Sender, so the turn the close just asked for could never \
             be dealt"
        );
    }

    /// **s-f6a441ff: the paste gate.** One 2.9 MB user paste wedged both
    /// participants' contexts unrecoverably. Both user-text entry points —
    /// `broadcast` and `send_user_response` — refuse an oversized message at
    /// the top, whole, with the file-not-paste fix in the error.
    #[test]
    fn an_oversized_user_message_is_refused_with_the_fix_in_hand() {
        let cap = crate::storage::WIRE_BODY_CLAMP_BYTES;
        assert_eq!(
            AppState::oversized_message_refusal(cap),
            None,
            "at the cap passes — the wire clamp's boundary matches, so an \
             accepted message is never truncated"
        );
        let refusal = AppState::oversized_message_refusal(cap + 1)
            .expect("one byte over the cap refuses");
        assert!(
            refusal.contains("file") && refusal.contains(&cap.to_string()),
            "the refusal names the cap and the file-instead-of-paste fix: {refusal}"
        );

        // Both entry points consult the gate — the definition plus two calls.
        // A user-text path that skips it reintroduces the 2.9 MB session.
        let src = include_str!("state.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        assert_eq!(
            prod.matches("oversized_message_refusal(").count(),
            4,
            "the paste gate guards broadcast, send_user_response AND \
             stage_user_response — every user-text entry point"
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
    /// The mapping the two call sites are pinned against above.
    #[test]
    fn only_a_user_advance_releases_the_ring() {
        assert!(PhaseAdvanceSource::User.releases_ring());
        assert!(
            !PhaseAdvanceSource::Agent.releases_ring(),
            "an agent is mid-turn when it self-advances; releasing deals a turn \
             over an empty backlog (rc3 D28)"
        );
    }

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
            prod.matches(".clear_session_halt(").count(),
            1,
            "the session's halt slot is cleared in exactly one place — a second \
             call site is a path that can forget the other half (rc3 D35: the \
             slot lives on the SESSION, not in the tray)"
        );
        // The ring release: the bridge method, called once, from the same place.
        assert_eq!(
            prod.matches(".notify_ring_user_message(").count(),
            1,
            "the ring is released in exactly one place, for the same reason"
        );

        // A gate answer is the one response that must NOT clear the halt slot
        // (rc3 D35, s-86a81478: approving an unrelated command wiped the HALT
        // the user was looking at). The OOB release site carries the
        // distinction through `tray_answer_clears_halt` — a row READ and found
        // to be a question, and nothing else (round 10: an unreadable row used
        // to default to "clear"). Losing the derivation re-couples them.
        assert!(
            prod.contains("self.user_responded(session_id, Vec::new(), true, answered_a_question)"),
            "the OOB release must pass clear_halt = answered_a_question — a gate \
             answer answers that gate, not the session's halt"
        );
        assert!(
            prod.contains("answered_a_question = Self::tray_answer_clears_halt("),
            "and that flag is derived from the READ row, so an unreadable row clears nothing"
        );

        // **And the phase paths must not both pass the same source.** The ring
        // release is now a function of WHO advanced, and the failure this
        // catches shipped in review: the user-facing command reused the agent's
        // call verbatim, so picking a phase on a HALTED session cleared the halt
        // row, cleared the awaiting badge, advanced the phase, persisted the
        // notice — and dealt no turn. The session looked answered and did
        // nothing until the idle watchdog's grace elapsed.
        //
        // Source-level for the reason stated above: nothing in this crate can
        // build an `AppState` — the `sessions` map is only populated by a real
        // session start with a subprocess — so "drive the user path against a
        // halted session" is not a test that exists to be written here. This is
        // weaker than that test would be, and it is what the file can carry.
        let user_cmd = include_str!("../tauri_cmd/sessions.rs");
        assert!(
            user_cmd.contains("PhaseAdvanceSource::User"),
            "the user's phase command must advance as User — as Agent it clears \
             the halt and deals no turn, which is the silent-stall state"
        );
        assert!(
            !user_cmd.contains("PhaseAdvanceSource::Agent"),
            "nothing the USER invokes may advance as Agent"
        );
        // **The link that closes the chain.** Knowing the two call sites pass
        // the right variant, and that `releases_ring` maps them correctly, still
        // leaves `advance_phase` free to ignore both and pass a literal —
        // measured, not reasoned: replacing this call's third argument with
        // `false` restores the silent stall verbatim and passes all 1178 lib
        // tests. Three green links around a cut fourth is precisely the shape
        // this guard exists to refuse.
        assert!(
            prod.contains("source.releases_ring()"),
            "advance_phase must derive the ring release from `source` — a \
             literal there re-creates the silent stall with every other link \
             still green"
        );
        let agent_path = include_str!("../main.rs");
        assert!(
            agent_path.contains("PhaseAdvanceSource::Agent"),
            "a participant's own advance must stay Agent — releasing there deals \
             a turn over an empty backlog while the caller is still mid-turn"
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

    /// **A transition invalidates the votes cast about the phase it leaves.**
    ///
    /// The behaviour half of round 5's E1. Migration 0062 built the epoch to
    /// close the vote's TIME axis and justified it by having exactly one
    /// production call site; that call site did not exist, so the column stayed
    /// at 0 through every phase change the database recorded (125 by 2026-08-17,
    /// and the count only grows) and no tally was ever invalidated.
    ///
    /// This drives the seam directly with a real `Storage` — the reason the seam
    /// exists, since nothing in the crate can build an `AppState`. Asserted on
    /// BOTH halves the bump owes: the epoch moves, and the session's votes are
    /// cleared. Mutation-checked by deleting the bump from the seam.
    /// **The atomic-op deferral waits for the flag, and caps** (round 11 —
    /// this wait was inlined in the Tauri command, where no test could reach
    /// it). A flag that clears ends the wait then, uncapped; one that never
    /// clears ends it at the cap, flagged so the telemetry can say why the
    /// interrupt was late.
    #[tokio::test(start_paused = true)]
    async fn the_atomic_op_deferral_waits_for_the_flag_and_caps() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let flag = Arc::new(AtomicBool::new(true));
        let clears = Arc::clone(&flag);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            clears.store(false, Ordering::Release);
        });
        let (waited_ms, capped) =
            await_atomic_op_or_cap(&flag, std::time::Duration::from_secs(8)).await;
        assert!(!capped, "the op cleared before the cap");
        assert!((300..8000).contains(&waited_ms), "waited for the op, not the cap: {waited_ms}ms");

        let stuck = AtomicBool::new(true);
        let (waited_ms, capped) =
            await_atomic_op_or_cap(&stuck, std::time::Duration::from_millis(500)).await;
        assert!(capped, "a hung op is interrupted at the cap");
        assert!(waited_ms >= 500, "the full cap was given: {waited_ms}ms");
    }

    #[tokio::test]
    async fn a_phase_transition_invalidates_the_votes_cast_before_it() {
        let storage = Storage::memory().await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();
        storage.ensure_session_roster("s1", 2).await.unwrap();
        let roster = storage.participants_for_session("s1").await.unwrap();
        let ipav = Mutex::new(crate::core::ipav::IpavState::default());

        // A complete tally: every active participant has voted for Apply.
        let epoch = storage.phase_epoch("s1").await.unwrap();
        for p in &roster {
            storage
                .cast_phase_vote("s1", p.id, "Apply", "fp1", epoch)
                .await
                .unwrap();
        }
        assert!(
            storage
                .all_active_voted_to_advance("s1", "Apply", "fp1", epoch)
                .await
                .unwrap(),
            "precondition: the tally that authorizes this transition is complete"
        );

        commit_phase_transition(&storage, "s1", &ipav, IpavPhase::Apply).await;

        assert_eq!(
            ipav.lock().await.current_phase,
            IpavPhase::Apply,
            "the in-memory phase moved"
        );
        assert_eq!(
            storage.phase_epoch("s1").await.unwrap(),
            epoch + 1,
            "the epoch moved with the phase — without this every vote stays \
             valid forever and a Plan-era tally completes a later Plan (E1)"
        );
        assert!(
            !storage
                .all_active_voted_to_advance("s1", "Apply", "fp1", epoch)
                .await
                .unwrap(),
            "the tally that authorized this transition can no longer authorize \
             another one"
        );
        let left: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM phase_votes WHERE session_id = 's1'")
                .fetch_one(storage.pool())
                .await
                .unwrap();
        assert_eq!(
            left.0, 0,
            "the session's votes are cleared too — 0062's 'deletion, not \
             accumulation' half, which was equally inert while nothing called \
             the bump"
        );
        // 0063: the same transition is what makes the phase survive a restart.
        assert_eq!(
            storage.persisted_ipav_phase("s1").await.unwrap().as_deref(),
            Some("apply"),
            "the transition recorded the phase for the next session start — \
             without this a restart resumes at Investigate and hands a mid-Apply \
             executor 'No Edit, Write, or mutating Bash'"
        );
    }

    /// **A session with no recorded phase reads as Investigate, not as broken.**
    ///
    /// NULL is the state of every session that predates 0063 and of every session
    /// that has never transitioned, so it is the common case rather than an edge
    /// one. Pinned separately because the restore turns three different inputs
    /// into the same answer — NULL, an unparseable tag, and a read error — and a
    /// test of only the happy path would let any of the three become a panic.
    #[tokio::test]
    async fn a_session_that_never_transitioned_restores_to_investigate() {
        let storage = Storage::memory().await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();

        assert_eq!(
            storage.persisted_ipav_phase("s1").await.unwrap(),
            None,
            "a fresh session has recorded no phase"
        );
        assert_eq!(
            IpavPhase::parse("nonsense").unwrap_or_default(),
            IpavPhase::Investigate,
            "an unparseable tag falls back rather than failing the spawn"
        );

        // And the round trip the restore depends on, for every phase — the tag
        // written is the tag that parses back.
        for phase in [
            IpavPhase::Investigate,
            IpavPhase::Plan,
            IpavPhase::Apply,
            IpavPhase::Verify,
        ] {
            storage
                .set_persisted_ipav_phase("s1", phase.tag())
                .await
                .unwrap();
            let back = storage.persisted_ipav_phase("s1").await.unwrap().unwrap();
            assert_eq!(
                IpavPhase::parse(&back),
                Some(phase),
                "{} must survive the write/read round trip",
                phase.name()
            );
        }
    }

    /// **The seam stays mounted, and the bump stays inside it.**
    ///
    /// The test above never pins its own mount — it calls the seam directly, so
    /// deleting the call from `advance_phase` leaves it green and the epoch dead
    /// again. That is E1 moved up one level, and it is the failure this guard
    /// exists to make impossible.
    ///
    /// rustc does not cover it: a non-`pub` single-caller free fn would raise
    /// `dead_code`, but the test above is a same-file use, so the warning is
    /// silent under `cargo test` and `cargo clippy --all-targets` and appears
    /// only at `cargo build --release` — as a warning, exit 0. Detection at the
    /// last gate is not enforcement.
    ///
    /// Counts the DOTTED / CALL forms on the production half for the reason the
    /// sibling guard above records: a bare-name count measures the prose that
    /// explains the code. Both functions are therefore named BARE in every
    /// comment in this file — see the failure messages.
    #[test]
    fn the_phase_epoch_moves_with_the_phase_and_the_seam_stays_mounted() {
        let src = include_str!("state.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");

        assert_eq!(
            prod.matches("commit_phase_transition(").count(),
            2,
            "exactly one definition and one call. Zero call sites is round 5's \
             E1 one level up: the seam's own test would stay green while the \
             epoch went dead again. If this reads 3, a COMMENT wrote the name \
             with a trailing `(` — name it bare in prose"
        );
        assert_eq!(
            prod.matches(".bump_phase_epoch(").count(),
            1,
            "the epoch bump lives inside the seam and nowhere else. Kept as an \
             exact count, not a floor: migration 0062 is applied and immutable \
             and says the epoch has 'exactly ONE production call site', so a \
             second caller would falsify a claim the tree can never correct. If \
             this reads 2, a COMMENT wrote the name with a leading `.` — name \
             it bare in prose"
        );
    }
}
