//! Tauri command layer. Each `#[tauri::command]` is a thin wrapper over
//! existing `SignalingBridge` / `Storage` methods — no business logic lives
//! here. Errors map to [`error::AppError`], a `Serialize + Type` enum the
//! frontend can match on via `tauri-specta`-generated TypeScript bindings.
//!
//! Commands are domain-grouped:
//! - [`sessions`] — session CRUD + lifecycle
//! - [`messages`] — chronological chat fetch
//! - [`agent_configs`] — Settings page per-agent provider/model/token rows
//! - [`cl`] — Context Library index + folder search, audit, rescan
//! - [`questions`] — pending choices + resolve
//! - [`docs`] — session documents (IPAV tabs)
//!
//! Batch 4's `main.rs` wires the full set into `tauri::Builder::default()
//! .invoke_handler(tauri_specta_gen::builder().build())`.

pub mod agent_configs;
pub mod claude_config;
pub mod cl;
pub mod docs;
pub mod error;
pub mod messages;
pub mod models;
pub mod plugins;
pub mod policy;
pub mod questions;
pub mod screenshot;
pub mod sessions;
pub mod tool_gate;

pub use error::AppError;
