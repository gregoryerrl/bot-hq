//! Turn sequencer — the single loop that will replace today's reactive
//! per-agent tasks and `core::router`'s bilateral peer-forwarding.
//!
//! Exactly one participant holds the turn. When that turn ends the sequencer
//! picks the next active participant ([`Storage::next_active_participant`]),
//! hands it everything it has not read ([`Storage::unread_for_participant`]),
//! and waits. The cycle ends by consensus — every active participant has voted
//! done ([`Storage::all_active_voted_done`]) — or immediately, when a
//! participant parks a question for the user.
//!
//! **What is implemented is the ring advance and the delivery.** Consensus,
//! parked-question preemption and spin detection are later tasks and are NOT
//! here; [`SequencerCommand::Pause`] and [`SequencerCommand::Resume`] are still
//! accepted-and-logged. Nothing spawns this yet, so no session behaves
//! differently because it exists.
//!
//! ## What the storage helpers guarantee
//!
//! The skeleton was the first consumer of the `storage::participants` helpers
//! and found five defects in them. All five are fixed; what follows is the
//! contract this loop relies on, not a list of hazards it works around:
//!
//! - `unread_for_participant` excludes the participant's OWN rows, so a
//!   backlog is input rather than an echo. It is BOUNDED at
//!   [`UNREAD_BATCH_LIMIT`](crate::storage::UNREAD_BATCH_LIMIT) rows and
//!   returns a [`ChannelPage`](crate::storage::ChannelPage); see "how far a
//!   turn reads" below for what this loop does with `more`.
//! - `next_active_participant` takes the participant that HELD the turn, not
//!   its position, and steps by place in the rotation. Migration 0045 makes two
//!   active participants sharing a `turn_position` unrepresentable besides, so
//!   the ring reaches everyone and consensus is reachable.
//! - An empty rotation is DONE — `all_active_voted_done` is vacuously `true`
//!   there. The implication runs ONE WAY: no actives ⟹ done. The converse is
//!   false — with every active participant voted done, `all_active_voted_done`
//!   is `true` while `next_active_participant` still returns `Some`. So
//!   `is_none()` is not a consensus test, and this file does not use it as one:
//!   [`hand_over`] treats `None` as "nobody to wake", which is all it means.
//! - `commit_delivery` records the batch and moves the cursor in one
//!   transaction, so there is no way to advance a cursor past rows with no
//!   record of what was handed over.
//!
//! ## Delivery does not route around the session-scope check
//!
//! A receipt carries the session its row belongs to, and delivering it into
//! another session's agent wires one session's text into another's process. The
//! compare used to live on
//! [`SessionHandle::send_to_all`](crate::core::SessionHandle::send_to_all),
//! which was the only caller holding both ids — and that left two
//! receipt-carrying routes past it: `SessionAgent::deliver` and the three-hop
//! `agent.handle.input().deliver(&receipt)`. Receipt-gated is not scope-gated,
//! and those two were receipt-gated only.
//!
//! Both routes END at
//! [`ParticipantInput::deliver`](crate::agents::ParticipantInput::deliver), so
//! the compare moved down to that one point: the input now carries its session
//! id and checks every receipt against it. This loop holds
//! [`ParticipantInput`] clones and therefore inherits the check rather than
//! restating it — which is also why [`SequencerDeps`] carries inputs and not a
//! [`SessionHandle`](crate::core::SessionHandle). Handles live in
//! `AppState::sessions` behind a mutex and the sequencer's control side is
//! expected to sit ON a handle, so the task cannot own the thing that owns it.
//! Cloned stdin is what the router and the idle watchdog already hold for the
//! same reason.
//!
//! ## The forward ladder does not survive onto the turn path
//!
//! `router::route_forward` can drop a forward (convergence) or hold one (the
//! hard cap) AFTER its row is written, so today's chat can show a row that no
//! peer ever read. **This loop does not inherit that ladder.** Every row past a
//! participant's cursor is offered to it when its turn comes, and
//! [`Storage::commit_delivery`] records each one with no withheld reason.
//!
//! That is a consequence of the model, not a policy bolted onto it. The ladder
//! is a property of PUSHING: a forward is an interruption, so suppressing one
//! is how a volley is kept from running away. A turn is a PULL — a participant
//! reads the channel when the ring reaches it, and there is no volley to damp
//! because nobody speaks out of turn. Dropping a row there would buy nothing
//! and lose the reason the cursor exists.
//!
//! `withheld_reason` stays in `commit_delivery`'s signature because a POLICY
//! may withhold a row later; that is what the column is for. This path writes
//! `None` for every row, and nothing on it withholds today.
//!
//! ### What is recorded is the ENQUEUE, not the read
//!
//! An earlier draft of this section said "recorded but not delivered" ENDS
//! here. It does not, and the overstatement is worth naming because four later
//! tasks read this file as spec.
//! [`ParticipantInput::deliver`](crate::agents::ParticipantInput::deliver)
//! returns `true` once the row is in the participant's stdin channel — a
//! 64-slot buffer in front of the process — and this loop commits the delivery
//! on that. Three gaps follow, none of them closed here:
//!
//! - a row still sitting in that buffer when `agents::spawn::supervise` tears
//!   its incarnation down is discarded, and the cursor is already past it.
//!   Cursors do not rewind, so those rows are gone;
//! - a participant with no stdin at all gets nothing. This one IS handled:
//!   [`deliver_backlog`] commits only the prefix that actually reached the
//!   channel, so the cursor never moves past a row the agent did not get;
//! - a failed [`Storage::commit_delivery`] leaves the cursor behind rows that
//!   already went out, so the next turn hands them over a second time. The
//!   storage layer expects this — the delivery INSERT is `OR IGNORE` on
//!   `(participant_id, message_id)`, so the record is idempotent — but the
//!   participant reads the rows twice.
//!
//! So the claim this path supports is: it withholds nothing, and it records
//! what it handed to the transport. "The agent read it" is a stronger statement
//! than anything here establishes.
//!
//! ## A participant with no stdin freezes the cycle
//!
//! [`SequencerDeps::inputs`] can be missing a participant that is in the ring —
//! `SessionAgent::participant_id` is `None` whenever the roster read failed at
//! spawn. Handing that participant the turn delivers nothing, and no
//! [`SequencerCommand::TurnComplete`] can come back from a process that was
//! given no input, so **the cycle stops there**. Nothing in this file recovers
//! from that on its own: auto-advancing past an unreachable participant is a
//! recovery policy and belongs with spin detection.
//!
//! What is here is the way OUT of it that does not need a policy —
//! [`SequencerCommand::ParticipantJoined`] supplies the missing stdin, and if
//! the turn is already sitting on that participant its backlog is delivered
//! immediately. That is also the only way a participant invited AFTER the task
//! spawned can ever be reached, since [`run_sequencer`] owns its
//! [`SequencerDeps`] and nothing else can write to the map.
//!
//! Consensus (a later task) needs every active participant to vote done, and a
//! participant that never receives input never votes — so a ring member with no
//! stdin has to become reachable, not merely be stepped over.
//!
//! ## How far a turn reads
//!
//! A participant 500 rows behind gets 200 from one `unread_for_participant`.
//! **This loop drains before handing the turn over** rather than leaving the
//! rest for next time.
//!
//! Deferring would start a turn on stale context: the batch limit is a
//! transport bound — its own doc says so, and says a token budget belongs where
//! the model is known — so honouring it as a context policy would mean a
//! participant acting on rows 1–200 while the user's newest instruction sat in
//! 201–500, arriving a full lap of the ring later. Draining keeps the transport
//! bounded and the semantics simple: when your turn comes you have read
//! everything you had not read.
//!
//! The loop terminates on its own: a non-empty batch advances the cursor past
//! at least one row, and an empty batch reports `more == false`.
//! [`MAX_TURN_BATCHES`] is a liveness bound for the other case — a writer
//! appending faster than the drain — not the termination argument.
//!
//! ## The drain does not hold the command channel shut
//!
//! Draining is the longest thing this loop does: up to [`MAX_TURN_BATCHES`] ×
//! [`UNREAD_BATCH_LIMIT`](crate::storage::UNREAD_BATCH_LIMIT) writes into a
//! 64-slot stdin channel that PARKS when full. Awaited plainly, a participant
//! whose process has stopped reading would wedge the whole session's sequencer
//! inside one `deliver` with no way to reach it — session teardown included.
//!
//! So each row is written under a `select!` against the command channel, and
//! two things end a drain early:
//!
//! - the control channel CLOSING, which is session end. A teardown must not
//!   wait on a wedged agent's stdin;
//! - a [`SequencerCommand::UserMessage`], which resets the ring and therefore
//!   supersedes the turn being fed. This is the user's way out of a wedged
//!   participant, and it costs nothing correctness-wise: the rows that did not
//!   land stay past the cursor and are offered again when the ring returns.
//!
//! Every OTHER command is taken off the channel and deferred, so a sender never
//! parks behind a drain, and then handled in arrival order once the drain
//! finishes. Deferring rather than acting is what keeps "when your turn comes
//! you have read everything you had not read" true — acting on a `TurnComplete`
//! mid-drain would hand the turn over with rows undelivered, which is the
//! deferral this section rejects. Preemption proper (a parked question, a
//! pause) is tasks 7 and 9, and the `select!` is where they attach.

