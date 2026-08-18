//! Per-session activity — the source of truth for the chat-input lock +
//! Cancel button (interrupt redesign, Batch 2). Mirrors the `awaiting`
//! `Arc<AtomicBool>` pattern: created per session in `spawn_session_handle`,
//! shared with each participant's pump (which clears its own `busy` on
//! `TurnComplete`), the ring (`hand_turn_to` — the one busy-TRUE writer, rc3
//! D19b) and the boot phase (`boot_then_start`, which marks everyone busy while
//! they orient and lets each pump clear itself), and emits a `SessionActivity`
//! SignalingEvent whenever the derived state changes.

use crate::signaling::SignalingBridge;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Session-level activity, as surfaced to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionActivity {
    /// Every participant idle, not awaiting the user — input ENABLED.
    Idle,
    /// At least one agent mid-turn — input DISABLED, Cancel shown.
    Busy,
    /// An agent parked a question for the user — input ENABLED (it's the user's
    /// turn). Distinct from `Idle` so the UI can signal "your move".
    AwaitingUser,
    /// A cancel is in flight (kill issued, settling) — input DISABLED until it
    /// returns to `Idle`.
    Cancelling,
    /// The user clicked Stop — agents interrupted and HELD. Input ENABLED (the
    /// user steers by typing, resumes, or closes). Distinct from `Idle`: while
    /// paused nothing releases the ring — a tray answer's wake and the idle
    /// watchdog both check `holds_wakes` first — so the session cannot wake
    /// itself; only a user action (Send / Resume) clears it.
    Paused,
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
            SessionActivity::Paused => "paused",
        }
    }

    /// Pure derivation from the four inputs. Priority (highest first):
    /// `cancelling` > `paused` > `awaiting` > `busy` (ANY participant) > `idle`.
    /// `cancelling` wins so a kill-in-flight is never masked by residual busy;
    /// `paused` wins over `awaiting`/`busy` so a Stop lands in Paused the moment
    /// the interrupt settles (a straggler busy agent is about to be interrupted —
    /// the commanded state is Paused); `awaiting` wins over `busy` so a parked
    /// question re-opens input even though a turn is technically still in flight.
    ///
    /// B4b: took `brian_busy, rain_busy` before the participant rekey (those
    /// were the field names then; they are `slot0_busy`/`slot1_busy` now). The
    /// collapse to `any_busy` is what makes this N-participant-shaped — and it
    /// is also why per-participant edges must be tracked separately (see
    /// `Inner::per_agent_changed`), since this state cannot express a hand-off.
    pub fn derive(any_busy: bool, awaiting: bool, cancelling: bool, paused: bool) -> Self {
        if cancelling {
            SessionActivity::Cancelling
        } else if paused {
            SessionActivity::Paused
        } else if awaiting {
            SessionActivity::AwaitingUser
        } else if any_busy {
            SessionActivity::Busy
        } else {
            SessionActivity::Idle
        }
    }
}

/// Guarded mutable state. Kept behind one `Mutex` so concurrent set/clear from
/// every pump + the dispatch sites serialize — no interleave can emit a
/// spurious intermediate (e.g. a momentary `Idle` between one participant's
/// `Busy` and the next's during a hand-off).
struct Inner {
    /// Per-participant busy, keyed by participant **slug**. B4b: was
    /// `brian_busy` / `rain_busy` — the names of the day, now `slot0_busy` /
    /// `slot1_busy`.
    ///
    /// Slug, not `participant_id`, on purpose: `set_busy` is slug-keyed and runs
    /// at every turn end, so an id key would force a slug→id lookup on the hot
    /// path for
    /// no gain. The slug IS the legacy author string by construction — 0044
    /// mapped `slug` 1:1 onto `author` and `ensure_session_roster` preserves
    /// that — so it is the one key both worlds already agree on. B5 switches the
    /// key to the id in one diff.
    ///
    /// An absent slug reads as `false`; nothing pre-registers a roster here.
    busy: HashMap<String, bool>,
    cancelling: bool,
    /// Last derived state we emitted — so we only fire on an actual change.
    last: SessionActivity,
    /// Last per-participant flags we emitted. Tracked separately from `last`
    /// because the derived `state` collapses every participant into a single
    /// `Busy`: a turn handover (one participant's `busy` clears, the next's is
    /// set by the ring) leaves `state == Busy` throughout, so without this the
    /// per-participant transition would be suppressed and the UI would
    /// mislabel who's working.
    last_busy: HashMap<String, bool>,
}

