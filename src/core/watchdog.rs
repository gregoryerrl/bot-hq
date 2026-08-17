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
use crate::storage::{Envelope, MessageKind, Storage};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
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
/// - `halted` — the session's halt slot is filled: the stop is DECLARED
///   (an agent recap, the provider-limit stall, the error streak, spin, the
///   all-pass yield, the round cap, consensus — every stop fills the slot as
///   of 2026-08-15). Suppress like `pending_tray`: the banner is the state,
///   and the floor is the user's. With every legitimate stop declared, this
///   watchdog's remaining subject is the WEDGE — idle with an empty slot,
///   empty tray and no gate, which normal operation can no longer produce.
pub(crate) fn idle_unflagged_decision(
    idle_for: Option<Duration>,
    user_broadcasts: u64,
    pending_tray: bool,
    nudged_at: Option<u64>,
    hands_down: bool,
    halted: bool,
    grace: Duration,
) -> IdleDecision {
    let chip = idle_for.is_some_and(|d| d >= grace)
        && user_broadcasts > 0
        && !pending_tray
        && !halted;
    let nudge = chip && nudged_at != Some(user_broadcasts) && !hands_down;
    IdleDecision { chip, nudge }
}

/// The idle-unflagged watchdog's handles into one session, threaded from the
/// spawn site. Separate struct so `run_stall_watchdog`'s signature stays
/// readable.
pub struct IdleWatch {
    pub storage: Storage,
    /// The participant that can act on the nudge — the first one holding
    /// `edit_files` (rc3 D10/D11: a capability, not a name). A review-only
    /// participant has no state-declaring verbs to answer with.
    ///
    /// An id to SUMMON with, not a stdin to write to. The nudge used to go
    /// straight into that stdin, which woke a generation the ring had not dealt:
    /// its rows posted, its completion carried a stale epoch and was discarded,
    /// and the busy flag was set for turn slot 0 whether or not slot 0 was this
    /// participant. Now the row is persisted and the ring is released with this
    /// id as a mention, so the wake is a real turn — dealt, marked and completed
    /// like every other.
    pub hands_participant_id: Option<i64>,
    pub ipav: Arc<tokio::sync::Mutex<IpavState>>,
    /// Bumped by `AppState::broadcast` on every user prompt. In-memory on
    /// purpose: a storage count races the first poll at session start.
    pub user_broadcasts: Arc<AtomicU64>,
    /// The session this watch reads halt state for. Every declared stop now
    /// fills the session's HALT SLOT (agent halts, provider-limit,
    /// error-streak, spin, the all-pass yield, the round cap, consensus), so
    /// the slot is the one suppression signal the nudge needs — a halted
    /// session is declared, not stalled.
    pub session_id: String,
}

