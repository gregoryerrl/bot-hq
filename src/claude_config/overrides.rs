//! Per-agent **overrides** bot-hq applies to inherited Claude Code config at
//! spawn time, stored at `<data_dir>/config/claude-overrides.json` (0600).
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

/// The full override store: a fan-out `_all` default plus per-ROLE entries.
///
/// **rc3 D10 re-key.** The per-agent buckets were two fixed fields named after
/// people, and `resolve_agent_overrides` matched a participant slug against
/// those two literals. Role-derived slugs match neither, so every per-agent
/// override silently resolved to `_all` — the store had an editor, a file and a
/// resolver, and changed nothing at spawn.
///
/// The key is now the **role slug**, which is the unit the user actually
/// configures (the Roles tab owns a role's prose, capabilities and default
/// model, so its Claude-config overrides belong beside them). Not the
/// participant slug: those are per-session and gain numeric suffixes
/// (`hands-2`), so a global config panel could neither enumerate nor address
/// them, and two participants of one role would need the override entered
/// twice.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct ClaudeOverrides {
    #[serde(rename = "_all", default)]
    pub all: AgentOverride,
    /// role slug → that role's overrides, layered over `_all` at resolve time.
    #[serde(default)]
    pub per_role: BTreeMap<String, AgentOverride>,
}

