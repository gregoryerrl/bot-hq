//! Tauri command layer. Each `#[tauri::command]` is a thin wrapper over
//! existing `SignalingBridge` / `Storage` methods — no business logic lives
//! here. Errors map to [`error::AppError`], a `Serialize + Type` enum the
//! frontend can match on via `tauri-specta`-generated TypeScript bindings.
//!
//! Commands are domain-grouped:
//! - [`sessions`] — session CRUD + lifecycle
//! - [`messages`] — chronological chat fetch
//! - [`agent_configs`] — Settings page per-agent provider/model/token rows
//! - [`models`] — saved-model registry + default-model app setting
//! - [`cl`] — Context Library index + folder search, audit, rescan
//! - [`claude_config`] — Claude Config surface (global + per-agent overrides)
//! - [`questions`] — pending choices + resolve
//! - [`docs`] — session documents (IPAV tabs)
//! - [`policy`] — session/global/project policy get + set
//! - [`tool_gate`] — global + per-session Tool Gate keyword lists
//! - [`plugins`] — plugin install/enable/disable
//! - [`updates`] — GitHub-release update check (check-and-notify)
//! - [`screenshot`] — webview capture for agent-driven UI testing
//! - [`error`] — the shared [`error::AppError`] type
//!
//! `tauri_specta_gen::builder()` wires the full set into the Tauri builder's
//! invoke handler at startup (`main.rs`).

pub mod agent_configs;
pub mod claude_config;
pub mod cl;
pub mod docs;
pub mod error;
pub mod findings;
pub mod messages;
pub mod models;
pub mod plugin_api;
pub mod plugins;
pub mod policy;
pub mod screenshot;
pub mod sessions;
pub mod tool_gate;
pub mod tray;
pub mod updates;

pub use error::AppError;