/// Per-session watchdog loop. Holds `Weak<AgentLiveness>` per agent so it
/// self-terminates once every pump has exited (the session ended) — no leaked
/// task. Emits health only on change via the bridge registry, and runs the
/// idle-unflagged nudge (see [`IdleWatch`]).
///
/// **It watches nothing else.** Until the round-3 audit this doc also claimed it
/// "watches the peer-forward router (`router`): a dead router while agents are
/// live is an anomaly (forwarding is down) — warn + emit a router-health event
/// once." `core/router.rs` was deleted by task 14 and there has never been a
/// router-health event: a grep for one across `src/` returned exactly one hit,
/// that sentence. The claim survived its subject by four days and described a
/// monitor that does not exist, in the file that documents this session's
/// monitoring — so a reader checking whether forwarding was watched came away
/// believing it was. Nothing watches forwarding, and with one turn engine there
/// is no separate forwarder left to watch.
///
/// `agents` is keyed by participant SLUG, so a session with three participants
/// gets three health streams instead of two (rc3 D10 — it was a `Vec<(Author,
/// …)>`, and `Author` had exactly two agent variants; the type itself is now
/// deleted). `hands_slug` names the participant whose health the idle nudge is
/// suppressed on; `None` when this session has nobody that can act on it.
///
/// **This doc block has now produced two stale claims in one session** — the
/// phantom router-health watch above (round-3 F1) and the tense of that clause
/// (EYES, `a6ea28ff`), fourteen lines apart. Both were justifying asides about
/// subsystems that had been deleted, and neither was reachable by any check the
/// repo runs. If a third turns up, the block is the defect, not the sentence.
pub async fn run_stall_watchdog(
    session_id: String,
    agents: Vec<(String, Weak<AgentLiveness>)>,
    hands_slug: Option<String>,
    activity: Arc<ActivityTracker>,
    bridge: Arc<SignalingBridge>,
    idle_watch: Option<IdleWatch>,
) {
    // Idle-unflagged tracking (loop-local; see `idle_unflagged_decision`).
    let mut idle_since: Option<Instant> = None;
    let mut nudged_at: Option<u64> = None;
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        let mut any_alive = false;
        for (slug, weak) in &agents {
            let Some(liveness) = weak.upgrade() else {
                continue; // this agent's pump has exited
            };
            any_alive = true;
            let current = bridge.current_agent_health(&session_id, slug);
            let decision = stall_decision(
                activity.is_busy_slug(slug),
                liveness.tools_in_flight(),
                liveness.idle_for(),
                current.as_deref(),
                STALL_THRESHOLD,
            );
            if let Some(next) = decision {
                bridge.notify_agent_health(session_id.clone(), slug, next);
            }
        }
        // ── Idle-unflagged watchdog (the "What happened?" fix) ──────────────
        // A session must always be either working or visibly asking. Bare
        // `Idle` past IDLE_GRACE with no pending tray row, after the first
        // user prompt, gets an attention chip + one HANDS nudge per
        // user-silence window. Detection is host-side because only the host
        // has post-settlement truth (a Stop hook fires before the final text
        // is routed and would false-block turns whose text wakes the peer).
        if let Some(idle_watch) = idle_watch.as_ref() {
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
                // A filled halt slot is a DECLARED stop — every stop fills it
                // now (2026-08-15: "HALT means the floor is the user's"), so
                // idle-under-a-halt is the design, never the anomaly. Same
                // lazy gate and same fail-quiet posture as the tray query.
                let halted = if candidate {
                    match idle_watch.storage.session_halt(&idle_watch.session_id).await {
                        Ok(h) => h.is_some(),
                        Err(e) => {
                            warn!(session_id = %session_id, error = %e,
                                  "idle watchdog: halt-slot query failed; skipping tick");
                            true
                        }
                    }
                } else {
                    true
                };
                // Suppress the nudge when the participant it is addressed to is
                // down — it would be pushed into a dead stdin. Keyed on that
                // participant's slug (rc3 D10); no `hands_slug` means nobody can
                // act on the nudge, which is not the same as "the actor is
                // down", so the decision runs unsuppressed and the chip still
                // reaches the user.
                let hands_down = hands_slug.as_deref().is_some_and(|s| {
                    matches!(
                        bridge.current_agent_health(&session_id, s).as_deref(),
                        Some("dead") | Some("retrying") | Some("stalled")
                    )
                });
                let decision = idle_unflagged_decision(
                    candidate.then(|| since.elapsed()),
                    broadcasts,
                    pending_tray,
                    nudged_at,
                    hands_down,
                    halted,
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
                        deliver_idle_nudge(&session_id, idle_watch, &activity, &bridge)
                            .await;
                    }
                } else if decision.chip && nudged_at == Some(broadcasts) {
                    // The wedge outlived this window's one nudge — measured in
                    // s-d6352684 (2026-08-15): a nudge-woken generation ended
                    // on prose with no tool, its completion was discarded
                    // outside the ring, and the session sat bannerless with
                    // the nudge spent. The net stops detecting and DECLARES:
                    // every stop is a HALT, even the stop nobody declared.
                    // Filling the slot flips `halted` on the next poll, so
                    // this fires once per wedge and never spams.
                    if activity.current() == SessionActivity::Idle
                        && !activity.holds_wakes()
                    {
                        let _ = bridge
                            .mark_awaiting_user(
                                session_id.clone(),
                                "system".to_string(),
                                "This session went idle with nothing declared — \
                                 no halt, no question, no gate — and the nudge \
                                 went unanswered. Send a message to resume."
                                    .to_string(),
                            )
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
        "Session idled with no question or halt parked — nudged the executor to declare state.";
    const NUDGE: &str = "[System: this session went idle with no question parked and no \
        halt flag — the user cannot tell settled from stalled. Declare state now, with a \
        tool rather than bare prose: continue work the user already directed if any \
        remains; park a question with your recommendation (ask_user_choice); yield with \
        a reason (halt / mark_awaiting_user); or ask to close if the task is done. Never \
        invent a direction or new work to satisfy this nudge — if no user-given \
        direction exists, the right response IS the question: ask which direction.]";
    // TWO rows, because this site says two different things. NOTICE is the
    // one-line summary the user reads in the chat; NUDGE is the instruction
    // the executor reads. Before B5 Task 2 only NOTICE was recorded and NUDGE
    // went straight to stdin, so the chat's account of what the watchdog sent
    // was short by the entire instruction — the widest of the divergences.
    //
    // Both are host-authored — `origin = 'system'`, NULL participant (0044):
    // neither is anybody's turn output and there is no `system` roster row to
    // attribute them to. NOTICE used to go through the user-row writer and
    // reached every participant as `[user] Session idled…` — bot-hq in the
    // user's voice, the exact misattribution `speaker_of`'s doc forbids.
    crate::core::post_system_notice(
        &idle_watch.storage,
        bridge,
        session_id,
        MessageKind::SystemNotice,
        NOTICE,
        None,
    )
    .await;
    let phase = idle_watch.ipav.lock().await.current_phase;
    // No row, no wire. The chip is already up and NOTICE may already be in the
    // chat, so the user still sees that the session stalled.
    let Some(_nudge) = crate::core::post_system_notice(
        &idle_watch.storage,
        bridge,
        session_id,
        MessageKind::SystemNotice,
        NUDGE,
        Some(Envelope::phase(phase.name())),
    )
    .await
    else {
        return;
    };
    // **The wake is a ring release, not a stdin write.** The nudge fires exactly
    // when no turn is coming — idle, nothing parked — so the row alone would sit
    // unread until the user typed, which is the one event this exists to spare
    // them. Releasing the ring deals a turn that reads the row off the backlog:
    // the participant is marked busy BY the ring (so the right one is marked,
    // and the chip stays clear while the nudge is unanswered), and its
    // completion is one the ring is expecting.
    //
    // Mentioned rather than left to the rotation: the nudge asks for a state
    // DECLARATION, and only the participant that can act on it should be woken
    // to give one.
    bridge
        .notify_ring_user_message(
            session_id,
            idle_watch.hands_participant_id.into_iter().collect(),
        )
        .await;
    let _ = activity;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

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
        let d = idle_unflagged_decision(OVER, 3, false, None, false, false, GRACE);
        assert_eq!(d, IdleDecision { chip: true, nudge: true });
    }

    #[test]
    fn idle_decision_quiet_before_first_prompt() {
        // The pre-first-task wait is legitimate — never chip, never nudge.
        let d = idle_unflagged_decision(OVER, 0, false, None, false, false, GRACE);
        assert_eq!(d, IdleDecision { chip: false, nudge: false });
    }

    #[test]
    fn idle_decision_quiet_under_grace_or_not_idle() {
        assert!(!idle_unflagged_decision(UNDER, 3, false, None, false, false, GRACE).chip);
        assert!(!idle_unflagged_decision(None, 3, false, None, false, false, GRACE).chip);
    }

    #[test]
    fn idle_decision_suppressed_by_pending_tray() {
        // A parked question/halt/gate = legitimately waiting on the user.
        let d = idle_unflagged_decision(OVER, 3, true, None, false, false, GRACE);
        assert_eq!(d, IdleDecision { chip: false, nudge: false });
    }

    #[test]
    fn idle_decision_nudges_once_per_user_silence_window() {
        // Already nudged at broadcast #3 → chip stays, nudge doesn't repeat.
        let d = idle_unflagged_decision(OVER, 3, false, Some(3), false, false, GRACE);
        assert_eq!(d, IdleDecision { chip: true, nudge: false });
        // A new user prompt (count moved to 4) re-arms the nudge.
        let d = idle_unflagged_decision(OVER, 4, false, Some(3), false, false, GRACE);
        assert_eq!(d, IdleDecision { chip: true, nudge: true });
    }

    #[test]
    fn idle_decision_chip_only_when_hands_down() {
        // A dead/retrying/stalled HANDS can't answer — chip without nudge,
        // and the nudge stays un-consumed (nudged_at untouched by caller) so
        // recovery gets exactly one.
        let d = idle_unflagged_decision(OVER, 3, false, None, true, false, GRACE);
        assert_eq!(d, IdleDecision { chip: true, nudge: false });
    }

    #[test]
    fn idle_decision_suppressed_while_working_declared() {
        // An unexpired declare_working means bare-Idle is EXPECTED — no chip,
        // no nudge, exactly like a pending tray row. A filled halt slot is a
        // DECLARED stop (2026-08-15: every stop is a HALT) — idle under a
        // banner is the design, never the anomaly.
        let d = idle_unflagged_decision(OVER, 3, false, None, false, true, GRACE);
        assert_eq!(d, IdleDecision { chip: false, nudge: false });
    }

    #[test]
    fn stall_decision_never_overrides_supervisor() {
        // Retrying/Dead are supervisor-owned — the watchdog leaves them alone,
        // even if the agent looks stalled by the silence heuristic.
        assert_eq!(stall_decision(true, 0, PAST, Some("retrying"), T), None);
        assert_eq!(stall_decision(true, 0, PAST, Some("dead"), T), None);
    }

    #[tokio::test]
    async fn the_idle_nudge_writes_the_notice_and_the_nudge_as_two_rows() {
        // This site says two different things: NOTICE is the user's one-line
        // summary, NUDGE is Brian's instruction. Before B5 Task 2 only NOTICE
        // was recorded and NUDGE went straight to stdin, so the chat's account
        // of what the watchdog sent was incomplete by the whole instruction.
        let storage = Storage::memory().await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();
        let ipav = Arc::new(tokio::sync::Mutex::new(IpavState::default()));
        ipav.lock().await.advance(crate::core::ipav::IpavPhase::Apply);
        let idle_watch = IdleWatch {
            storage: storage.clone(),
            hands_participant_id: Some(1),
            ipav,
            user_broadcasts: Arc::new(AtomicU64::new(1)),
            session_id: "s1".to_string(),
        };
        let bridge = SignalingBridge::new();
        let activity =
            ActivityTracker::new(
                "s1",
                Arc::new(AtomicBool::new(false)),
                Arc::clone(&bridge),
                vec!["hands".into(), "eyes".into()],
            );
        // The ring, so the wake can be observed where it now happens.
        let (ring_tx, mut ring_rx) = tokio::sync::mpsc::channel(4);
        bridge
            .register_session_sequencer("s1".to_string(), ring_tx)
            .await;
        deliver_idle_nudge("s1", &idle_watch, &activity, &bridge).await;

        let rows = storage.channel_after("s1", 0, 100).await.unwrap().rows;
        assert_eq!(rows.len(), 2, "NOTICE and NUDGE are separate messages");
        // Host-authored too (round 7): the notice is bot-hq's one-line summary,
        // and until it posted as `system` every participant read it as
        // `[user] Session idled…` — the user "saying" a thing bot-hq did.
        assert_eq!(rows[0].origin, "system", "the notice is bot-hq talking, not the user");
        assert!(rows[0].participant_id.is_none());
        assert!(rows[0].content.starts_with("Session idled"));
        // The nudge is host-authored: `system` origin, no participant (0044).
        // rc3 D23: it renders `[system]` on the wire, and that matters here more
        // than anywhere — the nudge is bot-hq talking, and an agent that reads it
        // as the USER has been handed a fabricated instruction.
        assert_eq!(rows[1].origin, "system");
        assert!(rows[1].participant_id.is_none());
        assert!(rows[1].content.starts_with("[System: this session went idle"));

        // **And the wake is a ring RELEASE, mentioning the participant that can
        // answer it.** It used to be a write into that participant's stdin,
        // which opened a generation the ring had not dealt: rows posted, the
        // completion carried a stale epoch and was discarded, and the busy flag
        // went to turn slot 0 whoever slot 0 happened to be. The nudge only
        // works if something wakes — it fires when no turn is coming — so the
        // wake stays; what changed is that the ring makes it a real turn.
        match ring_rx.try_recv() {
            Ok(crate::core::sequencer::SequencerCommand::UserMessage { mentions }) => {
                assert_eq!(
                    mentions,
                    vec![1],
                    "the nudge asks for a state declaration, so it summons the \
                     participant that can give one"
                );
            }
            other => panic!("the idle nudge did not release the ring: {other:?}"),
        }
        assert!(
            ring_rx.try_recv().is_err(),
            "one wake, not one per row — the NOTICE is not addressed to anyone"
        );
    }
}
