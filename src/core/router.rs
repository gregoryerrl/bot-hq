//! Central peer-forward router (host-mediated reroute, option a).
//!
//! One task per duo session. The two per-agent pumps (`core::duo`) no longer
//! forward to each other directly; instead each pump emits a `RouterCommand` and
//! THIS task is the single decision point for whether a turn's prose is forwarded
//! to the peer, suppressed, or breaks the volley. Centralizing buys: (1) one place
//! the forward policy lives, (2) a SINGLE interleaved convergence stream with full
//! visibility into BOTH agents' forwards (the old per-pump detector only saw its
//! own), so a same-phrase cross-agent volley breaks across the agent boundary
//! instead of escaping to the hard-cap.
//!
//! Scope is deliberately 2-agent with named Brian/Rain resolution — the central
//! receive-decide-route loop is the seam an N-agent plugin or a coordinator model
//! extends later; the data-structure generalization (a peer map + a forward-policy
//! trait) is built against a real use case, not speculatively.

use crate::agents::OutgoingUserMessage;
use crate::core::activity::ActivityTracker;
use crate::core::broadcast::peer_forward_message;
use crate::core::ipav::IpavState;
use crate::signaling::SignalingBridge;
use crate::storage::Author;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::debug;

/// A command from a pump to the router. One variant today (the extensible seam):
/// a completed turn's buffered prose that MIGHT be forwarded to the peer.
#[derive(Debug)]
pub enum RouterCommand {
    Forward {
        /// The agent that produced the prose.
        from: Author,
        /// The turn's buffered text (the router trims the trailing end again).
        body: String,
        /// Whether the producing turn called `peer_ack` (suppress, don't volley).
        peer_ack: bool,
    },
}

/// Everything the router task needs. The Arcs (`awaiting`, `user_silent_forwards`,
/// `activity`) are CLONES of the same session-level state the pumps + `broadcast`
/// hold — so `broadcast`'s counter reset and a user-blocking MCP tool's `awaiting`
/// set are both visible here with no extra plumbing.
pub struct RouterDeps {
    pub session_id: String,
    /// Await-halt: while set, suppress all peer-forwarding (the user is being
    /// asked). Set by user-blocking MCP tools; cleared by `broadcast`.
    pub awaiting: Arc<AtomicBool>,
    /// L2 hard-cap counter — consecutive peer-forwards with no intervening user
    /// message. `broadcast` resets it to 0 (UNCHANGED from the pre-router model).
    pub user_silent_forwards: Arc<AtomicU32>,
    /// Drives the chat-input lock. The router owns the busy hand-off on the
    /// forward path: set peer busy BEFORE the sender idle (no Idle flicker).
    /// `None` in tests that don't assert activity.
    pub activity: Option<Arc<ActivityTracker>>,
    /// For the open-blocking-findings banner. `None` in tests → banner count 0.
    pub bridge: Option<Arc<SignalingBridge>>,
    /// Current IPAV phase, read at forward time for the wire envelope.
    pub ipav: Arc<Mutex<IpavState>>,
    /// Brian's stdin sender (peer target when Rain speaks).
    pub brian_input: mpsc::Sender<OutgoingUserMessage>,
    /// Rain's stdin sender (peer target when Brian speaks). `None` = solo; the
    /// pump never emits a Forward in solo mode, so the router isn't spawned then.
    pub rain_input: Option<mpsc::Sender<OutgoingUserMessage>>,
}

impl RouterDeps {
    /// The stdin sender for `author` — the peer-resolution target. Named 2-agent
    /// resolution; the seam an N-agent peer map replaces later.
    fn input_for(&self, author: Author) -> Option<&mpsc::Sender<OutgoingUserMessage>> {
        match author {
            Author::Brian => Some(&self.brian_input),
            Author::Rain => self.rain_input.as_ref(),
            Author::User => None,
        }
    }

    fn set_idle(&self, author: Author) {
        if let Some(activity) = &self.activity {
            activity.set_busy(author, false);
        }
    }
}

/// The peer of an agent in the 2-agent duo. Brian↔Rain; User has no peer.
fn peer_of(author: Author) -> Author {
    match author {
        Author::Brian => Author::Rain,
        Author::Rain => Author::Brian,
        Author::User => Author::User,
    }
}

