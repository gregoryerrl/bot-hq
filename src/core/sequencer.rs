//! Turn sequencer — the single loop that will replace today's reactive
//! per-agent tasks and `core::router`'s bilateral peer-forwarding.
//!
//! The shape it is being built toward: exactly one participant holds the turn.
//! When that turn ends the sequencer picks the next active participant
//! ([`Storage::next_active_participant`]), hands it every channel row past its
//! cursor ([`Storage::unread_for_participant`]), and waits. The cycle ends by
//! consensus — every active participant has voted done
//! ([`Storage::all_active_voted_done`]) — or immediately, when a participant
//! parks a question for the user.
//!
//! **What is in this file is the skeleton: the loop and its lifecycle, and
//! nothing else.** No ring advance, no consensus, no delivery — each of those
//! is its own task and its own review. Every [`SequencerCommand`] is accepted
//! and logged and produces no other effect; the `match` arms are where those
//! tasks land. Nothing spawns this yet, so no session behaves differently
//! because it exists.
//!
//! ## Lifecycle
//!
//! [`run_sequencer`] returns when its command channel closes — i.e. when the
//! last sender is dropped, which is session end. That is the same exit
//! `run_router` has, and it is how router-inventory behaviour #20 survives the
//! handover: a session teardown must end the task, not leave it holding a
//! session's state alive.
//!
//! #20's other half — `RouterControl::drop` aborting the task outright — has no
//! counterpart here yet, because nothing holds a sequencer handle to drop. It
//! belongs with the control struct that wires this into a session, not with the
//! loop.
//!
//! ## Delivery must not route around the session-scope check
//!
//! Design note for the delivery task; none of it is code yet.
//!
//! [`SessionHandle::send_to_all`](crate::core::SessionHandle::send_to_all)
//! compares the receipt's `session_id` against the handle's and refuses a
//! mismatch. It is the only place that comparison appears — `grep session_id()`
//! across `core` finds it and nothing else — and that is not an oversight:
//! `SessionAgent` carries a participant id, a slug and a process handle, and no
//! session id at all, so `deliver` has nothing on hand to compare a receipt
//! against.
//!
//! The sequencer delivers to ONE participant rather than fanning out, so
//! reaching for `SessionAgent::deliver` would step around the only guard in the
//! system. The intended fix is a per-participant entry point on `SessionHandle`
//! — which does know both ids — with the comparison itself factored into one
//! private helper that both it and `send_to_all` call. The check then gains a
//! second caller instead of a copy.
//!
//! What that must not become is a compare written here. This module will hold
//! both a receipt and a session id, so the check would type-check locally, and
//! then there would be two of them to keep in agreement.
//!
//! Where the task gets a handle from is left open: handles live in
//! `AppState::sessions` behind a mutex, and the sequencer's control side is
//! expected to sit ON a handle, so the task cannot simply own the thing that
//! owns it. [`SequencerDeps`] deliberately carries no handle today.
//!
//! ## Open question: does "recorded but not delivered" survive?
//!
//! Today it is a real state, not a hypothetical. `router::route_forward` can
//! drop a forward (convergence) or hold one (the hard cap) AFTER its row is
//! written, so the chat can show a row that no peer ever read. The sequencer
//! replaces that ladder, so it either inherits that state — deliberately, with
//! [`Storage::record_delivery`] and a withheld reason making it auditable — or
//! ends it, by handing every row past the cursor to whoever holds the turn and
//! letting the participant judge.
//!
//! **Not decided here, on purpose.** Recording the question is the point: left
//! implicit, it gets answered by whichever behaviour falls out of the first
//! delivery implementation, which is how the ladder being replaced acquired its
//! own.

use crate::signaling::SignalingBridge;
use crate::storage::Storage;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;