use crate::agents::ParticipantInput;
use crate::storage::{Participant, PersistedMessage, Storage};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// How many [`UNREAD_BATCH_LIMIT`](crate::storage::UNREAD_BATCH_LIMIT) batches
/// one turn will drain before handing over anyway.
///
/// Not the termination argument — see the module doc. This bounds the case
/// termination does not cover: a writer appending rows faster than the drain
/// consumes them would otherwise hold the turn open indefinitely.
///
/// 32 × 200 = 6,400 rows. **Measured 2026-08-07** against the live database:
/// the largest single channel holds 4,719 rows, which is 74% of this cap — not
/// the comfortable margin the figure this doc carried first (3,585, measured
/// some weeks earlier) implied. Re-measure before treating the headroom as
/// real; the number moves, and four later tasks read this file as spec:
///
/// ```sql
/// SELECT MAX(c) FROM (SELECT COUNT(*) c FROM messages GROUP BY session_id);
/// ```
///
/// Reaching the cap is therefore no longer only "something is producing during
/// a turn". It is one long-lived session away, and what happens there is: the
/// drain stops, the turn is handed over, and the remainder arrives on this
/// participant's next turn — the deferral the module doc rejects as a policy,
/// used here only as a backstop, with a `warn!` to say it happened.
///
/// `the_batch_cap_hands_over_with_the_remainder_still_past_the_cursor` pins
/// that, on a small `max_batches` rather than a 6,401-row fixture — see
/// [`deliver_backlog`]'s parameter.
const MAX_TURN_BATCHES: usize = 32;

/// What the sequencer task needs, cloned from the session's own state at spawn
/// — the same arrangement [`RouterDeps`](crate::core::RouterDeps) uses.
pub struct SequencerDeps {
    /// The session whose turn cycle this task runs. Every ring, cursor and
    /// consensus query is scoped by it.
    pub session_id: Arc<str>,
    /// The channel is the transport: rows, cursors, done votes and the roster
    /// all live in storage, so this is where a turn's context comes from.
    pub storage: Storage,
    /// Each live participant's stdin, keyed by `session_participants.id`.
    ///
    /// A clone per participant rather than a [`SessionHandle`] — see the
    /// scope-check section of the module doc for why the task cannot hold the
    /// handle, and why holding stdin still gets the session-scope compare.
    ///
    /// **The key is not checked against the input.** The scope compare inside
    /// `deliver` is on the SESSION, so filing participant A's stdin under
    /// participant B's id inside one session is silent: B's turn would be read
    /// by A, and every row would pass the check. Nothing here can catch that —
    /// a `ParticipantInput` carries a session id and no participant id — so it
    /// is a build-time obligation on whoever assembles this map.
    ///
    /// A participant in the ring but ABSENT from the map has no live process
    /// behind it: `SessionAgent::participant_id` is `None` whenever the roster
    /// read failed at spawn. [`deliver_backlog`] warns and delivers nothing,
    /// and it does not skip ahead in the ring — see "a participant with no
    /// stdin freezes the cycle" in the module doc for what that costs and for
    /// the way out, which is [`SequencerCommand::ParticipantJoined`].
    pub inputs: HashMap<i64, ParticipantInput>,
}

