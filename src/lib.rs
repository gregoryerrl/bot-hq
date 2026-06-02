//! bot-hq library crate. Exposes modules so integration tests in `tests/`
//! can import them via `use bot_hq::storage::...`.

pub mod agents;
pub mod claude_config;
pub mod core;
pub mod paths;
pub mod plugins;
pub mod policy;
pub mod signaling;
pub mod storage;
pub mod tauri_cmd;
pub mod tauri_events;
pub mod tauri_specta_gen;
