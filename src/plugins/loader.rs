//! Plugin loader. Scans `<data_dir>/plugins/` at startup.
//!
//! Each subdirectory is treated as a plugin; the loader looks for
//! `manifest.json` inside it. Invalid manifests are logged + skipped (don't
//! brick the app over a broken plugin install).

use crate::plugins::manifest::PluginManifest;
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Loader {
    plugins_root: PathBuf,
    loaded: Vec<LoadedPlugin>,
}

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub dir: PathBuf,
}

impl Loader {
    /// Scan `<data_dir>/plugins/`. Missing dir is fine — returns empty loader.
    pub fn scan(data_dir: &Path) -> Result<Self> {
        let plugins_root = data_dir.join("plugins");
        let mut loaded = Vec::new();

        if !plugins_root.is_dir() {
            return Ok(Self {
                plugins_root,
                loaded,
            });
        }

        let entries = std::fs::read_dir(&plugins_root)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("manifest.json");
            if !manifest_path.is_file() {
                tracing::warn!(
                    plugin_dir = %path.display(),
                    "skipping: no manifest.json"
                );
                continue;
            }
            match std::fs::read_to_string(&manifest_path) {
                Ok(body) => match PluginManifest::parse(&body) {
                    Ok(manifest) => {
                        loaded.push(LoadedPlugin {
                            manifest,
                            dir: path,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = ?e,
                            manifest = %manifest_path.display(),
                            "skipping plugin: invalid manifest"
                        );
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        manifest = %manifest_path.display(),
                        "skipping plugin: unable to read manifest"
                    );
                }
            }
        }
        loaded.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));

        Ok(Self {
            plugins_root,
            loaded,
        })
    }

    pub fn plugins_root(&self) -> &Path {
        &self.plugins_root
    }

    pub fn loaded(&self) -> &[LoadedPlugin] {
        &self.loaded
    }

    pub fn get(&self, id: &str) -> Option<&LoadedPlugin> {
        self.loaded.iter().find(|p| p.manifest.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_manifest(dir: &Path, id: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let body = format!(
            r#"{{
                "id": "{id}",
                "name": "Test {id}",
                "version": "0.1.0",
                "entry": "index.html",
                "requested_capabilities": []
            }}"#
        );
        std::fs::write(dir.join("manifest.json"), body).unwrap();
    }

    #[test]
    fn scan_returns_empty_when_root_missing() {
        let tmp = TempDir::new().unwrap();
        let loader = Loader::scan(tmp.path()).unwrap();
        assert!(loader.loaded().is_empty());
    }

    #[test]
    fn scan_finds_valid_plugin() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugins").join("discord");
        write_manifest(&plugin_dir, "discord");

        let loader = Loader::scan(tmp.path()).unwrap();
        assert_eq!(loader.loaded().len(), 1);
        assert_eq!(loader.get("discord").unwrap().manifest.id, "discord");
    }

    #[test]
    fn scan_skips_dirs_without_manifest() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("plugins").join("missing")).unwrap();
        write_manifest(&tmp.path().join("plugins").join("good"), "good");

        let loader = Loader::scan(tmp.path()).unwrap();
        assert_eq!(loader.loaded().len(), 1);
        assert_eq!(loader.loaded()[0].manifest.id, "good");
    }

    #[test]
    fn scan_skips_invalid_manifest() {
        let tmp = TempDir::new().unwrap();
        let bad_dir = tmp.path().join("plugins").join("bad");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("manifest.json"), "this is not json").unwrap();

        let loader = Loader::scan(tmp.path()).unwrap();
        assert!(loader.loaded().is_empty());
    }
}
