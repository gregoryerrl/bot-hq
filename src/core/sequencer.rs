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
//! ## "Recorded but not delivered" ends on the turn path
//!
//! `router::route_forward` can drop a forward (convergence) or hold one (the
//! hard cap) AFTER its row is written, so today's chat can show a row that no
//! peer ever read. **This loop does not inherit that.** Every row past a
//! participant's cursor is handed to it when its turn comes, and
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
//! One case remains where a row is recorded and not read, and it is handled
//! rather than ignored: a participant whose stdin is gone. [`deliver_backlog`]
//! commits only the prefix that actually reached the channel, so the cursor
//! never moves past a row the agent did not get. Cursors do not rewind, so
//! committing optimistically would lose those rows permanently.
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

use crate::agents::ParticipantInput;
use crate::storage::{Participant, PersistedMessage, Storage};
use std::collections::HashMap;
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
/// 32 × 200 = 6,400 rows. The largest channel in the live database is 3,585
/// rows, so no real backlog reaches this; hitting it means something is
/// producing during a turn, which is worth the `warn!` it gets. The remainder
/// then arrives on this participant's next turn — the deferral the module doc
/// rejects as a policy, used here only as a backstop.
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
    /// A participant in the ring but ABSENT from this map is one that has no
    /// live process behind it: `SessionAgent::participant_id` is `None`
    /// whenever the roster read failed at spawn, and a participant invited
    /// after spawn has no entry either. [`deliver_backlog`] warns and delivers
    /// nothing; it does not skip ahead in the ring, because auto-advancing past
    /// an unreachable agent is a recovery policy and belongs with spin
    /// detection.
    pub inputs: HashMap<i64, ParticipantInput>,
}

