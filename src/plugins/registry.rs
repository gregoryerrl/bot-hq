//! Plugin runtime registry — the long-lived disk/heartbeat state shared
//! across the plugin Tauri commands.

use super::{Heartbeat, Loader};
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Tauri-managed plugin runtime state. Wraps the disk Loader (re-scanned
/// after every mutation), the long-lived Heartbeat (survives reloads), and
/// the enabled-id cache (the sync source of truth for the `bhq-plugin://`
/// scheme handler, which can't await the DB).
///
/// Has no Tauri dependency itself — the command layer wraps it in
/// `tauri::State` at registration time.
pub struct PluginRegistry {
    pub loader: Mutex<Loader>,
    pub heartbeat: Arc<Heartbeat>,
    pub data_dir: PathBuf,
    /// Ids of ENABLED plugins. Seeded from storage at boot; kept in sync by
    /// the install / enable / disable / uninstall commands.
    enabled: Mutex<HashSet<String>>,
}

impl PluginRegistry {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        let loader = Loader::scan(&data_dir)?;
        Ok(Self {
            loader: Mutex::new(loader),
            heartbeat: Arc::new(Heartbeat::new()),
            data_dir,
            enabled: Mutex::new(HashSet::new()),
        })
    }

    /// Re-scan disk into the loader. Call after any mutation (install,
    /// uninstall, enable, disable) so subsequent reads see the new state.
    pub fn reload(&self) -> Result<()> {
        let mut g = self.loader.lock().unwrap_or_else(|p| p.into_inner());
        *g = Loader::scan(&self.data_dir)?;
        Ok(())
    }

    /// Replace the whole enabled set (boot seed from storage).
    pub fn set_enabled_ids(&self, ids: HashSet<String>) {
        let mut g = self.enabled.lock().unwrap_or_else(|p| p.into_inner());
        *g = ids;
    }

    /// Flip one plugin in the enabled cache (install/enable → true,
    /// disable/uninstall → false).
    pub fn set_enabled(&self, plugin_id: &str, enabled: bool) {
        let mut g = self.enabled.lock().unwrap_or_else(|p| p.into_inner());
        if enabled {
            g.insert(plugin_id.to_string());
        } else {
            g.remove(plugin_id);
        }
    }

    pub fn is_enabled(&self, plugin_id: &str) -> bool {
        let g = self.enabled.lock().unwrap_or_else(|p| p.into_inner());
        g.contains(plugin_id)
    }

    /// Snapshot of the enabled set (what the scheme handler resolves against).
    pub fn enabled_ids(&self) -> HashSet<String> {
        let g = self.enabled.lock().unwrap_or_else(|p| p.into_inner());
        g.clone()
    }
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
    fn enabled_cache_flips_and_snapshots() {
        let tmp = TempDir::new().unwrap();
        let reg = PluginRegistry::new(tmp.path().to_path_buf()).unwrap();
        assert!(!reg.is_enabled("notes"));

        reg.set_enabled("notes", true);
        assert!(reg.is_enabled("notes"));
        assert!(reg.enabled_ids().contains("notes"));

        reg.set_enabled("notes", false);
        assert!(!reg.is_enabled("notes"));

        reg.set_enabled_ids(["a".to_string(), "b".to_string()].into_iter().collect());
        assert!(reg.is_enabled("a") && reg.is_enabled("b"));
        assert_eq!(reg.enabled_ids().len(), 2);
    }

    #[test]
    fn heartbeat_outlives_reload() {
        let tmp = TempDir::new().unwrap();
        let reg = PluginRegistry::new(tmp.path().to_path_buf()).unwrap();
        reg.heartbeat.register("notes");
        reg.reload().unwrap();
        assert!(reg.heartbeat.status_of("notes").is_some());
    }
}
