//! Silence/stall watchdog (Batch 7). The retry supervisor classifies failures by
//! HTTP status, but a gateway "HTTP 200 empty/malformed" loop returns 200 — not a
//! retryable status — so a silently-hung agent reads `Running` forever (the
//! 2026-06-22 incident). This watchdog watches per-agent event silence: an agent
//! that is mid-turn (busy) but has emitted no token/tool event for
//! `STALL_THRESHOLD`, with no tool in flight, is flagged `Stalled`; when it
//! resumes it returns to `Running`. It only manages Running↔Stalled — the
//! supervisor owns Retrying/Dead. Runs in solo too (catches a hung Brian).

use crate::core::activity::ActivityTracker;
use crate::signaling::SignalingBridge;
use crate::storage::Author;
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
) {
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
        if !any_alive {
            break; // all pumps gone → session ended
        }
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

    #[test]
    fn stall_decision_never_overrides_supervisor() {
        // Retrying/Dead are supervisor-owned — the watchdog leaves them alone,
        // even if the agent looks stalled by the silence heuristic.
        assert_eq!(stall_decision(true, 0, PAST, Some("retrying"), T), None);
        assert_eq!(stall_decision(true, 0, PAST, Some("dead"), T), None);
    }
}
