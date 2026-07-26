//! Native Rust agent loop — bot-hq owning the loop instead of renting it from
//! `claude-code`.
//!
//! ## Why
//!
//! On a third-party API key the CLI buys nothing: subscription OAuth is bound
//! server-side to claude-code, so an agent on a gateway key pays for ~418 MB
//! RSS, an opaque loop, and context accounting we can only scrape. A native
//! loop computes `ContextUsage` from our OWN request accounting, gates tools
//! inline instead of through the exit-2 PreToolUse hook dance, and can enforce
//! a read root (which `Read`/`Grep`/`Glob` never reach today — they are not
//! `Bash`, so the Tool Gate never sees them).
//!
//! ## Why it is additive, not a rewrite
//!
//! [`AgentHandle`](crate::agents::spawn::AgentHandle) is a pure channel struct:
//! `core/duo.rs`, `core/router.rs`, the policy layer, the UI event path and the
//! context meter speak only [`AgentEvent`](crate::agents::spawn::AgentEvent) and
//! `OutgoingUserMessage`. Nothing downstream knows a subprocess exists. A native
//! spawner that returns an `AgentHandle` needs zero downstream changes.
//!
//! It also slots into the EXISTING retry supervisor: `supervise` is generic over
//! `FnMut(SpawnConfig) -> Future<Output = Result<AgentHandle>>`, ends an
//! incarnation on **event-channel closure** (not process death — `Exited` is
//! explicitly suppressed), and classifies failures purely from
//! `TurnComplete { is_error, api_error_status }`. Satisfying that contract is
//! the whole integration.
//!
//! ## Target
//!
//! v1 is **Rain (EYES)**. Brian's subscription pins him to the CLI anyway.
//!
//! ## Provenance
//!
//! Written against the public Anthropic Messages spec, each gateway's published
//! compatibility docs, the open MCP spec, and bot-hq's own `llm_proxy.rs` /
//! `events.rs` / `spawn.rs`. No part derives from any leaked source.

pub mod profile;
pub mod wire;

pub use profile::ProviderProfile;
pub use wire::{ParsedTurn, RequestSpec, ToolCall, ToolOutcome, TurnStep};
