//! Per-plugin Tauri capability JSON generation.
//!
//! Given a plugin manifest's `requested_capabilities` (e.g.
//! `["cl_index_search", "session_doc_search"]`), produce a [`PluginCapability`]
//! that Tauri can use to gate `invoke()` calls coming from the plugin's
//! iframe origin (`https://plugin-<id>.localhost`).
//!
//! Generation runs at startup via [`CapabilityGen::write_all`], which writes
//! `capabilities/plugin-<id>.json` next to the main capability JSON. Tauri
//! picks these up via the `tauri.conf.json` capability glob.

use crate::plugins::loader::LoadedPlugin;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Tauri v2 capability JSON shape for a single plugin iframe origin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginCapability {
    pub identifier: String,
    pub description: String,
    pub windows: Vec<String>,
    pub webviews: Vec<String>,
    pub remote: RemoteCapability,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteCapability {
    pub urls: Vec<String>,
}

pub struct CapabilityGen;

impl CapabilityGen {
    /// Build the capability JSON for a single plugin.
    pub fn for_plugin(plugin: &LoadedPlugin) -> PluginCapability {
        let id = &plugin.manifest.id;
        let permissions = plugin
            .manifest
            .requested_capabilities
            .iter()
            .map(|cap| format!("allow-{}", cap.replace('_', "-")))
            .collect();
        PluginCapability {
            identifier: format!("plugin-{id}-capability"),
            description: format!("capability set for plugin {id}"),
            windows: vec!["main".to_string()],
            webviews: vec!["main".to_string()],
            remote: RemoteCapability {
                urls: vec![format!("https://plugin-{id}.localhost/*")],
            },
            permissions,
        }
    }

    /// Write per-plugin capability JSONs to `<capabilities_dir>/plugin-<id>.json`.
    /// Idempotent — overwrites existing files.
    pub fn write_all(plugins: &[LoadedPlugin], capabilities_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(capabilities_dir).with_context(|| {
            format!(
                "creating capabilities dir {}",
                capabilities_dir.display()
            )
        })?;
        for plugin in plugins {
            let cap = Self::for_plugin(plugin);
            let out = capabilities_dir.join(format!("plugin-{}.json", plugin.manifest.id));
            let body = serde_json::to_string_pretty(&cap)
                .context("serializing capability JSON")?;
            std::fs::write(&out, body)
                .with_context(|| format!("writing capability file {}", out.display()))?;
        }
        Ok(())
    }

    /// Check if the given command is allowed by the plugin's capability set.
    /// Used by the iframe origin → command dispatch path.
    pub fn is_command_allowed(plugin: &LoadedPlugin, command: &str) -> bool {
        let cap = Self::for_plugin(plugin);
        let target = format!("allow-{}", command.replace('_', "-"));
        cap.permissions.iter().any(|p| p == &target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::PluginManifest;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn test_plugin(id: &str, caps: &[&str]) -> LoadedPlugin {
        LoadedPlugin {
            manifest: PluginManifest {
                id: id.to_string(),
                name: format!("Test {id}"),
                version: "0.1.0".to_string(),
                entry: "i.html".to_string(),
                requested_capabilities: caps.iter().map(|s| s.to_string()).collect(),
                slots: vec![],
            },
            dir: PathBuf::from("/tmp/notreal"),
        }
    }

    #[test]
    fn capability_lists_only_requested_commands() {
        let p = test_plugin("discord", &["cl_index_search"]);
        let cap = CapabilityGen::for_plugin(&p);
        assert!(cap.permissions.contains(&"allow-cl-index-search".to_string()));
        assert!(!cap.permissions.contains(&"allow-create-session".to_string()));
    }

    #[test]
    fn capability_scopes_remote_url_to_per_plugin_origin() {
        let p = test_plugin("clive", &[]);
        let cap = CapabilityGen::for_plugin(&p);
        assert_eq!(cap.remote.urls, vec!["https://plugin-clive.localhost/*"]);
    }

    #[test]
    fn is_command_allowed_honors_capability_set() {
        let p = test_plugin("x", &["cl_index_search"]);
        assert!(CapabilityGen::is_command_allowed(&p, "cl_index_search"));
        assert!(!CapabilityGen::is_command_allowed(&p, "create_session"));
    }

    #[test]
    fn write_all_emits_per_plugin_files() {
        let tmp = TempDir::new().unwrap();
        let plugins = vec![test_plugin("discord", &["cl_index_search"])];
        CapabilityGen::write_all(&plugins, tmp.path()).unwrap();
        let out = tmp.path().join("plugin-discord.json");
        assert!(out.exists());
        let body = std::fs::read_to_string(&out).unwrap();
        let parsed: PluginCapability = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.identifier, "plugin-discord-capability");
    }
}