/// A wake for the sequencer.
///
/// **Bodies stay out of these.** A command says that something happened; WHAT
/// happened is a row in `messages`, and the sequencer reads rows from cursors.
/// Carrying the text in the command instead would put a second copy of it in
/// flight with no row identity behind it — the thing this batch's receipt work
/// removed.
///
/// [`ParticipantJoined`](Self::ParticipantJoined) carries a
/// [`ParticipantInput`] and is not an exception to that: an stdin is the
/// capability to write to a process, not a message, and it carries no text.
#[derive(Debug)]
pub enum SequencerCommand {
    /// The turn identified by `epoch` finished — advance the ring.
    ///
    /// **Both fields are the guard, and the epoch is the load-bearing one.**
    /// The sequencer stamps a fresh `epoch` on every handover
    /// ([`hand_over`]) and accepts a completion only when both fields match the
    /// turn in flight.
    ///
    /// `participant_id` alone is not enough, and the case it misses is the
    /// commonest one there is. A user message resets the ring to its first
    /// place while the previous holder is still mid-turn. When that holder IS
    /// the first place in the ring — the ordinary "user interjects while the
    /// first agent works" — the reset re-wakes the same participant, so the
    /// stale completion names the current holder, passes an identity check, and
    /// steps the ring off a participant that was woken half a second ago. Two
    /// agents on a turn at once, which is the one invariant this loop exists to
    /// keep. `a_completion_from_a_turn_the_user_restarted_is_discarded` is that
    /// case; with the epoch compare removed it fails.
    ///
    /// **There is no case running the other way**, so do not read "both fields"
    /// as a symmetry. For a sender that returns the epoch it was handed, the
    /// identity compare is redundant — the epoch already names one turn and one
    /// holder — and disabling it leaves every test here green. It is kept as
    /// defence against a malformed sender; see the comment on the guard itself.
    ///
    /// A user message is the reachable producer today. Pause/Resume adds a
    /// second once it is implemented, and a supervisor that respawns an agent
    /// mid-turn a third.
    ///
    /// **Where a sender gets the epoch is not solved here.** The sequencer
    /// mints it at handover, and it has to travel out with the turn and come
    /// back on the completion. Nothing spawns this loop yet, so nothing carries
    /// it yet; the tests below are the only senders, and wiring the round trip
    /// belongs with the task that spawns the loop.
    TurnComplete { participant_id: i64, epoch: u64 },
    /// The user posted to the channel. Resets the cycle to the first active
    /// participant and hands it the turn.
    ///
    /// Also the one command that cuts a drain short — see "the drain does not
    /// hold the command channel shut" in the module doc.
    UserMessage,
    /// A participant's stdin, arriving after the task was spawned.
    ///
    /// The map in [`SequencerDeps`] is owned by [`run_sequencer`], so this is
    /// the only way to add to it: a participant invited mid-session, or one
    /// whose roster read failed at spawn and left it unreachable. Replaces any
    /// existing entry for the id, which is what a respawn needs.
    ///
    /// If the turn is already sitting on this participant, its backlog goes out
    /// on arrival — the turn was handed to it and could not be delivered, and
    /// this is when it becomes deliverable. The ring does NOT move: no turn
    /// ended.
    ParticipantJoined {
        participant_id: i64,
        input: ParticipantInput,
    },
    /// Stop: hold the cycle where it stands, hand out no further turns.
    ///
    /// Still a no-op. Implementing it is a later task; what is in place for it
    /// is [`TurnComplete`](Self::TurnComplete)'s epoch, without which a turn
    /// finishing during a pause would advance the ring on resume. Note that a
    /// pause cannot yet cut a drain short either — only a user message and the
    /// channel closing do — so task 9 attaches to the same `select!`.
    Pause,
    /// Release a [`Pause`](Self::Pause) and continue the cycle. Still a no-op.
    Resume,
}

/// Run the turn sequencer for one session.
///
/// Returns when `rx` closes — i.e. when the last sender is dropped, which is
/// session end. That is the same exit `run_router` has, and it is how
/// router-inventory behaviour #20 survives the handover: a session teardown
/// must end the task, not leave it holding a session's state alive.
///
/// #20's other half — `RouterControl::drop` aborting the task outright — has no
/// counterpart here yet, because nothing holds a sequencer handle to drop. It
/// belongs with the control struct that wires this into a session.
pub async fn run_sequencer(mut deps: SequencerDeps, mut rx: mpsc::Receiver<SequencerCommand>) {
    debug!(session = %deps.session_id, "sequencer: started");
    // The turn in flight. `None` is "the cycle has not started", which is also
    // what `next_active_participant` reads as "reset to the front".
    let mut holder: Option<Participant> = None;
    // Which turn that is. Bumped by every ring step, so a completion minted
    // before the step cannot be mistaken for one minted after it — including
    // when the step lands on the same participant. See `TurnComplete`.
    let mut epoch: u64 = 0;
    // Commands a drain took off `rx` without acting on them. Drained BEFORE
    // `recv`, so arrival order is preserved end to end.
    let mut deferred: VecDeque<SequencerCommand> = VecDeque::new();
    loop {
        let cmd = match deferred.pop_front() {
            Some(cmd) => cmd,
            None => match rx.recv().await {
                Some(cmd) => cmd,
                None => break,
            },
        };
        match cmd {
            SequencerCommand::TurnComplete {
                participant_id,
                epoch: completed,
            } => {
                // The completion has to name the turn in flight: the same
                // participant AND the same turn.
                //
                // **The two halves are not symmetric.** The epoch is what
                // separates two turns — see the variant doc for the case that
                // `participant_id` alone lets through, and
                // `a_completion_from_a_turn_the_user_restarted_is_discarded`,
                // which fails without it. There is no mirror case: an epoch
                // names exactly one turn and a turn has exactly one holder, so
                // for a sender that returns the epoch it was handed the identity
                // compare is redundant — mutating it to `true` leaves the whole
                // suite green, and no test here pins it.
                //
                // It stays as defence against a MALFORMED sender: one that
                // echoes a live epoch back under the wrong participant id (a
                // crossed round trip, a copied field). Nothing in this file mints
                // the epochs senders will carry, so "well-formed" is an
                // assumption about code that is not written yet, and this is the
                // cheap half of the guard to keep.
                let live = completed == epoch
                    && holder.as_ref().is_some_and(|h| h.id == participant_id);
                if live {
                    advance_turn(&deps, &mut rx, &mut holder, &mut epoch, &mut deferred, false)
                        .await;
                } else {
                    debug!(
                        session = %deps.session_id,
                        participant_id,
                        completed,
                        epoch,
                        holder = ?holder.as_ref().map(|h| h.id),
                        "sequencer: completion does not name the turn in flight; discarded"
                    );
                }
            }
            SequencerCommand::UserMessage => {
                // The user speaking resets the cycle to the front of the
                // rotation, whoever held the turn — `None` is what
                // `next_active_participant` reads as "reset". The previous
                // holder's turn is not cancelled; nothing here can stop it. What
                // happens instead is that the epoch moves, so its completion is
                // discarded when it arrives.
                advance_turn(&deps, &mut rx, &mut holder, &mut epoch, &mut deferred, true).await;
            }
            SequencerCommand::ParticipantJoined {
                participant_id,
                input,
            } => {
                let replaced = deps.inputs.insert(participant_id, input).is_some();
                debug!(
                    session = %deps.session_id,
                    participant_id,
                    replaced,
                    "sequencer: participant stdin registered"
                );
                // The turn may already be sitting on this participant, unable to
                // be delivered — that is the frozen cycle the module doc
                // describes. It is deliverable now. The ring does not move and
                // the epoch does not change: no turn ended, one finally started.
                if let Some(to) = holder.as_ref().filter(|h| h.id == participant_id) {
                    deliver_backlog(&deps, to, &mut rx, MAX_TURN_BATCHES, &mut deferred).await;
                }
            }
            SequencerCommand::Pause => {
                debug!(session = %deps.session_id, "sequencer: Pause (no-op)");
            }
            SequencerCommand::Resume => {
                debug!(session = %deps.session_id, "sequencer: Resume (no-op)");
            }
        }
    }
    debug!(session = %deps.session_id, "sequencer: control channel closed; exiting");
}

