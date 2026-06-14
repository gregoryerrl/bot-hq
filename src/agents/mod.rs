//! claude-code subprocess management + stream-json IO.
//!
//! See `docs/stream-json-events.md` for the empirical schema this code is
//! built against, and `docs/decisions.md#mcp-server` for transport choices.

pub mod events;
pub mod general_rules;
pub mod input;
pub mod llm_proxy;
pub mod prompts;
pub mod protocol;
pub mod spawn;

pub use general_rules::GENERAL_RULES;
pub use prompts::role_for;
pub use protocol::{OutgoingUserMessage, StreamEvent};
pub use spawn::{
    spawn_agent, spawn_supervised_agent, AgentEvent, AgentHandle, AgentHealth, RetryPolicy,
    SpawnConfig,
};
