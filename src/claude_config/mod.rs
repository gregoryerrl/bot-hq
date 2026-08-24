//! Surface + control the user's **Claude Code** configuration that bot-hq's
//! agents inherit.
//!
//! bot-hq's agents are `claude-code` headless subprocesses
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

/// Replace `path` atomically: the body goes to a sibling temp file created
/// with `mode` (unix) from its first byte, then `rename` moves it over the
/// destination. `rename` replaces the INODE, so the destination ends up with
/// the temp's mode — which is why the mode is a parameter and set at create,
/// not chmodded after (round 9, review 4a90f9ed: a plain-`std::fs::write` temp
/// silently turned a 0600 `settings.json` into 0644 on rename, and a chmod
/// after the write leaves the full body world-readable in between).
pub(crate) fn replace_file_atomically(
    path: &std::path::Path,
    body: &[u8],
    mode: u32,
) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.tmp"));
    write_with_mode(&tmp, body, mode)?;
    std::fs::rename(&tmp, path)
}

/// The mode of an existing file at `path`, or `fallback` when it does not
/// exist yet (or on non-unix, where modes are not a thing).
pub(crate) fn existing_mode_or(path: &std::path::Path, fallback: u32) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(fallback)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        fallback
    }
}

/// Create-or-truncate `path` with `mode` from the first byte (unix); a plain
/// write elsewhere. An existing temp keeps its old mode past `create`, so the
/// mode is set explicitly as well.
#[cfg(unix)]
fn write_with_mode(path: &std::path::Path, body: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(path)?;
    f.set_permissions(std::fs::Permissions::from_mode(mode))?;
    f.write_all(body)
}
#[cfg(not(unix))]
fn write_with_mode(path: &std::path::Path, body: &[u8], _mode: u32) -> std::io::Result<()> {
    std::fs::write(path, body)
}

pub use overrides::{
    load_overrides, resolve_agent_overrides, save_overrides, AgentOverride, ClaudeOverrides,
    SkillVisibility, DEFAULT_EFFORT,
};
pub use reader::{config_dir, read_claude_config};
pub use writer::{set_bool, set_plugin_enabled, set_string};

use serde::{Deserialize, Serialize};
use specta::Type;

/// Which agents pick up a given config surface from `~/.claude` at spawn, and
/// which don't. Drives the per-surface inheritance badges in the UI. This is
/// the canonical mapping derived from `spawn.rs::build_command` behavior: every
/// participant runs full claude-code and inherits skills/plugins/hooks/CLAUDE.md
/// (a read-only participant's tool access is gated server-side, not by skipping inheritance);
/// model/permissions are overridden per-agent by bot-hq.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct Inheritance {
    /// Agents that inherit this surface from `~/.claude` at spawn.
    pub inherited_by: Vec<String>,
    /// Agents that do NOT — because bot-hq overrides the surface per-agent.
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

/// The whole roster, as the inheritance chips name it.
///
/// **rc3 D10: these lists used to be the two literals `brian` / `rain`.** A
/// session's roster is now whatever roles the user picked, so naming two agents
/// here was both wrong and a person's name in a panel the user reads. Every
/// surface below is all-or-nothing across the roster, so one collective chip
/// says exactly what the two used to say and stays true at any roster size.
const EVERY_AGENT: &str = "every agent";

/// The canonical inheritance lens for a surface. See the design doc §2.
pub fn inheritance(surface: Surface) -> Inheritance {
    match surface {
        Surface::Skills => Inheritance {
            inherited_by: agents(&[EVERY_AGENT]),
            skipped_by: agents(&[]),
            note: "Every agent loads your skills (a skill can self-invoke). bot-hq gates each agent's tools server-side from its capabilities.".into(),
            overridable: true,
        },
        Surface::Plugins => Inheritance {
            inherited_by: agents(&[EVERY_AGENT]),
            skipped_by: agents(&[]),
            note: "Every agent loads enabled plugins (and their skills/hooks/MCP). bot-hq gates each agent's tools server-side from its capabilities.".into(),
            overridable: true,
        },
        Surface::Hooks => Inheritance {
            inherited_by: agents(&[EVERY_AGENT]),
            skipped_by: agents(&[]),
            note: "Every agent runs your hooks alongside bot-hq's PreToolUse hook. An agent whose role lacks edit_files has its write tools denied at spawn.".into(),
            overridable: false,
        },
        Surface::Memory => Inheritance {
            inherited_by: agents(&[EVERY_AGENT]),
            skipped_by: agents(&[]),
            note: "Every agent autodiscovers CLAUDE.md + auto-memory. bot-hq adds its own system prompt regardless.".into(),
            overridable: true,
        },
        Surface::CoreKnobs => Inheritance {
            inherited_by: agents(&[EVERY_AGENT]),
            skipped_by: agents(&[]),
            note: "Read from settings.json by every agent (effort, thinking, output tokens, …).".into(),
            overridable: true,
        },
        Surface::Model => Inheritance {
            inherited_by: agents(&[]),
            skipped_by: agents(&[EVERY_AGENT]),
            note: "Overridden by bot-hq via ANTHROPIC_MODEL — the model comes from the participant's own pick or its role's default (Roles tab).".into(),
            overridable: true,
        },
        Surface::Permissions => Inheritance {
            inherited_by: agents(&[]),
            skipped_by: agents(&[EVERY_AGENT]),
            note: "bot-hq sets each agent's permission posture from its role's capabilities (edit_files → bypass; otherwise dontAsk + allow/deny).".into(),
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
    /// Agents bot-hq forwards it into (reserved keys excluded).
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
    fn skills_are_inherited_by_the_whole_roster() {
        let inh = inheritance(Surface::Skills);
        assert_eq!(inh.inherited_by, vec![EVERY_AGENT.to_string()]);
        assert!(inh.skipped_by.is_empty());
        assert!(inh.overridable);
    }

    #[test]
    fn core_knobs_inherited_by_all() {
        let inh = inheritance(Surface::CoreKnobs);
        assert_eq!(inh.inherited_by, vec![EVERY_AGENT.to_string()]);
        assert!(inh.skipped_by.is_empty());
    }

    #[test]
    fn model_overridden_for_all() {
        let inh = inheritance(Surface::Model);
        assert!(inh.inherited_by.is_empty());
        assert_eq!(inh.skipped_by, vec![EVERY_AGENT.to_string()]);
    }

    /// **No inheritance chip names a person (rc3 D10).** These strings render
    /// verbatim in Settings → Claude Config, so a name here is a name the user
    /// reads. Sweeps every surface, so a new one cannot reintroduce it.
    #[test]
    fn no_inheritance_label_or_note_names_an_agent() {
        for surface in [
            Surface::Skills,
            Surface::Plugins,
            Surface::Hooks,
            Surface::Memory,
            Surface::CoreKnobs,
            Surface::Model,
            Surface::Permissions,
        ] {
            let inh = inheritance(surface);
            let rendered = format!("{:?} {:?} {}", inh.inherited_by, inh.skipped_by, inh.note);
            let lower = rendered.to_lowercase();
            for banned in ["hands", "eyes"] {
                assert!(
                    !lower.split(|c: char| !c.is_alphanumeric()).any(|w| w == banned),
                    "{surface:?} still names {banned}: {rendered}"
                );
            }
        }
    }

    #[test]
    fn hooks_not_overridable() {
        assert!(!inheritance(Surface::Hooks).overridable);
    }
}