/// Step the ring, stamp the new turn, and deliver its backlog.
///
/// `reset` is a user message: the ring goes back to its first place instead of
/// one past the current holder.
///
/// Takes `reset` rather than the current participant because `holder` is behind
/// a `&mut` here — the caller cannot lend it out and have it written back in
/// the same call.
async fn advance_turn(
    deps: &SequencerDeps,
    rx: &mut mpsc::Receiver<SequencerCommand>,
    holder: &mut Option<Participant>,
    epoch: &mut u64,
    deferred: &mut VecDeque<SequencerCommand>,
    reset: bool,
) {
    let current = if reset { None } else { holder.as_ref() };
    match hand_over(deps, current).await {
        // The ring could not be read. Keeping the holder AND the epoch is what
        // makes the retry in `hand_over`'s comment real: the same holder's
        // completion still matches, so it re-attempts the step. Overwriting
        // `holder` with `None` here instead would strand the cycle — every
        // later completion would fail the guard above, and nothing but another
        // user message would ever move it again.
        Handover::Held => {}
        Handover::To(next) => {
            *holder = next;
            // Every step, including a reset that lands on the same participant.
            // That case is exactly why the epoch exists.
            *epoch += 1;
            if let Some(to) = holder.as_ref() {
                deliver_backlog(deps, to, rx, MAX_TURN_BATCHES, deferred).await;
            }
        }
    }
}

/// Where a ring step landed.
enum Handover {
    /// The turn moved. `None` inside means nobody is active — "nobody to wake",
    /// and NOT a consensus test; see the module doc.
    To(Option<Participant>),
    /// The rotation could not be read, so the turn stays exactly where it was —
    /// holder and epoch both. A separate variant rather than echoing the
    /// current holder back, because the caller cannot tell those apart on the
    /// reset path: a user message passes `None` as the current holder, so
    /// "unchanged" and "reset to nobody" would be the same value.
    Held,
}

/// Step the ring past `current`. Delivery is the caller's next move, not this
/// function's, so a failed step cannot half-deliver.
///
/// `current == None` resets to the front of the rotation, which is what a user
/// message does.
async fn hand_over(deps: &SequencerDeps, current: Option<&Participant>) -> Handover {
    let next = match deps
        .storage
        .next_active_participant(&deps.session_id, current)
        .await
    {
        Ok(next) => next,
        // The ring is a roster read, so a failure here is a storage problem,
        // not an empty rotation. Holding the turn where it is keeps the two
        // apart: a later completion from the same holder retries the step,
        // whereas reporting "nobody is active" would reset the cycle on the
        // next user message. `Held` is what makes that retry reachable — see
        // `advance_turn`, which leaves the epoch alone for it.
        Err(e) => {
            warn!(
                session = %deps.session_id,
                error = %e,
                "sequencer: ring read failed; holding the turn where it is"
            );
            return Handover::Held;
        }
    };
    if next.is_none() {
        // Nobody active. NOT a consensus test — see the module doc; consensus
        // is `all_active_voted_done` and is a later task.
        debug!(
            session = %deps.session_id,
            "sequencer: no active participant to hand the turn to"
        );
    }
    Handover::To(next)
}

/// Why a drain stopped before the end of its page.
enum Stop {
    /// A user message arrived: the ring is about to reset, so the turn being
    /// fed is superseded. Already pushed onto the deferred queue.
    Superseded,
    /// The control channel closed — session end.
    SessionEnd,
    /// `deliver` returned `false`.
    Unreachable,
}

