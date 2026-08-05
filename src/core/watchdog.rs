//! Silence/stall watchdog (Batch 7). The retry supervisor classifies failures by
//! HTTP status, but a gateway "HTTP 200 empty/malformed" loop returns 200 — not a
//! retryable status — so a silently-hung agent reads `Running` forever (the
//! 2026-06-22 incident). This watchdog watches per-agent event silence: an agent
//! that is mid-turn (busy) but has emitted no token/tool event for
//! `STALL_THRESHOLD`, with no tool in flight, is flagged `Stalled`; when it
//! resumes it returns to `Running`. It only manages Running↔Stalled — the
//! supervisor owns Retrying/Dead. Runs in solo too (catches a hung Brian).

use crate::core::activity::{ActivityTracker, SessionActivity};
use crate::core::ipav::IpavState;
use crate::signaling::SignalingBridge;
use crate::storage::{Author, MessageKind, Storage};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tracing::warn;

/// How long an agent can be busy + silent (no events, no tool in flight) before
/// it's flagged Stalled. Generous: tool execution is covered by `tools_in_flight`,
/// so this only bounds model "thinking" / API latency between events.
pub const STALL_THRESHOLD: Duration = Duration::from_secs(90);
/// How often the watchdog re-checks each agent.
pub const POLL_INTERVAL: Duration = Duration::from_secs(10);
/// How long a session may sit `Idle` after the first user prompt, with no tray
/// flag pending, before it's flagged idle-unflagged (chip + one HANDS nudge).
/// The measured stalls this exists for (13 "what happened?" probes, 9 with zero
/// flags) had silent gaps of 2 min–9.7 h; 90 s converts them into a parked
/// question ~2 min after the settle. User-picked 2026-08-05 (variant 1).
pub const IDLE_GRACE: Duration = Duration::from_secs(90);

/// Per-agent liveness, shared between the agent's pump (updates it) and the
/// session watchdog task (reads it). `std`-sync — the pump touches it from a
/// sync path between awaits.
pub struct AgentLiveness {
    last_event: Mutex<Instant>,
    tools_in_flight: AtomicU32,
}

impl AgentLiveness {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            last_event: Mutex::new(Instant::now()),
            tools_in_flight: AtomicU32::new(0),
        })
    }

    /// Any event from the agent → it's alive; reset the silence timer.
    pub fn touch(&self) {
        *self.last_event.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
    }

    /// A tool call started (ToolUse). While > 0, stall detection is suppressed —
    /// a long `cargo build` / `npm install` emits no events until its ToolResult.
    /// A counter (not a bool) because claude-code can emit parallel tool calls.
    pub fn tool_started(&self) {
        self.tools_in_flight.fetch_add(1, Ordering::Release);
    }

    /// A tool call's result returned (ToolResult). Saturating — never underflow.
    pub fn tool_finished(&self) {
        let _ = self.tools_in_flight.fetch_update(
            Ordering::Release,
            Ordering::Acquire,
            |n| Some(n.saturating_sub(1)),
        );
    }

    /// Turn ended → no tools can still be in flight (results precede
    /// TurnComplete). Resets the counter so a stranded ToolUse-without-ToolResult
    /// can't wedge stall detection off forever.
    pub fn reset_tools(&self) {
        self.tools_in_flight.store(0, Ordering::Release);
    }

    pub fn tools_in_flight(&self) -> u32 {
        self.tools_in_flight.load(Ordering::Acquire)
    }

    pub fn idle_for(&self) -> Duration {
        self.last_event
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .elapsed()
    }
}

/// Pure decision: given an agent's current signals + its last-reported health,
/// what health (if any) should the watchdog emit? Only flips Running↔Stalled;
/// returns `None` (no change) for everything else — crucially, it never
/// overrides a supervisor-owned `Retrying`/`Dead`. `None` current = no
/// transition reported = assume running.
fn stall_decision(
    busy: bool,
    tools_in_flight: u32,
    idle: Duration,
    current: Option<&str>,
    threshold: Duration,
) -> Option<&'static str> {
    let stalled_now = busy && tools_in_flight == 0 && idle > threshold;
    match current {
        None | Some("running") if stalled_now => Some("stalled"),
        Some("stalled") if !stalled_now => Some("running"),
        _ => None,
    }
}

/// What the idle-unflagged check decided for one poll tick.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct IdleDecision {
    /// Show / keep the "needs direction" attention chip.
    pub chip: bool,
    /// Additionally nudge HANDS to declare state (once per user-silence window).
    pub nudge: bool,
}