/// `<data_dir>/config/claude-overrides.json`.
pub fn config_path(data_dir: &Path) -> PathBuf {
    crate::paths::config_dir_path(data_dir).join("claude-overrides.json")
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
///
/// Written to a sibling temp file that is CREATED 0600 on unix, then renamed
/// over the real one (round 9): the old write-then-chmod left a umask-default
/// window on a file that may hold per-role env/tokens, and a bare
/// `std::fs::write` could leave it half-written.
pub fn save_overrides(data_dir: &Path, store: &ClaudeOverrides) -> Result<()> {
    let path = config_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating data dir {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(store).context("serializing claude-overrides")?;
    // Always 0600 — this store is bot-hq's own and may carry per-role env.
    super::replace_file_atomically(&path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {} atomically", path.display()))?;
    Ok(())
}

/// The effort a participant spawns with when neither its per-run pick nor its
/// role's default says otherwise — the floor `reconcile_spawn_knobs` applies.
///
/// Mirrored by `DEFAULT_EFFORT` in `frontend/src/lib/effort.ts` (every dropdown
/// shows this as the unconfigured-role value); a test in this module pins the
/// two literals together, because the two layers disagreeing is exactly the
/// silent-mislabel bug the no-inherit change closed.
pub const DEFAULT_EFFORT: &str = "medium";

/// Resolve the effective override for a participant playing `role_slug`: the
/// `_all` default with that role's entry layered on top — EXCEPT effort and
/// ultracode, which are per-role-only (no-inherit, 2026-08-25). `_all` still
/// fans out skills/plugins/mcp/memory knobs, but a role without its own
/// effort/ultracode falls to the spawn floor ([`DEFAULT_EFFORT`]), never to
/// `_all` or to the user's own settings.json knob.
///
/// **The key is a ROLE slug, not an agent name and not a participant slug.**
/// This used to match the literals `"brian"` / `"rain"`; both production callers
/// pass a role-derived value now, so every branch but the fallback was dead and
/// per-agent overrides resolved to the global config without a word.
pub fn resolve_agent_overrides(store: &ClaudeOverrides, role_slug: Option<&str>) -> AgentOverride {
    let Some(specific) = role_slug.and_then(|slug| store.per_role.get(slug)) else {
        let mut base = store.all.clone();
        base.effort = None;
        base.ultracode = None;
        return base;
    };
    let mut merged = store.all.clone();
    merged.skills.extend(specific.skills.clone());
    merged.plugins.extend(specific.plugins.clone());
    merged.mcp.extend(specific.mcp.clone());
    // Unconditional on purpose: a per-role entry that exists but carries no
    // effort (skills-only) must NOT leak `_all.effort` past the spawn floor.
    merged.effort = specific.effort.clone();
    merged.ultracode = specific.ultracode;
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
        let hands = store.per_role.entry("hands".into()).or_default();
        hands
            .skills
            .insert("my-skill".into(), SkillVisibility::UserInvocableOnly);
        hands.plugins.insert("alpha@mkt".into(), false);
        hands.effort = Some("high".into());
        save_overrides(dir.path(), &store).unwrap();
        assert_eq!(load_overrides(dir.path()), store);
        // Round 9: temp + rename, and owner-only from the first byte.
        let path = config_path(dir.path());
        assert!(!path.with_extension("json.tmp").exists(), "temp file must be renamed away");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "override store must be owner-only, got {mode:o}");
        }
    }

    #[test]
    fn corrupt_store_fails_open() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(config_path(dir.path()).parent().unwrap()).unwrap();
        std::fs::write(config_path(dir.path()), "{ not json ]").unwrap();
        assert_eq!(load_overrides(dir.path()), ClaudeOverrides::default());
    }

    #[test]
    fn per_agent_wins_over_all() {
        let mut store = ClaudeOverrides::default();
        store.all.effort = Some("medium".into());
        store.all.skills.insert("a".into(), SkillVisibility::Off);
        let hands = store.per_role.entry("hands".into()).or_default();
        hands.effort = Some("xhigh".into());
        hands.skills.insert("b".into(), SkillVisibility::NameOnly);
        let merged = resolve_agent_overrides(&store, Some("hands"));
        assert_eq!(merged.effort.as_deref(), Some("xhigh"));
        // _all's skill "a" survives; hands' "b" is layered on.
        assert_eq!(merged.skills.get("a"), Some(&SkillVisibility::Off));
        assert_eq!(merged.skills.get("b"), Some(&SkillVisibility::NameOnly));
    }

    /// No-inherit (2026-08-25): effort/ultracode never come from `_all`, in
    /// BOTH shapes of miss — no per-role entry at all, and an entry that exists
    /// but is skills-only. The second is the one an `is_some()` guard would
    /// leak: the entry is found, so the old code kept `_all.effort` as the
    /// merge base and the floor downstream never saw the absence.
    #[test]
    fn all_effort_and_ultracode_never_leak_into_a_role() {
        let mut store = ClaudeOverrides::default();
        store.all.effort = Some("max".into());
        store.all.ultracode = Some(true);
        store.all.skills.insert("a".into(), SkillVisibility::Off);
        // Skills-only entry for hands; no entry at all for eyes.
        store
            .per_role
            .entry("hands".into())
            .or_default()
            .skills
            .insert("b".into(), SkillVisibility::NameOnly);

        for slug in [Some("hands"), Some("eyes"), None] {
            let merged = resolve_agent_overrides(&store, slug);
            assert_eq!(merged.effort, None, "effort must not inherit from _all ({slug:?})");
            assert_eq!(merged.ultracode, None, "ultracode must not inherit from _all ({slug:?})");
            // The fan-out itself is untouched.
            assert_eq!(merged.skills.get("a"), Some(&SkillVisibility::Off));
        }
    }

    /// The two layers of the no-inherit floor show the same literal. Rust owns
    /// the resolution (`reconcile_spawn_knobs`), TypeScript owns every dropdown
    /// that DISPLAYS the unconfigured-role value — drift between them is the
    /// shows-medium-runs-something-else bug this change exists to close.
    #[test]
    fn frontend_default_effort_matches_the_rust_floor() {
        let ts = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/frontend/src/lib/effort.ts"
        ))
        .expect("frontend/src/lib/effort.ts must exist — the display half of the effort floor");
        let needle = format!("DEFAULT_EFFORT = \"{DEFAULT_EFFORT}\"");
        assert!(
            ts.contains(&needle),
            "effort.ts must carry `{needle}` so both layers show the same floor"
        );
    }

    #[test]
    fn settings_fragment_shapes_skilloverrides_and_plugins() {
        let mut ov = AgentOverride::default();
        ov.skills.insert("my-skill".into(), SkillVisibility::Off);
        ov.plugins.insert("alpha@mkt".into(), false);
        ov.ultracode = Some(true);
        let frag = settings_fragment(&ov);
        assert_eq!(frag["skillOverrides"]["my-skill"], Value::String("off".into()));
        assert_eq!(frag["enabledPlugins"]["alpha@mkt"], Value::Bool(false));
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
