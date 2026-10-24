//! Host-owned turn boundary for changing a live session's participants.
//!
//! This is scheduling authority, not authorization to edit a roster. The caller
//! still validates/claims the durable change and retires the affected process.
use super::sequencer::SequencerCommand;
use crate::agents::ParticipantInput;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub struct BoundaryRequest {
    pub(crate) operation: String,
    pub(crate) token: String,
    pub(crate) alive: Arc<AtomicBool>,
    pub(crate) reply: oneshot::Sender<Result<(), String>>,
}

#[derive(Debug)]
pub enum BoundaryCommand {
    Reserve(BoundaryRequest),
    Bind {
        token: String,
        participant_id: i64,
        binding: Option<(ParticipantInput, Arc<AtomicU64>)>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Release {
        token: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Internal replay fence; a stale fence cannot release another owner.
    Replayed {
        token: String,
    },
    Continue {
        user_spoke: bool,
        restarts_rotation: bool,
    },
}

/// Cancellation is observable even if the command queue is full. Constructed
/// BEFORE sending/awaiting the acknowledgement, so cancellation at that await
/// cannot leave an ownerless granted boundary. A lost granted owner fails closed.
#[derive(Debug)]
pub struct RosterBoundary {
    tx: mpsc::Sender<SequencerCommand>,
    token: String,
    alive: Arc<AtomicBool>,
}

impl RosterBoundary {
    pub(crate) async fn acquire(
        tx: mpsc::Sender<SequencerCommand>,
        operation: String,
    ) -> Result<Self, String> {
        let guard = Self {
            tx,
            token: uuid::Uuid::new_v4().to_string(),
            alive: Arc::new(AtomicBool::new(true)),
        };
        let (reply, ack) = oneshot::channel();
        guard
            .send(BoundaryCommand::Reserve(BoundaryRequest {
                operation,
                token: guard.token.clone(),
                alive: guard.alive.clone(),
                reply,
            }))
            .await?;
        ack.await
            .map_err(|_| "The session stopped before reserving its turn boundary".to_string())??;
        Ok(guard)
    }

    async fn send(&self, command: BoundaryCommand) -> Result<(), String> {
        self.tx
            .send(SequencerCommand::RosterBoundary(command))
            .await
            .map_err(|_| "The session's turn ring has stopped".to_string())
    }

    /// Install both halves together, without waking the process. `None` removes
    /// a binding; it does not delete participant history or pending gates.
    pub async fn bind(
        &self,
        participant_id: i64,
        binding: Option<(ParticipantInput, Arc<AtomicU64>)>,
    ) -> Result<(), String> {
        let (reply, ack) = oneshot::channel();
        self.send(BoundaryCommand::Bind {
            token: self.token.clone(),
            participant_id,
            binding,
            reply,
        })
        .await?;
        ack.await
            .map_err(|_| "The session stopped while installing the participant".to_string())?
    }

    /// Call only after publication or a completed rollback. Dropping instead
    /// keeps the ring stopped: a partial mutation must never resume by accident.
    pub async fn release(self) -> Result<(), String> {
        let (reply, ack) = oneshot::channel();
        self.send(BoundaryCommand::Release {
            token: self.token.clone(),
            reply,
        })
        .await?;
        ack.await
            .map_err(|_| "The session stopped while releasing the turn boundary".to_string())?
    }
}

impl Drop for RosterBoundary {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

#[derive(Default)]
pub(crate) struct BoundaryState {
    pub operation: String,
    pub token: String,
    pub alive: Arc<AtomicBool>,
    pub reply: Option<oneshot::Sender<Result<(), String>>>,
    pub granted: bool,
    pub abandoned: bool,
    pub replaying: bool,
    pub release_reply: Option<oneshot::Sender<Result<(), String>>>,
    pub queued: std::collections::VecDeque<SequencerCommand>,
    pub advance: Option<(bool, bool)>,
}

impl BoundaryState {
    pub fn new(request: BoundaryRequest) -> Self {
        Self {
            operation: request.operation,
            token: request.token,
            alive: request.alive,
            reply: Some(request.reply),
            ..Self::default()
        }
    }

    pub fn owns(&self, token: &str) -> bool {
        self.token == token
            && self.granted
            && !self.abandoned
            && !self.replaying
            && self.alive.load(Ordering::Acquire)
    }

    pub fn retain_advance(&mut self, user: bool, restart: bool) {
        let prior = self.advance.unwrap_or_default();
        self.advance = Some((prior.0 || user, prior.1 || restart));
    }
}
