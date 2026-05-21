//! Embedded MCP server for UI-signaling tools.
//!
//! ARCHITECTURE.md "UI signaling — embedded MCP server" is the spec. We
//! expose exactly two tools to claude-code subprocesses:
//!
//! - `ask_user_choice(question, options)` — blocking; returns the picked option
//! - `mark_awaiting_user(reason)` — non-blocking; flags the session
//!
//! Transport: streamable HTTP, one server in the GUI process. Each spawned
//! agent gets a per-agent `mcp-config.json` pointing at
//! `http://127.0.0.1:<port>/sessions/<session_id>/<agent>/mcp` so the bridge
//! can attribute tool calls to the right (session, agent).
//!
//! **Autonomous decision** (logged in PROGRESS.md): we deviate from
//! `docs/decisions.md#mcp-server`'s stdio + UDS-bridge sketch. HTTP in-process
//! is simpler — no re-exec, no IPC framing, direct AppState access. The
//! decision doc itself flagged HTTP as the "promote if IPC gets hairy" fallback.

mod bridge;
pub mod external_jsonrpc;
pub mod external_server;
mod jsonrpc;
pub mod protocol;
mod response;
mod server;

pub use bridge::{PendingChoice, ResolveOutcome, SignalingBridge, SignalingEvent};
pub use external_server::{start_external_server, ExternalServer};
pub use server::{
    default_user_settings_paths, load_user_mcp_servers, mcp_config_json, start_signaling_server,
    SignalingServer,
};
