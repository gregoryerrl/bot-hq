//! Surface + control the user's **Claude Code** configuration that bot-hq's
//! agents inherit.
//!
//! bot-hq's agents (Brian/Rain) are `claude-code` headless subprocesses
//! (`src/agents/spawn.rs`), so whatever the user's `~/.claude` install has —
//! skills, plugins, hooks, CLAUDE.md/memory, MCP servers, effort — flows into
//! the agents (see the inheritance table in
//! `docs/plans/2026-06-02-claude-config-surface-design.md`). This module:
//!
//! - **reads/resolves** that config into a masked, provenance-annotated view
//!   ([`reader`]), honoring `CLAUDE_CONFIG_DIR`;
//! - models the **inheritance lens** ([`inheritance`]) — the single source of
//!   truth for which agents pick up each surface;
//! - (Phase 2) stores **per-agent overrides** ([`overrides`]) that the spawn
//!   path merges into the `--settings`/env/`--mcp-config` it already injects.
//!
//! `policy.yaml` is deliberately NOT handled here — it is a bot-hq-internal
//! artifact injected into agents, not user Claude config.

pub mod overrides;
pub mod reader;
pub mod writer;

pub use overrides::{
    load_overrides, resolve_agent_overrides, save_overrides, AgentOverride, ClaudeOverrides,
    SkillVisibility,
};
pub use reader::{config_dir, read_claude_config};
pub use writer::{set_bool, set_plugin_enabled, set_string};

use serde::{Deserialize, Serialize};
use specta::Type;

/// Which agents pick up a given config surface from `~/.claude` at spawn, and
/// which don't. Drives the per-surface inheritance badges in the UI. This is
/// the canonical mapping derived from `spawn.rs::build_command` behavior:
/// Brian runs full claude-code (inherit), Rain runs `--bare` (skips
/// skills/plugins/hooks/CLAUDE.md), MCP is forwarded to Brian only, and
/// model/permissions are overridden per-agent by bot-hq.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct Inheritance {
    /// Agents that inherit this surface from `~/.claude` at spawn.
    pub inherited_by: Vec<String>,
    /// Agents that do NOT (Rain via `--bare`, or because bot-hq overrides it).
    pub skipped_by: Vec<String>,
    /// Short human note for the badge tooltip.
    pub note: String,
    /// Whether bot-hq can override this surface per-agent at spawn (Phase 2).
    pub overridable: bool,
}

/// A config surface, for [`inheritance`] lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Skills,
    Plugins,
    Hooks,
    Memory,
    CoreKnobs,
    Model,
    Permissions,
}

fn agents(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// The canonical inheritance lens for a surface. See the design doc §2.
pub fn inheritance(surface: Surface) -> Inheritance {
    match surface {
        Surface::Skills => Inheritance {
            inherited_by: agents(&["brian"]),
            skipped_by: agents(&["rain"]),
            note: "Brian loads your skills (a skill can self-invoke). Rain runs --bare and skips them.".into(),
            overridable: true,
        },
        Surface::Plugins => Inheritance {
            inherited_by: agents(&["brian"]),
            skipped_by: agents(&["rain"]),
            note: "Brian loads enabled plugins (and their skills/hooks/MCP). Rain --bare skips plugin sync.".into(),
            overridable: true,
        },
        Surface::Hooks => Inheritance {
            inherited_by: agents(&["brian"]),
            skipped_by: agents(&["rain"]),
            note: "Brian runs your hooks alongside bot-hq's PreToolUse hook. Rain --bare skips hooks. Granular per-hook suppression is limited.".into(),
            overridable: false,
        },
        Surface::Memory => Inheritance {
            inherited_by: agents(&["brian"]),
            skipped_by: agents(&["rain"]),
            note: "Brian autodiscovers CLAUDE.md + auto-memory. Rain --bare skips it. bot-hq adds its own system prompt regardless.".into(),
            overridable: true,
        },
        Surface::CoreKnobs => Inheritance {
            inherited_by: agents(&["brian", "rain"]),
            skipped_by: agents(&[]),
            note: "Read from settings.json by all agents (effort, thinking, output tokens, …).".into(),
            overridable: true,
        },
        Surface::Model => Inheritance {
            inherited_by: agents(&[]),
            skipped_by: agents(&["brian", "rain"]),
            note: "Overridden per-agent by bot-hq via ANTHROPIC_MODEL (the Agents tab).".into(),
            overridable: true,
        },
        Surface::Permissions => Inheritance {
            inherited_by: agents(&[]),
            skipped_by: agents(&["brian", "rain"]),
            note: "bot-hq sets each agent's permission posture (Brian bypass; Rain dontAsk + allow/deny).".into(),
            overridable: false,
        },
    }
}

// ---------------------------------------------------------------------------
// View types returned to the frontend (all `specta::Type` for tauri-specta).
// ---------------------------------------------------------------------------

/// Stat for a single config file (present / path / size).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct FileStat {
    pub present: bool,
    pub path: String,
    pub bytes: u64,
}