/// Pure decision for the idle-unflagged watchdog (the "What happened?" fix —
/// a session must always be working or visibly asking; bare `Idle` past grace
/// with no tray flag after the first prompt is the anomaly).
///
/// - `idle_for` — how long the session has been continuously `Idle`; `None`
///   when it isn't idle.
/// - `user_broadcasts` — count of user prompts broadcast this session. 0 =
///   pre-first-task (the duo legitimately waits; never fire).
/// - `pending_tray` — a question/halt/gate is parked: legitimately waiting on
///   the user; never fire.
/// - `nudged_at` — the broadcast count when the last nudge was sent. A nudge
///   fires at most once per user-silence window; a new user prompt moves the
///   count and re-arms it.
/// - `hands_down` — HANDS health is dead/retrying/stalled: a nudge can't be
///   answered, so chip only (and the nudge stays un-consumed for recovery).
pub(crate) fn idle_unflagged_decision(
    idle_for: Option<Duration>,
    user_broadcasts: u64,
    pending_tray: bool,
    nudged_at: Option<u64>,
    hands_down: bool,
    grace: Duration,
) -> IdleDecision {
    let chip = idle_for.is_some_and(|d| d >= grace) && user_broadcasts > 0 && !pending_tray;
    let nudge = chip && nudged_at != Some(user_broadcasts) && !hands_down;
    IdleDecision { chip, nudge }
}

/// The idle-unflagged watchdog's handles into one session, threaded from the
/// spawn site. Separate struct so `run_stall_watchdog`'s signature stays
/// readable.
pub struct IdleWatch {
    pub storage: Storage,
    /// HANDS' stdin — the nudge goes to Brian only (EYES has no state-declaring
    /// verbs; her settles resolve to whatever Brian flags).
    pub brian_input_tx: tokio::sync::mpsc::Sender<crate::agents::OutgoingUserMessage>,
    pub ipav: Arc<tokio::sync::Mutex<IpavState>>,
    /// Bumped by `AppState::broadcast` on every user prompt. In-memory on
    /// purpose: a storage count races the first poll at session start.
    pub user_broadcasts: Arc<AtomicU64>,
}

/// Weak refs to a duo session's router liveness + per-direction counters, so the
/// watchdog can surface a router that died while agents are still live (the
/// peer-forward subsystem going down without taking the agents with it). `Weak`
/// so the watchdog never keeps the router state alive past the session. `None`
/// for solo sessions (no router).
pub struct RouterWatch {
    pub alive: Weak<AtomicBool>,
    pub fwd_brian_to_rain: Weak<AtomicU64>,
    pub fwd_rain_to_brian: Weak<AtomicU64>,
}

