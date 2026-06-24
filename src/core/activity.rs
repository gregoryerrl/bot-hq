//! Per-session duo activity — the source of truth for the chat-input lock +
//! Cancel button (interrupt redesign, Batch 2). Mirrors the `awaiting`
//! `Arc<AtomicBool>` pattern: created per session in `spawn_session_handle`,
//! shared with the duo pump (which clears `busy` on `TurnComplete`) and the
//! dispatch sites (which set `busy` when input is sent to an agent), and emits a
//! `SessionActivity` SignalingEvent whenever the derived state changes.

use crate::signaling::SignalingBridge;
use crate::storage::Author;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Session-level duo activity, as surfaced to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionActivity {
    /// Both agents idle, not awaiting the user — input ENABLED.
    Idle,
    /// At least one agent mid-turn — input DISABLED, Cancel shown.
    Busy,
    /// An agent parked a question for the user — input ENABLED (it's the user's
    /// turn). Distinct from `Idle` so the UI can signal "your move".
    AwaitingUser,
    /// A cancel is in flight (kill issued, settling) — input DISABLED until it
    /// returns to `Idle`.
    Cancelling,
}

impl SessionActivity {
    /// Wire string — the contract with the frontend `session:activity` payload.
    /// Locked by a test; don't drift.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionActivity::Idle => "idle",
            SessionActivity::Busy => "busy",
            SessionActivity::AwaitingUser => "awaiting_user",
            SessionActivity::Cancelling => "cancelling",
        }
    }

    /// Pure derivation from the four inputs. Priority (highest first):
    /// `cancelling` > `awaiting` > `busy` (either agent) > `idle`. `cancelling`
    /// wins so a kill-in-flight is never masked by residual busy; `awaiting`
    /// wins over `busy` so a parked question re-opens input even though a turn
    /// is technically still in flight.
    pub fn derive(brian_busy: bool, rain_busy: bool, awaiting: bool, cancelling: bool) -> Self {
        if cancelling {
            SessionActivity::Cancelling
        } else if awaiting {
            SessionActivity::AwaitingUser
        } else if brian_busy || rain_busy {
            SessionActivity::Busy
        } else {
            SessionActivity::Idle
        }
    }
}

/// Guarded mutable state. Kept behind one `Mutex` so concurrent set/clear from
/// both pumps + the dispatch sites serialize — no interleave can emit a
/// spurious intermediate (e.g. a momentary `Idle` between `Busy{brian}` and
/// `Busy{rain}` during a peer hand-off).
struct Inner {
    brian_busy: bool,
    rain_busy: bool,
    cancelling: bool,
    /// Last state we emitted — so we only fire on an actual change.
    last: SessionActivity,
}

/// Per-session activity tracker. Hold via `Arc`; all mutators take `&self`.
pub struct ActivityTracker {
    inner: Mutex<Inner>,
    /// Shared with the session's `awaiting` halt flag (set by
    /// `mark_awaiting_user` / `ask_user_choice`). Read at derive time; a caller
    /// that flips it must also call [`refresh`](Self::refresh).
    awaiting: Arc<AtomicBool>,
    bridge: Arc<SignalingBridge>,
    session_id: String,
}

