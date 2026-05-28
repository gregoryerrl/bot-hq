//! Plugin module — manifest, loader, capability generation, heartbeat.
//!
//! Plugin runtime model (per the design doc):
//!
//! - Each plugin lives under `<data_dir>/plugins/<plugin_id>/` with a top-
//!   level `manifest.json` and a frontend bundle (entry HTML + assets).
//! - At app startup the [`loader::Loader`] scans the directory, parses
//!   manifests via [`manifest::PluginManifest::parse`], and registers a
//!   capability JSON per plugin via [`capabilities::CapabilityGen`].
//! - Plugins render as iframes inside the React shell at origin
//!   `https://plugin-<id>.localhost` (per-plugin origin via Tauri custom
//!   URI scheme). Iframe calls `window.__TAURI__.invoke('cmd', args)`;
//!   Tauri matches the origin against the capability `remote.urls` and
//!   allows/denies per the per-plugin permission list.
//! - The host-side [`heartbeat::Heartbeat`] watcher pings each iframe
//!   every 5s and expects a pong within 1s. Misses trip the recovery
//!   path (v1: surface a fallback; v2: exponential-backoff auto-restart
//!   before third-party plugin authors ship).
//!
//! The plugin backend is live (manifest parse, loader, capability gen,
//! heartbeat watcher); no bundled plugins ship yet. The React
//! PluginManager + future Discord/Clive plugins integrate against this
//! Rust-side surface.

pub mod capabilities;
pub mod heartbeat;
pub mod loader;
pub mod manifest;

#[cfg(test)]
mod iframe_ipc_test;

pub use capabilities::{CapabilityGen, PluginCapability};
pub use heartbeat::{Heartbeat, PluginStatus};
pub use loader::{LoadedPlugin, Loader};
pub use manifest::{PluginManifest, PluginSlot};
