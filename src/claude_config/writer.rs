//! Write-back to the user's real `settings.json` (global edits — affect both
//! the user's own interactive Claude AND the agents that inherit it).
//!
//! Every write is a read-modify-write that **preserves all other keys** (incl.
//! secrets like MCP bearer tokens) — we round-trip the whole object and only
//! touch the targeted key. A malformed `settings.json` is a hard error (we must
//! never clobber a file the user has a typo in).

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

/// Read settings.json into an object. Missing file → empty object. Malformed →
/// error (so a write never silently destroys an unparseable file's contents).
fn read_object(path: &Path) -> Result<Map<String, Value>> {
    match std::fs::read_to_string(path) {
        Ok(body) => match serde_json::from_str::<Value>(&body)
            .with_context(|| format!("{} is not valid JSON", path.display()))?
        {
            Value::Object(m) => Ok(m),
            _ => bail!("{} is not a JSON object", path.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn write_object(path: &Path, root: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut body = serde_json::to_string_pretty(root).context("serializing settings.json")?;
    body.push('\n');
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Set (or remove, when `value` is `None`) a key in settings.json, preserving
/// every other key. Supports one level of dotting (e.g. `env.FOO`); a parent
/// left empty by a removal is pruned.
pub fn set_value(config_dir: &Path, key: &str, value: Option<Value>) -> Result<()> {
    let path = settings_path(config_dir);
    let mut root = read_object(&path)?;
    if let Some((parent, child)) = key.split_once('.') {
        if value.is_none() && !root.contains_key(parent) {
            return write_object(&path, &root); // nothing to remove
        }
        let entry = root
            .entry(parent.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        let obj = entry.as_object_mut().expect("just ensured object");
        match value {
            Some(v) => {
                obj.insert(child.to_string(), v);
            }
            None => {
                obj.remove(child);
            }
        }
        if obj.is_empty() {
            root.remove(parent);
        }
    } else {
        match value {
            Some(v) => {
                root.insert(key.to_string(), v);
            }
            None => {
                root.remove(key);
            }
        }
    }
    write_object(&path, &root)
}

/// Convenience: set/remove a string-valued key.
pub fn set_string(config_dir: &Path, key: &str, value: Option<String>) -> Result<()> {
    set_value(config_dir, key, value.map(Value::String))
}

/// Convenience: set/remove a bool-valued key.
pub fn set_bool(config_dir: &Path, key: &str, value: Option<bool>) -> Result<()> {
    set_value(config_dir, key, value.map(Value::Bool))
}

/// Enable/disable a plugin in `enabledPlugins` (or remove the entry → inherit
/// marketplace default). Keyed by `name@marketplace`, which may not be dotted.
pub fn set_plugin_enabled(config_dir: &Path, key: &str, enabled: Option<bool>) -> Result<()> {
    let path = settings_path(config_dir);
    let mut root = read_object(&path)?;
    let entry = root
        .entry("enabledPlugins".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    let obj = entry.as_object_mut().expect("just ensured object");
    match enabled {
        Some(b) => {
            obj.insert(key.to_string(), Value::Bool(b));
        }
        None => {
            obj.remove(key);
        }
    }
    if obj.is_empty() {
        root.remove("enabledPlugins");
    }
    write_object(&path, &root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn read_back(dir: &Path) -> Value {
        let body = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        serde_json::from_str(&body).unwrap()
    }

    #[test]
    fn set_string_preserves_other_keys_and_secrets() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{ "voiceEnabled": true, "mcpServers": { "x": { "headers": { "Authorization": "Bearer SECRET" } } } }"#,
        )
        .unwrap();
        set_string(dir.path(), "effortLevel", Some("xhigh".into())).unwrap();
        let v = read_back(dir.path());
        assert_eq!(v["effortLevel"], "xhigh");
        assert_eq!(v["voiceEnabled"], true);
        // The secret is round-tripped untouched.
        assert_eq!(
            v["mcpServers"]["x"]["headers"]["Authorization"],
            "Bearer SECRET"
        );
    }

    #[test]
    fn set_nested_env_key_creates_and_removes_parent() {
        let dir = TempDir::new().unwrap();
        set_string(
            dir.path(),
            "env.CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            Some("32000".into()),
        )
        .unwrap();
        assert_eq!(read_back(dir.path())["env"]["CLAUDE_CODE_MAX_OUTPUT_TOKENS"], "32000");
        // Removing the only env key prunes the now-empty env object.
        set_string(dir.path(), "env.CLAUDE_CODE_MAX_OUTPUT_TOKENS", None).unwrap();
        assert!(read_back(dir.path()).get("env").is_none());
    }

    #[test]
    fn set_bool_and_remove_scalar() {
        let dir = TempDir::new().unwrap();
        set_bool(dir.path(), "alwaysThinkingEnabled", Some(false)).unwrap();
        assert_eq!(read_back(dir.path())["alwaysThinkingEnabled"], false);
        set_bool(dir.path(), "alwaysThinkingEnabled", None).unwrap();
        assert!(read_back(dir.path()).get("alwaysThinkingEnabled").is_none());
    }

    #[test]
    fn plugin_toggle_roundtrip() {
        let dir = TempDir::new().unwrap();
        set_plugin_enabled(dir.path(), "warp@mkt", Some(false)).unwrap();
        assert_eq!(read_back(dir.path())["enabledPlugins"]["warp@mkt"], false);
        set_plugin_enabled(dir.path(), "warp@mkt", None).unwrap();
        assert!(read_back(dir.path()).get("enabledPlugins").is_none());
    }

    #[test]
    fn malformed_settings_errors_without_clobber() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{ bad json ]").unwrap();
        let err = set_string(dir.path(), "effortLevel", Some("high".into()));
        assert!(err.is_err());
        // Original content is untouched.
        let raw = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert_eq!(raw, "{ bad json ]");
    }
}
