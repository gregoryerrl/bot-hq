//! Per-agent **overrides** bot-hq applies to inherited Claude Code config at
//! spawn time, stored at `<data_dir>/claude-overrides.json` (0600).
//!
//! The spawn path (`agents::spawn::build_command` + `core::session`) merges the
//! resolved override for each agent into the `--settings` JSON / env / mcp-config
//! it already injects — so a user can disable a self-invoking skill (or a
//! plugin/MCP/effort) JUST for the agents without touching their own `~/.claude`.
//!
//! Feasibility per surface is documented in the design doc §3: skills
//! (`skillOverrides`), plugins (`enabledPlugins`), MCP (per-agent mcp-config),
//! effort/ultracode, and auto-memory/CLAUDE.md are cleanly per-spawn; granular
//! per-hook suppression is not, so it is intentionally absent here.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Per-skill visibility — mirrors claude-code's `skillOverrides` states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum SkillVisibility {
    /// Default — listed to the model + in the `/` menu.
    On,
    /// Name only listed to the model; still in the `/` menu.
    NameOnly,
    /// Hidden from the model (no auto-invoke); still manually invocable.
    UserInvocableOnly,
    /// Fully disabled (no auto-invoke, not in the `/` menu).
    Off,
}

impl SkillVisibility {
    /// The exact string claude-code expects in `skillOverrides`.
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillVisibility::On => "on",
            SkillVisibility::NameOnly => "name-only",
            SkillVisibility::UserInvocableOnly => "user-invocable-only",
            SkillVisibility::Off => "off",
        }
    }
}

/// The override set for one agent (or the `_all` fan-out default).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentOverride {
    /// skill name → visibility. Maps to `skillOverrides` in `--settings`.
    #[serde(default)]
    pub skills: BTreeMap<String, SkillVisibility>,
    /// plugin key (`name@marketplace`) → enabled. Maps to `enabledPlugins`.
    #[serde(default)]
    pub plugins: BTreeMap<String, bool>,
    /// MCP server name → forwarded. `false` removes it from the agent's mcp-config.
    #[serde(default)]
    pub mcp: BTreeMap<String, bool>,
    /// Effort level (low/medium/high/xhigh/max). Maps to `CLAUDE_CODE_EFFORT_LEVEL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// ultracode toggle. Maps to `"ultracode": true` in `--settings`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ultracode: Option<bool>,
    /// Disable auto-memory. Maps to `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_auto_memory: Option<bool>,
    /// Disable ALL CLAUDE.md autodiscovery. Maps to `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_claude_md: Option<bool>,
}

/// The full override store: a fan-out `_all` default plus per-agent entries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct ClaudeOverrides {
    #[serde(rename = "_all", default)]
    pub all: AgentOverride,
    #[serde(default)]
    pub brian: AgentOverride,
    #[serde(default)]
    pub rain: AgentOverride,
}

/// `<data_dir>/claude-overrides.json`.
pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("claude-overrides.json")
}

/// Load the override store. **FAIL-OPEN**: missing/unreadable/malformed → an
/// empty store (logged), never an error — a bad file must not brick spawn.
pub fn load_overrides(data_dir: &Path) -> ClaudeOverrides {
    let path = config_path(data_dir);
    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ClaudeOverrides::default(),
        Err(e) => {
            tracing::warn!(?e, path = %path.display(), "claude-overrides.json read failed; treating as empty");
            return ClaudeOverrides::default();
        }
    };
    match serde_json::from_str::<ClaudeOverrides>(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(?e, path = %path.display(), "claude-overrides.json parse failed; treating as empty");
            ClaudeOverrides::default()
        }
    }
}

/// Persist the override store (pretty JSON, 0600). Creates the data dir.
pub fn save_overrides(data_dir: &Path, store: &ClaudeOverrides) -> Result<()> {
    let path = config_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating data dir {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(store).context("serializing claude-overrides")?;
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    set_owner_only(&path);
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}

/// Resolve the effective override for `agent`: the `_all` default with the
/// per-agent entry layered on top (per-key, agent wins). Unknown agent → `_all`.
pub fn resolve_agent_overrides(store: &ClaudeOverrides, agent: &str) -> AgentOverride {
    let specific = match agent {
        "brian" => &store.brian,
        "rain" => &store.rain,
        _ => return store.all.clone(),
    };
    let mut merged = store.all.clone();
    merged.skills.extend(specific.skills.clone());
    merged.plugins.extend(specific.plugins.clone());
    merged.mcp.extend(specific.mcp.clone());
    if specific.effort.is_some() {
        merged.effort = specific.effort.clone();
    }
    if specific.ultracode.is_some() {
        merged.ultracode = specific.ultracode;
    }
    if specific.disable_auto_memory.is_some() {
        merged.disable_auto_memory = specific.disable_auto_memory;
    }
    if specific.disable_claude_md.is_some() {
        merged.disable_claude_md = specific.disable_claude_md;
    }
    merged
}

/// The partial settings-JSON object this override contributes — merged into the
/// spawn `--settings` payload alongside bot-hq's PreToolUse hook. Empty when the
/// override adds nothing.
pub fn settings_fragment(ov: &AgentOverride) -> Map<String, Value> {
    let mut out = Map::new();
    if !ov.skills.is_empty() {
        let map: Map<String, Value> = ov
            .skills
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.as_str().to_string())))
            .collect();
        out.insert("skillOverrides".into(), Value::Object(map));
    }
    if !ov.plugins.is_empty() {
        let map: Map<String, Value> = ov
            .plugins
            .iter()
            .map(|(k, v)| (k.clone(), Value::Bool(*v)))
            .collect();
        out.insert("enabledPlugins".into(), Value::Object(map));
    }
    if ov.ultracode == Some(true) {
        out.insert("ultracode".into(), Value::Bool(true));
    }
    out
}