impl ActivityTracker {
    pub fn new(
        session_id: impl Into<String>,
        awaiting: Arc<AtomicBool>,
        bridge: Arc<SignalingBridge>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                brian_busy: false,
                rain_busy: false,
                cancelling: false,
                last: SessionActivity::Idle,
            }),
            awaiting,
            bridge,
            session_id: session_id.into(),
        })
    }

    /// Mark an agent busy (input dispatched) or idle (`TurnComplete`/`Exited`).
    /// No-op for `Author::User`. Emits iff the derived session state changed.
    pub fn set_busy(&self, author: Author, busy: bool) {
        if matches!(author, Author::User) {
            return;
        }
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match author {
            Author::Brian => g.brian_busy = busy,
            Author::Rain => g.rain_busy = busy,
            Author::User => unreachable!(),
        }
        self.recompute_locked(&mut g);
    }

    /// Mark a cancel in-flight (`true`) or settled (`false`). Emits on change.
    pub fn set_cancelling(&self, cancelling: bool) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.cancelling = cancelling;
        self.recompute_locked(&mut g);
    }

    /// Re-derive + emit after the shared `awaiting` flag was flipped elsewhere
    /// (the dispatch path owns that flag; this reflects the change into activity).
    pub fn refresh(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        self.recompute_locked(&mut g);
    }

    /// Current derived state (test/observability).
    pub fn current(&self) -> SessionActivity {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        SessionActivity::derive(
            g.brian_busy,
            g.rain_busy,
            self.awaiting.load(Ordering::Acquire),
            g.cancelling,
        )
    }

    /// Whether a specific agent is mid-turn — the Batch 7 stall watchdog reads
    /// this to tell a stall (busy + silent) from expected silence (idle).
    pub fn is_busy(&self, author: Author) -> bool {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match author {
            Author::Brian => g.brian_busy,
            Author::Rain => g.rain_busy,
            Author::User => false,
        }
    }

    /// Poll (50ms) until NEITHER agent is busy, or `deadline` elapses; returns
    /// whether they went idle in time. The cancel interrupt-escalation uses this:
    /// after a `control_request` interrupt, the turn's `result` event clears both
    /// `busy` flags (and auto-clears Cancelling→Idle). If that doesn't land within
    /// the window — interrupt dropped, or a wedged agent — the caller escalates to
    /// a SIGKILL. A solo session never has Rain busy, so this just waits on Brian.
    pub async fn await_both_idle(&self, deadline: tokio::time::Instant) -> bool {
        loop {
            {
                let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
                if !g.brian_busy && !g.rain_busy {
                    return true;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    fn recompute_locked(&self, g: &mut Inner) {
        // A cancel auto-completes once BOTH agents have gone idle (the kill
        // settled) — clear `cancelling` so the state transitions
        // Cancelling → Idle instead of sticking. Done BEFORE derive so the
        // emitted state reflects it. (Set true by `set_cancelling` on Stop;
        // the agents' pumps then clear their `busy` as they die.)
        if g.cancelling && !g.brian_busy && !g.rain_busy {
            g.cancelling = false;
        }
        let next = SessionActivity::derive(
            g.brian_busy,
            g.rain_busy,
            self.awaiting.load(Ordering::Acquire),
            g.cancelling,
        );
        if next != g.last {
            g.last = next;
            self.bridge
                .notify_session_activity(self.session_id.clone(), next.as_str());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signaling::SignalingEvent;

    #[test]
    fn as_str_wire_contract() {
        assert_eq!(SessionActivity::Idle.as_str(), "idle");
        assert_eq!(SessionActivity::Busy.as_str(), "busy");
        assert_eq!(SessionActivity::AwaitingUser.as_str(), "awaiting_user");
        assert_eq!(SessionActivity::Cancelling.as_str(), "cancelling");
    }

    #[test]
    fn derive_priority() {
        use SessionActivity::*;
        // idle: nothing set.
        assert_eq!(SessionActivity::derive(false, false, false, false), Idle);
        // busy: either agent.
        assert_eq!(SessionActivity::derive(true, false, false, false), Busy);
        assert_eq!(SessionActivity::derive(false, true, false, false), Busy);
        assert_eq!(SessionActivity::derive(true, true, false, false), Busy);
        // awaiting beats busy (parked question re-opens input mid-turn).
        assert_eq!(SessionActivity::derive(true, true, true, false), AwaitingUser);
        assert_eq!(SessionActivity::derive(false, false, true, false), AwaitingUser);
        // cancelling beats everything.
        assert_eq!(SessionActivity::derive(true, true, true, true), Cancelling);
        assert_eq!(SessionActivity::derive(false, false, false, true), Cancelling);
    }

    fn activity_state(ev: &SignalingEvent) -> Option<&str> {
        match ev {
            SignalingEvent::SessionActivity { state, .. } => Some(state),
            _ => None,
        }
    }

    #[tokio::test]
    async fn tracker_emits_only_on_change() {
        let bridge = SignalingBridge::new();
        let mut rx = bridge.subscribe();
        let awaiting = Arc::new(AtomicBool::new(false));
        let t = ActivityTracker::new("s1", awaiting.clone(), bridge.clone());

        // idle -> busy
        t.set_busy(Author::Brian, true);
        assert_eq!(activity_state(&rx.recv().await.unwrap()), Some("busy"));

        // busy -> busy (rain also starts): no change, no emit.
        t.set_busy(Author::Rain, true);
        assert!(rx.try_recv().is_err(), "no emit when still busy");

        // one agent idle, other still busy: still busy, no emit.
        t.set_busy(Author::Brian, false);
        assert!(rx.try_recv().is_err(), "still busy via rain, no emit");

        // both idle -> idle.
        t.set_busy(Author::Rain, false);
        assert_eq!(activity_state(&rx.recv().await.unwrap()), Some("idle"));

        // awaiting flips externally -> awaiting_user (after refresh).
        awaiting.store(true, Ordering::Release);
        t.refresh();
        assert_eq!(
            activity_state(&rx.recv().await.unwrap()),
            Some("awaiting_user")
        );
        awaiting.store(false, Ordering::Release);

        // Cancelling only "sticks" while an agent is busy (the kill is
        // settling). An agent goes busy, Stop sets cancelling (overrides busy),
        // then the agent dies (idle) → cancelling AUTO-CLEARS → Idle.
        t.set_busy(Author::Brian, true);
        assert_eq!(activity_state(&rx.recv().await.unwrap()), Some("busy"));
        t.set_cancelling(true);
        assert_eq!(activity_state(&rx.recv().await.unwrap()), Some("cancelling"));
        t.set_busy(Author::Brian, false);
        assert_eq!(activity_state(&rx.recv().await.unwrap()), Some("idle"));
    }

    #[tokio::test]
    async fn cancelling_set_while_idle_auto_clears() {
        // Defensive: if a cancel is somehow set when no agent is busy, it
        // auto-clears immediately (the kill has nothing to settle) — never a
        // stuck Cancelling that locks the input forever.
        let bridge = SignalingBridge::new();
        let awaiting = Arc::new(AtomicBool::new(false));
        let t = ActivityTracker::new("s1", awaiting, bridge);
        t.set_cancelling(true);
        assert_eq!(t.current(), SessionActivity::Idle);
    }

    #[tokio::test]
    async fn await_both_idle_true_when_already_idle() {
        let t = ActivityTracker::new("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(500);
        assert!(t.await_both_idle(deadline).await);
    }

    #[tokio::test]
    async fn await_both_idle_false_when_busy_past_deadline() {
        let t = ActivityTracker::new("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        t.set_busy(Author::Brian, true);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(120);
        assert!(!t.await_both_idle(deadline).await);
    }

    #[tokio::test]
    async fn await_both_idle_true_once_agent_goes_idle() {
        // The realistic interrupt case: an agent is busy when cancel fires, then
        // the interrupt's `result` event flips it idle within the window.
        let t = Arc::new(ActivityTracker::new(
            "s1",
            Arc::new(AtomicBool::new(false)),
            SignalingBridge::new(),
        ));
        t.set_busy(Author::Brian, true);
        let t2 = Arc::clone(&t);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            t2.set_busy(Author::Brian, false);
        });
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1000);
        assert!(t.await_both_idle(deadline).await);
    }
}