/// What the sequencer task needs, cloned from the session's own state at spawn
/// — the same arrangement [`RouterDeps`](crate::core::RouterDeps) uses.
///
/// The skeleton reads only `session_id`, for its log lines. The other two are
/// named now because the tasks that follow need them (`storage` for the ring,
/// the cursors and the votes; `bridge` for the turn-transition emits) and
/// threading a field in one task at a time churns every construction site. That
/// is the whole justification — so if one of those tasks lands without touching
/// a field here, the field should come out rather than sit here unread.
///
/// Nothing else belongs here yet. In particular there is no `SessionHandle` and
/// no participant stdin: see the delivery note in the module doc.
pub struct SequencerDeps {
    /// The session whose turn cycle this task runs. Every ring, cursor and
    /// consensus query is scoped by it.
    pub session_id: Arc<str>,
    /// The channel is the transport: rows, cursors, done votes and the roster
    /// all live in storage, so this is where a turn's context comes from.
    pub storage: Storage,
    /// Event surface back to the UI.
    pub bridge: Arc<SignalingBridge>,
}

/// A wake for the sequencer.
///
/// **Payload-free on purpose.** A command says that something happened; WHAT
/// happened is a row in `messages`, and the sequencer reads rows from cursors.
/// Carrying the text in the command instead would put a second copy of it in
/// flight with no row identity behind it — the thing this batch's receipt work
/// removed.
///
/// `TurnComplete` carries no participant for a narrower reason: the sequencer
/// hands out the turn, so it already knows whose it is. If the task that
/// implements it needs to tell a stale completion (one from a turn that was
/// paused or killed) from the live one, that is when the field earns its place.
/// There are no send sites yet, so adding it then costs nothing.
#[derive(Debug)]
pub enum SequencerCommand {
    /// The participant holding the turn finished it — advance the ring.
    TurnComplete,
    /// The user posted to the channel. Resets the cycle to the first active
    /// participant.
    UserMessage,
    /// Stop: hold the cycle where it stands, hand out no further turns.
    Pause,
    /// Release a [`Pause`](Self::Pause) and continue the cycle.
    Resume,
}

/// Run the turn sequencer for one session.
///
/// Returns when `rx` closes. See the module doc for what this does NOT do yet —
/// which is everything except read its channel.
pub async fn run_sequencer(deps: SequencerDeps, mut rx: mpsc::Receiver<SequencerCommand>) {
    debug!(session = %deps.session_id, "sequencer: started");
    while let Some(cmd) = rx.recv().await {
        // One arm per variant rather than a catch-all: each arm is where its
        // own task lands, and a catch-all would let a variant added later be
        // absorbed by an arm that was never written for it.
        match cmd {
            SequencerCommand::TurnComplete => {
                debug!(session = %deps.session_id, "sequencer: TurnComplete (no-op)");
            }
            SequencerCommand::UserMessage => {
                debug!(session = %deps.session_id, "sequencer: UserMessage (no-op)");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::task::JoinHandle;

    /// Every await in this file carries this deadline.
    ///
    /// Not stylistic. A bare `.await` on an event that stopped arriving turns a
    /// deleted emit into a hung test instead of a failing one — earlier in this
    /// batch exactly that shape cost seven minutes per run before anything was
    /// printed. Two seconds is orders of magnitude above what an in-process
    /// `recv()` needs to observe a closed channel.
    const DEADLINE: Duration = Duration::from_secs(2);

    async fn deps() -> SequencerDeps {
        SequencerDeps {
            session_id: "s1".into(),
            storage: Storage::memory().await.unwrap(),
            bridge: SignalingBridge::new(),
        }
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

    #[tokio::test]
    async fn the_sequencer_exits_when_its_control_channel_closes() {
        // Router-inventory #20, carried forward: the task must end when its
        // last sender goes (session end), not linger holding a session's worth
        // of state alive.
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps().await, rx));
        drop(tx);
        assert!(
            exited(task).await,
            "sequencer must exit on session end, not linger"
        );
    }

    #[tokio::test]
    async fn every_command_is_consumed_and_the_loop_comes_back_for_more() {
        // "No-op" has to mean the loop CONSUMED the command and looped, not
        // that the first one ended the task or panicked an arm nobody wrote.
        // Buffered sends are drained before `recv()` reports the close, so
        // reaching the exit is proof all four were handled.
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps().await, rx));
        for cmd in [
            SequencerCommand::TurnComplete,
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