/// One scalar/env setting with provenance + inheritance.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SettingItem {
    /// Dotted settings key, e.g. `effortLevel` or `env.CLAUDE_CODE_MAX_OUTPUT_TOKENS`.
    pub key: String,
    pub label: String,
    /// Effective value (masked if secret). `None` = unset (uses default).
    pub value: Option<String>,
    /// Where the value resolved from, or "unset (default)".
    pub source: String,
    pub inheritance: Inheritance,
}

/// One skill (user-dir skill or plugin-bundled skill).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SkillItem {
    pub name: String,
    /// "user" (a dir under `<config>/skills/`) or "plugin" (bundled).
    pub kind: String,
    /// From SKILL.md frontmatter `disable-model-invocation` (user skills).
    pub disable_model_invocation: bool,
    pub description: Option<String>,
    /// SKILL.md path (user skills only).
    pub path: Option<String>,
    pub inheritance: Inheritance,
}

/// One installed plugin and its enablement.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct PluginItem {
    /// `name@marketplace` key as used in `enabledPlugins`.
    pub key: String,
    pub enabled: bool,
    pub inheritance: Inheritance,
}

/// One MCP server visible to the user's claude-code, with the trap flagged.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct McpServerItem {
    pub name: String,
    /// "http" | "sse" | "stdio" | "unknown".
    pub transport: String,
    /// Human source: "~/.claude.json", "~/.claude/settings.json (ignored)", "both".
    pub loaded_from: String,
    /// Whether claude-code actually loads it (settings.json mcpServers is ignored).
    pub effective: bool,
    /// Masked one-line detail (url or command), secrets redacted.
    pub detail: String,
    /// Agents bot-hq forwards it into (Brian; reserved keys excluded).
    pub forwarded_to_agents: Vec<String>,
    /// True if bot-hq filters this server from agents (bot-hq / claude-in-chrome).
    pub reserved_filtered: bool,
}

/// CLAUDE.md + auto-memory presence summary.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct MemoryView {
    /// `<config_dir>/CLAUDE.md` (user-global instructions).
    pub user_claude_md: FileStat,
    /// `~/CLAUDE.md` (home-root, project-checked-in global rules).
    pub home_claude_md: FileStat,
    /// Number of `projects/<slug>/memory/` dirs with a MEMORY.md.
    pub projects_with_memory: u32,
    pub inheritance: Inheritance,
}

/// Permission posture summary (counts only; bot-hq overrides per agent anyway).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct PermissionsView {
    pub default_mode: Option<String>,
    pub allow: u32,
    pub ask: u32,
    pub deny: u32,
    pub additional_directories: u32,
    pub inheritance: Inheritance,
}

/// The full resolved view of the user's Claude Code config.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ClaudeConfigView {
    /// Resolved config dir (CLAUDE_CONFIG_DIR or `~/.claude`).
    pub config_dir: String,
    /// "CLAUDE_CONFIG_DIR" if the env var is set, else "default (~/.claude)".
    pub config_dir_source: String,
    /// `~/.claude.json` exists (the file claude-code actually loads MCP from).
    pub home_claude_json: FileStat,
    /// A managed/enterprise policy was detected (it beats everything, incl. bot-hq).
    pub managed_settings_present: bool,
    pub core_knobs: Vec<SettingItem>,
    pub skills: Vec<SkillItem>,
    pub plugins: Vec<PluginItem>,
    pub mcp_servers: Vec<McpServerItem>,
    pub memory: MemoryView,
    pub permissions: PermissionsView,
    /// Non-fatal notices (unparseable files, ignored mcpServers, managed policy…).
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_inherited_by_brian_not_rain() {
        let inh = inheritance(Surface::Skills);
        assert!(inh.inherited_by.contains(&"brian".to_string()));
        assert!(inh.skipped_by.contains(&"rain".to_string()));
        assert!(inh.overridable);
    }

    #[test]
    fn core_knobs_inherited_by_all() {
        let inh = inheritance(Surface::CoreKnobs);
        assert_eq!(inh.inherited_by.len(), 2);
        assert!(inh.skipped_by.is_empty());
    }

    #[test]
    fn model_overridden_for_all() {
        let inh = inheritance(Surface::Model);
        assert!(inh.inherited_by.is_empty());
        assert_eq!(inh.skipped_by.len(), 2);
    }

    #[test]
    fn hooks_not_overridable() {
        assert!(!inheritance(Surface::Hooks).overridable);
    }
}
