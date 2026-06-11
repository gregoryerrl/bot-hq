//! Session lifecycle, IPAV cache, duo coordination.
//!
//! `AppState` is the top-level handle the UI holds. It owns:
//! - persistent storage (sqlite)
//! - the signaling MCP bridge
//! - the per-session in-memory IPAV state
//! - the per-session live agent handles

mod broadcast;
pub mod duo;
pub mod ipav;
pub mod session;
pub mod state;
pub mod updates;
pub mod worktree;

pub use broadcast::peer_forward_message;
pub use ipav::{IpavPhase, IpavState};
pub use session::{open_session, OpenSessionRequest, SessionHandle};
pub use state::AppState;
