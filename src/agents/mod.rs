//! claude-code subprocess management + stream-json IO.
//!
//! See `docs/stream-json-events.md` for the empirical schema this code is
//! built against, and `docs/decisions.md#mcp-server` for transport choices.

pub mod capability;
pub mod capability_prompt;
pub mod events;
pub mod general_rules;
pub mod input;
pub mod llm_proxy;
pub mod prompts;
pub mod protocol;
pub mod spawn;

pub use capability::{Capability, CapabilitySet, ResolvedCapabilities};
pub use capability_prompt::{PeerFact, RosterFacts};
pub use general_rules::GENERAL_RULES;
pub use protocol::{OutgoingUserMessage, StreamEvent};
pub use spawn::{
    reconcile_spawn_knobs, spawn_agent, spawn_supervised_agent, AgentEvent, AgentHandle,
    AgentHealth, ParticipantInput, RetryPolicy, SpawnConfig, NO_INTERRUPT_EPOCH,
};
