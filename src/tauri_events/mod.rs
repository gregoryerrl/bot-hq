//! Tauri events layer.
//!
//! Routes `SignalingBridge` events into typed Tauri events for the frontend.
//! `MessagePersisted` events carry IDs only (zero-delta bridge core), so the
//! hot path goes through [`batch_emitter::BatchEmitter`]: it accumulates
//! per-session `since_id` watermarks, batches across N=20 messages or a 50ms
//! window, fetches via the existing `storage::messages_for_session(sid,
//! since_id)` query, and hands the resulting `Vec<AgentMessage>` to an
//! emit-closure. Tauri runtime wiring (`app.emit(...)`) lives in Batch 4's
//! main.rs cut-over; this module is testable without a running Tauri app.
//!
//! Other `SignalingEvent` variants (PendingChoice, AwaitingUser,
//! ChoiceResolved, AgentAdvancePhase) are direct-emit (no batching).

pub mod batch_emitter;
pub mod bridge_subscriber;
pub mod fs_watcher;
pub mod types;

pub use batch_emitter::BatchEmitter;
pub use bridge_subscriber::spawn_subscriber;
pub use fs_watcher::spawn_fs_watcher;
pub use types::{AgentMessage, AwaitingUser, ChoiceResolvedEvent, PhaseChangedEvent};