/// A wake for the sequencer.
///
/// **Bodies stay out of these.** A command says that something happened; WHAT
/// happened is a row in `messages`, and the sequencer reads rows from cursors.
/// Carrying the text in the command instead would put a second copy of it in
/// flight with no row identity behind it — the thing this batch's receipt work
/// removed.
#[derive(Debug)]
pub enum SequencerCommand {
    /// The participant holding the turn finished it — advance the ring.
    ///
    /// **Carries WHOSE turn ended**, and the sequencer ignores a completion
    /// that does not name the current holder. The sequencer does know whose
    /// turn it handed out, which is exactly why the id is needed: it is the
    /// only way to tell the live completion from a superseded one.
    ///
    /// The reachable producer of a superseded completion is a user message. A
    /// user message resets the ring to its first place while the previous
    /// holder is still mid-turn; that turn then ends and its completion arrives
    /// behind the reset. Payload-free, it would advance the ring off the
    /// participant that was just woken — two agents holding a turn at once,
    /// which is the one invariant this loop exists to keep. Pause/Resume adds a
    /// second producer once it is implemented, and a supervisor that respawns
    /// an agent mid-turn a third.
    ///
    /// It cost nothing to add: there were no send sites when it was added, and
    /// the tests below are the first.
    TurnComplete { participant_id: i64 },
    /// The user posted to the channel. Resets the cycle to the first active
    /// participant and hands it the turn.
    UserMessage,
    /// Stop: hold the cycle where it stands, hand out no further turns.
    ///
    /// Still a no-op. Implementing it is a later task; what is in place for it
    /// is [`TurnComplete`](Self::TurnComplete)'s holder check, without which a
    /// turn finishing during a pause would advance the ring on resume.
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
pub async fn run_sequencer(deps: SequencerDeps, mut rx: mpsc::Receiver<SequencerCommand>) {
    debug!(session = %deps.session_id, "sequencer: started");
    // Who holds the turn. `None` is "the cycle has not started", which is also
    // what `next_active_participant` reads as "reset to the front".
    let mut holder: Option<Participant> = None;
    while let Some(cmd) = rx.recv().await {
        match cmd {
            SequencerCommand::TurnComplete { participant_id } => {
                // Only the participant that HOLDS the turn can end it: a
                // completion naming anyone else does not describe the turn in
                // flight, and with no holder there is no turn in flight at all.
                // Why that matters is in the variant doc. That it is
                // load-bearing was measured, not argued — with this condition
                // replaced by `true`,
                // `a_completion_from_a_superseded_turn_does_not_advance_the_ring`
                // fails with B re-woken behind A: two participants on a turn at
                // once.
                if holder.as_ref().is_some_and(|h| h.id == participant_id) {
                    holder = hand_over(&deps, holder.as_ref()).await;
                } else {
                    debug!(
                        session = %deps.session_id,
                        participant_id,
                        holder = ?holder.as_ref().map(|h| h.id),
                        "sequencer: completion from a participant that does not hold the turn"
                    );
                }
            }
            SequencerCommand::UserMessage => {
                // The user speaking resets the cycle to the front of the
                // rotation, whoever held the turn — `None` is what
                // `next_active_participant` reads as "reset". The previous
                // holder's turn is not cancelled here; nothing stops it, and its
                // completion is discarded by the check above when it arrives.
                holder = hand_over(&deps, None).await;
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

/// Step the ring past `current` and wake whoever is next. Returns the new
/// holder.
///
/// `current == None` resets to the front of the rotation, which is what a user
/// message does.
async fn hand_over(deps: &SequencerDeps, current: Option<&Participant>) -> Option<Participant> {
    let next = match deps
        .storage
        .next_active_participant(&deps.session_id, current)
        .await
    {
        Ok(next) => next,
        // The ring is a roster read, so a failure here is a storage problem,
        // not an empty rotation. Holding the turn where it is keeps the two
        // apart: a later completion from the same holder retries the step,
        // whereas returning `None` would silently look like "nobody is active"
        // and reset the cycle on the next user message.
        Err(e) => {
            warn!(
                session = %deps.session_id,
                error = %e,
                "sequencer: ring read failed; holding the turn where it is"
            );
            return current.cloned();
        }
    };
    let Some(next) = next else {
        // Nobody active. NOT a consensus test — see the module doc; consensus
        // is `all_active_voted_done` and is a later task.
        debug!(
            session = %deps.session_id,
            "sequencer: no active participant to hand the turn to"
        );
        return None;
    };
    deliver_backlog(deps, &next).await;
    Some(next)
}

/// Hand `to` everything it has not read, and record what it got.
///
/// Drains rather than delivering one batch — see "how far a turn reads" in the
/// module doc.
async fn deliver_backlog(deps: &SequencerDeps, to: &Participant) {
    let Some(input) = deps.inputs.get(&to.id) else {
        warn!(
            session = %deps.session_id,
            participant_id = to.id,
            slug = %to.slug,
            "sequencer: the participant holding the turn has no stdin; delivering nothing"
        );
        return;
    };
    for _ in 0..MAX_TURN_BATCHES {
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
        // `from_row` is what makes a row READ BACK deliverable: receipts are
        // otherwise minted only by the INSERT, and every row written before a
        // restart is only ever available this way.
        let mut landed: Vec<(i64, Option<&str>)> = Vec::with_capacity(page.rows.len());
        for row in &page.rows {
            if !input.deliver(&PersistedMessage::from_row(row)).await {
                break;
            }
            // `None` = delivered. Nothing on the turn path withholds; see the
            // module doc.
            landed.push((row.id, None));
        }
        let short = landed.len() < page.rows.len();
        // Committing only the PREFIX that landed is what keeps "recorded but
        // not delivered" off this path. The cursor moves to the highest id in
        // whatever is passed here and never rewinds, so committing the whole
        // page after a failed write would lose the undelivered tail forever.
        if let Err(e) = deps.storage.commit_delivery(to.id, &landed).await {
            warn!(
                session = %deps.session_id,
                participant_id = to.id,
                error = %e,
                "sequencer: delivery not recorded; the batch will be re-offered"
            );
            return;
        }
        if short {
            warn!(
                session = %deps.session_id,
                participant_id = to.id,
                slug = %to.slug,
                delivered = landed.len(),
                of = page.rows.len(),
                "sequencer: stdin closed mid-batch; the rest stays past the cursor"
            );
            return;
        }
        if !page.more {
            return;
        }
    }
    warn!(
        session = %deps.session_id,
        participant_id = to.id,
        batches = MAX_TURN_BATCHES,
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

    /// Every stubbed participant's stdin buffer.
    ///
    /// Sized above the largest backlog any test here posts (201 rows), because
    /// `deliver` PARKS on a full channel: a delivery that stalled for want of
    /// buffer would deadlock the sequencer, and the test would report a
    /// deadline rather than the wrong count it is actually looking for.
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

        /// Wait for exactly `n` wires, then assert nothing else is queued.
        ///
        /// A synchronisation point: it returns only once the sequencer has
        /// finished the delivery, which is what lets a test post a NEW row
        /// between two commands and know which turn will read it.
        async fn expect(&mut self, n: usize) -> Vec<String> {
            let mut out = Vec::new();
            for i in 0..n {
                let m = tokio::time::timeout(DEADLINE, self.rx.recv())
                    .await
                    .unwrap_or_else(|_| panic!("wire {} of {n} never arrived", i + 1))
                    .expect("the sequencer dropped this participant's stdin");
                out.push(m.message.content);
            }
            out
        }
    }

    /// A session whose rotation is `roster` — `(slug, participation_mode)` in
    /// turn-position order — with one stdin per participant.
    ///
    /// Returns the deps (moved into the task), a clone of the storage the test
    /// posts and asserts with, and one [`Seat`] per roster entry.
    async fn ring(roster: &[(&str, &str)]) -> (SequencerDeps, Storage, Vec<Seat>) {
        let storage = Storage::memory().await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();
        let mut inputs = HashMap::new();
        let mut seats = Vec::new();
        for (position, (slug, mode)) in roster.iter().enumerate() {
            let id = storage
                .insert_participant("s1", slug, slug, None, None, "[]", mode, position as i64)
                .await
                .unwrap();
            let (tx, rx) = mpsc::channel(STDIN_CAPACITY);
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
        // turn; then A finishes it.
        send(&tx, SequencerCommand::UserMessage).await;
        send(&tx, SequencerCommand::TurnComplete { participant_id: a }).await;
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[0].drain(),
            vec!["user one", "host note"],
            "A read the channel, but not its own last turn back as fresh input"
        );
        assert_eq!(
            seats[1].drain(),
            vec!["user one", "host note", "a's last turn"],
            "the turn steps to the next PLACE in the rotation, carrying every unread row"
        );
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
        send(&tx, SequencerCommand::UserMessage).await;
        send(&tx, SequencerCommand::TurnComplete { participant_id: a }).await;
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[1].drain(),
            nothing(),
            "the observer sits between A and B in the rotation and must not be woken"
        );
        assert_eq!(
            seats[2].drain(),
            vec!["go"],
            "the turn steps OVER the observer to the next active participant"
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
        send(&tx, SequencerCommand::TurnComplete { participant_id: a }).await;
        assert_eq!(seats[1].expect(1).await, vec!["r1"], "B now holds the turn");

        // The user speaks over B's turn. Waiting on A's wake is what makes the
        // next line's ordering a fact rather than a race.
        post(&storage, "user", None, "r2").await;
        send(&tx, SequencerCommand::UserMessage).await;
        assert_eq!(seats[0].expect(1).await, vec!["r2"], "the ring reset to A");

        // B's turn ends, late.
        send(&tx, SequencerCommand::TurnComplete { participant_id: b }).await;
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
    async fn a_backlog_past_the_batch_limit_is_drained_before_the_turn_is_handed_over() {
        // `ChannelPage::more` is a normal outcome, not an edge case — 266 of
        // 382 live sessions hold more rows than one batch. Leaving the rest for
        // next time would start a turn on context the participant is already
        // known to be missing, and the newest rows would arrive a full lap of
        // the ring later.
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
        send(&tx, SequencerCommand::UserMessage).await;
        send(&tx, SequencerCommand::TurnComplete { participant_id: a }).await;
        drop(tx);
        assert!(exited(task).await);

        let got = seats[1].drain();
        assert_eq!(got.len(), overflow, "every unread row, not the first batch");
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
        send(&tx, SequencerCommand::UserMessage).await;
        send(&tx, SequencerCommand::TurnComplete { participant_id: a }).await;
        drop(tx);
        assert!(exited(task).await);

        // A's wire first, and it is not decoration: without it this test passed
        // for a whole session against a sequencer that delivered NOTHING to
        // anyone. B receiving nothing has to mean "B has no stdin", and the only
        // way to say that is to show the same run delivering to a seat that has
        // one.
        assert_eq!(seats[0].drain(), vec!["go"], "delivery is live in this run");
        assert_eq!(seats[1].drain(), nothing());
        assert_eq!(
            storage.cursor_for(b).await.unwrap(),
            0,
            "an undelivered backlog stays undelivered — the cursor did not move"
        );
    }

    #[tokio::test]
    async fn every_command_is_consumed_and_the_loop_comes_back_for_more() {
        // "No-op" has to mean the loop CONSUMED the command and looped, not
        // that the first one ended the task or panicked an arm nobody wrote.
        // Buffered sends are drained before `recv()` reports the close, so
        // reaching the exit is proof all four were handled.
        let (deps, _storage, seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        for cmd in [
            SequencerCommand::TurnComplete { participant_id: a },
            SequencerCommand::UserMessage,
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
