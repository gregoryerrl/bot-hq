//! bot-hq library crate. Exposes modules so integration tests in `tests/`
//! can import them via `use bot_hq::storage::...`.

pub mod agents;
pub mod core;
pub mod paths;
pub mod policy;
pub mod signaling;
pub mod storage;
pub mod tauri_events;
pub mod tauri_specta_gen;
pub mod ui;

// Slint-generated UI types. Lives at the crate root so any module in the
// library can reference `AppWindow`, `AppState`, `SessionTile`, etc.
slint::include_modules!();