impl Inner {
    fn busy_of(&self, slug: &str) -> bool {
        self.busy.get(slug).copied().unwrap_or(false)
    }

    fn any_busy(&self) -> bool {
        self.busy.values().any(|b| *b)
    }

    /// Did any participant's busy flag change since the last emit?
    ///
    /// Compared by EFFECTIVE value (absent == `false`) in both directions, not
    /// by map equality: `set_busy(x, false)` on a participant that was never
    /// registered inserts a `false` where `last_busy` holds no key at all, and
    /// structural inequality there would emit a spurious transition that the
    /// two-bool version never emitted.
    fn per_agent_changed(&self) -> bool {
        self.busy
            .iter()
            .any(|(slug, busy)| self.last_busy.get(slug).copied().unwrap_or(false) != *busy)
            || self
                .last_busy
                .iter()
                .any(|(slug, was)| self.busy.get(slug).copied().unwrap_or(false) != *was)
    }
}

/// Per-session activity tracker. Hold via `Arc`; all mutators take `&self`.
pub struct ActivityTracker {
    inner: Mutex<Inner>,
    /// Shared with the session's `awaiting` halt flag (set by
    /// `mark_awaiting_user` / `ask_user_choice`). Read at derive time; a caller
    /// that flips it must also call [`refresh`](Self::refresh).
    awaiting: Arc<AtomicBool>,
    /// The Stop/pause latch (interrupt lands the session in `Paused`, not
    /// `Idle`). Unlike `awaiting` it is tracker-owned — no MCP tool flips it;
    /// only core paths do, via [`set_paused`](Self::set_paused). The wake paths
    /// read it (with `cancelling`, via `holds_wakes`): a tray answer's release
    /// and the idle watchdog; the sequencer reads `is_paused` at its D35 gate.
    paused: AtomicBool,
    /// Did EVERY participant reach idle at any point since the current cancel began?
    ///
    /// This is the difference between "the interrupt was honored and the agent
    /// has since started the user's new turn" and "the agent never stopped".
    /// Both look busy at the end of the escalation window, but only the first
    /// one may skip the SIGKILL — see `AppState::escalation_outcome`. Cleared
    /// by `set_cancelling(true)`, set by `recompute_locked` whenever both
    /// per-agent flags are down.
    idled_since_cancel: AtomicBool,
    bridge: Arc<SignalingBridge>,
    session_id: String,
    /// This session's participant slugs in TURN ORDER, fixed at spawn.
    ///
    /// Two jobs, both of which used to be done by the literal strings `"brian"`
    /// and `"rain"` (rc3 D10):
    ///   * translating the legacy `Author` two-party discriminant (the deleted
    ///     router's vocabulary) onto the slug this tracker keys `busy` by,
    ///   * filling the frozen two-boolean wire payload, whose fields name slots
    ///     0 and 1 and no longer name agents.
    ///
    /// A session with one participant has one entry and a session with three has
    /// three; only the first two reach the wire.
    slugs: Vec<String>,
}

