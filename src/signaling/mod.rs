//! Embedded MCP servers for agent ↔ host signaling.
//!
//! Two in-process HTTP MCP servers live under this module: the **internal**
//! server (UI-signaling tools served to spawned claude-code agents) and the
//! **external** driver server (session-management tools for outside MCP
//! clients). The internal tool surface — `ask_user_choice`, `advance_phase`,
//! `request_approval`, `check_commit_message`, `cl_index_search`,
//! `session_doc_*`, `grant`/`revoke_session_permission`, `webview_*`, and
//! more — is defined by the descriptors in `protocol.rs`; see ARCHITECTURE.md
//! and README.md for the full list (26 internal + 21 external tools).
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
mod tool_args;
mod webview_js;

pub use bridge::{PendingChoice, ResolveOutcome, SignalingBridge, SignalingEvent};
pub use external_server::{start_external_server, ExternalServer};
pub use server::{
    default_user_settings_paths, load_user_mcp_servers, mcp_config_json, start_signaling_server,
    SignalingServer,
};