/// Env vars this override contributes (effort / auto-memory / CLAUDE.md).
pub fn env_vars(ov: &AgentOverride) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(effort) = &ov.effort {
        if !effort.is_empty() {
            out.push(("CLAUDE_CODE_EFFORT_LEVEL".into(), effort.clone()));
        }
    }
    if ov.disable_auto_memory == Some(true) {
        out.push(("CLAUDE_CODE_DISABLE_AUTO_MEMORY".into(), "1".into()));
    }
    if ov.disable_claude_md == Some(true) {
        out.push(("CLAUDE_CODE_DISABLE_CLAUDE_MDS".into(), "1".into()));
    }
    out
}

/// MCP server names this override disables (set to `false`) — dropped from the
/// agent's forwarded mcp-config.
pub fn disabled_mcp(ov: &AgentOverride) -> Vec<String> {
    ov.mcp
        .iter()
        .filter(|(_, &enabled)| !enabled)
        .map(|(name, _)| name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_store_is_default() {
        let dir = tempdir().unwrap();
        assert_eq!(load_overrides(dir.path()), ClaudeOverrides::default());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempdir().unwrap();
        let mut store = ClaudeOverrides::default();
        store
            .brian
            .skills
            .insert("note".into(), SkillVisibility::UserInvocableOnly);
        store.brian.plugins.insert("warp@mkt".into(), false);
        store.brian.effort = Some("high".into());
        save_overrides(dir.path(), &store).unwrap();
        assert_eq!(load_overrides(dir.path()), store);
    }

    #[test]
    fn corrupt_store_fails_open() {
        let dir = tempdir().unwrap();
        std::fs::write(config_path(dir.path()), "{ not json ]").unwrap();
        assert_eq!(load_overrides(dir.path()), ClaudeOverrides::default());
    }

    #[test]
    fn per_agent_wins_over_all() {
        let mut store = ClaudeOverrides::default();
        store.all.effort = Some("medium".into());
        store.all.skills.insert("a".into(), SkillVisibility::Off);
        store.brian.effort = Some("xhigh".into());
        store
            .brian
            .skills
            .insert("b".into(), SkillVisibility::NameOnly);
        let merged = resolve_agent_overrides(&store, "brian");
        assert_eq!(merged.effort.as_deref(), Some("xhigh"));
        // _all's skill "a" survives; brian's "b" is layered on.
        assert_eq!(merged.skills.get("a"), Some(&SkillVisibility::Off));
        assert_eq!(merged.skills.get("b"), Some(&SkillVisibility::NameOnly));
    }

    #[test]
    fn settings_fragment_shapes_skilloverrides_and_plugins() {
        let mut ov = AgentOverride::default();
        ov.skills.insert("note".into(), SkillVisibility::Off);
        ov.plugins.insert("warp@mkt".into(), false);
        ov.ultracode = Some(true);
        let frag = settings_fragment(&ov);
        assert_eq!(frag["skillOverrides"]["note"], Value::String("off".into()));
        assert_eq!(frag["enabledPlugins"]["warp@mkt"], Value::Bool(false));
        assert_eq!(frag["ultracode"], Value::Bool(true));
    }

    #[test]
    fn empty_override_yields_empty_fragment_and_env() {
        let ov = AgentOverride::default();
        assert!(settings_fragment(&ov).is_empty());
        assert!(env_vars(&ov).is_empty());
        assert!(disabled_mcp(&ov).is_empty());
    }

    #[test]
    fn env_and_mcp_helpers() {
        let mut ov = AgentOverride {
            effort: Some("max".into()),
            disable_auto_memory: Some(true),
            ..Default::default()
        };
        ov.mcp.insert("discord".into(), false);
        ov.mcp.insert("github".into(), true);
        let env = env_vars(&ov);
        assert!(env.contains(&("CLAUDE_CODE_EFFORT_LEVEL".into(), "max".into())));
        assert!(env.contains(&("CLAUDE_CODE_DISABLE_AUTO_MEMORY".into(), "1".into())));
        assert_eq!(disabled_mcp(&ov), vec!["discord".to_string()]);
    }
}