/// Hand `to` everything it has not read, and record what it got.
///
/// Drains rather than delivering one batch — see "how far a turn reads" in the
/// module doc.
/// `max_batches` is [`MAX_TURN_BATCHES`] on every production path; it is a
/// parameter so the cap's own behaviour can be exercised without a 6,401-row
/// fixture. The caller that does that is
/// `the_batch_cap_hands_over_with_the_remainder_still_past_the_cursor`, which
/// calls this function directly — the loop above has no way to pass anything
/// but the constant.
///
/// Commands that arrive mid-drain go onto `deferred` — see "the drain does not
/// hold the command channel shut" in the module doc for which two end the drain
/// and which are merely set aside.
async fn deliver_backlog(
    deps: &SequencerDeps,
    to: &Participant,
    rx: &mut mpsc::Receiver<SequencerCommand>,
    max_batches: usize,
    deferred: &mut VecDeque<SequencerCommand>,
) {
    let Some(input) = deps.inputs.get(&to.id) else {
        warn!(
            session = %deps.session_id,
            participant_id = to.id,
            slug = %to.slug,
            "sequencer: the participant holding the turn has no stdin; delivering nothing \
             and the cycle stops here until one arrives"
        );
        return;
    };
    for _ in 0..max_batches {
        let page = match deps.storage.unread_for_participant(to.id).await {
            Ok(page) => page,
            Err(e) => {
                warn!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    error = %e,
                    "sequencer: backlog read failed"
                );
                return;
            }
        };
        if page.rows.is_empty() {
            return;
        }
        let mut landed: Vec<(i64, Option<&str>)> = Vec::with_capacity(page.rows.len());
        let mut stop: Option<Stop> = None;
        'rows: for row in &page.rows {
            // `from_row` is what makes a row READ BACK deliverable: receipts are
            // otherwise minted only by the INSERT, and every row written before
            // a restart is only ever available this way. Built once per row
            // rather than inside the retry below, because it clones the body.
            let receipt = PersistedMessage::from_row(row);
            loop {
                tokio::select! {
                    // Commands first. Both futures here are cancel-safe —
                    // `recv` by documentation, and a dropped `Sender::send`
                    // enqueues nothing — so the losing branch costs at most a
                    // re-attempt of the same row, never a half-written one.
                    // Biased so a command already waiting always wins: the
                    // whole point is that a full stdin cannot hide it.
                    biased;
                    cmd = rx.recv() => match cmd {
                        Some(cmd @ SequencerCommand::UserMessage) => {
                            deferred.push_back(cmd);
                            stop = Some(Stop::Superseded);
                            break 'rows;
                        }
                        // Set aside and re-attempt this row. Deferring rather
                        // than acting is what keeps the drain-before-handover
                        // rule true.
                        Some(cmd) => deferred.push_back(cmd),
                        None => {
                            stop = Some(Stop::SessionEnd);
                            break 'rows;
                        }
                    },
                    landed_ok = input.deliver(&receipt) => {
                        if !landed_ok {
                            stop = Some(Stop::Unreachable);
                            break 'rows;
                        }
                        // `None` = delivered. Nothing on the turn path
                        // withholds; see the module doc.
                        landed.push((row.id, None));
                        break;
                    }
                }
            }
        }
        // Committing only the PREFIX that landed is what keeps the cursor from
        // outrunning the transport. It moves to the highest id in whatever is
        // passed here and never rewinds, so committing the whole page after a
        // short write would lose the tail forever.
        if let Err(e) = deps.storage.commit_delivery(to.id, &landed).await {
            warn!(
                session = %deps.session_id,
                participant_id = to.id,
                error = %e,
                "sequencer: delivery not recorded; the batch will be re-offered"
            );
            return;
        }
        match stop {
            None => {}
            Some(Stop::Superseded) => {
                debug!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    delivered = landed.len(),
                    of = page.rows.len(),
                    "sequencer: a user message superseded this turn mid-drain"
                );
                return;
            }
            Some(Stop::SessionEnd) => {
                debug!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    delivered = landed.len(),
                    of = page.rows.len(),
                    "sequencer: session ended mid-drain"
                );
                return;
            }
            Some(Stop::Unreachable) => {
                // `deliver` returns `false` for two unrelated reasons — a dead
                // input pump, and a receipt from another session — and this
                // warning named only the first for a while, so a routing bug
                // would have read as a dead pipe. `is_closed` separates them.
                //
                // It is a second look, not the same observation: the channel
                // can close between the refusal and this check, which would
                // report a scope refusal as a closed pipe. That direction is
                // harmless; the reverse cannot happen, because a closed sender
                // never re-opens.
                if input.is_closed() {
                    warn!(
                        session = %deps.session_id,
                        participant_id = to.id,
                        slug = %to.slug,
                        delivered = landed.len(),
                        of = page.rows.len(),
                        "sequencer: stdin closed mid-batch; the rest stays past the cursor"
                    );
                } else {
                    warn!(
                        session = %deps.session_id,
                        participant_id = to.id,
                        slug = %to.slug,
                        delivered = landed.len(),
                        of = page.rows.len(),
                        "sequencer: a row was refused mid-batch with stdin still open — the \
                         receipt is out of this participant's session scope"
                    );
                }
                return;
            }
        }
        if !page.more {
            return;
        }
    }
    warn!(
        session = %deps.session_id,
        participant_id = to.id,
        batches = max_batches,
        "sequencer: backlog still not drained at the batch cap; the rest waits for the next turn"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::OutgoingUserMessage;
    use crate::storage::{MessageKind, UNREAD_BATCH_LIMIT};
    use std::time::Duration;
    use tokio::task::JoinHandle;

    /// Every await in this file carries this deadline.
    ///
    /// Not stylistic. A bare `.await` on an event that stopped arriving turns a
    /// deleted emit into a hung test instead of a failing one — earlier in this
    /// batch exactly that shape cost seven minutes per run before anything was
    /// printed. Two seconds is orders of magnitude above what an in-process
    /// `recv()` needs.
    const DEADLINE: Duration = Duration::from_secs(2);

    /// How long a negative assertion waits before believing the silence.
    /// Short — it is paid on every `quiet()` call — but well past the
    /// in-process delivery these tests measure in microseconds.
    const QUIET: Duration = Duration::from_millis(250);

    /// The default stubbed stdin buffer.
    ///
    /// Sized above the largest backlog these tests post (201 rows) so that a
    /// test about ring order is not also a test about back-pressure. That is a
    /// convenience, not an assumption the loop makes: production stdin is 64
    /// slots, and a drain that outruns it PARKS inside `deliver`.
    ///
    /// [`ring_sized`] is how a test opts out of the convenience.
    /// `a_backlog_larger_than_the_stdin_buffer_lands_in_full` is the one that
    /// does, and it is the only coverage of the parking path: every other test
    /// here runs with more slots than it posts rows, so none of them would
    /// notice a drain that dropped a row instead of waiting for a slot.
    const STDIN_CAPACITY: usize = 512;

    /// One stubbed participant: its roster id and the stdin a test reads.
    struct Seat {
        id: i64,
        rx: mpsc::Receiver<OutgoingUserMessage>,
    }

    impl Seat {
        /// Everything on this stdin right now.
        ///
        /// `try_recv`, not `recv().await`: a test only calls this once the
        /// sequencer task has exited, so anything not already queued is
        /// something that was never sent. Awaiting for it would hang where
        /// this fails.
        fn drain(&mut self) -> Vec<String> {
            let mut out = Vec::new();
            while let Ok(m) = self.rx.try_recv() {
                out.push(m.message.content);
            }
            out
        }

        /// One [`QUIET`] window of silence, or the wire that broke it.
        ///
        /// The shared body of [`quiet`](Self::quiet) and the tail of
        /// [`expect`](Self::expect) — the two negative assertions in this file
        /// differ only in what they say when they fail.
        async fn extra_wire(&mut self) -> Option<String> {
            match tokio::time::timeout(QUIET, self.rx.recv()).await {
                Ok(Some(m)) => Some(m.message.content),
                // Elapsed, or the sender was dropped. Both are silence.
                _ => None,
            }
        }

        /// Assert nothing arrives within a bounded window, **with the control
        /// channel still open**.
        ///
        /// `drain()` cannot express this and the difference is load-bearing.
        /// `drain` is only valid after the task exits, and closing the control
        /// channel is itself what aborts an in-flight drain — the select is
        /// biased on commands, so a closed `rx` wins every iteration. A test
        /// that drops `tx` and then finds an empty seat therefore cannot tell
        /// "the sequencer refused to wake this participant" from "the wake was
        /// cut short by the close", and a guard asserted that way passes with
        /// the guard removed. Waiting while the channel is open is what makes
        /// the silence mean something.
        async fn quiet(&mut self) {
            if let Some(w) = self.extra_wire().await {
                panic!("expected no wire, got {w:?}");
            }
        }

        /// Wait for exactly `n` wires, then assert nothing else is queued.
        ///
        /// A synchronisation point: it returns only once the sequencer has
        /// finished the delivery, which is what lets a test post a NEW row
        /// between two commands and know which turn will read it.
        ///
        /// **Both halves are asserted here, and the second one is why this is
        /// not just a bounded `recv` loop.** For a while it was: the body read
        /// `n` wires and returned, so `expect(n)` caught UNDER-delivery only
        /// while the doc claimed both. That is not a hypothetical gap — four
        /// tests below were moved off `drain()` (which compares whole contents)
        /// onto `expect(n)`, and appending one duplicate wire to the end of
        /// every drain left all four green:
        /// `a_completed_turn_wakes_exactly_one_participant`,
        /// `an_observer_is_skipped_not_given_a_no_op_turn`,
        /// `a_participant_with_no_stdin_holds_the_turn_rather_than_losing_its_rows`
        /// and `a_backlog_past_the_batch_limit_is_drained_before_the_turn_is_handed_over`.
        /// With the [`QUIET`] window below, that same duplicate fails all four.
        ///
        /// The cost is one `QUIET` per call, paid on the happy path. That is
        /// the price of a negative assertion — [`quiet`](Self::quiet) pays it
        /// too — and it buys the half of this contract that was being asserted
        /// nowhere.
        async fn expect(&mut self, n: usize) -> Vec<String> {
            let mut out = Vec::new();
            for i in 0..n {
                let m = tokio::time::timeout(DEADLINE, self.rx.recv())
                    .await
                    .unwrap_or_else(|_| panic!("wire {} of {n} never arrived", i + 1))
                    .expect("the sequencer dropped this participant's stdin");
                out.push(m.message.content);
            }
            if let Some(w) = self.extra_wire().await {
                panic!("expected exactly {n} wires, then {w:?} arrived as well");
            }
            out
        }
    }

    /// A session whose rotation is `roster` — `(slug, participation_mode)` in
    /// turn-position order — with one [`STDIN_CAPACITY`]-slot stdin per
    /// participant.
    ///
    /// Returns the deps (moved into the task), a clone of the storage the test
    /// posts and asserts with, and one [`Seat`] per roster entry.
    async fn ring(roster: &[(&str, &str)]) -> (SequencerDeps, Storage, Vec<Seat>) {
        ring_sized(roster, STDIN_CAPACITY).await
    }

    /// [`ring`], with the stdin buffer named rather than defaulted.
    ///
    /// For the tests that want a drain to RUN OUT of buffer: pass a capacity
    /// below the number of rows the test posts and `deliver` parks mid-drain,
    /// the way production's 64 slots do behind a slow child.
    async fn ring_sized(
        roster: &[(&str, &str)],
        stdin_capacity: usize,
    ) -> (SequencerDeps, Storage, Vec<Seat>) {
        let storage = Storage::memory().await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();
        let mut inputs = HashMap::new();
        let mut seats = Vec::new();
        for (position, (slug, mode)) in roster.iter().enumerate() {
            let id = storage
                .insert_participant("s1", slug, slug, None, None, "[]", mode, position as i64)
                .await
                .unwrap();
            let (tx, rx) = mpsc::channel(stdin_capacity);
            inputs.insert(id, ParticipantInput::new("s1", tx));
            seats.push(Seat { id, rx });
        }
        let deps = SequencerDeps {
            session_id: "s1".into(),
            storage: storage.clone(),
            inputs,
        };
        (deps, storage, seats)
    }

    /// A stdin that arrives AFTER the task was spawned, as
    /// [`SequencerCommand::ParticipantJoined`] carries one: the input to send
    /// and the seat that reads it.
    ///
    /// Deliberately not built by [`ring`] — the point of these cases is a
    /// participant the deps map does not have (or no longer has), so the
    /// channel has to be made outside it.
    fn late_stdin(id: i64) -> (ParticipantInput, Seat) {
        let (tx, rx) = mpsc::channel(STDIN_CAPACITY);
        (ParticipantInput::new("s1", tx), Seat { id, rx })
    }

    async fn post(storage: &Storage, origin: &str, slug: Option<&str>, body: &str) {
        storage
            .post_to_channel("s1", origin, slug, MessageKind::Text.as_str(), body, None)
            .await
            .unwrap();
    }

    /// `send` with a deadline. A bounded channel's `send` parks once the buffer
    /// fills, so a sequencer that stopped draining would hang the caller rather
    /// than fail it — the same trap [`exited`] avoids on the other end.
    async fn send(tx: &mpsc::Sender<SequencerCommand>, cmd: SequencerCommand) {
        tokio::time::timeout(DEADLINE, tx.send(cmd))
            .await
            .expect("sequencer stopped draining its command channel")
            .expect("sequencer dropped its receiver while a sender was live");
    }

    /// Did the task END within [`DEADLINE`]? A panic inside the loop also ends
    /// the task, so the join result is unwrapped rather than counted as an exit
    /// — an unwritten `match` arm must fail here, not read as a clean shutdown.
    async fn exited(task: JoinHandle<()>) -> bool {
        match tokio::time::timeout(DEADLINE, task).await {
            Ok(joined) => {
                joined.expect("sequencer task panicked");
                true
            }
            Err(_) => false,
        }
    }

    fn nothing() -> Vec<String> {
        Vec::new()
    }

    #[tokio::test]
    async fn the_sequencer_exits_when_its_control_channel_closes() {
        // Router-inventory #20, carried forward: the task must end when its
        // last sender goes (session end), not linger holding a session's worth
        // of state alive.
        let (deps, _storage, _seats) = ring(&[("a", "active")]).await;
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        drop(tx);
        assert!(
            exited(task).await,
            "sequencer must exit on session end, not linger"
        );
    }

    #[tokio::test]
    async fn a_completed_turn_wakes_exactly_one_participant() {
        let (deps, storage, mut seats) =
            ring(&[("a", "active"), ("b", "active"), ("c", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "user one").await;
        post(&storage, "system", None, "host note").await;
        post(&storage, "participant", Some("a"), "a's last turn").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // The user message resets the ring to its first place, so A takes the
        // turn; then A finishes it. Each wake is awaited before the next command
        // goes in: the drain selects commands first and biased, so a closed
        // control channel wins every iteration and would stop the drain before a
        // row landed.
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(
            seats[0].expect(2).await,
            vec!["user one", "host note"],
            "A read the channel, but not its own last turn back as fresh input"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
            },
        )
        .await;
        assert_eq!(
            seats[1].expect(3).await,
            vec!["user one", "host note", "a's last turn"],
            "the turn steps to the next PLACE in the rotation, carrying every unread row"
        );
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[2].drain(),
            nothing(),
            "one completed turn wakes ONE participant, not the rotation"
        );
    }

    #[tokio::test]
    async fn an_observer_is_skipped_not_given_a_no_op_turn() {
        // A wake that cannot produce output is pure waste, so the ring filters
        // observers out rather than handing them a turn they end immediately.
        let (deps, storage, mut seats) = ring(&[
            ("a", "active"),
            ("watcher", "observer"),
            ("b", "active"),
        ])
        .await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // Await each wake before sending the next command. The drain selects
        // commands FIRST and biased, so a control channel that is already closed
        // wins every iteration and stops the drain before a row lands — firing
        // everything up front and dropping `tx` would assert against a delivery
        // the sequencer correctly refused to make.
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
            },
        )
        .await;
        assert_eq!(
            seats[2].expect(1).await,
            vec!["go"],
            "the turn steps OVER the observer to the next active participant"
        );
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[1].drain(),
            nothing(),
            "the observer sits between A and B in the rotation and must not be woken"
        );
    }

    #[tokio::test]
    async fn a_completion_from_a_superseded_turn_does_not_advance_the_ring() {
        // The hazard `TurnComplete`'s participant id exists for. A user message
        // resets the ring while the previous holder is still mid-turn; that
        // turn's completion then arrives BEHIND the reset. Payload-free it
        // would advance the ring off the participant the reset just woke, so
        // two agents would hold a turn at once.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "r1").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["r1"]);
        send(&tx, SequencerCommand::TurnComplete { participant_id: a, epoch: 1 }).await;
        assert_eq!(seats[1].expect(1).await, vec!["r1"], "B now holds the turn");

        // The user speaks over B's turn. Waiting on A's wake is what makes the
        // next line's ordering a fact rather than a race.
        post(&storage, "user", None, "r2").await;
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["r2"], "the ring reset to A");

        // B's turn ends, late.
        send(&tx, SequencerCommand::TurnComplete { participant_id: b, epoch: 2 }).await;
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[1].drain(),
            nothing(),
            "the superseded completion advanced nothing: B was not re-woken behind A"
        );
        assert_eq!(seats[0].drain(), nothing(), "and A was not woken twice");
    }

    #[tokio::test]
    async fn a_completion_from_a_turn_the_user_restarted_is_discarded() {
        // The case `participant_id` alone CANNOT catch, and the commonest one
        // there is: the user interjects while the FIRST participant is mid-turn,
        // so the reset re-wakes that same participant. The stale completion then
        // names the live holder and passes an identity check — stepping the ring
        // off a participant woken moments ago, which puts two agents on a turn at
        // once. Only the epoch separates the two turns.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "r1").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["r1"]);

        // The user speaks over A's own turn. The ring resets to its first place,
        // which IS A — same participant, new turn. Waiting on the wake is what
        // makes the ordering below a fact rather than a race.
        post(&storage, "user", None, "r2").await;
        send(&tx, SequencerCommand::UserMessage).await; // epoch 2, A again
        assert_eq!(
            seats[0].expect(1).await,
            vec!["r2"],
            "the reset re-woke the SAME participant — the whole point of this case"
        );

        // A's first turn ends, late, carrying the epoch it was handed.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
            },
        )
        .await;
        // Proven with the channel still OPEN. Dropping `tx` here instead would
        // abort any delivery the guard wrongly allowed, and an empty seat would
        // then prove nothing — this test passed with the epoch compare removed
        // until it was written this way.
        seats[1].quiet().await;

        // And the live turn is untouched: completing THAT epoch does advance,
        // so the silence above was the guard working rather than the ring being
        // stuck.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 2,
            },
        )
        .await;
        assert_eq!(
            seats[1].expect(2).await,
            vec!["r1", "r2"],
            "the live completion steps the ring the stale one could not"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_backlog_past_the_batch_limit_is_drained_before_the_turn_is_handed_over() {
        // `ChannelPage::more` is a normal outcome, not an edge case — 268 of
        // 384 live sessions held more rows than one batch when this was measured
        // (2026-08-10). The figure drifts; the shape does not, and it only ever
        // drifts upward. Leaving the rest for next time would start a turn on
        // context the participant is already known to be missing, and the newest
        // rows would arrive a full lap of the ring later.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        let overflow = UNREAD_BATCH_LIMIT as usize + 1;
        for i in 0..overflow {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        assert!(
            storage.unread_for_participant(b).await.unwrap().more,
            "the fixture has to outgrow one batch or this test proves nothing"
        );

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // Each wake awaited before the next command: the drain selects commands
        // first and biased, so a closed control channel would stop it mid-batch.
        send(&tx, SequencerCommand::UserMessage).await;
        let _ = seats[0].expect(overflow).await;
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
            },
        )
        .await;
        let got = seats[1].expect(overflow).await;
        drop(tx);
        assert!(exited(task).await);

        // The COUNT is asserted by `expect(overflow)` itself and is not restated
        // here: short of `overflow` it times out, past it the quiescence window
        // fires. An `assert_eq!(got.len(), overflow)` at this point held for a
        // while and was tautological — `expect(n)` returns exactly `n` by
        // construction — so it read as a check on the sequencer while testing
        // the helper's `Vec::push`. What is left is what `expect` does NOT pin:
        // the rows are the right ones, in order, spanning both batches.
        assert_eq!(got.first().map(String::as_str), Some("row 0"));
        assert_eq!(
            got.last().map(String::as_str),
            Some(format!("row {}", overflow - 1).as_str())
        );
        // And the delivery was RECORDED, not just written: the cursor sits on
        // the last row, so nothing is re-offered next turn.
        assert!(
            storage
                .unread_for_participant(b)
                .await
                .unwrap()
                .rows
                .is_empty(),
            "the cursor moved with the batches"
        );
        assert!(
            storage.withheld_for_participant(b).await.unwrap().is_empty(),
            "nothing on the turn path is withheld"
        );
    }

    #[tokio::test]
    async fn a_participant_with_no_stdin_holds_the_turn_rather_than_losing_its_rows() {
        // A spawned agent whose roster read failed has no `participant_id`, so
        // it has no entry in `inputs`. Its cursor must not move: a cursor never
        // rewinds, so advancing it here would drop those rows permanently.
        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        deps.inputs.remove(&b);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // A's wake is awaited before the completion goes in — the drain selects
        // commands first and biased, so a closed control channel would stop the
        // drain and this test's "delivery is live" anchor would be the thing
        // that broke, not B's missing stdin.
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "delivery is live in this run"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
            },
        )
        .await;
        drop(tx);
        assert!(exited(task).await);

        // A's wire is asserted ABOVE, before the completion is sent, and it is
        // not decoration: without it this test passed for a whole session
        // against a sequencer that delivered NOTHING to anyone. B receiving
        // nothing has to mean "B has no stdin", and the only way to say that is
        // to show the same run delivering to a seat that has one.
        assert_eq!(seats[1].drain(), nothing());
        assert_eq!(
            storage.cursor_for(b).await.unwrap(),
            0,
            "an undelivered backlog stays undelivered — the cursor did not move"
        );
    }

    #[tokio::test]
    async fn a_backlog_larger_than_the_stdin_buffer_lands_in_full() {
        // The only test here that lets a drain run out of buffer. Production
        // stdin is 64 slots and `deliver` PARKS when it fills, so a drain of any
        // real backlog parks and resumes repeatedly; every other test in this
        // file runs with more slots than it posts rows and would not notice a
        // drain that dropped a row rather than waiting for one.
        //
        // Two slots against eight rows: the drain cannot finish without the
        // reader freeing space three times over.
        let (deps, storage, mut seats) =
            ring_sized(&[("a", "active"), ("b", "active")], 2).await;
        let (a, b) = (seats[0].id, seats[1].id);
        for i in 0..8 {
            post(&storage, "user", None, &format!("row {i}")).await;
        }

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await;
        // `expect` is the reader: it drains the seat as the sequencer fills it,
        // so the parking and the unparking both happen inside this call.
        let want: Vec<String> = (0..8).map(|i| format!("row {i}")).collect();
        assert_eq!(
            seats[0].expect(8).await,
            want,
            "a full stdin delays a row; it does not lose one"
        );

        // And the loop was not wedged by the parking — the turn still hands over.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
            },
        )
        .await;
        assert_eq!(seats[1].expect(8).await, want);
        drop(tx);
        assert!(exited(task).await);

        for id in [a, b] {
            assert!(
                storage.unread_for_participant(id).await.unwrap().rows.is_empty(),
                "the cursor moved with the rows, not ahead of the ones that parked"
            );
        }
    }

    #[tokio::test]
    async fn the_batch_cap_hands_over_with_the_remainder_still_past_the_cursor() {
        // What `MAX_TURN_BATCHES` actually does at the cap. It is the backstop
        // for a writer appending faster than the drain consumes, and what it
        // does there IS the deferral the module doc rejects as a policy — so it
        // has to be pinned rather than described.
        //
        // Driven through `deliver_backlog` directly with a small `max_batches`.
        // That parameter exists for exactly this caller and had none: both
        // production sites pass the constant, and reaching the real cap needs
        // 6,401 rows.
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let cap = 2usize;
        let per_batch = UNREAD_BATCH_LIMIT as usize;
        let capped = cap * per_batch;
        for i in 0..capped + 1 {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        let holder = storage
            .next_active_participant("s1", None)
            .await
            .unwrap()
            .expect("the fixture's one active participant");
        assert_eq!(holder.id, a);

        // Held, not dropped: a closed command channel is `Stop::SessionEnd`
        // inside the drain, which would end it before the cap ever bit.
        let (_cmd_tx, mut rx) = mpsc::channel(8);
        let mut deferred = VecDeque::new();
        deliver_backlog(&deps, &holder, &mut rx, cap, &mut deferred).await;

        // Exactly `cap` batches went out — `expect` times out below that and
        // fails its quiescence window above it.
        let got = seats[0].expect(capped).await;
        assert_eq!(got.first().map(String::as_str), Some("row 0"));
        assert_eq!(
            got.last().map(String::as_str),
            Some(format!("row {}", capped - 1).as_str()),
            "the drain stopped at the cap, mid-backlog"
        );
        assert!(deferred.is_empty(), "nothing arrived to defer");

        // The half that matters: the cap DEFERS the remainder, it does not drop
        // it. The cursor sits where delivery stopped, so the rest is offered
        // again when the ring comes back round.
        let left = storage.unread_for_participant(a).await.unwrap();
        assert_eq!(
            left.rows.iter().map(|r| r.content.as_str()).collect::<Vec<_>>(),
            vec![format!("row {capped}").as_str()],
            "the remainder is still past the cursor"
        );
    }

    #[tokio::test]
    async fn a_participant_that_joins_while_holding_the_turn_gets_its_backlog_at_once() {
        // The way OUT of the frozen cycle: A was handed the turn with no stdin,
        // so nothing could be delivered and no completion can ever come back.
        // The stdin arriving is when that turn becomes deliverable — and it must
        // go out WITHOUT the ring moving, because no turn ended.
        let (mut deps, storage, mut seats) =
            ring(&[("a", "active"), ("b", "active"), ("c", "active")]).await;
        let a = seats[0].id;
        deps.inputs.remove(&a);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // No wire to await between these two: A has no stdin yet, so the frozen
        // state has nothing to observe. Ordering is the command channel's — the
        // join is handled after the reset because it was sent after it.
        send(&tx, SequencerCommand::UserMessage).await; // epoch 1, A holds, undeliverable
        let (input, mut joined) = late_stdin(a);
        send(
            &tx,
            SequencerCommand::ParticipantJoined {
                participant_id: a,
                input,
            },
        )
        .await;
        assert_eq!(
            joined.expect(1).await,
            vec!["go"],
            "the stdin arriving delivered the turn A was already holding"
        );

        // The EPOCH did not change: the turn in flight is still the one minted
        // at the reset, so the completion carrying epoch 1 is the live one. Had
        // the join stamped a new turn, this would name a stale epoch and be
        // discarded — and the wake below would never come.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
            },
        )
        .await;
        assert_eq!(
            seats[1].expect(1).await,
            vec!["go"],
            "the ring stepped one place, A→B, from where the join left it"
        );
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[2].drain(),
            nothing(),
            "and it did not step twice: C sits past B and was never woken"
        );
    }

    #[tokio::test]
    async fn a_participant_that_joins_without_the_turn_is_registered_but_not_woken() {
        // The other half of the arm's conditional. A join is a map insert, not a
        // wake: B has a backlog the whole time, and delivering it here would put
        // B and the holder on a turn at once.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "A holds the turn — and delivery is live in this run"
        );

        let (input, mut joined) = late_stdin(b);
        send(
            &tx,
            SequencerCommand::ParticipantJoined {
                participant_id: b,
                input,
            },
        )
        .await;
        // Asserted with the control channel still OPEN. Dropping `tx` first
        // would abort any delivery the arm wrongly made, and the empty seat
        // would then prove nothing.
        joined.quiet().await;

        // The insert DID take, though — B's turn, when it comes, is delivered on
        // the stdin that arrived. Without this the silence above would also be
        // what a dropped-on-the-floor join looks like.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
            },
        )
        .await;
        assert_eq!(joined.expect(1).await, vec!["go"]);
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_second_join_replaces_the_first_and_the_turn_follows_the_new_stdin() {
        // Replace semantics, which is what a RESPAWN needs: same participant id,
        // different process. An insert that kept the first entry would keep
        // writing into the dead incarnation's pipe — silently, since a dropped
        // receiver only shows up as `deliver` returning false much later.
        let (mut deps, storage, seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let a = seats[0].id;
        deps.inputs.remove(&a);
        post(&storage, "user", None, "first").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await; // epoch 1, A holds
        let (first_input, mut first_seat) = late_stdin(a);
        send(
            &tx,
            SequencerCommand::ParticipantJoined {
                participant_id: a,
                input: first_input,
            },
        )
        .await;
        assert_eq!(first_seat.expect(1).await, vec!["first"]);

        // A respawns mid-turn. Awaiting the wire above puts this row strictly
        // after the first delivery, so which stdin reads it is a fact rather
        // than a race.
        post(&storage, "user", None, "second").await;
        let (second_input, mut second_seat) = late_stdin(a);
        send(
            &tx,
            SequencerCommand::ParticipantJoined {
                participant_id: a,
                input: second_input,
            },
        )
        .await;
        assert_eq!(
            second_seat.expect(1).await,
            vec!["second"],
            "the backlog followed the stdin that arrived LAST"
        );
        // And nothing more went to the first one. Its sender was dropped by the
        // replace, but a queued wire would still be read back here.
        first_seat.quiet().await;
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn every_command_is_consumed_and_the_loop_comes_back_for_more() {
        // "No-op" has to mean the loop CONSUMED the command and looped, not
        // that the first one ended the task or panicked an arm nobody wrote.
        // Buffered sends are drained before `recv()` reports the close, so
        // reaching the exit is proof all five were handled.
        let (deps, _storage, seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let (joined_input, _joined_seat) = late_stdin(a);
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        for cmd in [
            SequencerCommand::TurnComplete { participant_id: a, epoch: 0 },
            SequencerCommand::UserMessage,
            SequencerCommand::ParticipantJoined { participant_id: a, input: joined_input },
            SequencerCommand::Pause,
            SequencerCommand::Resume,
        ] {
            send(&tx, cmd).await;
        }
        drop(tx);
        assert!(
            exited(task).await,
            "a command must not end the loop, and must not panic an arm"
        );
    }
}