/// Run the router task. Returns when the command channel closes (both pumps
/// dropped their `router_tx` — session end). Owns the SINGLE interleaved
/// convergence stream (`last_forward`/`similar_streak`): unlike the old per-pump
/// detector, this sees BOTH agents' forwards in arrival order, so a same-phrase
/// cross-agent volley (Brian "🤝" → Rain "🤝" → Brian "🤝") builds a breaking
/// streak across the agent boundary instead of escaping to the hard-cap.
pub async fn run_router(deps: RouterDeps, mut rx: mpsc::Receiver<RouterCommand>) {
    let mut last_forward: Option<String> = None;
    let mut similar_streak: u32 = 0;
    while let Some(cmd) = rx.recv().await {
        match cmd {
            RouterCommand::Forward {
                from,
                body,
                peer_ack,
            } => {
                route_forward(
                    &deps,
                    &mut last_forward,
                    &mut similar_streak,
                    from,
                    body,
                    peer_ack,
                )
                .await;
            }
        }
    }
}

/// The forward ladder — same order/semantics as the pre-router `flush_buffer`,
/// now in ONE place. Each suppression path still clears the sender's `busy` (the
/// pump delegated self-idle to us on the forward path), so the session settles
/// correctly. On a real forward we set the peer busy BEFORE the sender idle.
async fn route_forward(
    deps: &RouterDeps,
    last_forward: &mut Option<String>,
    similar_streak: &mut u32,
    from: Author,
    body: String,
    peer_ack: bool,
) {
    let trimmed = body.trim_end();
    if trimmed.is_empty() {
        deps.set_idle(from);
        return;
    }
    let peer = peer_of(from);
    let Some(peer_tx) = deps.input_for(peer) else {
        // No peer sender for `from`'s peer. In a duo session both agents always
        // have a sender, and the router is never spawned for a solo session — so
        // this is reachable only via the impossible `from == User`. Log the
        // invariant breach (review advisory) and never strand `from` busy.
        debug!(agent = ?from, "router: no peer sender (unexpected non-duo author); dropping forward");
        deps.set_idle(from);
        return;
    };

    // 1. Await-halt: the user is being asked — suppress, settle the sender idle.
    if deps.awaiting.load(Ordering::Acquire) {
        debug!(agent = ?from, "router: awaiting user; suppressing peer forward");
        deps.set_idle(from);
        return;
    }
    // 2. peer_ack: explicit ack — suppress BEFORE the counters (not a volley
    //    contribution, so it must not bump the hard-cap or extend the streak).
    if peer_ack {
        debug!(agent = ?from, "router: peer_ack; suppressing peer forward");
        deps.set_idle(from);
        return;
    }
    // 3. L2 hard-cap: bound consecutive peer-forwards with no user message.
    let n = deps.user_silent_forwards.fetch_add(1, Ordering::AcqRel) + 1;
    if n > VOLLEY_HARD_CAP {
        debug!(agent = ?from, count = n, "router: hard-cap reached; breaking volley + unlocking input");
        break_volley(deps);
        return;
    }
    // 4. L2 convergence over the SINGLE interleaved stream: a forward
    //    ≥VOLLEY_SIMILARITY_THRESHOLD similar to the PREVIOUS forward (from either
    //    agent) extends the streak; a dissimilar one resets it. Deliberately NOT
    //    reset on break — a sustained repetition keeps suppressing until content
    //    changes.
    match last_forward.as_deref() {
        Some(prev) if jaccard_similarity(prev, trimmed) >= VOLLEY_SIMILARITY_THRESHOLD => {
            *similar_streak += 1;
        }
        _ => *similar_streak = 0,
    }
    *last_forward = Some(trimmed.to_string());
    if *similar_streak >= VOLLEY_SIMILAR_BREAK {
        debug!(agent = ?from, streak = *similar_streak, "router: convergence breaker tripped; breaking volley + unlocking input");
        break_volley(deps);
        return;
    }
    // 5. Forward, then hand off busy IN ORDER (peer busy BEFORE sender idle) so
    //    `derive()` never sees both-idle → no momentary Idle that unlocks input
    //    mid-handoff.
    let phase = deps.ipav.lock().await.current_phase;
    let open_blocking = match &deps.bridge {
        Some(bridge) => bridge.open_blocking_count(&deps.session_id).await,
        None => 0,
    };
    peer_forward_message(from, trimmed, phase, open_blocking, peer_tx).await;
    if let Some(activity) = &deps.activity {
        activity.set_busy(peer, true);
        activity.set_busy(from, false);
    }
}

/// Break a volley: set BOTH agents idle so `ActivityTracker::derive` returns Idle
/// and the chat input unlocks. Shared by the L2 hard-cap and the convergence
/// breaker. (2-agent named: Brian + Rain.)
fn break_volley(deps: &RouterDeps) {
    if let Some(activity) = &deps.activity {
        activity.set_busy(Author::Brian, false);
        activity.set_busy(Author::Rain, false);
    }
}

/// Max consecutive peer-forwards with no intervening user message before the L2
/// hard-cap breaks the volley. High by design — productive duo collaboration
/// (a multi-turn review) must never trip it; only a genuine runaway reaches it
/// (`s-e4fc25`: 34 messages, 0 from the user).
const VOLLEY_HARD_CAP: u32 = 18;

