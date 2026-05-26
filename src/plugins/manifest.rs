//! Plugin manifest parsing.
//!
//! Manifests are JSON files at `<data_dir>/plugins/<plugin_id>/manifest.json`.
//! Schema (v1):
//!
//! ```json
//! {
//!   "id": "discord",
//!   "name": "Discord Bridge",
//!   "version": "0.1.0",
//!   "entry": "index.html",
//!   "requested_capabilities": ["cl_index_search", "session_doc_search"],
//!   "slots": [
//!     { "slot_name": "sidebar.bottom", "panel_route": null },
//!     { "slot_name": null, "panel_route": "/plugins/discord" }
//!   ]
//! }
//! ```
//!
//! The `id` field doubles as the URL host (`plugin-<id>.localhost`) so it
//! must be a valid hostname segment: lowercase ASCII, digits, and `-`.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry: String,
    #[serde(default)]
    pub requested_capabilities: Vec<String>,
    #[serde(default)]
    pub slots: Vec<PluginSlot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct PluginSlot {
    /// React shell slot name (e.g., "sidebar.bottom"). `None` means the
    /// plugin contributes a dedicated panel via [`panel_route`] instead.
    pub slot_name: Option<String>,
    /// Frontend router path for a dedicated panel. `None` means slot-only.
    pub panel_route: Option<String>,
}

impl PluginManifest {
    /// Parse + validate a manifest JSON blob.
    pub fn parse(json: &str) -> Result<Self> {
        let m: Self = serde_json::from_str(json)
            .map_err(|e| anyhow!("invalid manifest json: {e}"))?;
        m.validate()?;
        Ok(m)
    }

    fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(anyhow!("manifest.id must not be empty"));
        }
        if !is_valid_id(&self.id) {
            return Err(anyhow!(
                "manifest.id must be lowercase alphanumeric or '-' (got: {})",
                self.id
            ));
        }
        if self.name.is_empty() {
            return Err(anyhow!("manifest.name must not be empty"));
        }
        if self.version.is_empty() {
            return Err(anyhow!("manifest.version must not be empty"));
        }
        if self.entry.is_empty() {
            return Err(anyhow!("manifest.entry must not be empty"));
        }
        for cap in &self.requested_capabilities {
            if cap.is_empty() || cap.contains(' ') {
                return Err(anyhow!(
                    "requested_capabilities entries must be tokens (got: {cap:?})"
                ));
            }
        }
        for slot in &self.slots {
            if slot.slot_name.is_none() && slot.panel_route.is_none() {
                return Err(anyhow!(
                    "each slot must specify slot_name OR panel_route"
                ));
            }
        }
        Ok(())
    }

    /// The expected iframe origin for this plugin.
    pub fn iframe_origin(&self) -> String {
        format!("https://plugin-{}.localhost", self.id)
    }
}

fn is_valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest_succeeds() {
        let json = r#"{
            "id": "discord",
            "name": "Discord Bridge",
            "version": "0.1.0",
            "entry": "index.html",
            "requested_capabilities": ["cl_index_search"]
        }"#;
        let m = PluginManifest::parse(json).unwrap();
        assert_eq!(m.id, "discord");
        assert_eq!(m.iframe_origin(), "https://plugin-discord.localhost");
    }

    #[test]
    fn parse_rejects_uppercase_id() {
        let json = r#"{
            "id": "Discord",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": []
        }"#;
        let err = PluginManifest::parse(json).unwrap_err();
        assert!(err.to_string().contains("lowercase"));
    }

    #[test]
    fn parse_rejects_id_with_dots() {
        let json = r#"{
            "id": "my.plugin",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": []
        }"#;
        assert!(PluginManifest::parse(json).is_err());
    }

    #[test]
    fn parse_rejects_id_starting_with_hyphen() {
        let json = r#"{
            "id": "-bad",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": []
        }"#;
        assert!(PluginManifest::parse(json).is_err());
    }

    #[test]
    fn parse_rejects_capability_with_space() {
        let json = r#"{
            "id": "x",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": ["cl_index search"]
        }"#;
        assert!(PluginManifest::parse(json).is_err());
    }

    #[test]
    fn parse_rejects_empty_slot() {
        let json = r#"{
            "id": "x",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": [],
            "slots": [ { "slot_name": null, "panel_route": null } ]
        }"#;
        assert!(PluginManifest::parse(json).is_err());
    }

    #[test]
    fn iframe_origin_uses_plugin_prefix() {
        let m = PluginManifest {
            id: "clive".to_string(),
            name: "Clive".to_string(),
            version: "0.1.0".to_string(),
            entry: "main.html".to_string(),
            requested_capabilities: vec![],
            slots: vec![],
        };
        assert_eq!(m.iframe_origin(), "https://plugin-clive.localhost");
    }
}
