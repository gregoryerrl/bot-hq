//! Plugin runtime registry — the long-lived disk/heartbeat state shared
//! across the plugin Tauri commands.

use super::{Heartbeat, Loader};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Tauri-managed plugin runtime state. Wraps the disk Loader (re-scanned
/// after every mutation), the long-lived Heartbeat (survives reloads), and
/// the on-disk paths used by install / capability generation.
///
/// Has no Tauri dependency itself — the command layer wraps it in
/// `tauri::State` at registration time.
pub struct PluginRegistry {
    pub loader: Mutex<Loader>,
    pub heartbeat: Arc<Heartbeat>,
    pub data_dir: PathBuf,
    pub capabilities_dir: PathBuf,
}

impl PluginRegistry {
    pub fn new(data_dir: PathBuf, capabilities_dir: PathBuf) -> Result<Self> {
        let loader = Loader::scan(&data_dir)?;
        Ok(Self {
            loader: Mutex::new(loader),
            heartbeat: Arc::new(Heartbeat::new()),
            data_dir,
            capabilities_dir,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn new_on_empty_dir_yields_empty_loader() {
        let tmp = TempDir::new().unwrap();
        let caps = tmp.path().join("capabilities");
        let reg = PluginRegistry::new(tmp.path().to_path_buf(), caps).unwrap();
        let loaded = reg.loader.lock().unwrap();
        assert!(loaded.loaded().is_empty());
    }

    #[test]
    fn reload_picks_up_new_plugin_on_disk() {
        let tmp = TempDir::new().unwrap();
        let caps = tmp.path().join("capabilities");
        let reg = PluginRegistry::new(tmp.path().to_path_buf(), caps).unwrap();
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
        let caps = tmp.path().join("capabilities");
        let reg = PluginRegistry::new(tmp.path().to_path_buf(), caps).unwrap();
        reg.heartbeat.register("notes");
        reg.reload().unwrap();
        assert!(reg.heartbeat.status_of("notes").is_some());
    }
}