impl ActivityTracker {
    pub fn new(
        session_id: impl Into<String>,
        awaiting: Arc<AtomicBool>,
        bridge: Arc<SignalingBridge>,
        slugs: Vec<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                busy: HashMap::new(),
                cancelling: false,
                last: SessionActivity::Idle,
                last_busy: HashMap::new(),
            }),
            awaiting,
            paused: AtomicBool::new(false),
            idled_since_cancel: AtomicBool::new(false),
            bridge,
            session_id: session_id.into(),
            slugs,
        })
    }

    /// Participant-keyed [`set_busy`], and now the only form — the two-party
    /// `Author` discriminant it was named against was deleted in the D10
    /// retirement. `"user"` cannot enter the map: `set_busy` early-returns on it.
    pub fn set_busy_slug(&self, slug: &str, busy: bool) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.busy.insert(slug.to_string(), busy);
        self.recompute_locked(&mut g);
    }

    /// Is any participant mid-turn right now? The ring hands a turn out by
    /// setting exactly one of these flags, so this is the closest thing the
    /// app layer has to "the ring is running" — and it is the same signal the
    /// chat input locks on (rc3 D33), so the UI and the tray-answer path agree
    /// on what "working" means by construction.
    pub fn any_busy(&self) -> bool {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.any_busy()
    }

    /// Mark a cancel in-flight (`true`) or settled (`false`). Emits on change.
    pub fn set_cancelling(&self, cancelling: bool) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.cancelling = cancelling;
        if cancelling {
            // A fresh cancel starts with no evidence the agent stopped. Must be
            // reset here, or a previous cancel's idle would let this one skip
            // its SIGKILL.
            self.idled_since_cancel.store(false, Ordering::Release);
        }
        self.recompute_locked(&mut g);
    }

    /// Whether every participant has been idle at any point since the current cancel
    /// started. `escalation_outcome` uses it to tell an honored-then-resumed
    /// turn from one that never stopped.
    pub fn idled_since_cancel(&self) -> bool {
        self.idled_since_cancel.load(Ordering::Acquire)
    }

    /// Latch (`true`) or release (`false`) the pause. Set alongside
    /// `set_cancelling(true)` on Stop — when the interrupt settles the state
    /// lands in `Paused` instead of `Idle`. Cleared by Resume, a user Send
    /// (steer), or a supersede. Emits on change.
    ///
    /// ORDERING (Stop path): call `set_cancelling(true)` BEFORE this. Paused
    /// outranks Busy, so latching first would emit a momentary `paused`
    /// (input-enabled frame) before `cancelling` re-locks it. Cancelling-first
    /// makes the latch silent until the interrupt settles. Locked by
    /// `stop_settles_to_paused_not_idle`.
    pub fn set_paused(&self, paused: bool) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        self.paused.store(paused, Ordering::Release);
        self.recompute_locked(&mut g);
    }

    /// Whether the pause latch is set. The sequencer reads it at its D35 gate;
    /// the wake paths read it through [`holds_wakes`](Self::holds_wakes).
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    /// The wake gate: while a cancel is settling OR the pause latch is set,
    /// nothing releases the ring — a stopped session must not wake itself
    /// (the router-era Exited best-effort forward after a SIGKILL fallback was
    /// exactly that bug). Read by `AppState::tray_wake` and the idle watchdog.
    pub fn holds_wakes(&self) -> bool {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.cancelling || self.paused.load(Ordering::Acquire)
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
            g.any_busy(),
            self.awaiting.load(Ordering::Acquire),
            g.cancelling,
            self.paused.load(Ordering::Acquire),
        )
    }

    /// Participant-keyed [`is_busy`]. An unknown slug reads idle, which is the
    /// honest answer for a participant that has never taken a turn.
    pub fn is_busy_slug(&self, slug: &str) -> bool {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.busy_of(slug)
    }

    /// How long "nobody is busy" has to hold before it means the LAP ended
    /// rather than the ring being between two turns.
    ///
    /// `hand_turn_to` does not clear the previous holder — the pump does, at its
    /// own turn end, and the next deal marks the next participant later. That
    /// doc calls the window "the sub-second gap between a completion and the
    /// next handover" and keeps it deliberately (closing it buys a wedge). So a
    /// single idle reading is indistinguishable from mid-lap, and anything
    /// waiting for a lap to finish has to outlast the gap instead of racing it.
    pub const LAP_QUIET_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

    /// Poll (50ms) until SOME participant is busy, or `deadline` elapses; returns
    /// whether a turn started in time.
    ///
    /// The arming half of "wait for the turn I asked for". A host-side broadcast
    /// marks nobody busy (`state::broadcast_marks_nobody_busy` pins that the ring
    /// is the only busy-true writer), so at the instant a prompt is delivered the
    /// tracker still reads idle — and a waiter that starts by asking "is everyone
    /// idle?" is answered yes by the state it created. That is exactly how the
    /// D15 close epilogue reported every session as declining in 7ms.
    ///
    /// A `false` here is its own outcome, not a timeout: the ring never dealt the
    /// turn. Callers should say so rather than folding it into "still running".
    pub async fn await_turn_started(&self, deadline: tokio::time::Instant) -> bool {
        loop {
            if self.any_busy() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Poll (50ms) until the LAP ends, or `deadline` elapses.
    ///
    /// The lap is over when either
    /// - the halt slot refills (`awaiting`) — an all-pass yield, consensus, the
    ///   round cap or an agent's own recap all fill it, and `derive` ranks
    ///   awaiting above busy, so this lands even if a straggler is still marked;
    ///   or
    /// - nobody has been busy for a continuous `quiet` window
    ///   ([`LAP_QUIET_WINDOW`]), which is what distinguishes the end of the lap
    ///   from the handover gap inside it. Any busy reading restarts the window.
    ///
    /// This is the difference between waiting for a lap and waiting for a turn:
    /// [`await_both_idle`](Self::await_both_idle) returns on the first idle poll,
    /// which is correct for the cancel escalation that owns it (an interrupt
    /// settles when the CURRENT turn stops) and wrong for anything that needs
    /// every participant to have had its say.
    pub async fn await_lap_end(&self, deadline: tokio::time::Instant, quiet: std::time::Duration) -> bool {
        let mut idle_since: Option<tokio::time::Instant> = None;
        loop {
            let (any_busy, awaiting) = {
                let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
                (g.any_busy(), self.awaiting.load(Ordering::Acquire))
            };
            if awaiting {
                return true;
            }
            let now = tokio::time::Instant::now();
            if any_busy {
                idle_since = None;
            } else if now.duration_since(*idle_since.get_or_insert(now)) >= quiet {
                return true;
            }
            if now >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Poll (50ms) until NEITHER agent is busy, or `deadline` elapses; returns
    /// whether they went idle in time. The cancel interrupt-escalation uses this:
    /// after a `control_request` interrupt, the turn's `result` event clears the
    /// agent's `busy` flag (and auto-clears Cancelling→Idle). If that doesn't land
    /// within the window — interrupt dropped, or a wedged agent — the caller
    /// escalates to a SIGKILL. A participant that never took a turn is absent from
    /// the map and reads idle, so a solo session just waits on HANDS.
    pub async fn await_both_idle(&self, deadline: tokio::time::Instant) -> bool {
        loop {
            {
                let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
                if !g.any_busy() {
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
        // A cancel auto-completes once EVERY participant has gone idle (the kill
        // settled) — clear `cancelling` so the state transitions
        // Cancelling → Paused (Stop sets the pause latch) or → Idle, instead
        // of sticking. Done BEFORE derive so the emitted state reflects it.
        // (Set true by `set_cancelling` on Stop; the agents' pumps then clear
        // their `busy` as they die.)
        if !g.any_busy() {
            // Record that the agents reached idle. Read after the escalation
            // window by `escalation_outcome`, which must not skip the SIGKILL
            // for an agent that never got here.
            self.idled_since_cancel.store(true, Ordering::Release);
        }
        if g.cancelling && !g.any_busy() {
            g.cancelling = false;
        }
        let next = SessionActivity::derive(
            g.any_busy(),
            self.awaiting.load(Ordering::Acquire),
            g.cancelling,
            self.paused.load(Ordering::Acquire),
        );
        // Fire on a change to the derived state OR to any per-participant flag —
        // the latter so a within-`Busy` transition (broadcast both-busy, peer
        // hand-off) still reaches the UI to relabel who's working.
        if next != g.last || g.per_agent_changed() {
            g.last = next;
            g.last_busy.clone_from(&g.busy);
            // The wire payload carries exactly two booleans, derived from the
            // map, and they name the first two TURN SLOTS — not two agents
            // (rc3 D10). `busy_of` on a slug this session does not have reads
            // false, which is the right answer for a slot nobody occupies.
            //
            // **A roster-shaped payload here would be a UI change**, so the
            // shape stays: `list_session_participants` is the roster-shaped
            // read, and the session view asks it rather than growing this
            // event. That is the reason, and it does not expire — the previous
            // wording ("frozen while the session view is rewritten … a parallel
            // unit owns those files") was a merge-era instruction that outlived
            // its merge, and round 4 deleted the same premise from the three
            // sites that restated it.
            let slot = |i: usize| {
                self.slugs
                    .get(i)
                    .is_some_and(|s| g.busy_of(s))
            };
            let (slot0_busy, slot1_busy) = (slot(0), slot(1));
            self.bridge.notify_session_activity(
                self.session_id.clone(),
                next.as_str(),
                slot0_busy,
                slot1_busy,
            );
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
        assert_eq!(SessionActivity::Paused.as_str(), "paused");
    }

    #[test]
    fn derive_priority() {
        use SessionActivity::*;
        // (any_busy, awaiting, cancelling, paused) — B4b collapsed the two
        // per-agent bools into one "any participant busy".
        // idle: nothing set.
        assert_eq!(SessionActivity::derive(false, false, false, false), Idle);
        // busy: any participant.
        assert_eq!(SessionActivity::derive(true, false, false, false), Busy);
        // awaiting beats busy (parked question re-opens input mid-turn).
        assert_eq!(SessionActivity::derive(true, true, false, false), AwaitingUser);
        assert_eq!(SessionActivity::derive(false, true, false, false), AwaitingUser);
        // paused beats awaiting + busy (Stop is the commanded state, even for a
        // straggler busy agent about to be interrupted / a parked question).
        assert_eq!(SessionActivity::derive(false, false, false, true), Paused);
        assert_eq!(SessionActivity::derive(true, false, false, true), Paused);
        assert_eq!(SessionActivity::derive(true, true, false, true), Paused);
        // cancelling beats everything, including paused (Stop sets both; the
        // UI shows "Stopping…" until the interrupt settles, then Paused).
        assert_eq!(SessionActivity::derive(true, true, true, false), Cancelling);
        assert_eq!(SessionActivity::derive(false, false, true, false), Cancelling);
        assert_eq!(SessionActivity::derive(true, true, true, true), Cancelling);
    }

    fn activity_state(ev: &SignalingEvent) -> Option<&str> {
        match ev {
            SignalingEvent::SessionActivity { state, .. } => Some(state),
            _ => None,
        }
    }

    /// `rx.recv()` with a deadline. A missing emit IS the regression these tests
    /// exist to catch, and a bare `recv().await` turns it into a hung test
    /// rather than a failing one — the B4b injection run (per-participant edge
    /// check removed) wedged instead of reporting, which is the worst shape a
    /// guard can have. 2s is far above the in-process broadcast latency.
    async fn next_event(
        rx: &mut tokio::sync::broadcast::Receiver<SignalingEvent>,
    ) -> SignalingEvent {
        tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("expected a SessionActivity emit within 2s; none arrived")
            .unwrap()
    }

    fn tracker(
        session: &str,
        awaiting: Arc<AtomicBool>,
        bridge: Arc<SignalingBridge>,
    ) -> Arc<ActivityTracker> {
        // The two turn slots the deleted `Author` discriminant used to address. Named
        // after the seeded roles, because that is what a real session now
        // carries; these tests exercise the slot mapping, not the names.
        ActivityTracker::new(session, awaiting, bridge, vec!["hands".into(), "eyes".into()])
    }

    fn activity_tuple(ev: &SignalingEvent) -> Option<(&str, bool, bool)> {
        match ev {
            SignalingEvent::SessionActivity {
                state,
                slot0_busy,
                slot1_busy,
                ..
            } => Some((state, *slot0_busy, *slot1_busy)),
            _ => None,
        }
    }

    #[tokio::test]
    async fn tracker_emits_only_on_change() {
        let bridge = SignalingBridge::new();
        let mut rx = bridge.subscribe();
        let awaiting = Arc::new(AtomicBool::new(false));
        let t = tracker("s1", awaiting.clone(), bridge.clone());

        // idle -> busy(brian)
        t.set_busy_slug("hands", true);
        assert_eq!(
            activity_tuple(&next_event(&mut rx).await),
            Some(("busy", true, false))
        );

        // redundant set (brian already busy): nothing changed → no emit.
        t.set_busy_slug("hands", true);
        assert!(rx.try_recv().is_err(), "no emit when nothing changed");

        // busy(brian) -> busy(brian+rain): derived state stays `busy`, but the
        // per-agent flags changed (a broadcast) → MUST re-emit so the UI can show
        // BOTH agents working, not just Brian.
        t.set_busy_slug("eyes", true);
        assert_eq!(
            activity_tuple(&next_event(&mut rx).await),
            Some(("busy", true, true))
        );

        // brian idle, rain still busy: still `busy`, but per-agent changed (a
        // peer hand-off) → re-emit with the new flags so the label follows.
        t.set_busy_slug("hands", false);
        assert_eq!(
            activity_tuple(&next_event(&mut rx).await),
            Some(("busy", false, true))
        );

        // both idle -> idle.
        t.set_busy_slug("eyes", false);
        assert_eq!(activity_state(&next_event(&mut rx).await), Some("idle"));

        // awaiting flips externally -> awaiting_user (after refresh).
        awaiting.store(true, Ordering::Release);
        t.refresh();
        assert_eq!(
            activity_state(&next_event(&mut rx).await),
            Some("awaiting_user")
        );
        awaiting.store(false, Ordering::Release);

        // Cancelling only "sticks" while an agent is busy (the kill is
        // settling). An agent goes busy, Stop sets cancelling (overrides busy),
        // then the agent dies (idle) → cancelling AUTO-CLEARS → Idle.
        t.set_busy_slug("hands", true);
        assert_eq!(activity_state(&next_event(&mut rx).await), Some("busy"));
        t.set_cancelling(true);
        assert_eq!(activity_state(&next_event(&mut rx).await), Some("cancelling"));
        t.set_busy_slug("hands", false);
        assert_eq!(activity_state(&next_event(&mut rx).await), Some("idle"));
    }

    #[tokio::test]
    async fn a_three_participant_handoff_still_emits_per_participant() {
        // B4b: the tracker is no longer two-agent-shaped. `derive` collapses
        // every participant into one `Busy`, so a hand-off produces NO state
        // change — the per-participant edge is the only thing that tells the UI
        // who is working. With a third participant the two-bool version fails
        // twice over: it cannot represent `scout` at all, so a session where
        // only `scout` is mid-turn would read `idle` and unlock the input under
        // a running agent.
        let bridge = SignalingBridge::new();
        let mut rx = bridge.subscribe();
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), bridge.clone());

        t.set_busy_slug("hands", true);
        assert_eq!(
            activity_tuple(&next_event(&mut rx).await),
            Some(("busy", true, false))
        );

        // A third participant joins the turn. Derived state stays `busy` and the
        // wire tuple is UNCHANGED (it carries only brian/rain until B8), but the
        // transition is real and must still emit.
        t.set_busy_slug("scout", true);
        assert_eq!(
            activity_tuple(&next_event(&mut rx).await),
            Some(("busy", true, false)),
            "a third participant's transition emits even though the wire can't name it"
        );

        // The hand-off: brian idles, scout keeps working. Both wire booleans are
        // now false — and the session is STILL busy. This is the assertion the
        // two-bool tracker could not make.
        t.set_busy_slug("hands", false);
        assert_eq!(
            activity_tuple(&next_event(&mut rx).await),
            Some(("busy", false, false)),
            "an unnamed participant still holds the session busy"
        );
        assert_eq!(t.current(), SessionActivity::Busy);
        assert!(t.is_busy_slug("scout"));
        assert!(!t.is_busy_slug("hands"));

        // Redundant set on the third participant: nothing changed → no emit.
        t.set_busy_slug("scout", true);
        assert!(rx.try_recv().is_err(), "no emit when nothing changed");

        // Last participant idles → idle.
        t.set_busy_slug("scout", false);
        assert_eq!(activity_state(&next_event(&mut rx).await), Some("idle"));
    }

    #[tokio::test]
    async fn clearing_an_unregistered_participant_emits_nothing() {
        // `set_busy(x, false)` on a participant that never took a turn inserts a
        // `false` where the last-emitted map holds no key at all. Comparing the
        // maps structurally would emit a spurious transition here; comparing by
        // effective value (absent == false) does not. The two-bool tracker was
        // silent, and so must this be.
        let bridge = SignalingBridge::new();
        let mut rx = bridge.subscribe();
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), bridge.clone());
        t.set_busy_slug("eyes", false);
        assert!(rx.try_recv().is_err(), "clearing an idle participant is not a transition");
        assert_eq!(t.current(), SessionActivity::Idle);
    }

    #[tokio::test]
    async fn transitions_are_persisted_including_flag_only_changes() {
        // The reported-bug sequence, and the reason this table records per-agent
        // flag changes and not just derived-state changes: while `awaiting_user`
        // holds, slot 0 stopping produces NO state change. If that weren't
        // recorded, the newest row would still assert slot0_busy = 1 long after
        // that participant finished, and every query would inherit the stale claim.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.create_session("s1", "One", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;

        let awaiting = Arc::new(AtomicBool::new(false));
        let t = tracker("s1", awaiting.clone(), bridge.clone());

        t.set_busy_slug("hands", true); // idle -> busy
        awaiting.store(true, Ordering::Release);
        t.refresh(); // busy -> awaiting_user, Brian STILL working
        t.set_busy_slug("hands", false); // stays awaiting_user; flags change only

        // The writes are detached (recompute_locked is sync), so let them land.
        for _ in 0..50 {
            if storage.list_activity_events(Some("s1")).await.unwrap().len() >= 3 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let rows = storage.list_activity_events(Some("s1")).await.unwrap();
        let seq: Vec<(String, i64)> = rows
            .iter()
            .rev() // list is newest-first; read it forwards
            .map(|r| (r.state.clone(), r.slot0_busy))
            .collect();
        assert_eq!(
            seq,
            vec![
                ("busy".to_string(), 1),
                ("awaiting_user".to_string(), 1),
                ("awaiting_user".to_string(), 0),
            ],
            "the flag-only transition inside a stable awaiting_user must be recorded"
        );
    }

    #[tokio::test]
    async fn cancelling_set_while_idle_auto_clears() {
        // Defensive: if a cancel is somehow set when no agent is busy, it
        // auto-clears immediately (the kill has nothing to settle) — never a
        // stuck Cancelling that locks the input forever.
        let bridge = SignalingBridge::new();
        let awaiting = Arc::new(AtomicBool::new(false));
        let t = tracker("s1", awaiting, bridge);
        t.set_cancelling(true);
        assert_eq!(t.current(), SessionActivity::Idle);
    }

    #[tokio::test]
    async fn stop_settles_to_paused_not_idle() {
        // The Stop sequence: agents busy → Stop sets cancelling THEN paused
        // (that order — see set_paused's ORDERING note; the latch must be
        // silent while Cancelling holds the state) → both agents go idle →
        // cancelling auto-clears → the session lands in "paused" (input
        // enabled, duo held), NOT "idle".
        let bridge = SignalingBridge::new();
        let mut rx = bridge.subscribe();
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), bridge.clone());
        t.set_busy_slug("hands", true);
        assert_eq!(activity_state(&next_event(&mut rx).await), Some("busy"));
        t.set_cancelling(true);
        t.set_paused(true);
        assert_eq!(activity_state(&next_event(&mut rx).await), Some("cancelling"));
        t.set_busy_slug("hands", false);
        assert_eq!(activity_state(&next_event(&mut rx).await), Some("paused"));
        assert!(t.is_paused());
        // Resume releases the latch → Idle.
        t.set_paused(false);
        assert_eq!(activity_state(&next_event(&mut rx).await), Some("idle"));
        assert!(!t.is_paused());
    }

    #[tokio::test]
    async fn pause_while_idle_latches_immediately() {
        // Stop on an already-idle duo (e.g. between turns): no cancelling to
        // settle — the state flips straight to "paused" and holds until
        // released. Guards the walk-away case landing before any busy flag.
        let bridge = SignalingBridge::new();
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), bridge);
        t.set_paused(true);
        assert_eq!(t.current(), SessionActivity::Paused);
        t.set_paused(false);
        assert_eq!(t.current(), SessionActivity::Idle);
    }

    #[test]
    fn idled_since_cancel_is_false_while_an_agent_never_stops() {
        // The wedged case: Stop fires while HANDS is mid-Bash and the agent
        // never reaches idle, so there is no proof the turn stopped and the
        // SIGKILL must not be skipped.
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        t.set_busy_slug("hands", true);
        t.set_cancelling(true);
        assert!(!t.idled_since_cancel(), "agent never went idle");
    }

    #[test]
    fn idled_since_cancel_flips_once_both_agents_stop() {
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        t.set_busy_slug("hands", true);
        t.set_busy_slug("eyes", true);
        t.set_cancelling(true);
        assert!(!t.idled_since_cancel());
        t.set_busy_slug("hands", false);
        assert!(!t.idled_since_cancel(), "one agent idle is not both");
        t.set_busy_slug("eyes", false);
        assert!(t.idled_since_cancel(), "both idle = the interrupt landed");
    }

    #[test]
    fn idled_since_cancel_survives_the_agent_going_busy_again() {
        // Interrupt honored, then the user's message starts a fresh turn. Busy
        // again at the deadline, but the earlier idle is the proof that lets
        // the supersede branch protect that new turn.
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        t.set_busy_slug("hands", true);
        t.set_cancelling(true);
        t.set_busy_slug("hands", false);
        t.set_busy_slug("hands", true);
        assert!(t.idled_since_cancel());
    }

    #[test]
    fn a_new_cancel_clears_the_previous_cancels_proof() {
        // Otherwise an earlier cancel's idle would let the NEXT cancel skip its
        // SIGKILL against an agent that never stopped.
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        t.set_cancelling(true);
        assert!(t.idled_since_cancel(), "starts idle");
        t.set_busy_slug("hands", true);
        t.set_cancelling(false);
        t.set_cancelling(true);
        assert!(!t.idled_since_cancel(), "a fresh cancel starts with no proof");
    }

    #[tokio::test]
    async fn await_both_idle_true_when_already_idle() {
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(500);
        assert!(t.await_both_idle(deadline).await);
    }

    #[tokio::test]
    async fn await_both_idle_false_when_busy_past_deadline() {
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        t.set_busy_slug("hands", true);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(120);
        assert!(!t.await_both_idle(deadline).await);
    }

    #[tokio::test]
    async fn await_both_idle_true_once_agent_goes_idle() {
        // The realistic interrupt case: an agent is busy when cancel fires, then
        // the interrupt's `result` event flips it idle within the window.
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        t.set_busy_slug("hands", true);
        let t2 = Arc::clone(&t);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            t2.set_busy_slug("hands", false);
        });
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1000);
        assert!(t.await_both_idle(deadline).await);
    }

    #[tokio::test]
    async fn await_turn_started_is_false_when_the_ring_never_deals() {
        // The arming half's whole job: a broadcast marks nobody busy, so if the
        // ring never deals the turn nothing ever flips. Reported as its own
        // outcome (`NeverStarted`), not as a timeout.
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(120);
        assert!(!t.await_turn_started(deadline).await);
    }

    #[tokio::test]
    async fn await_turn_started_sees_the_busy_edge() {
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        let t2 = Arc::clone(&t);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            t2.set_busy_slug("hands", true);
        });
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(2000);
        assert!(t.await_turn_started(deadline).await);
    }

    /// **The fixture that separates "a turn ended" from "the lap ended".**
    ///
    /// `hand_turn_to` clears the previous holder at turn end and marks the next
    /// one later, so a two-participant lap reads busy → idle → busy. A waiter
    /// that returns on the first idle poll — which is what `await_both_idle`
    /// does, correctly, for the cancel escalation — stops in that gap and the
    /// caller tears the session down on top of the participant whose turn is
    /// about to start. That is the D15 epilogue bug one turn later.
    ///
    /// A busy → idle fixture cannot tell the two semantics apart: both return
    /// true, both look right. Only a fixture with a gap INSIDE it can, which is
    /// the CL's rule about aggregate-vs-point-in-time fixtures in its other
    /// clothes.
    #[tokio::test]
    async fn the_handover_gap_is_not_a_lap_end() {
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        let quiet = std::time::Duration::from_millis(300);
        let gap = std::time::Duration::from_millis(100);

        t.set_busy_slug("hands", true);
        let t2 = Arc::clone(&t);
        let driver = tokio::spawn(async move {
            // Participant 1 finishes.
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            t2.set_busy_slug("hands", false);
            // The handover gap — shorter than `quiet`, and the ring is mid-lap.
            tokio::time::sleep(gap).await;
            // Participant 2 is dealt, takes its turn, finishes. The lap ends
            // here, and only here.
            t2.set_busy_slug("eyes", true);
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            t2.set_busy_slug("eyes", false);
        });

        let started = tokio::time::Instant::now();
        let deadline = started + std::time::Duration::from_secs(5);
        assert!(t.await_lap_end(deadline, quiet).await, "the lap does end");
        let elapsed = started.elapsed();
        driver.await.unwrap();

        // Returning in the gap would land at ~180ms. The earliest honest answer
        // is p1 (80) + gap (100) + p2 (120) + quiet (300) = 600ms.
        assert!(
            elapsed >= std::time::Duration::from_millis(500),
            "returned after {elapsed:?} — that is inside the handover gap, before \
             the second participant had its turn"
        );
    }

    #[tokio::test]
    async fn a_refilled_halt_slot_ends_the_lap_even_with_a_straggler_busy() {
        // Every stop fills the halt slot, and `derive` ranks awaiting above busy
        // — so an all-pass yield / consensus / declared recap ends the wait
        // immediately, without paying the quiet window, even if a participant is
        // still marked busy on its way out.
        let awaiting = Arc::new(AtomicBool::new(false));
        let t = tracker("s1", awaiting.clone(), SignalingBridge::new());
        t.set_busy_slug("hands", true);
        awaiting.store(true, Ordering::Release);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(120);
        assert!(
            t.await_lap_end(deadline, ActivityTracker::LAP_QUIET_WINDOW)
                .await,
            "a refilled halt slot is the lap's end, whatever the busy map says"
        );
    }

    #[tokio::test]
    async fn await_lap_end_is_false_while_a_turn_runs_past_the_deadline() {
        let t = tracker("s1", Arc::new(AtomicBool::new(false)), SignalingBridge::new());
        t.set_busy_slug("hands", true);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(150);
        assert!(
            !t.await_lap_end(deadline, ActivityTracker::LAP_QUIET_WINDOW)
                .await
        );
    }
}