/// Per-session watchdog loop. Holds `Weak<AgentLiveness>` per agent so it
/// self-terminates once every pump has exited (the session ended) — no leaked
/// task. Emits health only on change via the bridge registry. Also watches the
/// peer-forward router (`router`): a dead router while agents are live is an
/// anomaly (forwarding is down) — warn + emit a router-health event once.
pub async fn run_stall_watchdog(
    session_id: String,
    agents: Vec<(Author, Weak<AgentLiveness>)>,
    activity: Arc<ActivityTracker>,
    bridge: Arc<SignalingBridge>,
    router: Option<RouterWatch>,
    idle_watch: IdleWatch,
) {
    // Idle-unflagged tracking (loop-local; see `idle_unflagged_decision`).
    let mut idle_since: Option<Instant> = None;
    let mut nudged_at: Option<u64> = None;
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        let mut any_alive = false;
        for (author, weak) in &agents {
            let Some(liveness) = weak.upgrade() else {
                continue; // this agent's pump has exited
            };
            any_alive = true;
            let current = bridge.current_agent_health(&session_id, author.as_str());
            let decision = stall_decision(
                activity.is_busy(*author),
                liveness.tools_in_flight(),
                liveness.idle_for(),
                current.as_deref(),
                STALL_THRESHOLD,
            );
            if let Some(next) = decision {
                bridge.notify_agent_health(session_id.clone(), author.as_str(), next);
            }
        }
        // Router liveness: flag ONLY the anomaly — router dead while agents still
        // live. At session end agents are gone too (`any_alive` false → we break
        // below), so a normal shutdown never trips this. Emit once on transition
        // (the registry is the only-on-change guard, like agent health).
        if let (true, Some(rw)) = (any_alive, &router) {
            if let Some(alive) = rw.alive.upgrade() {
                if !alive.load(Ordering::Acquire)
                    && bridge.current_router_health(&session_id) != Some(false)
                {
                    let load = |w: &Weak<AtomicU64>| {
                        w.upgrade().map(|c| c.load(Ordering::Relaxed)).unwrap_or(0)
                    };
                    warn!(
                        session_id = %session_id,
                        fwd_brian_to_rain = load(&rw.fwd_brian_to_rain),
                        fwd_rain_to_brian = load(&rw.fwd_rain_to_brian),
                        "peer-forward router DIED while agents are live — forwarding is DOWN"
                    );
                    bridge.notify_router_health(session_id.clone(), false);
                }
            }
        }
        // ── Idle-unflagged watchdog (the "What happened?" fix) ──────────────
        // A session must always be either working or visibly asking. Bare
        // `Idle` past IDLE_GRACE with no pending tray row, after the first
        // user prompt, gets an attention chip + one HANDS nudge per
        // user-silence window. Detection is host-side because only the host
        // has post-settlement truth (a Stop hook fires before the final text
        // is routed and would false-block turns whose text wakes the peer).
        {
            let state = activity.current();
            if state == SessionActivity::Idle {
                let since = *idle_since.get_or_insert_with(Instant::now);
                let broadcasts = idle_watch.user_broadcasts.load(Ordering::Acquire);
                // Only pay the (indexed) tray query once the cheap gates pass.
                let candidate = since.elapsed() >= IDLE_GRACE && broadcasts > 0;
                let pending_tray = if candidate {
                    match idle_watch.storage.has_pending_tray(&session_id).await {
                        Ok(p) => p,
                        // Fail closed-to-quiet: a storage error must not spam
                        // chips/nudges off a guess.
                        Err(e) => {
                            warn!(session_id = %session_id, error = %e,
                                  "idle watchdog: pending-tray query failed; skipping tick");
                            true
                        }
                    }
                } else {
                    true // not a candidate yet — value unused beyond suppressing
                };
                let hands_down = matches!(
                    bridge
                        .current_agent_health(&session_id, Author::Brian.as_str())
                        .as_deref(),
                    Some("dead") | Some("retrying") | Some("stalled")
                );
                let decision = idle_unflagged_decision(
                    candidate.then(|| since.elapsed()),
                    broadcasts,
                    pending_tray,
                    nudged_at,
                    hands_down,
                    IDLE_GRACE,
                );
                // One call covers both directions: sets the chip while the
                // anomaly holds, clears it if a tray flag appears mid-idle
                // (legitimately waiting again). The bridge dedupes.
                bridge.notify_session_attention(
                    session_id.clone(),
                    decision.chip.then_some("idle_unflagged"),
                );
                if decision.nudge {
                    // Re-verify at the send boundary: the user may have paused
                    // or spoken between the poll read and here.
                    if activity.current() == SessionActivity::Idle
                        && !activity.holds_wakes()
                    {
                        nudged_at = Some(broadcasts);
                        deliver_idle_nudge(&session_id, &idle_watch, &activity, &bridge)
                            .await;
                    }
                }
            } else {
                idle_since = None;
                // Transition out of Idle clears the chip (bridge dedupes, so
                // calling this every non-idle poll is a cheap no-op).
                bridge.notify_session_attention(session_id.clone(), None);
            }
        }
        if !any_alive {
            break; // all pumps gone → session ended
        }
    }
}

