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

use crate::agents::ParticipantInput;
use crate::core::activity::ActivityTracker;
use crate::core::broadcast::peer_forward_message;
use crate::core::ipav::IpavState;
use crate::storage::Author;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// A command from a pump (or the session core) to the router.
#[derive(Debug)]
pub enum RouterCommand {
    /// A completed turn's buffered prose that MIGHT be forwarded to the peer.
    Forward {
        /// The agent that produced the prose.
        from: Author,
        /// The turn's buffered text (the router trims the trailing end again).
        body: String,
        /// Whether the producing turn called `peer_ack` (suppress, don't volley).
        peer_ack: bool,
        /// Whether that `peer_ack` passed `final: true` — the agent ASSERTING
        /// this is its closing turn, so suppression no longer depends on the
        /// length proxy. See the guard in `route_forward`.
        peer_ack_final: bool,
    },
    /// Release the pause hold queue: re-run every held forward through the
    /// normal ladder. Sent by `broadcast` after it clears the pause latch (a
    /// user Send/Resume) — through the SAME channel as forwards, so held
    /// entries flush in arrival order relative to each other. No-op when the
    /// gate is still closed (a re-pause raced the flush) or nothing is held.
    FlushHeld,
}

/// Everything the router task needs. The Arcs (`awaiting`, `user_silent_forwards`,
/// `activity`) are CLONES of the same session-level state the pumps + `broadcast`
/// hold — so `broadcast`'s counter reset and a user-blocking MCP tool's `awaiting`
/// set are both visible here with no extra plumbing.
pub struct RouterDeps {
    /// Await-halt: while set, suppress all peer-forwarding (the user is being
    /// asked). Set by user-blocking MCP tools; cleared by `broadcast`.
    pub awaiting: Arc<AtomicBool>,
    /// L2 hard-cap counter — consecutive peer-forwards with no intervening user
    /// message. `broadcast` resets it to 0 (UNCHANGED from the pre-router model).
    pub user_silent_forwards: Arc<AtomicU32>,
    /// Set true by `broadcast` on each user message; consumed (swap→false) at the
    /// convergence STAGE of `route_forward` to clear `last_forward`/`similar_streak`.
    /// A user message is a hard boundary — without this, a pre-message convergence
    /// streak survives an honored interrupt and can suppress the first post-resume
    /// peer-forward (the bug Rain flagged). Consumed at the convergence stage (not
    /// the top) so an awaiting/peer_ack/hard-cap early-return doesn't burn it.
    pub convergence_reset: Arc<AtomicBool>,
    /// Per-direction delivered-forward counters (diagnostics). Bumped AFTER a
    /// forward actually reaches the peer's stdin. A one-sided break shows one
    /// counter flat while the other climbs — the asymmetry signal a closed-channel
    /// `warn!` can't give when the channel is wedged-open rather than dropped.
    pub fwd_brian_to_rain: Arc<AtomicU64>,
    pub fwd_rain_to_brian: Arc<AtomicU64>,
    /// Liveness flag, true while the router task runs. An [`AliveGuard`] inside
    /// `run_router` flips it false when the task ends for ANY reason — normal
    /// return OR panic-unwind (tokio swallows task panics, so without this a
    /// panicked router reads alive forever). The watchdog reads it: a dead router
    /// while agents are alive = forwarding is down.
    pub alive: Arc<AtomicBool>,
    /// Drives the chat-input lock. The router owns the busy hand-off on the
    /// forward path: set peer busy BEFORE the sender idle (no Idle flicker).
    /// `None` in tests that don't assert activity.
    pub activity: Option<Arc<ActivityTracker>>,
    /// Open-blocking-findings count for the wire banner — read LOCK-FREE per
    /// forward. Owned by the bridge (which recomputes it via `refresh_open_blocking`
    /// when findings change); the router holds this read clone. Replaces a
    /// per-forward `SELECT COUNT(*)` + storage-`Mutex` acquire that ran on EVERY
    /// peer-forward.
    pub open_blocking: Arc<AtomicUsize>,
    /// Current IPAV phase, read at forward time for the wire envelope.
    pub ipav: Arc<Mutex<IpavState>>,
    /// Which session this router serves — only used to attribute a dropped-
    /// forward row. The router is per-session, so this is fixed at spawn.
    pub session_id: Arc<str>,
    /// Storage handle used ONLY to record a DISCARDED forward.
    ///
    /// Deliberately not touched on the delivery path: this struct is
    /// intentionally lock-free (see `open_blocking` above, which exists to keep
    /// a storage-mutex acquire off every forward). Drops are rare, so a write
    /// there costs nothing in the common case — and a lost message with no trace
    /// is precisely the failure this exists to end. `None` in tests that don't
    /// assert telemetry.
    pub storage: Option<crate::storage::Storage>,
    /// Brian's stdin (peer target when Rain speaks).
    pub brian_input: ParticipantInput,
    /// Rain's stdin (peer target when Brian speaks). `None` = solo; the
    /// pump never emits a Forward in solo mode, so the router isn't spawned then.
    pub rain_input: Option<ParticipantInput>,
}

