//! Plugin module — manifest, catalog, loader, serving, registry, heartbeat.
//!
//! Plugin runtime v1 (`api_version: 1`):
//!
//! - Each plugin lives under `<data_dir>/plugins/<plugin_id>/` with a top-
//!   level `manifest.json` and a frontend bundle (entry HTML + assets).
//! - Bundles are served over the `bhq-plugin://` custom URI scheme
//!   ([`serve`] resolves + guards; the http glue is registered once at
//!   Builder time in `main.rs`). Only installed + ENABLED plugins are
//!   served — the [`registry::PluginRegistry`] enabled-cache is the sync
//!   source of truth, seeded from storage at boot.
//! - Plugins render as sandboxed iframes (`frontend/src/app/PluginHost.tsx`)
//!   and talk to the host over postMessage ONLY. The shell forwards invokes
//!   to the single Rust enforcement point (`tauri_cmd/plugin_api.rs`),
//!   which re-checks enabled + granted against the [`catalog`] per call.
//!   There is no Tauri ACL / capability-JSON path — plugins never call
//!   Tauri directly.
//! - The host-side [`heartbeat::Heartbeat`] state machine is fed by the
//!   frontend's 5s ping loop (`plugin_note_ping` / `plugin_note_pong`);
//!   the sweep loop in `main.rs` emits `plugin:crashed` when a mounted
//!   plugin misses three in a row, and PluginHost swaps in a fallback card.
//!
//! The authoring contract (manifest schema, catalog, RPC shapes) is
//! documented in `docs/PLUGINS.md`.

pub mod catalog;
pub mod heartbeat;
pub mod loader;
pub mod manifest;
pub mod registry;
pub mod serve;

pub use heartbeat::{Heartbeat, PluginStatus};
pub use loader::{LoadedPlugin, Loader};
pub use manifest::{PluginManifest, PluginSlot};
pub use registry::PluginRegistry;