/// Persist the chat-visible notice and push the declare-state nudge into
/// HANDS' stdin. Best-effort on every edge: a dead input channel or a failed
/// insert degrades to the chip alone (already emitted by the caller).
async fn deliver_idle_nudge(
    session_id: &str,
    idle_watch: &IdleWatch,
    activity: &ActivityTracker,
    bridge: &SignalingBridge,
) {
    const NOTICE: &str =
        "Session idled with no question or halt parked — nudged Brian to declare state.";
    const NUDGE: &str = "[System: this session went idle with no question parked and no \
        halt flag — the user cannot tell settled from stalled. Declare state now, with a \
        tool rather than bare prose: continue work the user already directed if any \
        remains; park a question with your recommendation (ask_user_choice); yield with \
        a reason (halt / mark_awaiting_user); or ask to close if the task is done. Never \
        invent a direction or new work to satisfy this nudge — if no user-given \
        direction exists, the right response IS the question: ask which direction.]";
    match idle_watch
        .storage
        .insert_message(session_id, Author::User, MessageKind::SystemNotice, NOTICE)
        .await
    {
        Ok(id) => bridge.notify_message_persisted(Arc::from(session_id), id),
        Err(e) => warn!(session_id = %session_id, error = %e,
                        "idle watchdog: failed to persist system notice"),
    }
    let phase = idle_watch.ipav.lock().await.current_phase;
    let wire = crate::core::broadcast::with_phase_envelope(phase, NUDGE);
    if idle_watch
        .brian_input_tx
        .send(crate::agents::OutgoingUserMessage::text(wire))
        .await
        .is_ok()
    {
        // Mirror the dispatch sites: input sent → HANDS is mid-turn, so the
        // session reads Busy (and the chip clears) while he declares state.
        activity.set_busy(Author::Brian, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveness_touch_and_tools() {
        let l = AgentLiveness::new();
        assert_eq!(l.tools_in_flight(), 0);
        l.tool_started();
        l.tool_started();
        assert_eq!(l.tools_in_flight(), 2);
        l.tool_finished();
        assert_eq!(l.tools_in_flight(), 1);
        l.reset_tools();
        assert_eq!(l.tools_in_flight(), 0);
        // Saturating: never underflow below 0.
        l.tool_finished();
        assert_eq!(l.tools_in_flight(), 0);
    }

    const T: Duration = Duration::from_secs(90);
    const PAST: Duration = Duration::from_secs(120); // > threshold
    const FRESH: Duration = Duration::from_secs(5); // < threshold

    #[test]
    fn stall_decision_flags_busy_silent_no_tool() {
        // Running + busy + silent past threshold + no tool → Stalled.
        assert_eq!(stall_decision(true, 0, PAST, Some("running"), T), Some("stalled"));
        // None (no transition yet) is treated as running.
        assert_eq!(stall_decision(true, 0, PAST, None, T), Some("stalled"));
    }

    #[test]
    fn stall_decision_suppressed_by_tool_in_flight() {
        // A long tool call (no events while it runs) must NOT flag stalled.
        assert_eq!(stall_decision(true, 1, PAST, Some("running"), T), None);
    }

    #[test]
    fn stall_decision_needs_busy_and_silence() {
        // Idle agent (not busy) is expected to be silent → not stalled.
        assert_eq!(stall_decision(false, 0, PAST, Some("running"), T), None);
        // Busy but recently active → not stalled.
        assert_eq!(stall_decision(true, 0, FRESH, Some("running"), T), None);
    }

    #[test]
    fn stall_decision_recovers_from_stalled() {
        // Was stalled, now active (fresh) → back to running.
        assert_eq!(stall_decision(true, 0, FRESH, Some("stalled"), T), Some("running"));
        // Was stalled, tool now in flight → recovered.
        assert_eq!(stall_decision(true, 1, PAST, Some("stalled"), T), Some("running"));
        // Still stalled → no re-emit (only on change).
        assert_eq!(stall_decision(true, 0, PAST, Some("stalled"), T), None);
    }

    const GRACE: Duration = Duration::from_secs(90);
    const OVER: Option<Duration> = Some(Duration::from_secs(120));
    const UNDER: Option<Duration> = Some(Duration::from_secs(30));

    #[test]
    fn idle_decision_fires_after_grace_with_task_and_no_flag() {
        let d = idle_unflagged_decision(OVER, 3, false, None, false, GRACE);
        assert_eq!(d, IdleDecision { chip: true, nudge: true });
    }

    #[test]
    fn idle_decision_quiet_before_first_prompt() {
        // The pre-first-task wait is legitimate — never chip, never nudge.
        let d = idle_unflagged_decision(OVER, 0, false, None, false, GRACE);
        assert_eq!(d, IdleDecision { chip: false, nudge: false });
    }

    #[test]
    fn idle_decision_quiet_under_grace_or_not_idle() {
        assert!(!idle_unflagged_decision(UNDER, 3, false, None, false, GRACE).chip);
        assert!(!idle_unflagged_decision(None, 3, false, None, false, GRACE).chip);
    }

    #[test]
    fn idle_decision_suppressed_by_pending_tray() {
        // A parked question/halt/gate = legitimately waiting on the user.
        let d = idle_unflagged_decision(OVER, 3, true, None, false, GRACE);
        assert_eq!(d, IdleDecision { chip: false, nudge: false });
    }

    #[test]
    fn idle_decision_nudges_once_per_user_silence_window() {
        // Already nudged at broadcast #3 → chip stays, nudge doesn't repeat.
        let d = idle_unflagged_decision(OVER, 3, false, Some(3), false, GRACE);
        assert_eq!(d, IdleDecision { chip: true, nudge: false });
        // A new user prompt (count moved to 4) re-arms the nudge.
        let d = idle_unflagged_decision(OVER, 4, false, Some(3), false, GRACE);
        assert_eq!(d, IdleDecision { chip: true, nudge: true });
    }

    #[test]
    fn idle_decision_chip_only_when_hands_down() {
        // A dead/retrying/stalled HANDS can't answer — chip without nudge,
        // and the nudge stays un-consumed (nudged_at untouched by caller) so
        // recovery gets exactly one.
        let d = idle_unflagged_decision(OVER, 3, false, None, true, GRACE);
        assert_eq!(d, IdleDecision { chip: true, nudge: false });
    }

    #[test]
    fn stall_decision_never_overrides_supervisor() {
        // Retrying/Dead are supervisor-owned — the watchdog leaves them alone,
        // even if the agent looks stalled by the silence heuristic.
        assert_eq!(stall_decision(true, 0, PAST, Some("retrying"), T), None);
        assert_eq!(stall_decision(true, 0, PAST, Some("dead"), T), None);
    }
}
