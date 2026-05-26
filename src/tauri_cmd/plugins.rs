//! Plugin manager — Tauri command surface.
//!
//! Slice 2 of the PluginManager work: signatures + managed state so
//! tauri-specta regenerates the TS bindings. Bodies are `todo!()` — Slice 3
//! adds the install / uninstall / enable / disable logic over Storage +
//! Loader + Heartbeat.

use crate::plugins::{Heartbeat, Loader, PluginManifest, PluginStatus};
use crate::storage::Storage;
use crate::tauri_cmd::error::AppError;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Tauri-managed plugin runtime state. Wraps the disk Loader (re-scanned
/// after every mutation) and the long-lived Heartbeat (survives reloads).
pub struct PluginRegistry {
    pub loader: Mutex<Loader>,
    pub heartbeat: Arc<Heartbeat>,
    pub data_dir: PathBuf,
}

impl PluginRegistry {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        let loader = Loader::scan(&data_dir)?;
        Ok(Self {
            loader: Mutex::new(loader),
            heartbeat: Arc::new(Heartbeat::new()),
            data_dir,
        })
    }

    /// Re-scan disk into the loader. Call after any mutation (install,
    /// uninstall, enable, disable) so subsequent reads see the new state.
    pub fn reload(&self) -> Result<()> {
        let mut g = self.loader.lock().unwrap_or_else(|p| p.into_inner());
        *g = Loader::scan(&self.data_dir)?;
        Ok(())
    }
}

/// What the frontend sees for each installed plugin. Combines DB row state,
/// parsed manifest, and live heartbeat status.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InstalledPluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub status: PluginStatus,
    pub manifest: PluginManifest,
    pub dir_path: String,
    pub installed_at: String,
}

/// Install a plugin from a URL or local directory path.
///
/// Slice 3 implements:
/// - URL: fetch `manifest.json` first, validate, download bundle into
///   `<data_dir>/plugins/<id>/`.
/// - Path: read `<source>/manifest.json`, validate, copy directory.
/// Both branches insert a DB row + reload the registry + regenerate
/// capability JSON for Tauri.
#[tauri::command]
#[specta::specta]
pub async fn install_plugin(
    _storage: tauri::State<'_, Arc<Storage>>,
    _registry: tauri::State<'_, Arc<PluginRegistry>>,
    _source: String,
) -> Result<InstalledPluginView, AppError> {
    todo!("PluginManager Slice 3: install_plugin")
}

#[tauri::command]
#[specta::specta]
pub async fn list_installed_plugins(
    _storage: tauri::State<'_, Arc<Storage>>,
    _registry: tauri::State<'_, Arc<PluginRegistry>>,
) -> Result<Vec<InstalledPluginView>, AppError> {
    todo!("PluginManager Slice 3: list_installed_plugins")
}

#[tauri::command]
#[specta::specta]
pub async fn enable_plugin(
    _storage: tauri::State<'_, Arc<Storage>>,
    _registry: tauri::State<'_, Arc<PluginRegistry>>,
    _plugin_id: String,
) -> Result<(), AppError> {
    todo!("PluginManager Slice 3: enable_plugin")
}

#[tauri::command]
#[specta::specta]
pub async fn disable_plugin(
    _storage: tauri::State<'_, Arc<Storage>>,
    _registry: tauri::State<'_, Arc<PluginRegistry>>,
    _plugin_id: String,
) -> Result<(), AppError> {
    todo!("PluginManager Slice 3: disable_plugin")
}

#[tauri::command]
#[specta::specta]
pub async fn uninstall_plugin(
    _storage: tauri::State<'_, Arc<Storage>>,
    _registry: tauri::State<'_, Arc<PluginRegistry>>,
    _plugin_id: String,
) -> Result<(), AppError> {
    todo!("PluginManager Slice 3: uninstall_plugin")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn new_on_empty_dir_yields_empty_loader() {
        let tmp = TempDir::new().unwrap();
        let reg = PluginRegistry::new(tmp.path().to_path_buf()).unwrap();
        let loaded = reg.loader.lock().unwrap();
        assert!(loaded.loaded().is_empty());
    }

    #[test]
    fn reload_picks_up_new_plugin_on_disk() {
        let tmp = TempDir::new().unwrap();
        let reg = PluginRegistry::new(tmp.path().to_path_buf()).unwrap();
        assert!(reg.loader.lock().unwrap().loaded().is_empty());

        let plugin_dir = tmp.path().join("plugins").join("notes");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("manifest.json"),
            r#"{
                "id": "notes",
                "name": "Notes",
                "version": "0.1.0",
                "entry": "index.html",
                "requested_capabilities": []
            }"#,
        )
        .unwrap();

        reg.reload().unwrap();
        let loaded = reg.loader.lock().unwrap();
        assert_eq!(loaded.loaded().len(), 1);
        assert_eq!(loaded.loaded()[0].manifest.id, "notes");
    }

    #[test]
    fn heartbeat_outlives_reload() {
        let tmp = TempDir::new().unwrap();
        let reg = PluginRegistry::new(tmp.path().to_path_buf()).unwrap();
        reg.heartbeat.register("notes");
        reg.reload().unwrap();
        // After reload, the registered heartbeat entry must still be tracked.
        assert!(reg.heartbeat.status_of("notes").is_some());
    }
}
