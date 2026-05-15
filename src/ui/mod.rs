//! Slint bindings + view-model adapters.
//!
//! `setup_ui` wires the Slint global `AppState` callbacks to the Rust
//! `core::AppState`, refreshes Slint models from storage, and pumps
//! signaling events into the UI.

pub mod view_model;

pub use view_model::install_view_model;