/// Token-set Jaccard similarity of two forward bodies — the shape-based
/// convergence signal (no length threshold, no keyword/prefix list). Tokenize:
/// split on whitespace, trim each token of leading/trailing non-alphanumerics,
/// lowercase, drop empties — so "OK.", "OK", "ok" all reduce to {ok}. Edge: BOTH
/// token sets empty (pure punctuation / emoji like "." or "🤝", the canonical
/// s-e4fc25 volley) → 1.0, so convergence catches it fast rather than deferring to
/// the hard-cap. One empty, one not → 0.0. Two DISTINCT substantive messages
/// always carry alphanumeric tokens, so they can never collide at 1.0 via the
/// both-empty path.
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    fn token_set(s: &str) -> HashSet<String> {
        s.split_whitespace()
            .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }
    let (sa, sb) = (token_set(a), token_set(b));
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    if union == 0 {
        1.0
    } else {
        inter as f64 / union as f64
    }
}

/// Jaccard similarity at or above which two consecutive forwards count as "the
/// same content" for convergence detection.
const VOLLEY_SIMILARITY_THRESHOLD: f64 = 0.85;

/// Consecutive near-identical forwards before the convergence breaker trips. With
/// 2: forward-1 sets the baseline (streak 0), forward-2 (similar) → streak 1,
/// forward-3 (similar) → streak 2 → break. So the 3rd near-identical forward
/// breaks the volley.
const VOLLEY_SIMILAR_BREAK: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ipav::IpavState;

    fn deps(
        brian_input: mpsc::Sender<OutgoingUserMessage>,
        rain_input: Option<mpsc::Sender<OutgoingUserMessage>>,
        awaiting: Arc<AtomicBool>,
        counter: Arc<AtomicU32>,
    ) -> RouterDeps {
        RouterDeps {
            session_id: "s1".into(),
            awaiting,
            user_silent_forwards: counter,
            activity: None,
            bridge: None,
            ipav: Arc::new(Mutex::new(IpavState::default())),
            brian_input,
            rain_input,
        }
    }

    /// Run `cmds` through a fresh router, then count how many forwards landed on
    /// Brian's and Rain's channels. Drops the command tx so `run_router` returns.
    async fn run_and_count(
        deps: RouterDeps,
        cmds: Vec<RouterCommand>,
        mut brian_rx: mpsc::Receiver<OutgoingUserMessage>,
        mut rain_rx: mpsc::Receiver<OutgoingUserMessage>,
    ) -> (u32, u32) {
        let (tx, rx) = mpsc::channel(512);
        let task = tokio::spawn(run_router(deps, rx));
        for c in cmds {
            tx.send(c).await.unwrap();
        }
        drop(tx);
        task.await.unwrap();
        let mut b = 0;
        while brian_rx.try_recv().is_ok() {
            b += 1;
        }
        let mut r = 0;
        while rain_rx.try_recv().is_ok() {
            r += 1;
        }
        (b, r)
    }

    fn fwd(from: Author, body: &str) -> RouterCommand {
        RouterCommand::Forward {
            from,
            body: body.into(),
            peer_ack: false,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hard_cap_breaks_after_cap() {
        // Distinct bodies so convergence never trips first — the cap is the sole
        // reason forwarding stops. All from Brian → all land on Rain's channel.
        let (btx, brx) = mpsc::channel(512);
        let (rtx, rrx) = mpsc::channel(512);
        let counter = Arc::new(AtomicU32::new(0));
        let d = deps(btx, Some(rtx), Arc::new(AtomicBool::new(false)), counter);
        let cmds: Vec<_> = (0..(VOLLEY_HARD_CAP + 3))
            .map(|i| fwd(Author::Brian, &format!("distinct line {i}")))
            .collect();
        let (b, r) = run_and_count(d, cmds, brx, rrx).await;
        assert_eq!(b, 0);
        assert_eq!(
            r, VOLLEY_HARD_CAP,
            "peer receives exactly VOLLEY_HARD_CAP forwards, then the volley breaks"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn single_stream_cross_agent_same_phrase_breaks_fast() {
        // THE WIN: a same-phrase volley that ALTERNATES agents (Brian 🤝 → Rain 🤝
        // → Brian 🤝 → …). The single interleaved stream sees 🤝,🤝,🤝,🤝 →
        // forward-1 streak 0 (fwd), forward-2 streak 1 (fwd), forward-3 streak 2 →
        // BREAK. Exactly 2 forwards reach a peer. A per-author detector would never
        // build a cross-agent streak here and would run to the hard-cap.
        let (btx, brx) = mpsc::channel(64);
        let (rtx, rrx) = mpsc::channel(64);
        let counter = Arc::new(AtomicU32::new(0));
        let d = deps(btx, Some(rtx), Arc::new(AtomicBool::new(false)), counter);
        let cmds = vec![
            fwd(Author::Brian, "🤝"),
            fwd(Author::Rain, "🤝"),
            fwd(Author::Brian, "🤝"),
            fwd(Author::Rain, "🤝"),
            fwd(Author::Brian, "🤝"),
        ];
        let (b, r) = run_and_count(d, cmds, brx, rrx).await;
        assert_eq!(
            b + r,
            VOLLEY_SIMILAR_BREAK,
            "cross-agent same-phrase volley must break at VOLLEY_SIMILAR_BREAK forwards (the full-visibility win)"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn varied_substantive_cross_agent_never_breaks() {
        // LOAD-BEARING false-fire guard: genuine alternating collaboration (distinct
        // substantive content each turn, even on the same topic) must NEVER trip the
        // single-stream convergence breaker. Each consecutive pair is well below the
        // 0.85 threshold → the streak resets every turn → all forwards reach a peer.
        let (btx, brx) = mpsc::channel(64);
        let (rtx, rrx) = mpsc::channel(64);
        let counter = Arc::new(AtomicU32::new(0));
        let d = deps(btx, Some(rtx), Arc::new(AtomicBool::new(false)), counter);
        let cmds = vec![
            fwd(Author::Brian, "The hard-cap counter should reset in broadcast on the user's next message."),
            fwd(Author::Rain, "Agreed, but the convergence streak is router-local now and needs no reset path."),
            fwd(Author::Brian, "Right — the migration only moves flush_buffer's ladder; state.rs stays untouched."),
            fwd(Author::Rain, "One concern: the busy hand-off ordering must keep peer-busy ahead of sender-idle."),
        ];
        let (b, r) = run_and_count(d, cmds, brx, rrx).await;
        assert_eq!(
            b + r,
            4,
            "distinct substantive cross-agent forwards must all reach a peer — convergence must not false-fire"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn awaiting_suppresses_forward() {
        // While the await-halt flag is set, no forward reaches the peer.
        let (btx, brx) = mpsc::channel(8);
        let (rtx, rrx) = mpsc::channel(8);
        let awaiting = Arc::new(AtomicBool::new(true));
        let counter = Arc::new(AtomicU32::new(0));
        let d = deps(btx, Some(rtx), awaiting, Arc::clone(&counter));
        let (b, r) = run_and_count(d, vec![fwd(Author::Brian, "waiting for the user")], brx, rrx).await;
        assert_eq!(b + r, 0, "awaiting must suppress the peer forward");
        assert_eq!(
            counter.load(Ordering::Acquire),
            0,
            "a suppressed-by-awaiting forward must not bump the hard-cap counter"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn peer_ack_suppresses_and_doesnt_count() {
        // A peer_ack forward is suppressed and does NOT bump the counter; the next
        // (normal) forward goes through and counts as the first.
        let (btx, brx) = mpsc::channel(8);
        let (rtx, rrx) = mpsc::channel(8);
        let counter = Arc::new(AtomicU32::new(0));
        let d = deps(btx, Some(rtx), Arc::new(AtomicBool::new(false)), Arc::clone(&counter));
        let cmds = vec![
            RouterCommand::Forward {
                from: Author::Brian,
                body: "Agreed — nothing to add.".into(),
                peer_ack: true,
            },
            fwd(Author::Rain, "Here's the actual next step."),
        ];
        let (b, r) = run_and_count(d, cmds, brx, rrx).await;
        assert_eq!(b + r, 1, "only the non-ack forward reaches a peer");
        assert_eq!(
            counter.load(Ordering::Acquire),
            1,
            "peer_ack must not count toward the hard-cap; only the real forward does"
        );
    }

    #[test]
    fn jaccard_similarity_normalizes_and_handles_edges() {
        assert_eq!(jaccard_similarity("ready to go", "ready to go"), 1.0);
        assert_eq!(jaccard_similarity("OK.", "ok"), 1.0);
        assert_eq!(jaccard_similarity(".", "."), 1.0);
        assert_eq!(jaccard_similarity("...", "—"), 1.0);
        assert_eq!(jaccard_similarity(".", "check line forty two"), 0.0);
        assert_eq!(jaccard_similarity("alpha beta", "gamma delta"), 0.0);
        let partial = jaccard_similarity("the quick brown fox", "the quick red hen");
        assert!(
            partial > 0.0 && partial < VOLLEY_SIMILARITY_THRESHOLD,
            "partial overlap should not trip the breaker: {partial}"
        );
    }

    #[test]
    fn peer_of_is_bilateral() {
        assert_eq!(peer_of(Author::Brian), Author::Rain);
        assert_eq!(peer_of(Author::Rain), Author::Brian);
    }
}
