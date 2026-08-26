//! Session lifecycle, IPAV state, the turn ring and its per-participant pumps.
//!
//! `AppState` is the top-level handle the UI holds. It owns:
//! - persistent storage (sqlite)
//! - the signaling MCP bridge
//! - the per-session in-memory IPAV state
//! - the per-session live agent handles

pub mod activity;
mod broadcast;
pub use broadcast::post_system_notice;
pub mod mentions;
pub mod close_learnings;
pub mod pump;
pub mod ipav;
pub mod sequencer;
pub mod session;
pub mod state;
pub mod telemetry;
pub mod terminal;
pub mod webview_watchdog;
pub mod updates;
pub mod watchdog;
pub mod worktree;

pub use activity::{ActivityTracker, SessionActivity};
pub use close_learnings::{decide as decide_close_epilogue, Epilogue};
pub use ipav::{IpavPhase, IpavState};
pub use session::SessionHandle;
pub use state::AppState;
pub use terminal::{SessionTerminal, TerminalRegistry};
pub use watchdog::{run_stall_watchdog, AgentLiveness};
