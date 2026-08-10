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
//! **What is implemented is the ring advance, the delivery and both halts —
//! consensus, and a parked question.** Spin detection is a later task and is NOT
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
//! Consensus needs every active participant to vote done, and a participant
//! that never receives input never votes — so a ring member with no stdin has
//! to become reachable, not merely be stepped over. That is now a live cost
//! rather than a future one: an unreachable ring member cannot vote, so the
//! tally can never complete, so the session can never halt by consensus.
//!
//! ## The halt is a yield, not a stop
//!
//! Every accepted completion carries a vote —
//! [`TurnComplete`](SequencerCommand::TurnComplete)'s `done` — and
//! [`halted_on_consensus`] records it and asks
//! [`Storage::all_active_voted_done`] BEFORE the ring is stepped. Arriving
//! means waking nobody, so a step taken first would have to be taken back.
//! **`all_active_voted_done` is the halt test and the only one**; the ring's
//! own `None` is "nobody to wake" and is not a substitute, as above.
//!
//! That paragraph is about the CONSENSUS halt, which is a tally. The other halt
//! reason — a parked question — is not, and has its own section below.
//!
//! Two things reset the tally, both of them substantive output: a completion
//! with `done: false`, and a [`UserMessage`](SequencerCommand::UserMessage).
//! The reset is session-wide, not per-participant, because a vote cast before
//! someone else spoke was a statement about a session that no longer exists —
//! left standing, one stale done and one fresh one add up to an arrival nobody
//! voted for, and the session halts with a participant never having read what
//! the other said.
//!
//! The second of those resets is bound to the RESTART, not to the command:
//! [`advance_turn`] empties the tally whenever it steps to the front of the
//! rotation. A user message is still the only way to the front, so the two are
//! the same event and no test can tell the SHAPES apart — but the parked-question
//! halt below made the binding earn its keep anyway. That halt leaves votes
//! standing (it touches none), so the cycle its release restarts is the one case
//! where the tally arriving at the front is non-empty for a reason other than
//! "the user spoke over a turn". `a_parked_question_halts_the_cycle_unilaterally`
//! is the test: it parks with one `done` standing and then requires the first
//! `done: true` of the restarted cycle to STEP the ring rather than complete a
//! tally of two.
//!
//! **Mechanically the halt is: no holder, no live epoch, and the loop goes back
//! to `recv`.** It emits nothing and marks nothing extra. Three consequences,
//! and the third is a gap:
//!
//! - the DURABLE record already exists — the votes are in
//!   `session_participants.done_vote`, so a host that wants to know whether a
//!   session arrived asks `all_active_voted_done` rather than watching for an
//!   event. Adding a second marker here would be a second copy of a fact that
//!   is already stored;
//! - the halt SURVIVES a repeat. `holder` is `None` and the epoch has moved, so
//!   a completion arriving afterwards cannot name the turn in flight — there is
//!   not one — and the cycle restarts only on a user message, which is what
//!   "yields to the user" means operationally.
//!   `the_cycle_halts_when_every_active_participant_votes_done` pins this with a
//!   late SUBSTANTIVE completion, because repeating the halting VOTE pins
//!   nothing: a loop that wrongly accepted that would record the same vote,
//!   find the same consensus and fall silent for the same reason. **The two
//!   clears are belt and braces, and the test is honest about it** — delete
//!   either one alone and the suite stays green, since the other half of the
//!   guard rejects unaided; delete both and that test fails;
//! - **nothing NOTIFIES the user.** A yield the user is not told about is a
//!   session that has gone quiet, and telling them needs a sink
//!   [`SequencerDeps`] does not have — it carries storage and stdins, no event
//!   emitter. Which sink that is (a `SessionActivity::AwaitingUser` transition,
//!   a tray row) is the host's contract, and guessing it here would be inventing
//!   the interface the task that spawns this loop has to define. That task owns
//!   it, along with the epoch round trip.
//!
//! ### The second halt reason: a parked question
//!
//! [`QuestionParked`](SequencerCommand::QuestionParked) halts the same way and
//! on different grounds. Consensus is an ARRIVAL — every active participant
//! agreed there is nothing left to do — so it is a tally and needs all of them.
//! A parked question is a YIELD by one: whoever is blocking on a human stops the
//! cycle regardless of what the others would have done, so it is not counted,
//! not guarded and not voted on. `a_parked_question_halts_the_cycle_unilaterally`
//! pins the difference — the ring stops with one participant's `done` standing
//! and the other's never cast, which no consensus test would ever produce.
//!
//! Mechanically it is the same two lines, in the same place: [`halt`]. So the
//! two reasons cannot drift apart, and everything the bullets above say holds
//! here too — no event, no marker, both halves of `TurnComplete`'s guard
//! rejecting afterwards, and nothing notifying the user.
//!
//! **Nothing releases it that does not also release a consensus halt**, and that
//! is a decision rather than an omission. The obvious trigger is the user
//! answering, and answering is not always a `UserMessage`:
//! `SignalingBridge::resolve_choice` hands the pick back in-band through the
//! agent's own MCP call and writes NO row; only its out-of-band fall-back posts
//! one (`origin = "user"`). A second command for "the user answered" would
//! nevertheless be handled identically to `UserMessage` — reset to the front,
//! clear the tally, hand out a turn — and two commands the loop cannot tell
//! apart are one command. The row-writing event is also the better trigger on
//! its merits: a restart wakes the front of the ring, and that wake is worth
//! taking only if something sits past the participant's cursor. The user's row
//! is what puts it there.
//!
//! The cost is named rather than papered over. An answer delivered in-band has
//! no row, so it has no command behind it today and the halt stands until the
//! user types something. That is the mirror of the gap below — there a row
//! arrives with no command; here an event arrives with no row — and it falls to
//! the same task, the one that wires this loop to a session.
//!
//! ### A row can arrive with no command behind it
//!
//! One more misfit, and it belongs to the command set rather than to consensus:
//! **nothing says "a row arrived"**, so a halt is broken only by a
//! `UserMessage`. That costs two things, and the second contradicts the design
//! rather than merely limiting it:
//!
//! - a row written by anything else — a host note, a tool result landing late —
//!   WAKES nobody;
//! - it resets NO VOTE either. "Any substantive output resets the tally" is
//!   therefore true only of output this loop is told about, and the gap is not
//!   theoretical: **a session can arrive at consensus with rows no participant
//!   has read.**
//!
//! `the_cycle_halts_when_every_active_participant_votes_done` demonstrates that
//! rather than merely permitting it. It posts `host note`, both participants
//! vote done, and the cycle halts — at which instant neither cursor has moved
//! past `go`. The row reaches A only because the test then sends a user
//! message; without one it would sit unread behind an arrival that declared
//! there was nothing left to do.
//!
//! Production writers on that path exist today, all of them `origin =
//! "system"` host injections: `watchdog`'s idle nudge, `session`'s first-spawn
//! phase nudge, `state`'s per-agent phase instruction and `duo`'s two adherence
//! nudges. (`broadcast` and `tray` write `origin = "user"` — that is the one
//! origin a command already covers, so they are not on this list.)
//!
//! Closing it needs a fifth command and is out of scope here. Read it with the
//! notification gap above rather than apart from it: the session yields to a
//! user who is not told it has yielded, possibly over rows nobody read.
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
//! three things end a drain early:
//!
//! - the control channel CLOSING, which is session end. A teardown must not
//!   wait on a wedged agent's stdin;
//! - a [`SequencerCommand::UserMessage`], which resets the ring and therefore
//!   supersedes the turn being fed. This is the user's way out of a wedged
//!   participant, and it costs nothing correctness-wise: the rows that did not
//!   land stay past the cursor and are offered again when the ring returns;
//! - a [`SequencerCommand::QuestionParked`], which halts the cycle, so there is
//!   no longer a turn to feed.
//!
//! **The third one stops the drain rather than letting it finish, and both were
//! available.** Stopping costs the rows already read but not delivered — which
//! is not a loss, because [`Storage::commit_delivery`] records only the prefix
//! that landed, so the remainder stays past the cursor exactly as it does for a
//! user message. Finishing costs the word "immediately": the participant being
//! fed is normally the one that just blocked on a human, so every further row
//! goes into a 64-slot buffer in front of a process that has stopped reading —
//! and `deliver` PARKS when that buffer fills, for as long as the human takes to
//! answer. A halt that waits out a human before taking effect is not a halt.
//! `a_parked_question_stops_the_drain_rather_than_finishing_it` pins it.
//!
//! Every OTHER command is taken off the channel and deferred, so a sender never
//! parks behind a drain, and then handled in arrival order once the drain
//! finishes. Deferring rather than acting is what keeps "when your turn comes
//! you have read everything you had not read" true — acting on a `TurnComplete`
//! mid-drain would hand the turn over with rows undelivered, which is the
//! deferral this section rejects. The two that END a drain are deferred as well,
//! not acted on here: the drain sets `stop` and returns, and the loop applies
//! the reset or the halt in arrival order. A pause is task 9 and attaches at the
//! same `select!`.

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
    /// **`done` is the consensus vote**, and it is a field on this command
    /// rather than a command of its own. A turn ends exactly once and ends one
    /// of two ways — substantive output, or nothing left to do — so the vote is
    /// a property of the ending, not a second event. Split into
    /// `TurnComplete` + `Done`, both would mean "my turn ended", both would
    /// need this same two-field guard, and both would have to step the ring;
    /// a sender that emitted the pair would then step it twice and put two
    /// participants on a turn at once, which is the one invariant this loop
    /// exists to keep. One field keeps "one accepted completion, one ring step,
    /// one vote" true by construction.
    ///
    /// `done: false` is substantive output and RESETS the tally for the whole
    /// session — see [`halted_on_consensus`]. The vote is recorded only for a
    /// completion that passes the guard below, for the same reason the ring is
    /// only stepped for one.
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
    /// belongs with the task that spawns the loop. `done` now rides the same
    /// unsolved round trip, so the task that carries the epoch OUT is the one
    /// that has to carry the vote back. A participant that arrives by
    /// [`ParticipantJoined`](Self::ParticipantJoined) is the sharpest case:
    /// that command hands the loop an stdin and gets nothing back, so a late
    /// joiner holding the turn has no way to learn which epoch to complete
    /// with — and therefore no way to cast a vote that passes the guard.
    TurnComplete {
        participant_id: i64,
        epoch: u64,
        done: bool,
    },
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
    /// A participant parked a question for the user: halt the cycle now.
    ///
    /// **A yield, not a vote.** A completion is an opinion about one turn and is
    /// guarded by [`TurnComplete`](Self::TurnComplete)'s two fields. This is a
    /// fact about the SESSION — a human is being waited on — so it carries no
    /// participant id and no epoch, and nothing counts it. One participant
    /// blocking on the user stops the cycle regardless of what the others would
    /// have done; that is what "unilaterally" means, and it is why there is
    /// nothing here to guard.
    ///
    /// The asymmetry is deliberate and it costs something. A duplicate or
    /// late-arriving park halts a cycle a user message has already restarted —
    /// one wasted wake, recoverable by another user message. A guard that
    /// rejected a park for not naming the holder would be wrong in the worse
    /// direction: the parker need NOT be the holder (an on-demand participant,
    /// or one whose turn a reset superseded while its process kept running), and
    /// refusing its park leaves the ring handing out turns while a human is
    /// blocking — the state this command exists to end.
    ///
    /// **A command rather than a flag read.** The bridge already keeps a
    /// per-session `Arc<AtomicBool>`
    /// (`SignalingBridge::register_session_awaiting`), and `core::router` reads
    /// exactly that, lock-free, per forward — so a flag in [`SequencerDeps`]
    /// would need no sender at all. It was not taken, because a flag is a LEVEL
    /// and this loop needs an EDGE: it sits in `recv().await` between turns, so
    /// a flag is only ever seen wherever the loop happens to look, and it has no
    /// defined order against [`UserMessage`](Self::UserMessage) — the one pair
    /// whose ordering decides whether the release restarts the cycle or a `true`
    /// that has not been cleared yet re-halts it on the spot. "Nothing mints
    /// commands yet" does not tell the two apart either: nothing mints
    /// `TurnComplete` or `UserMessage` yet either, and the same task owes all
    /// three.
    ///
    /// **Released by [`UserMessage`](Self::UserMessage)**, like the consensus
    /// halt — see "the second halt reason" in the module doc for why there is no
    /// release command of its own and what that leaves unwired.
    QuestionParked,
    /// Stop: hold the cycle where it stands, hand out no further turns.
    ///
    /// Still a no-op. Implementing it is a later task; what is in place for it
    /// is [`TurnComplete`](Self::TurnComplete)'s epoch, without which a turn
    /// finishing during a pause would advance the ring on resume. Note that a
    /// pause cannot yet cut a drain short either — only a user message, a parked
    /// question and the channel closing do — so task 9 attaches to the same
    /// `select!`.
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
                done,
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
                    // The vote is recorded and consensus asked BEFORE the ring
                    // is stepped: arriving means waking nobody, so a step taken
                    // first would have to be taken back. Both are inside the
                    // guard, because a superseded turn's vote is an opinion
                    // about a turn that no longer exists — counting it would
                    // let a discarded completion do the one thing discarding it
                    // was meant to prevent.
                    if !halted_on_consensus(&deps, &mut holder, &mut epoch, participant_id, done)
                        .await
                    {
                        advance_turn(&deps, &mut rx, &mut holder, &mut epoch, &mut deferred, false)
                            .await;
                    }
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
                // The user's own output is substantive, so it resets the tally —
                // but the reset is NOT written here. It rides the restart itself,
                // in `advance_turn`; see the comment there for why this arm is
                // the wrong place to own it.
                //
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
            SequencerCommand::QuestionParked => {
                // Unguarded and uncounted, unlike a completion — see the variant
                // doc. Nothing else happens: no vote is touched, so the tally
                // standing when the question was parked is exactly the tally the
                // release has to clear, and `advance_turn` is where that happens.
                //
                // Halting twice is harmless by construction: the second call
                // finds no holder and moves an epoch nothing is carrying.
                halt(&mut holder, &mut epoch);
                debug!(
                    session = %deps.session_id,
                    "sequencer: a question was parked for the user; the cycle yields"
                );
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

/// Step the ring, stamp the new turn, and deliver its backlog — emptying the
/// tally first if this step RESTARTS the cycle rather than continuing it.
///
/// `reset` is a user message: the ring goes back to its first place instead of
/// one past the current holder. It is the only restart today, which is exactly
/// why the tally clear lives in here and not at its call site; see the comment
/// on the clear.
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
    // A cycle starting at the front of the ring starts with an empty tally.
    //
    // **Bound to the mechanism, not to a call site.** This lived in the
    // `UserMessage` arm, where it was correct and invisible: a user message is
    // the only restart there is, so "the user spoke" and "a cycle restarts" were
    // the same event and nothing said which one the clear belonged to. The
    // parked-question halt is what separates them. It touches no vote, so the
    // cycle its release restarts is the one that arrives here with a tally left
    // over from BEFORE the halt — and left standing, the first `done: true` of
    // the new cycle completes it and halts again with the rest of the ring never
    // having taken a turn. That is the false arrival this file exists to
    // prevent, and `a_parked_question_halts_the_cycle_unilaterally` is the test
    // that would go red: delete this clear and its last `expect(3)` times out.
    //
    // The test is `current.is_none()` rather than `reset` because those are the
    // TWO ways to the front of the rotation: an explicit reset, and a `None`
    // holder — which is what a consensus halt leaves behind. Anything that
    // restarts a cycle passes through here, so a restart cannot forget the
    // clear by not calling a helper.
    //
    // A failure warns and continues, like the other storage faults on this
    // path. It is the one that leans the wrong way — stale votes can only make
    // an arrival come EARLY — but the alternative is refusing to hand out a
    // turn because a write failed, which strands the session outright.
    if current.is_none() {
        if let Err(e) = deps.storage.clear_done_votes(&deps.session_id).await {
            warn!(
                session = %deps.session_id,
                error = %e,
                "sequencer: the tally was not cleared for a restart; the next arrival may \
                 count votes cast before it"
            );
        }
    }
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

/// Record how a turn ended, then answer: has the cycle arrived?
///
/// `true` means every active participant has declared done and the caller must
/// NOT step the ring — the turn is already cleared here. See "the halt is a
/// yield, not a stop" in the module doc for what that leaves observable.
///
/// **A storage failure answers `false`.** Neither error is a vote, and of the
/// two ways to be wrong the cycle continuing is the recoverable one: a spurious
/// extra lap costs a turn and the participant votes again, whereas a halt
/// nobody voted for parks the session waiting on a user who was never told they
/// are being waited on. Same instinct as [`Handover::Held`] — do not invent a
/// state out of a failure.
async fn halted_on_consensus(
    deps: &SequencerDeps,
    holder: &mut Option<Participant>,
    epoch: &mut u64,
    participant_id: i64,
    done: bool,
) -> bool {
    let recorded = if done {
        deps.storage.set_done_vote(participant_id, true).await
    } else {
        // Substantive output resets the tally for the WHOLE session, not just
        // for this participant. A done cast before this turn was a statement
        // about a session that no longer exists — see
        // `substantive_output_resets_the_tally` for the arithmetic that lets
        // one stale vote and one fresh one add up to an arrival nobody voted
        // for.
        deps.storage.clear_done_votes(&deps.session_id).await
    };
    if let Err(e) = recorded {
        warn!(
            session = %deps.session_id,
            participant_id,
            done,
            error = %e,
            "sequencer: done vote not recorded; continuing the cycle"
        );
        return false;
    }
    match deps.storage.all_active_voted_done(&deps.session_id).await {
        Ok(true) => {
            halt(holder, epoch);
            debug!(
                session = %deps.session_id,
                participant_id,
                "sequencer: every active participant voted done; the cycle yields to the user"
            );
            true
        }
        Ok(false) => false,
        Err(e) => {
            warn!(
                session = %deps.session_id,
                error = %e,
                "sequencer: consensus read failed; continuing the cycle"
            );
            false
        }
    }
}

/// Take the turn out of flight. **This is the whole of a halt** — no event, no
/// marker, no vote touched.
///
/// One function for both reasons the cycle can yield ([`halted_on_consensus`]
/// and [`SequencerCommand::QuestionParked`]) so they cannot drift apart. They
/// differ in what leads here, not in what a halted cycle IS.
///
/// Both lines are load-bearing, but for DIFFERENT reasons — remove either and a
/// test fails, and it is worth saying which, because only one of the two catches
/// is about the guard:
///
/// - **the holder clear shuts the other delivery door.**
///   [`SequencerCommand::ParticipantJoined`] delivers on arrival whenever the
///   holder is the participant that joined, so a halt that moved only the epoch
///   would still feed a respawn arriving mid-halt.
///   `cursors_do_not_advance_while_awaiting` probes exactly that with a join;
///   drop `*holder = None` and it fails on the wire. `None` is also what
///   `next_active_participant` reads as "reset to the front", so the user
///   message that ends the halt starts the next cycle where a fresh session
///   would, tally clear included;
/// - **the epoch bump is belt and braces on the discard path.** The module doc
///   says so of the consensus halt and it is no different here: with the holder
///   gone, `TurnComplete`'s identity compare already rejects every later
///   completion unaided. What actually goes red without the bump is the epoch
///   NUMBERING — `a_parked_question_halts_the_cycle_unilaterally` names the
///   epochs it completes, and a halt that skipped the bump mints one fewer, so
///   the test's last completion names a turn that was never handed out. A real
///   failure, but an arithmetic one; do not read it as the guard being pinned
///   from both sides.
fn halt(holder: &mut Option<Participant>, epoch: &mut u64) {
    *holder = None;
    *epoch += 1;
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
        // is `all_active_voted_done`, asked in `halted_on_consensus` before the
        // caller ever gets here.
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
    /// A question was parked: the cycle is about to halt, so there is no turn
    /// left to feed. Already pushed onto the deferred queue.
    Parked,
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
/// hold the command channel shut" in the module doc for which of them also END
/// the drain and which are merely set aside.
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
                        // Deferred like the user message, and for the same
                        // reason: the ACT is the loop's, not this function's.
                        // Ending the drain here is what makes the halt
                        // immediate; see the module doc for what stopping costs
                        // against what finishing would.
                        Some(cmd @ SequencerCommand::QuestionParked) => {
                            deferred.push_back(cmd);
                            stop = Some(Stop::Parked);
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
            Some(Stop::Parked) => {
                debug!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    delivered = landed.len(),
                    of = page.rows.len(),
                    "sequencer: a parked question halted this turn mid-drain"
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
                done: false,
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
                done: false,
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
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, done: false },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["r1"], "B now holds the turn");

        // The user speaks over B's turn. Waiting on A's wake is what makes the
        // next line's ordering a fact rather than a race.
        post(&storage, "user", None, "r2").await;
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["r2"], "the ring reset to A");

        // B's turn ends, late.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, done: false },
        )
        .await;
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
                done: false,
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
                done: false,
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
    async fn the_cycle_halts_when_every_active_participant_votes_done() {
        // The cycle runs until every ACTIVE participant agrees there is nothing
        // left to do. Two actives here, so it takes two done votes with no
        // substantive output between them.
        //
        // "Halts" means NO FURTHER WAKE ARRIVES, which a bare `recv().await`
        // cannot tell from a hang and a dropped `tx` cannot tell from a delivery
        // it aborted — the drain is biased on commands, so a closed control
        // channel stops a wake the halt was supposed to prevent, and a halt
        // asserted that way passes with the consensus check removed. `quiet()`
        // is the instrument: a bounded window with the channel still OPEN.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // A has nothing to add. One vote of two is not consensus, so the ring
        // steps — and this is also what says the halt below is the SECOND vote
        // arriving rather than the first one halting everything.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, done: true },
        )
        .await;
        assert_eq!(
            seats[1].expect(1).await,
            vec!["go"],
            "one done vote of two is not consensus — the cycle continues"
        );

        // An unread row for whoever would be woken next. Without it every
        // silence below would also be what a sequencer that stepped the ring
        // and found an empty backlog produces, and the test would prove nothing
        // — a ring step delivers no wire when there is nothing past the cursor.
        post(&storage, "system", None, "host note").await;
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence has to be the halt, not an empty backlog"
        );

        // B votes done too: every active participant has now declared done with
        // nothing substantive between the two votes, so the session yields.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, done: true },
        )
        .await;
        seats[0].quiet().await;
        seats[1].quiet().await;
        assert!(
            storage.all_active_voted_done("s1").await.unwrap(),
            "the arrival is durable in the roster, which is where a host reads it"
        );

        // Nothing can complete a turn that is not in flight. A late completion
        // for the turn that halted the cycle — a retry, a supervisor echoing
        // the last one back after a respawn — is discarded rather than
        // re-entering the cycle.
        //
        // **`done: false` is what makes this assertion mean anything.** Repeat
        // the halting vote instead and a loop that accepted it would record the
        // same vote, find the same consensus and fall silent for the same
        // reason: the silence proves nothing, and both `*holder = None` and
        // `*epoch += 1` can be deleted with the suite still green. A
        // SUBSTANTIVE late completion separates them — taken as live it clears
        // the tally and steps the ring, so the silence and the intact tally
        // below are two independent observations of the same discard.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, done: false },
        )
        .await;
        seats[0].quiet().await;
        assert!(
            storage.all_active_voted_done("s1").await.unwrap(),
            "a completion arriving after the halt did not reset the tally"
        );

        // Halted, not dead. The user speaking restarts the cycle at the front
        // of the ring — without this the silence above would also be what a
        // wedged loop looks like — and it clears the tally on the way in.
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["host note"],
            "the user's message restarts the cycle at the front of the ring"
        );
        assert!(
            !storage.all_active_voted_done("s1").await.unwrap(),
            "and resets the tally: a vote cast before the user spoke cannot count \
             toward the next arrival"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn substantive_output_resets_the_tally() {
        // The stale-done case. A votes done, then B speaks — and a vote cast
        // before that output describes a session that no longer exists. Let it
        // stand and A's stale done plus B's eventual fresh one add up to an
        // arrival neither of them voted for, halting the session with A never
        // having seen what B said.
        //
        // The shape is chosen so the two worlds DIVERGE: A's turn after B's
        // output must itself end non-done, or A re-votes done and the stale
        // vote is indistinguishable from the fresh one.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // The vote that must not survive what follows.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, done: true },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"]);

        // B produces substantive output. A row, because that is what
        // substantive MEANS — and posting it is also what makes A's next wake
        // observable at all.
        post(&storage, "participant", Some("b"), "b found something").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, done: false },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["b found something"],
            "the ring came back to A carrying B's output"
        );
        let roster = storage.participants_for_session("s1").await.unwrap();
        assert!(
            !roster.iter().find(|p| p.id == a).unwrap().done_vote,
            "B's output cleared A's done vote — asserted on the vote itself, because \
             consensus is `false` either way at this point and would not tell the two apart"
        );

        // A reads it and has nothing to add either, but that is not a vote — it
        // is output of its own, so the tally stays at zero.
        post(&storage, "participant", Some("a"), "a replied").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, done: false },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["a replied"]);

        // Unread by A, so A's wake below is a wire rather than a silent step.
        post(&storage, "system", None, "host note").await;
        // B now votes done — the vote that WOULD complete a tally still holding
        // A's stale done from three turns ago. It must not: A has spoken since,
        // so the ring comes back to A instead of the session halting.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 4, done: true },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["host note"],
            "A's done was cleared by B's output, so B's vote is one of two and the \
             cycle continues"
        );

        // And the tally does still ARRIVE — the reset delays consensus, it does
        // not make it unreachable. A votes done on top of B's live vote.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 5, done: true },
        )
        .await;
        seats[0].quiet().await;
        seats[1].quiet().await;
        assert!(
            storage.all_active_voted_done("s1").await.unwrap(),
            "two consecutive done votes, and B's unread `host note` proves the silence \
             is the halt rather than an empty backlog"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_discarded_completions_vote_does_not_count_toward_the_tally() {
        // The guard gates the VOTE as well as the ring step. A completion that
        // does not name the turn in flight is an opinion about a turn that no
        // longer exists, and counting it would let a completion the loop
        // discarded do the one thing discarding it was meant to prevent —
        // arrive at a halt on a vote nobody currently holding a turn cast.
        //
        // Both halves of the guard reject the injected completion here (dead
        // epoch, and not the holder), so this pins the vote against the guard
        // as a whole rather than against either half.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // B is not holding the turn and epoch 0 is spent. Discarded.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 0, done: true },
        )
        .await;
        // Unread by B, so B's wake below is a wire rather than a silent step.
        post(&storage, "system", None, "host note").await;

        // A votes done — the live half of a tally that would be COMPLETE if
        // B's discarded vote had been recorded.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, done: true },
        )
        .await;
        assert_eq!(
            seats[1].expect(2).await,
            vec!["go", "host note"],
            "one live vote of two: the ring steps to B, which has still not voted"
        );
        assert!(
            !storage.all_active_voted_done("s1").await.unwrap(),
            "the discarded completion left no vote behind"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn observers_do_not_vote() {
        // Only the rotation votes. Observers and on-demand participants are
        // skipped in it, so they never get a turn, so they can never declare
        // done — count them and one active plus three watchers would need four
        // yields to halt, which is a session that never halts at all.
        //
        // One active and two non-voters here: consensus has to arrive on A's
        // single done.
        let (deps, storage, mut seats) = ring(&[
            ("a", "active"),
            ("watcher", "observer"),
            ("helper", "on_demand"),
        ])
        .await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // Unread by all three when the vote lands. A ring of one WRAPS, so a
        // sequencer that did not halt would hand A the turn straight back and
        // deliver this row; without it A's silence would be ambiguous.
        post(&storage, "system", None, "host note").await;
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence has to be the halt, not an empty backlog"
        );

        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, done: true },
        )
        .await;
        seats[0].quiet().await;
        seats[1].quiet().await;
        seats[2].quiet().await;

        // The claim in the name, stated directly: the session has arrived while
        // two of its three participants have not voted, and cannot have.
        assert!(storage.all_active_voted_done("s1").await.unwrap());
        let roster = storage.participants_for_session("s1").await.unwrap();
        let silent: Vec<&str> = roster
            .iter()
            .filter(|p| !p.done_vote)
            .map(|p| p.slug.as_str())
            .collect();
        assert_eq!(
            silent,
            vec!["watcher", "helper"],
            "consensus arrived on the rotation's one vote, with neither non-voter having cast one"
        );

        // Halted, not wedged.
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["host note"]);
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_parked_question_halts_the_cycle_unilaterally() {
        // The second halt reason, and the one that is NOT a tally. Consensus
        // needs every active participant to agree; a parked question needs one
        // participant to block on a human. So the ring has to stop here with B's
        // `done` standing and A's never cast — a cycle that only ever halted on
        // `all_active_voted_done` would keep handing out turns.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, done: false },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"], "epoch 2, B holds");

        // A row for A to be woken by, and then B's done vote — which must
        // SURVIVE the halt below, because it is what the release has to clear.
        post(&storage, "user", None, "note for a").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, done: true },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["note for a"],
            "one done vote of two is not consensus — epoch 3, A holds"
        );

        // Unread by BOTH when the question is parked, so any silence below is
        // the halt rather than a ring step that found nothing to hand over.
        post(&storage, "user", None, "note for b").await;
        for id in [a, b] {
            assert!(
                !storage.unread_for_participant(id).await.unwrap().rows.is_empty(),
                "the silence has to be the halt, not an empty backlog"
            );
        }

        // A parks a question. Unilateral: A has cast no vote at all.
        send(&tx, SequencerCommand::QuestionParked).await;
        assert!(
            !storage.all_active_voted_done("s1").await.unwrap(),
            "the cycle is stopping with the rotation still one vote short — which is \
             the whole difference between this halt and the consensus one"
        );

        // The halt is proven by what happens to A's completion, not by the
        // silence right after the park: an ignored command produces silence too,
        // because nothing was due to happen yet. A SUBSTANTIVE completion for
        // the turn that was in flight is the discriminator — taken as live it
        // clears the tally and steps the ring to B, which has two rows waiting.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, done: false },
        )
        .await;
        // With the control channel still OPEN — dropping `tx` here would abort
        // the very delivery a broken halt would have made.
        seats[1].quiet().await;
        seats[0].quiet().await;
        let roster = storage.participants_for_session("s1").await.unwrap();
        assert!(
            roster.iter().find(|p| p.id == b).unwrap().done_vote,
            "and the discarded completion cleared no vote"
        );

        // Halted, not dead: the user's message restarts the cycle at the front.
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["note for b"],
            "epoch 5, A holds again"
        );

        // The release CLEARED the tally, and this is the shape that proves it
        // rather than restating it. B's done was standing when the question was
        // parked. If it survived the restart, A's first `done: true` completes a
        // tally of two and the cycle halts again with B never having taken a
        // turn — the false arrival `advance_turn`'s clear exists to stop.
        post(&storage, "user", None, "note for b again").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 5, done: true },
        )
        .await;
        assert_eq!(
            seats[1].expect(3).await,
            vec!["note for a", "note for b", "note for b again"],
            "one live vote of two: B's pre-park done did not survive the release"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn cursors_do_not_advance_while_awaiting() {
        // Router-inventory #4 (`awaiting_suppresses_forward`) carried onto the
        // turn path. There it was a forward the router declined to push; here
        // there is nothing to suppress, because a halted cycle hands out no
        // turns — so the behaviour shows up on the CURSORS, which is a durable
        // artefact rather than a wire that was not sent.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::UserMessage).await; // epoch 1, A holds
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "delivery is live in this run"
        );
        send(&tx, SequencerCommand::QuestionParked).await;

        // Written while the session is awaiting. Nobody may be handed it.
        post(&storage, "user", None, "while awaiting").await;
        // Both doors to a delivery, tried in turn. A completion for the turn the
        // park took away: live, it would step the ring onto B and move B's
        // cursor over two rows.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, done: false },
        )
        .await;
        // And a respawn, which delivers on arrival whenever the holder is the
        // participant that joined. A halt leaves no holder, so it must not — an
        // implementation that halted by refusing to ADVANCE while keeping the
        // holder would deliver here.
        let (input, mut joined) = late_stdin(a);
        send(
            &tx,
            SequencerCommand::ParticipantJoined { participant_id: a, input },
        )
        .await;
        joined.quiet().await;
        seats[1].quiet().await;

        assert_eq!(
            storage.cursor_for(a).await.unwrap(),
            1,
            "A's cursor sits where its pre-park turn left it"
        );
        assert_eq!(
            storage.cursor_for(b).await.unwrap(),
            0,
            "and B was never handed a turn, so its cursor never moved"
        );

        // The user answers. Cursors move again — without this the frozen pair
        // above would also be what a wedged loop looks like. The wire lands on
        // the stdin that JOINED, which is how the insert above is shown to have
        // taken effect rather than been dropped.
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(
            joined.expect(1).await,
            vec!["while awaiting"],
            "the release hands the front of the ring the row it could not have while awaiting"
        );
        assert_eq!(storage.cursor_for(a).await.unwrap(), 2);
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_parked_question_stops_the_drain_rather_than_finishing_it() {
        // A park arriving mid-drain ENDS it, joining the user message as the
        // second command that can. The participant being fed is the one that
        // just blocked on a human, so every further row goes into a buffer in
        // front of a process that has stopped reading — and `deliver` PARKS when
        // that buffer fills, for as long as the human takes.
        //
        // Driven through `deliver_backlog` directly, with the command already on
        // the channel when the drain reaches its first row. That is where the
        // biased select reads it; a park arriving at row 5 takes the same branch.
        //
        // The backlog spans TWO batches on purpose. `break 'rows` alone ends the
        // page, so on a single-page fixture the drain returns anyway at
        // `!page.more` and the `Stop::Parked` arm's own `return` is dead weight
        // no assertion could see. Past the batch limit the outer loop would come
        // back for a second page — with the park already consumed off the
        // channel, so nothing would stop it a second time.
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let overflow = UNREAD_BATCH_LIMIT as usize + 1;
        for i in 0..overflow {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        assert!(
            storage.unread_for_participant(a).await.unwrap().more,
            "the fixture has to outgrow one batch or the `return` below is untested"
        );
        let holder = storage
            .next_active_participant("s1", None)
            .await
            .unwrap()
            .expect("the fixture's one active participant");

        // `_cmd_tx` is HELD, not dropped. A closed command channel is
        // `Stop::SessionEnd` inside the drain, which stops it for a reason that
        // has nothing to do with the park — and would leave this test green with
        // the park branch deleted.
        let (_cmd_tx, mut rx) = mpsc::channel(8);
        send(&_cmd_tx, SequencerCommand::QuestionParked).await;
        let mut deferred = VecDeque::new();
        deliver_backlog(&deps, &holder, &mut rx, MAX_TURN_BATCHES, &mut deferred).await;

        seats[0].quiet().await;
        assert!(
            matches!(deferred.front(), Some(SequencerCommand::QuestionParked)),
            "the drain sets the park aside for the loop rather than swallowing it — \
             the halt itself is the main loop's arm, not this function's"
        );
        assert_eq!(deferred.len(), 1);
        // Stopping costs the rows read but not delivered, and that is not a
        // loss: the cursor never moved past them, so the whole backlog is
        // re-offered when the ring comes back.
        assert_eq!(storage.cursor_for(a).await.unwrap(), 0);
        let left = storage.unread_for_participant(a).await.unwrap();
        assert!(left.more);
        assert_eq!(left.rows.first().map(|r| r.content.as_str()), Some("row 0"));
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
                done: false,
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
                done: false,
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
                done: false,
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
                done: false,
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
                done: false,
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
        // reaching the exit is proof all six were handled.
        let (deps, _storage, seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let (joined_input, _joined_seat) = late_stdin(a);
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        for cmd in [
            SequencerCommand::TurnComplete { participant_id: a, epoch: 0, done: false },
            SequencerCommand::UserMessage,
            SequencerCommand::ParticipantJoined { participant_id: a, input: joined_input },
            SequencerCommand::QuestionParked,
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