impl RouterDeps {
    /// The stdin sender for `author` — the peer-resolution target. Named 2-agent
    /// resolution; the seam an N-agent peer map replaces later.
    fn input_for(&self, author: Author) -> Option<&ParticipantInput> {
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

    /// The Stop gate: hold (don't deliver) forwards while the session is
    /// cancelling or paused. `None` activity (tests) never gates.
    fn holds_wakes(&self) -> bool {
        self.activity.as_ref().is_some_and(|a| a.holds_wakes())
    }
}

/// Handle-side control + diagnostics for a duo session's router task. Stored as
/// `Option<RouterControl>` on `SessionHandle` (`None` = solo, no router). Holds
/// the Arcs/handle the SESSION side needs to touch the router; grows across the
/// instrument+harden batches. Batch 1 carries the convergence-reset flag.
pub struct RouterControl {
    /// Session-side sender into the router's command channel. `broadcast` uses
    /// it to send [`RouterCommand::FlushHeld`] after clearing the pause latch,
    /// so held forwards release in channel order behind the user's message.
    pub tx: mpsc::Sender<RouterCommand>,
    /// Shared with the router's [`RouterDeps`]. `broadcast` sets it true on a user
    /// message; the router consumes it to clear its convergence streak.
    pub convergence_reset: Arc<AtomicBool>,
    /// Per-direction delivered-forward counters (shared with [`RouterDeps`]). Held
    /// here so the Arc outlives the router task — the watchdog reads the values
    /// through its own `Weak` clones, not this struct.
    pub fwd_brian_to_rain: Arc<AtomicU64>,
    pub fwd_rain_to_brian: Arc<AtomicU64>,
    /// Liveness flag (shared with [`RouterDeps`]). Held here so it outlives the
    /// task: the watchdog's `Weak` upgrade stays valid (reads `false` after the
    /// task's guard ran) for as long as the session handle is alive.
    pub alive: Arc<AtomicBool>,
    /// The spawned router task. `Drop` aborts it so the router is torn down
    /// deterministically the instant the session handle is removed (close /
    /// evict / restart) — not left to the both-pumps-drop-their-`router_tx` race
    /// the old detached-task model relied on (a partial rebuild could violate it,
    /// leaving an old router alive alongside the new one — a split-brain one-way
    /// break). Abort on an already-finished task is a no-op.
    pub task: JoinHandle<()>,
}

impl Drop for RouterControl {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Flips a router's `alive` flag false on drop — i.e. when the router task ends
/// for ANY reason (normal return or panic-unwind). Held as a local inside
/// `run_router` so its destructor runs on both paths.
struct AliveGuard(Arc<AtomicBool>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
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
    // Liveness: dropped when this task ends (normal return OR panic-unwind) →
    // flips `alive` false so the watchdog can surface a dead router.
    let _alive_guard = AliveGuard(Arc::clone(&deps.alive));
    // Cache the PREVIOUS forward's token set (not its body string) — each forward
    // tokenizes only its own body for the convergence check, and nothing is cloned
    // just to seed the next comparison (O2).
    let mut last_forward: Option<HashSet<String>> = None;
    let mut similar_streak: u32 = 0;
    // Pause hold queue: forwards that arrived while the session was cancelling/
    // paused (the Stop gate). Naturally bounded — each agent emits at most one
    // trailing forward (final TurnComplete or Exited) as the interrupt settles,
    // and a paused duo produces nothing new. Flushed FIFO by `FlushHeld`; a
    // forward arriving AFTER the gate reopens but BEFORE the flush command can
    // deliver ahead of held entries — accepted, the held ones are stale partials
    // and the resume notice precedes both on each agent's stdin.
    // (from, body, peer_ack, peer_ack_final) — the ack flags ride along so a
    // held forward replays through the SAME ladder it would have taken live.
    // The `Instant` is when it was held. `forward_events` records losses, and a
    // hold is only supposed to be brief — so an old one IS a loss, just a
    // recoverable-looking kind. See HELD_LATE.
    let mut held: Vec<(Author, String, bool, bool, std::time::Instant)> = Vec::new();
    // Forwards the L2 hard-cap stopped (issues.md #24). Keyed by author, so a
    // genuine runaway overwrites rather than accumulates — at most one entry per
    // agent, which is what makes holding a runaway safe. Released by the same
    // FlushHeld the pause queue uses, but only once the budget has room again;
    // otherwise the release would trip the cap and re-hold on the spot.
    let mut capped: std::collections::HashMap<Author, (String, bool, bool, std::time::Instant)> =
        std::collections::HashMap::new();
    while let Some(cmd) = rx.recv().await {
        match cmd {
            RouterCommand::Forward {
                from,
                body,
                peer_ack,
                peer_ack_final,
            } => {
                // HOLD, don't drop, whenever the peer must not be woken right
                // now. Two conditions qualify, and they used to disagree:
                //
                //   - cancelling/paused (`holds_wakes`) → always held here.
                //   - awaiting-user → used to `return` inside `route_forward`,
                //     SILENTLY DISCARDING the turn.
                //
                // Both are transient states that end when the user speaks, so
                // both deserve the same treatment. The old asymmetry lost real
                // work: `awaiting` is set by ask_user_choice / mark_awaiting_user
                // / halt DURING a turn, while the Forward is emitted at
                // turn-END — so every turn that finished by asking the user a
                // question or yielding had its entire body dropped instead of
                // reaching the peer. That is a duo silently half-deaf exactly
                // when one side is summarising for the other.
                //
                // Held forwards still settle the sender idle — the pump
                // delegated self-idle to us, and `await_both_idle` must see the
                // interrupt land or the escalation SIGKILLs an already-stopped
                // agent.
                if deps.holds_wakes() || deps.awaiting.load(Ordering::Acquire) {
                    debug!(
                        agent = ?from,
                        held = held.len() + 1,
                        "router: session cancelling/paused; holding forward"
                    );
                    deps.set_idle(from);
                    held.push((from, body, peer_ack, peer_ack_final, std::time::Instant::now()));
                    continue;
                }
                if let Some((body, peer_ack, peer_ack_final)) = route_forward(
                    &deps,
                    &mut last_forward,
                    &mut similar_streak,
                    from,
                    body,
                    peer_ack,
                    peer_ack_final,
                )
                .await
                {
                    // Hard-capped: hold the newest from this agent instead of
                    // destroying it.
                    capped.insert(from, (body, peer_ack, peer_ack_final, std::time::Instant::now()));
                }
            }
            RouterCommand::FlushHeld => {
                // Still gated (a re-pause raced this flush, or a fresh question
                // was parked)? Keep holding — the next broadcast sends another
                // FlushHeld. Mirrors the hold condition above, so a forward can
                // never be released into a state that would have held it.
                // `capped` counts here too — guarding on `held` alone would skip
                // the hard-cap release entirely whenever the pause queue happened
                // to be empty, which is the common case.
                if deps.holds_wakes()
                    || deps.awaiting.load(Ordering::Acquire)
                    || (held.is_empty() && capped.is_empty())
                {
                    continue;
                }
                debug!(count = held.len(), "router: flushing held forwards");
                for (from, body, peer_ack, peer_ack_final, held_at) in held.drain(..) {
                    // A hold ends at the next user action, so minutes-old holds
                    // mean a wake path cleared `awaiting` WITHOUT sending
                    // FlushHeld and nobody noticed until the next typed message.
                    // That is exactly the bug fixed in `b87f97a` — and it left
                    // no trace, because `forward_events` only ever recorded
                    // drops and a late hold is not a drop. Now it is queryable.
                    let waited = held_at.elapsed();
                    if waited >= HELD_LATE {
                        warn!(
                            agent = ?from,
                            waited_secs = waited.as_secs(),
                            "router: held forward released far later than a hold should last — \
                             a wake path likely cleared `awaiting` without flushing"
                        );
                        record_drop(&deps, from, "held_late", &body).await;
                    }
                    if let Some((body, pa, paf)) = route_forward(
                        &deps,
                        &mut last_forward,
                        &mut similar_streak,
                        from,
                        body,
                        peer_ack,
                        peer_ack_final,
                    )
                    .await
                    {
                        capped.insert(from, (body, pa, paf, std::time::Instant::now()));
                    }
                }
                // Release hard-capped forwards too, but ONLY with budget to spare
                // — `broadcast` resets the counter, a phase advance does not, and
                // releasing into a still-full budget would re-trip the cap and
                // re-hold on the spot. Re-capped entries go straight back.
                if !capped.is_empty()
                    && deps.user_silent_forwards.load(Ordering::Acquire) <= VOLLEY_HARD_CAP
                {
                    debug!(count = capped.len(), "router: releasing hard-capped forwards");
                    for (from, (body, pa, paf, held_at)) in std::mem::take(&mut capped) {
                        if held_at.elapsed() >= HELD_LATE {
                            warn!(
                                agent = ?from,
                                waited_secs = held_at.elapsed().as_secs(),
                                "router: hard-capped forward released far later than it should have been"
                            );
                            record_drop(&deps, from, "held_late", &body).await;
                        }
                        if let Some((body, pa, paf)) = route_forward(
                            &deps,
                            &mut last_forward,
                            &mut similar_streak,
                            from,
                            body,
                            pa,
                            paf,
                        )
                        .await
                        {
                            capped.insert(from, (body, pa, paf, held_at));
                        }
                    }
                }
            }
        }
    }
    // The command channel closed — the session is going away and nothing will
    // ever flush what is still held. Those forwards are lost outright, which is
    // precisely what `forward_events` is for; without this they were dropped on
    // the floor with the local `held` vec and left no trace at all.
    for (from, body, _, _, held_at) in held.drain(..) {
        warn!(
            agent = ?from,
            waited_secs = held_at.elapsed().as_secs(),
            "router: session ended with a forward still held — it never reached the peer"
        );
        record_drop(&deps, from, "held_stranded", &body).await;
    }
    // Hard-capped forwards share the same fate and the same accounting: held is
    // only better than dropped if the hold eventually resolves, so one that
    // never does is still a loss and still says so.
    for (from, (body, _, _, held_at)) in std::mem::take(&mut capped) {
        warn!(
            agent = ?from,
            waited_secs = held_at.elapsed().as_secs(),
            "router: session ended with a hard-capped forward still held — it never reached the peer"
        );
        record_drop(&deps, from, "held_stranded", &body).await;
    }
}

/// A hold is meant to last until the user's next action. Past this, the hold
/// itself is the failure — some path cleared `awaiting` without sending
/// `FlushHeld`, and the peer sat half-deaf until an unrelated message shook it
/// loose. Sized like `STALE_GATE_MAX_AGE_SECS`: comfortably longer than a human
/// answering a question, far shorter than "the user went to lunch and came back
/// to a duo that had silently stopped talking to itself".
const HELD_LATE: std::time::Duration = std::time::Duration::from_secs(900);

/// The forward ladder — same order/semantics as the pre-router `flush_buffer`,
/// now in ONE place. Each suppression path still clears the sender's `busy` (the
/// pump delegated self-idle to us on the forward path), so the session settles
/// correctly. On a real forward we set the peer busy BEFORE the sender idle.
/// Returns `Some(body, peer_ack, peer_ack_final)` when the L2 hard-cap fired —
/// the caller must HOLD that forward rather than let it die. Every other path
/// (delivered, peer_ack-suppressed, convergence-broken, no-peer) returns `None`.
async fn route_forward(
    deps: &RouterDeps,
    last_forward: &mut Option<HashSet<String>>,
    similar_streak: &mut u32,
    from: Author,
    body: String,
    peer_ack: bool,
    peer_ack_final: bool,
) -> Option<(String, bool, bool)> {
    let trimmed = body.trim_end();
    if trimmed.is_empty() {
        deps.set_idle(from);
        return None;
    }
    let peer = peer_of(from);
    let Some(peer_tx) = deps.input_for(peer) else {
        // No peer sender for `from`'s peer. In a duo session both agents always
        // have a sender, and the router is never spawned for a solo session — so
        // this is reachable only via the impossible `from == User`. Log the
        // invariant breach (review advisory) and never strand `from` busy.
        debug!(agent = ?from, "router: no peer sender (unexpected non-duo author); dropping forward");
        record_drop(deps, from, "no_peer", trimmed).await;
        deps.set_idle(from);
        return None;
    };

    // 1. Await-halt is NO LONGER handled here. It used to `return`, discarding
    //    the turn; it is now a HOLD at the dispatch site (see `run_router`), so
    //    the message is replayed once the user replies instead of being lost.
    //    Deliberately not re-checked here: a replay runs through this function,
    //    and re-testing the flag would drop the very forward the hold preserved.
    // 2. peer_ack: explicit ack — suppress BEFORE the counters (not a volley
    //    contribution, so it must not bump the hard-cap or extend the streak).
    //    UNLESS the same turn carried substantive text: the 2026-07-27 archive
    //    study found four full reviews destroyed by an agent posting its
    //    verdict and calling peer_ack in the same turn (the tool name reads as
    //    "acknowledge my peer", the semantics were "throw my turn away"). A
    //    substantive turn forwards anyway, tagged, and counts like any other.
    //    `final: true` overrides the length proxy — the agent has ASSERTED this
    //    is its closing turn. Safe to honour because suppression skips the WAKE,
    //    never the record: the turn's text is persisted by the pump's
    //    `AgentEvent::Text` arm as it arrives, whether or not a Forward is sent.
    //    Without this, "I agree, and here is the one reason why" exceeds 200
    //    bytes, forwards, wakes the peer, and continues the volley the ack
    //    existed to end (filed from a live session as feedback #6).
    if peer_ack {
        if peer_ack_final || trimmed.len() <= PEER_ACK_MAX_SUPPRESSED_LEN {
            debug!(
                agent = ?from,
                final_asserted = peer_ack_final,
                "router: peer_ack; suppressing peer forward"
            );
            deps.set_idle(from);
            return None;
        }
        debug!(
            agent = ?from,
            len = trimmed.len(),
            "router: peer_ack on a substantive turn; forwarding anyway"
        );
    }
    // 3. L2 hard-cap: bound consecutive peer-forwards with no user message.
    let n = deps.user_silent_forwards.fetch_add(1, Ordering::AcqRel) + 1;
    if n > VOLLEY_HARD_CAP {
        warn!(
            agent = ?from,
            count = n,
            "router: hard-cap reached; breaking volley + holding this forward"
        );
        // NO LONGER LOSSY (issues.md #24). It used to `record_drop` and discard.
        // Measured on 2026-08-01 session s-d16364ee: 40 of Rain's forwards
        // destroyed here, including "`58fae66` is the risky one — rejection
        // without repair". The budget had been burned by filler turns (54 of her
        // 61 text turns under 200 chars — issue #8), so what the breaker actually
        // ate was the substantive minority.
        //
        // The cap exists to stop a runaway LOOP; it never needed to lose the
        // MESSAGE. Same move already made for `awaiting`, which used to discard
        // and now holds and replays. The caller keeps the most recent per agent,
        // so a genuine runaway still can't grow the queue.
        //
        // Convergence (below) stays lossy on purpose — that one suppresses
        // REPETITION, where the held copy would be a duplicate of what already
        // landed.
        break_volley(deps);
        return Some((body, peer_ack, peer_ack_final));
    }
    // 3.5 Convergence reset across the user boundary: `broadcast` sets this on a
    //     user message. Consumed HERE (not at the top) so the awaiting/peer_ack/
    //     hard-cap early-returns above never burn it — the reset survives until a
    //     forward actually reaches convergence evaluation, then clears the stale
    //     pre-message streak so it can't suppress the first post-message forward.
    if deps.convergence_reset.swap(false, Ordering::AcqRel) {
        *last_forward = None;
        *similar_streak = 0;
    }
    // 4. L2 convergence over the SINGLE interleaved stream: a forward
    //    ≥VOLLEY_SIMILARITY_THRESHOLD similar to the PREVIOUS forward (from either
    //    agent) extends the streak; a dissimilar one resets it. Deliberately NOT
    //    reset on break — a sustained repetition keeps suppressing until content
    //    changes.
    let cur_tokens = token_set(trimmed);
    match last_forward.as_ref() {
        Some(prev) if jaccard_from_sets(prev, &cur_tokens) >= VOLLEY_SIMILARITY_THRESHOLD => {
            *similar_streak += 1;
        }
        _ => *similar_streak = 0,
    }
    *last_forward = Some(cur_tokens);
    if *similar_streak >= VOLLEY_SIMILAR_BREAK {
        debug!(agent = ?from, streak = *similar_streak, "router: convergence breaker tripped; breaking volley + unlocking input");
        // Also deliberately lossy, also no longer silent. This is the breaker
        // most likely to swallow a genuine turn, because "I agree, because X"
        // resembles the previous turn far more than a new finding does.
        record_drop(deps, from, "convergence", trimmed).await;
        break_volley(deps);
        return None;
    }
    // 5. Forward, then hand off busy IN ORDER (peer busy BEFORE sender idle) so
    //    `derive()` never sees both-idle → no momentary Idle that unlocks input
    //    mid-handoff.
    let phase = deps.ipav.lock().await.current_phase;
    let open_blocking = deps.open_blocking.load(Ordering::Relaxed);
    let tagged;
    let body_to_send = if peer_ack {
        tagged = format!(
            "[peer_ack overridden — this turn carried substantive text, so it was \
             forwarded anyway]\n{trimmed}"
        );
        tagged.as_str()
    } else {
        trimmed
    };
    peer_forward_message(from, body_to_send, phase, open_blocking, peer_tx).await;
    // Diagnostics: count the DELIVERED forward by direction (after the send). A
    // one-sided break shows one counter flat while the other climbs. `User` can't
    // reach here (the peer-resolution early-return above handles it).
    match from {
        Author::Brian => {
            deps.fwd_brian_to_rain.fetch_add(1, Ordering::Relaxed);
        }
        Author::Rain => {
            deps.fwd_rain_to_brian.fetch_add(1, Ordering::Relaxed);
        }
        Author::User => {}
    }
    if let Some(activity) = &deps.activity {
        activity.set_busy(peer, true);
        activity.set_busy(from, false);
    }
    None // delivered — nothing for the caller to hold
}

/// Break a volley: set BOTH agents idle so `ActivityTracker::derive` returns Idle
/// and the chat input unlocks. Shared by the L2 hard-cap and the convergence
/// breaker. (2-agent named: Brian + Rain.)
/// Record a forward that was DISCARDED, so a half-deaf duo is diagnosable
/// instead of arguable. Best-effort: telemetry must never break routing, so a
/// storage failure warns and the router carries on.
async fn record_drop(deps: &RouterDeps, from: Author, reason: &str, body: &str) {
    let Some(storage) = deps.storage.as_ref() else {
        return;
    };
    if let Err(e) = storage
        .insert_forward_drop(
            &deps.session_id,
            from.as_str(),
            peer_of(from).as_str(),
            reason,
            body,
        )
        .await
    {
        warn!(?e, agent = ?from, reason, "router: could not record dropped forward");
    }
}

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

/// Longest turn text (bytes, trimmed) `peer_ack` may still suppress. Above
/// this the turn is substantive — a review verdict, a correction, a plan —
/// and suppressing it destroys peer-visible work (four such losses in the
/// 2026-07-27 archive study). Sized to fit a genuine ack ("Agreed — nothing
/// to add on the last two points.") while catching anything with content.
const PEER_ACK_MAX_SUPPRESSED_LEN: usize = 200;

/// Tokenize a forward body for convergence comparison: split on whitespace, trim
/// each token of leading/trailing non-alphanumerics, lowercase, drop empties — so
/// "OK.", "OK", "ok" all reduce to {ok}.
fn token_set(s: &str) -> HashSet<String> {
    s.split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Token-set Jaccard similarity — the shape-based convergence signal (no length
/// threshold, no keyword/prefix list). Edge: BOTH sets empty (pure punctuation /
/// emoji like "." or "🤝", the canonical s-e4fc25 volley) → 1.0, so convergence
/// catches it fast rather than deferring to the hard-cap. One empty, one not →
/// 0.0. Two DISTINCT substantive messages always carry alphanumeric tokens, so
/// they can never collide at 1.0 via the both-empty path.
fn jaccard_from_sets(sa: &HashSet<String>, sb: &HashSet<String>) -> f64 {
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(sb).count();
    let union = sa.union(sb).count();
    if union == 0 {
        1.0
    } else {
        inter as f64 / union as f64
    }
}

/// String-level convenience wrapper (tokenizes BOTH sides). Test-only: the hot
/// path keeps the previous forward's token set and calls `jaccard_from_sets`
/// directly, so it never re-tokenizes the previous body.
#[cfg(test)]
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    jaccard_from_sets(&token_set(a), &token_set(b))
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
    use crate::agents::OutgoingUserMessage;
    use crate::core::ipav::IpavState;

    /// One participant's stdin, plus the receiver a test counts forwards on.
    /// Capacity is uniform (512) rather than per-test: the old sizes were
    /// incidental, and a forward the router dropped for want of buffer would
    /// look exactly like a forward the ladder suppressed.
    fn stub_input() -> (ParticipantInput, mpsc::Receiver<OutgoingUserMessage>) {
        let (tx, rx) = mpsc::channel(512);
        (ParticipantInput::new("s1", tx), rx)
    }

    fn deps(
        brian_input: ParticipantInput,
        rain_input: Option<ParticipantInput>,
        awaiting: Arc<AtomicBool>,
        counter: Arc<AtomicU32>,
    ) -> RouterDeps {
        RouterDeps {
            awaiting,
            user_silent_forwards: counter,
            convergence_reset: Arc::new(AtomicBool::new(false)),
            fwd_brian_to_rain: Arc::new(AtomicU64::new(0)),
            fwd_rain_to_brian: Arc::new(AtomicU64::new(0)),
            alive: Arc::new(AtomicBool::new(true)),
            activity: None,
            open_blocking: Arc::new(AtomicUsize::new(0)),
            ipav: Arc::new(Mutex::new(IpavState::default())),
            session_id: "s1".into(),
            // Most router tests assert routing, not telemetry; the drop-recording
            // test below builds its own deps with a real Storage.
            storage: None,
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
            peer_ack_final: false,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hard_cap_breaks_after_cap() {
        // Distinct bodies so convergence never trips first — the cap is the sole
        // reason forwarding stops. All from Brian → all land on Rain's channel.
        let (btx, brx) = stub_input();
        let (rtx, rrx) = stub_input();
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
        let (btx, brx) = stub_input();
        let (rtx, rrx) = stub_input();
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
        let (btx, brx) = stub_input();
        let (rtx, rrx) = stub_input();
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
        let (btx, brx) = stub_input();
        let (rtx, rrx) = stub_input();
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
    async fn a_broken_volley_records_which_message_it_dropped() {
        // The breakers stay lossy on purpose, but they must stop being SILENT:
        // for a whole session nobody could tell whether a reviewer was careless
        // or the transport had eaten the message, because a drop left no trace
        // anywhere — not for the sender, the receiver, or the user.
        let storage = crate::storage::Storage::memory().await.unwrap();
        let (btx, _brx) = stub_input();
        let (rtx, _rrx) = stub_input();
        let mut d = deps(
            btx,
            Some(rtx),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(0)),
        );
        d.storage = Some(storage.clone());
        let (mut last, mut streak) = (None, 0u32);

        // Two near-identical turns trip the convergence breaker on the second.
        let body = "Agreed, and the reason I am confident is the drift-check output.";
        route_forward(&d, &mut last, &mut streak, Author::Brian, body.into(), false, false).await;
        route_forward(&d, &mut last, &mut streak, Author::Brian, body.into(), false, false).await;
        route_forward(&d, &mut last, &mut streak, Author::Brian, body.into(), false, false).await;

        let drops = storage.list_forward_drops(Some("s1")).await.unwrap();
        assert!(!drops.is_empty(), "a broken volley must leave a trace");
        assert_eq!(drops[0].reason, "convergence");
        assert_eq!(drops[0].from_agent, "brian");
        assert_eq!(drops[0].to_agent, "rain", "records the peer that never heard it");
        assert!(
            drops[0].body_preview.contains("drift-check output"),
            "the preview must identify WHICH message was lost: {}",
            drops[0].body_preview
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_delivered_forward_records_nothing() {
        // Only losses are recorded — the delivery path must not acquire storage
        // at all, which is why `open_blocking` exists as a lock-free cache.
        let storage = crate::storage::Storage::memory().await.unwrap();
        let (btx, _brx) = stub_input();
        let (rtx, _rrx) = stub_input();
        let mut d = deps(
            btx,
            Some(rtx),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(0)),
        );
        d.storage = Some(storage.clone());
        let (mut last, mut streak) = (None, 0u32);
        route_forward(&d, &mut last, &mut streak, Author::Brian, "alpha".into(), false, false).await;
        route_forward(&d, &mut last, &mut streak, Author::Rain, "beta".into(), false, false).await;
        assert!(
            storage.list_forward_drops(None).await.unwrap().is_empty(),
            "delivered forwards are not drops"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn awaiting_holds_the_forward_and_delivers_it_after_the_user_replies() {
        // The regression pin for the half-deaf duo. `awaiting` is set by
        // ask_user_choice / mark_awaiting_user / halt DURING a turn, while the
        // Forward is emitted at turn-END — so a turn that finished by asking the
        // user a question used to have its ENTIRE body discarded rather than
        // forwarded. The peer then genuinely never saw the plan/finding/review
        // it was being asked to act on, while the user could read it on screen.
        //
        // It must now be HELD and replayed, exactly like the pause path.
        let (btx, _brx) = stub_input();
        let (rtx, mut rrx) = stub_input();
        let awaiting = Arc::new(AtomicBool::new(true));
        let d = deps(
            btx,
            Some(rtx),
            Arc::clone(&awaiting),
            Arc::new(AtomicU32::new(0)),
        );
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(run_router(d, rx));

        // Brian's turn ends with a parked question; his summary must not vanish.
        tx.send(fwd(Author::Brian, "here is the plan you should review"))
            .await
            .unwrap();
        // Nothing reaches Rain while the user is being asked.
        tokio::task::yield_now().await;
        assert!(
            rrx.try_recv().is_err(),
            "the peer must not be woken while the user is being asked"
        );

        // The user answers: broadcast clears `awaiting`, then sends FlushHeld.
        awaiting.store(false, Ordering::Release);
        tx.send(RouterCommand::FlushHeld).await.unwrap();
        drop(tx);
        task.await.unwrap();

        let delivered = rrx.try_recv().expect("the held forward must be delivered, not lost");
        assert!(
            delivered.message.content.contains("here is the plan you should review"),
            "the ORIGINAL body must survive the hold: {}",
            delivered.message.content
        );
        assert!(
            rrx.try_recv().is_err(),
            "delivered exactly once — a hold must not duplicate"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn peer_ack_suppresses_and_doesnt_count() {
        // A peer_ack forward is suppressed and does NOT bump the counter; the next
        // (normal) forward goes through and counts as the first.
        let (btx, brx) = stub_input();
        let (rtx, rrx) = stub_input();
        let counter = Arc::new(AtomicU32::new(0));
        let d = deps(btx, Some(rtx), Arc::new(AtomicBool::new(false)), Arc::clone(&counter));
        let cmds = vec![
            RouterCommand::Forward {
                from: Author::Brian,
                body: "Agreed — nothing to add.".into(),
                peer_ack: true,
                peer_ack_final: false,
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

    #[tokio::test(flavor = "current_thread")]
    async fn peer_ack_on_substantive_turn_forwards_anyway() {
        // Archive study 2026-07-27: an agent posted a full 3-point plan review
        // and called peer_ack in the same turn — the review was silently
        // destroyed. A substantive turn must forward despite the ack, tagged.
        let (btx, mut brx) = stub_input();
        let (rtx, rrx) = stub_input();
        let counter = Arc::new(AtomicU32::new(0));
        let d = deps(btx, Some(rtx), Arc::new(AtomicBool::new(false)), Arc::clone(&counter));
        let review = "Plan review: (1) the `protected` flag is missing on the three \
                      already-incremental reports, which will re-materialize them on the \
                      next full run; (2) the migrate script's drop list has no twitter \
                      section; (3) the uniqueKey assertion still executes on type:table \
                      models, so the NULL-segment fix is only half the story.";
        assert!(review.len() > PEER_ACK_MAX_SUPPRESSED_LEN);
        let cmds = vec![RouterCommand::Forward {
            from: Author::Rain,
            body: review.into(),
            peer_ack: true,
            peer_ack_final: false,
        }];
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_router(d, rx));
        for c in cmds {
            tx.send(c).await.unwrap();
        }
        drop(tx);
        task.await.unwrap();
        drop(rrx);
        let delivered = brx.try_recv().expect("substantive ack-turn reaches the peer");
        assert!(
            delivered.message.content.contains("peer_ack overridden"),
            "forward is tagged so the receiver knows the sender acked"
        );
        assert!(delivered.message.content.contains("uniqueKey assertion"));
        assert_eq!(
            counter.load(Ordering::Acquire),
            1,
            "an overridden ack is a real forward and counts toward the hard-cap"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn peer_ack_final_suppresses_a_substantive_turn() {
        // The turn shape that ENDS a volley: agreement plus the single reason
        // for it. Well over the length proxy, but the agent has asserted this is
        // its closing statement — so it must NOT wake the peer. Feedback #6.
        let (btx, mut brx) = stub_input();
        let (rtx, rrx) = stub_input();
        let counter = Arc::new(AtomicU32::new(0));
        let d = deps(btx, Some(rtx), Arc::new(AtomicBool::new(false)), Arc::clone(&counter));
        let closing = "Agreed — and the reason I'm confident is that the drift-check \
                       output already disproves the generate_models.py concern, so there \
                       is nothing further for either of us to chase on that thread. No \
                       new finding here, no correction, and nothing you need to act on: \
                       this is simply where I stop.";
        assert!(
            closing.len() > PEER_ACK_MAX_SUPPRESSED_LEN,
            "the point of this test is a turn the LENGTH rule would have forwarded"
        );
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_router(d, rx));
        tx.send(RouterCommand::Forward {
            from: Author::Rain,
            body: closing.into(),
            peer_ack: true,
            peer_ack_final: true,
        })
        .await
        .unwrap();
        drop(tx);
        task.await.unwrap();
        drop(rrx);
        assert!(
            brx.try_recv().is_err(),
            "final:true must suppress the wake regardless of length"
        );
        assert_eq!(
            counter.load(Ordering::Acquire),
            0,
            "a suppressed ack is not a forward and must not count toward the hard-cap"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn substantive_turn_without_final_still_forwards() {
        // Regression pin for the 2026-07-27 bug (four reviews destroyed): adding
        // `final` must not weaken the DEFAULT. Same body as the test above, with
        // the flag omitted — this one has to reach the peer.
        let (btx, mut brx) = stub_input();
        let (rtx, rrx) = stub_input();
        let counter = Arc::new(AtomicU32::new(0));
        let d = deps(btx, Some(rtx), Arc::new(AtomicBool::new(false)), Arc::clone(&counter));
        let closing = "Agreed — and the reason I'm confident is that the drift-check \
                       output already disproves the generate_models.py concern, so there \
                       is nothing further for either of us to chase on that thread. No \
                       new finding here, no correction, and nothing you need to act on: \
                       this is simply where I stop.";
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_router(d, rx));
        tx.send(RouterCommand::Forward {
            from: Author::Rain,
            body: closing.into(),
            peer_ack: true,
            peer_ack_final: false,
        })
        .await
        .unwrap();
        drop(tx);
        task.await.unwrap();
        drop(rrx);
        let delivered = brx
            .try_recv()
            .expect("without final:true the length rule still governs");
        assert!(delivered.message.content.contains("peer_ack overridden"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn convergence_reset_clears_stale_streak() {
        // A user message (broadcast sets `convergence_reset`) is a hard boundary:
        // the pre-message convergence streak must NOT carry over to suppress the
        // first post-message forward. Without the reset, three identical "🤝"
        // forwards = deliver, deliver, SUPPRESS (streak hits VOLLEY_SIMILAR_BREAK).
        // With a reset consumed before the third, the streak clears → all three
        // deliver. Drives `route_forward` directly so the flag toggles
        // deterministically between forwards (no task/channel race).
        // Brian-origin forwards land on RAIN's channel (peer = Rain).
        let (btx, _brx) = stub_input();
        let (rtx, mut rrx) = stub_input();
        let reset = Arc::new(AtomicBool::new(false));
        let mut d = deps(
            btx,
            Some(rtx),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(0)),
        );
        d.convergence_reset = Arc::clone(&reset);
        let (mut last, mut streak) = (None, 0u32);
        route_forward(&d, &mut last, &mut streak, Author::Brian, "🤝".into(), false, false).await;
        route_forward(&d, &mut last, &mut streak, Author::Brian, "🤝".into(), false, false).await;
        assert_eq!(streak, 1, "two identical forwards build a streak of 1");
        // Simulate the user speaking → broadcast sets the flag.
        reset.store(true, Ordering::Release);
        route_forward(&d, &mut last, &mut streak, Author::Brian, "🤝".into(), false, false).await;
        assert_eq!(streak, 0, "the reset cleared the streak before the third forward");
        let mut delivered = 0;
        while rrx.try_recv().is_ok() {
            delivered += 1;
        }
        assert_eq!(
            delivered, 3,
            "all three delivered — the reset prevented the third's suppression"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn convergence_reset_survives_a_suppressed_forward() {
        // The reset is consumed at the CONVERGENCE stage, so a forward that
        // early-returns BEFORE that stage must NOT burn it — it stays set for
        // the next forward that actually reaches convergence. (Closes Rain's
        // review edge: a reset consumed by a not-actually-delivered forward.)
        //
        // The suppression used here is `peer_ack`, not awaiting: awaiting is no
        // longer a `route_forward` early-return at all — it HOLDS at the
        // dispatch site so the turn is replayed rather than lost. peer_ack still
        // returns before the convergence stage, so it exercises the same
        // invariant on a path that still exists.
        // Brian-origin forwards land on RAIN's channel (peer = Rain).
        let (btx, _brx) = stub_input();
        let (rtx, mut rrx) = stub_input();
        let reset = Arc::new(AtomicBool::new(true));
        let awaiting = Arc::new(AtomicBool::new(false));
        let mut d = deps(btx, Some(rtx), Arc::clone(&awaiting), Arc::new(AtomicU32::new(0)));
        d.convergence_reset = Arc::clone(&reset);
        let (mut last, mut streak) = (Some(HashSet::from(["stale".to_string()])), 5u32);
        // A short peer_ack suppresses this forward — and must leave reset intact.
        route_forward(&d, &mut last, &mut streak, Author::Brian, "ok".into(), true, false).await;
        assert!(
            reset.load(Ordering::Acquire),
            "a forward suppressed before the convergence stage must NOT consume the reset"
        );
        // The next real forward consumes it.
        route_forward(&d, &mut last, &mut streak, Author::Brian, "fresh line".into(), false, false).await;
        assert!(
            !reset.load(Ordering::Acquire),
            "the forward that reached convergence consumed the reset"
        );
        assert_eq!(streak, 0, "stale streak cleared");
        let mut delivered = 0;
        while rrx.try_recv().is_ok() {
            delivered += 1;
        }
        assert_eq!(delivered, 1, "only the un-suppressed forward was delivered");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn counters_track_per_direction_on_delivery() {
        // Delivered forwards bump the matching direction counter; a suppressed one
        // does not (the bump is after the actual send).
        let (btx, _brx) = stub_input();
        let (rtx, _rrx) = stub_input();
        let b2r = Arc::new(AtomicU64::new(0));
        let r2b = Arc::new(AtomicU64::new(0));
        let awaiting = Arc::new(AtomicBool::new(false));
        let mut d = deps(btx, Some(rtx), Arc::clone(&awaiting), Arc::new(AtomicU32::new(0)));
        d.fwd_brian_to_rain = Arc::clone(&b2r);
        d.fwd_rain_to_brian = Arc::clone(&r2b);
        let (mut last, mut streak) = (None, 0u32);
        // Distinct bodies → no convergence break; all delivered.
        route_forward(&d, &mut last, &mut streak, Author::Brian, "alpha".into(), false, false).await;
        route_forward(&d, &mut last, &mut streak, Author::Brian, "beta".into(), false, false).await;
        route_forward(&d, &mut last, &mut streak, Author::Rain, "gamma".into(), false, false).await;
        // A suppressed forward must NOT count. peer_ack rather than awaiting:
        // awaiting no longer suppresses here, it holds at the dispatch site.
        let _ = &awaiting;
        route_forward(&d, &mut last, &mut streak, Author::Brian, "ok".into(), true, false).await;
        assert_eq!(b2r.load(Ordering::Acquire), 2, "two delivered Brian→Rain forwards");
        assert_eq!(r2b.load(Ordering::Acquire), 1, "one delivered Rain→Brian forward");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_hard_cap_holds_the_forward_and_delivers_it_after_the_user_speaks() {
        // issues.md #24. The hard-cap used to DESTROY the forward that tripped
        // it; on 2026-08-01 that cost session s-d16364ee 40 of Rain's messages,
        // including a warning that a commit was risky. The cap must still break
        // the loop — but the message has to survive it.
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.create_session("s1", "One", None).await.unwrap();
        let (btx, _brx) = stub_input();
        let (rtx, mut rrx) = stub_input();
        let counter = Arc::new(AtomicU32::new(0));
        let mut d = deps(
            btx,
            Some(rtx),
            Arc::new(AtomicBool::new(false)),
            Arc::clone(&counter),
        );
        d.storage = Some(storage.clone());
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(run_router(d, rx));

        // Park the budget one under the cap, then send the forward that trips it.
        counter.store(VOLLEY_HARD_CAP, Ordering::Release);
        tx.send(fwd(Author::Brian, "`58fae66` is the risky one — rejection without repair"))
            .await
            .unwrap();
        // Give the router a turn to process before asserting the negative.
        tokio::task::yield_now().await;
        assert!(
            rrx.try_recv().is_err(),
            "the capped forward must NOT reach the peer yet — the volley is broken"
        );
        assert!(
            storage
                .list_forward_drops(Some("s1"))
                .await
                .unwrap()
                .is_empty(),
            "a held forward is not a loss and must not be recorded as one"
        );

        // The user speaks: `broadcast` resets the counter and sends FlushHeld.
        counter.store(0, Ordering::Release);
        tx.send(RouterCommand::FlushHeld).await.unwrap();
        drop(tx);
        task.await.unwrap();

        let delivered = rrx
            .try_recv()
            .expect("the capped forward must be delivered once the budget resets");
        assert!(
            format!("{delivered:?}").contains("58fae66"),
            "the surviving message must be the one the cap stopped"
        );
        assert!(
            storage
                .list_forward_drops(Some("s1"))
                .await
                .unwrap()
                .is_empty(),
            "delivered late is not lost — nothing to record"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_runaway_keeps_only_the_newest_capped_forward_per_agent() {
        // Holding a runaway is only safe because the queue cannot grow: each
        // agent has exactly one slot and a newer forward overwrites it.
        let (btx, _brx) = stub_input();
        let (rtx, mut rrx) = stub_input();
        let counter = Arc::new(AtomicU32::new(VOLLEY_HARD_CAP));
        let d = deps(
            btx,
            Some(rtx),
            Arc::new(AtomicBool::new(false)),
            Arc::clone(&counter),
        );
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(run_router(d, rx));

        for i in 0..5 {
            tx.send(fwd(Author::Brian, &format!("runaway turn {i}")))
                .await
                .unwrap();
        }
        // Let the router actually consume all five BEFORE the budget reopens —
        // otherwise the reset races the drain and the later forwards deliver
        // straight through, which is a test artefact rather than the behaviour
        // under test.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        counter.store(0, Ordering::Release);
        tx.send(RouterCommand::FlushHeld).await.unwrap();
        drop(tx);
        task.await.unwrap();

        let first = rrx.try_recv().expect("the newest capped forward survives");
        assert!(
            format!("{first:?}").contains("runaway turn 4"),
            "the NEWEST forward should be the one kept"
        );
        assert!(
            rrx.try_recv().is_err(),
            "only one forward per agent may be held, however long the runaway"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_forward_still_held_when_the_session_ends_is_recorded_as_lost() {
        // The blind spot behind bug B (`b87f97a`). `forward_events` was
        // drops-only, and a HOLD is not a drop — so a forward that was held and
        // then never flushed vanished with the router's local queue, leaving no
        // trace anywhere. Anything still held when the command channel closes
        // was definitively never delivered, which is exactly what this table
        // means.
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.create_session("s1", "One", None).await.unwrap();
        let (btx, _brx) = stub_input();
        let (rtx, _rrx) = stub_input();
        let awaiting = Arc::new(AtomicBool::new(true)); // parked question → hold
        let mut d = deps(btx, Some(rtx), awaiting, Arc::new(AtomicU32::new(0)));
        d.storage = Some(storage.clone());
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(run_router(d, rx));

        tx.send(fwd(Author::Brian, "a summary the peer never received"))
            .await
            .unwrap();
        // Session goes away with the forward still held.
        drop(tx);
        task.await.unwrap();

        let drops = storage.list_forward_drops(Some("s1")).await.unwrap();
        assert_eq!(drops.len(), 1, "the stranded forward must be recorded");
        assert_eq!(drops[0].reason, "held_stranded");
        assert_eq!(drops[0].from_agent, "brian");
        assert!(
            drops[0].body_preview.contains("never received"),
            "the row must identify WHICH message was lost"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_promptly_flushed_hold_is_not_recorded_as_a_loss() {
        // The inverse guard: a normal hold→flush cycle is not a loss, and
        // logging it as one would drown the table in noise and make every row
        // stop meaning "a message was lost".
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage.create_session("s1", "One", None).await.unwrap();
        let (btx, _brx) = stub_input();
        let (rtx, mut rrx) = stub_input();
        let awaiting = Arc::new(AtomicBool::new(true));
        let mut d = deps(
            btx,
            Some(rtx),
            Arc::clone(&awaiting),
            Arc::new(AtomicU32::new(0)),
        );
        d.storage = Some(storage.clone());
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(run_router(d, rx));

        tx.send(fwd(Author::Brian, "held briefly")).await.unwrap();
        awaiting.store(false, Ordering::Release); // user answered
        tx.send(RouterCommand::FlushHeld).await.unwrap();
        drop(tx);
        task.await.unwrap();

        assert!(
            rrx.try_recv().is_ok(),
            "the held forward should have been delivered on flush"
        );
        assert!(
            storage
                .list_forward_drops(Some("s1"))
                .await
                .unwrap()
                .is_empty(),
            "a prompt hold→flush is not a loss and must not be recorded"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn paused_holds_forwards_and_flush_delivers_exactly_once() {
        // The Stop gate end-to-end. While cancelling/paused a Forward is HELD:
        // the peer never wakes (the Exited best-effort forward after a SIGKILL
        // arrives as this same RouterCommand::Forward — the confirmed
        // "keeps working after Stop" bug), but the SENDER still settles idle so
        // `await_both_idle` sees the interrupt land instead of escalating to a
        // SIGKILL on an already-stopped agent. The session then derives Paused.
        // After the latch clears, FlushHeld delivers the held forward EXACTLY
        // once — regardless of how a FlushHeld sent while still gated races the
        // unpause (it either no-ops on a closed gate or performs the one flush;
        // the final delivered count is 1 either way, and the second FlushHeld
        // finds the queue empty).
        use crate::signaling::SignalingBridge;
        let (btx, _brx) = stub_input();
        let (rtx, mut rrx) = stub_input();
        let bridge = SignalingBridge::new();
        let tracker = ActivityTracker::new("s1", Arc::new(AtomicBool::new(false)), bridge);
        let mut d = deps(
            btx,
            Some(rtx),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(0)),
        );
        d.activity = Some(Arc::clone(&tracker));
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(run_router(d, rx));

        // Brian is mid-turn when the user clicks Stop (cancelling first, then
        // the pause latch — the ordering contract).
        tracker.set_busy(Author::Brian, true);
        tracker.set_cancelling(true);
        tracker.set_paused(true);
        // His dying turn's forward arrives at the gated router.
        tx.send(fwd(Author::Brian, "partial work before the stop"))
            .await
            .unwrap();
        // A FlushHeld racing in while (probably) still gated must not lose the
        // held forward or double-deliver it.
        tx.send(RouterCommand::FlushHeld).await.unwrap();
        // The held forward must settle the sender idle → cancelling auto-clears
        // → the session derives Paused. Poll (the router task runs async).
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1000);
        assert!(
            tracker.await_both_idle(deadline).await,
            "a held forward must still settle the sender idle"
        );
        assert_eq!(
            tracker.current(),
            crate::core::activity::SessionActivity::Paused,
            "after the interrupt settles the session derives Paused"
        );
        // User resumes: latch clears, flush releases the held forward.
        tracker.set_paused(false);
        tx.send(RouterCommand::FlushHeld).await.unwrap();
        drop(tx);
        task.await.unwrap();
        let mut delivered = 0;
        while rrx.try_recv().is_ok() {
            delivered += 1;
        }
        assert_eq!(
            delivered, 1,
            "the held forward reaches the peer exactly once, only after the latch clears"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_router_control_aborts_the_task() {
        // Explicit teardown: dropping the RouterControl (which happens whenever the
        // session handle is removed — close / evict / restart) must abort the
        // router task, so a rebuilt session can't leave an old router alive.
        let task = tokio::spawn(std::future::pending::<()>());
        let abort_handle = task.abort_handle();
        let rc = RouterControl {
            tx: mpsc::channel(1).0,
            convergence_reset: Arc::new(AtomicBool::new(false)),
            fwd_brian_to_rain: Arc::new(AtomicU64::new(0)),
            fwd_rain_to_brian: Arc::new(AtomicU64::new(0)),
            alive: Arc::new(AtomicBool::new(true)),
            task,
        };
        assert!(!abort_handle.is_finished(), "task runs before the drop");
        drop(rc); // RouterControl::Drop aborts the task.
        for _ in 0..50 {
            if abort_handle.is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            abort_handle.is_finished(),
            "dropping RouterControl must abort the router task"
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
