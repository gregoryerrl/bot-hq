//! Read + resolve the user's Claude Code config into a masked,
//! provenance-annotated [`ClaudeConfigView`].
//!
//! Honors `CLAUDE_CONFIG_DIR` (default `~/.claude`). Surfaces the known traps:
//! `settings.json` `mcpServers` is IGNORED by claude-code (it loads MCP from
//! `~/.claude.json`); secrets are masked before leaving the process.

use super::{
    inheritance, ClaudeConfigView, FileStat, McpServerItem, MemoryView, PermissionsView,
    PluginItem, SettingItem, SkillItem, Surface,
};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Substrings that mark a JSON key as secret-bearing (case-insensitive).
const SECRET_KEY_HINTS: &[&str] = &[
    "token",
    "secret",
    "password",
    "authorization",
    "apikey",
    "api_key",
    "bearer",
];

/// Read the user's Claude Code config, resolving the config dir from the
/// environment. Never fails — unreadable/malformed files become warnings.
pub fn read_claude_config() -> ClaudeConfigView {
    let (config_dir, source) = resolve_config_dir();
    let home = home_dir().unwrap_or_else(|| config_dir.clone());
    read_at(&config_dir, &home, source, managed_present())
}

/// The effective Claude config dir (honors `CLAUDE_CONFIG_DIR`). Used by the
/// write-back commands to target the same dir the read view resolved.
pub fn config_dir() -> PathBuf {
    resolve_config_dir().0
}

/// Resolve the effective config dir: `CLAUDE_CONFIG_DIR` if set, else
/// `$HOME/.claude`.
fn resolve_config_dir() -> (PathBuf, &'static str) {
    if let Some(v) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if !v.is_empty() {
            return (PathBuf::from(v), "CLAUDE_CONFIG_DIR");
        }
    }
    let home = home_dir().unwrap_or_default();
    (home.join(".claude"), "default (~/.claude)")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Whether an enterprise/managed policy file exists (it beats everything,
/// including bot-hq's injected `--settings`).
fn managed_present() -> bool {
    [
        "/Library/Application Support/ClaudeCode/managed-settings.json",
        "/etc/claude-code/managed-settings.json",
    ]
    .iter()
    .any(|p| Path::new(p).exists())
}

/// Pure core: resolve a view from explicit dirs (testable with tempdirs).
fn read_at(config_dir: &Path, home: &Path, source: &str, managed: bool) -> ClaudeConfigView {
    let mut warnings: Vec<String> = Vec::new();

    let settings = read_json_map(&config_dir.join("settings.json"), &mut warnings);
    let claude_json_path = home.join(".claude.json");
    let claude_json = read_json_map(&claude_json_path, &mut warnings);

    if managed {
        warnings.push(
            "A managed/enterprise policy is active — it overrides user settings AND bot-hq's injected --settings.".into(),
        );
    }

    ClaudeConfigView {
        config_dir: config_dir.display().to_string(),
        config_dir_source: source.to_string(),
        home_claude_json: stat(&claude_json_path),
        managed_settings_present: managed,
        core_knobs: core_knobs(&settings),
        skills: user_skills(config_dir),
        plugins: plugins(&settings),
        mcp_servers: mcp_servers(&settings, &claude_json, &mut warnings),
        memory: memory(config_dir, home),
        permissions: permissions(&settings),
        warnings,
    }
}

// --- core knobs -------------------------------------------------------------

fn core_knobs(settings: &Map<String, Value>) -> Vec<SettingItem> {
    // Effort level: bot-hq routes this through the CLAUDE_CODE_EFFORT_LEVEL env
    // var rather than the top-level `effortLevel` field, because the env var is
    // the only persistent lever that accepts `max` (the field rejects
    // `max`/`ultracode` — they are session-only in claude-code). Read env first,
    // fall back to the legacy `effortLevel` field so an existing setting still
    // shows; the writer clears the legacy field on the next change.
    let effort_env = settings
        .get("env")
        .and_then(|v| v.as_object())
        .and_then(|e| e.get("CLAUDE_CODE_EFFORT_LEVEL"))
        .and_then(scalar_str);
    let effort_legacy = settings.get("effortLevel").and_then(scalar_str);
    let (effort_value, effort_source) = match (&effort_env, &effort_legacy) {
        (Some(_), _) => (effort_env.clone(), "~/.claude/settings.json (env)"),
        (None, Some(_)) => (
            effort_legacy.clone(),
            "~/.claude/settings.json (effortLevel, legacy)",
        ),
        (None, None) => (None, "unset (default)"),
    };
    let mut items = vec![
        SettingItem {
            key: "env.CLAUDE_CODE_EFFORT_LEVEL".into(),
            label: "Effort level".into(),
            value: effort_value,
            source: effort_source.into(),
            inheritance: inheritance(Surface::CoreKnobs),
        },
        knob(settings, "model", "Model (default)", Surface::Model),
        knob(settings, "editorMode", "Editor mode", Surface::CoreKnobs),
        knob(
            settings,
            "alwaysThinkingEnabled",
            "Always thinking",
            Surface::CoreKnobs,
        ),
        knob(settings, "voiceEnabled", "Voice", Surface::CoreKnobs),
    ];
    // env.CLAUDE_CODE_MAX_OUTPUT_TOKENS lives one level down.
    let max_tokens = settings
        .get("env")
        .and_then(|v| v.as_object())
        .and_then(|e| e.get("CLAUDE_CODE_MAX_OUTPUT_TOKENS"))
        .and_then(scalar_str);
    items.push(SettingItem {
        key: "env.CLAUDE_CODE_MAX_OUTPUT_TOKENS".into(),
        label: "Max output tokens".into(),
        source: if max_tokens.is_some() {
            "~/.claude/settings.json".into()
        } else {
            "unset (default)".into()
        },
        value: max_tokens,
        inheritance: inheritance(Surface::CoreKnobs),
    });
    items
}

fn knob(settings: &Map<String, Value>, key: &str, label: &str, surface: Surface) -> SettingItem {
    let raw = settings.get(key).and_then(scalar_str);
    let value = match &raw {
        Some(v) if looks_secret(key) => Some(mask_str(v)),
        other => other.clone(),
    };
    let source = if value.is_some() {
        "~/.claude/settings.json".to_string()
    } else {
        "unset (default)".to_string()
    };
    SettingItem {
        key: key.to_string(),
        label: label.to_string(),
        value,
        source,
        inheritance: inheritance(surface),
    }
}

// --- skills -----------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(rename = "disable-model-invocation")]
    disable_model_invocation: Option<bool>,
}

fn user_skills(config_dir: &Path) -> Vec<SkillItem> {
    let skills_dir = config_dir.join("skills");
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&skills_dir) else {
        return out;
    };
    let inh = inheritance(Surface::Skills);
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let skill_md = dir.join("SKILL.md");
        let fm = parse_frontmatter(&skill_md);
        let dir_name = entry.file_name().to_string_lossy().to_string();
        out.push(SkillItem {
            name: fm.name.clone().unwrap_or(dir_name),
            kind: "user".into(),
            disable_model_invocation: fm.disable_model_invocation.unwrap_or(false),
            description: fm.description.clone(),
            path: Some(skill_md.display().to_string()),
            inheritance: inh.clone(),
        });
    }
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// Extract + parse the leading `---`-delimited YAML frontmatter of a SKILL.md.
fn parse_frontmatter(path: &Path) -> SkillFrontmatter {
    let Ok(body) = std::fs::read_to_string(path) else {
        return SkillFrontmatter::default();
    };
    let trimmed = body.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return SkillFrontmatter::default();
    };
    // The frontmatter ends at the next line that is exactly `---`.
    let Some(end) = rest.find("\n---") else {
        return SkillFrontmatter::default();
    };
    let yaml = &rest[..end];
    serde_yaml::from_str::<SkillFrontmatter>(yaml).unwrap_or_default()
}

// --- plugins ----------------------------------------------------------------

fn plugins(settings: &Map<String, Value>) -> Vec<PluginItem> {
    let inh = inheritance(Surface::Plugins);
    let mut out: Vec<PluginItem> = settings
        .get("enabledPlugins")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| PluginItem {
                    key: k.clone(),
                    enabled: v.as_bool().unwrap_or(false),
                    inheritance: inh.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|p| p.key.to_lowercase());
    out
}

// --- mcp --------------------------------------------------------------------

fn mcp_servers(
    settings: &Map<String, Value>,
    claude_json: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Vec<McpServerItem> {
    let settings_mcp = settings.get("mcpServers").and_then(|v| v.as_object());
    let home_mcp = claude_json.get("mcpServers").and_then(|v| v.as_object());

    // Union of names across both files.
    let mut names: Vec<String> = Vec::new();
    for m in [settings_mcp, home_mcp].into_iter().flatten() {
        for k in m.keys() {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    names.sort_by_key(|a| a.to_lowercase());

    let mut ignored = 0usize;
    let out: Vec<McpServerItem> = names
        .iter()
        .map(|name| {
            let in_home = home_mcp.map(|m| m.contains_key(name)).unwrap_or(false);
            let in_settings = settings_mcp.map(|m| m.contains_key(name)).unwrap_or(false);
            // claude-code loads MCP from ~/.claude.json; settings.json mcpServers
            // is ignored. (bot-hq forwards from BOTH files, though.)
            let effective = in_home;
            if in_settings && !in_home {
                ignored += 1;
            }
            let loaded_from = match (in_home, in_settings) {
                (true, true) => "~/.claude.json (settings.json copy ignored)".to_string(),
                (true, false) => "~/.claude.json".to_string(),
                (false, true) => "~/.claude/settings.json (ignored by claude-code)".to_string(),
                (false, false) => "unknown".to_string(),
            };
            let entry = home_mcp
                .and_then(|m| m.get(name))
                .or_else(|| settings_mcp.and_then(|m| m.get(name)));
            let (transport, detail) = entry
                .map(mcp_detail)
                .unwrap_or(("unknown".into(), String::new()));
            let reserved_filtered = crate::signaling::RESERVED_MCP_KEYS.contains(&name.as_str());
            // bot-hq's load_user_mcp_servers reads BOTH files, so any non-reserved
            // server (even a settings.json-only one) is forwarded to Brian.
            let forwarded_to_agents = if reserved_filtered {
                Vec::new()
            } else {
                vec!["brian".to_string()]
            };
            McpServerItem {
                name: name.clone(),
                transport,
                loaded_from,
                effective,
                detail,
                forwarded_to_agents,
                reserved_filtered,
            }
        })
        .collect();

    if ignored > 0 {
        warnings.push(format!(
            "{ignored} MCP server(s) live only in settings.json — claude-code ignores those (it loads MCP from ~/.claude.json). bot-hq still forwards them to agents."
        ));
    }
    out
}

fn mcp_detail(entry: &Value) -> (String, String) {
    let declared = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(url) = entry.get("url").and_then(|v| v.as_str()) {
        let transport = if declared.is_empty() {
            "http"
        } else {
            declared
        };
        return (transport.to_string(), url.to_string());
    }
    if let Some(cmd) = entry.get("command").and_then(|v| v.as_str()) {
        let args = entry
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let detail = if args.is_empty() {
            cmd.to_string()
        } else {
            format!("{cmd} {args}")
        };
        return ("stdio".to_string(), detail);
    }
    ("unknown".to_string(), String::new())
}

// --- memory -----------------------------------------------------------------

fn memory(config_dir: &Path, home: &Path) -> MemoryView {
    let projects_with_memory = std::fs::read_dir(config_dir.join("projects"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().join("memory").join("MEMORY.md").exists())
                .count() as u32
        })
        .unwrap_or(0);
    MemoryView {
        user_claude_md: stat(&config_dir.join("CLAUDE.md")),
        home_claude_md: stat(&home.join("CLAUDE.md")),
        projects_with_memory,
        inheritance: inheritance(Surface::Memory),
    }
}

// --- permissions ------------------------------------------------------------

fn permissions(settings: &Map<String, Value>) -> PermissionsView {
    let perms = settings.get("permissions").and_then(|v| v.as_object());
    let count = |key: &str| -> u32 {
        perms
            .and_then(|p| p.get(key))
            .and_then(|v| v.as_array())
            .map(|a| a.len() as u32)
            .unwrap_or(0)
    };
    PermissionsView {
        default_mode: perms
            .and_then(|p| p.get("defaultMode"))
            .and_then(|v| v.as_str())
            .map(String::from),
        allow: count("allow"),
        ask: count("ask"),
        deny: count("deny"),
        additional_directories: count("additionalDirectories"),
        inheritance: inheritance(Surface::Permissions),
    }
}

// --- helpers ----------------------------------------------------------------

fn read_json_map(path: &Path, warnings: &mut Vec<String>) -> Map<String, Value> {
    match std::fs::read_to_string(path) {
        Ok(body) => match serde_json::from_str::<Value>(&body) {
            Ok(Value::Object(m)) => m,
            Ok(_) => {
                warnings.push(format!("{} is not a JSON object", path.display()));
                Map::new()
            }
            Err(e) => {
                warnings.push(format!("{} is not valid JSON: {e}", path.display()));
                Map::new()
            }
        },
        Err(_) => Map::new(),
    }
}

fn stat(path: &Path) -> FileStat {
    match std::fs::metadata(path) {
        Ok(m) => FileStat {
            present: true,
            path: path.display().to_string(),
            bytes: m.len(),
        },
        Err(_) => FileStat {
            present: false,
            path: path.display().to_string(),
            bytes: 0,
        },
    }
}

fn scalar_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn looks_secret(key: &str) -> bool {
    let k = key.to_lowercase();
    SECRET_KEY_HINTS.iter().any(|h| k.contains(h))
}

fn mask_str(s: &str) -> String {
    let chars: Vec<char> = s.trim().chars().collect();
    if chars.len() <= 4 {
        "••••".to_string()
    } else {
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("••••{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn fixture() -> (TempDir, TempDir) {
        let config = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write(
            &config.path().join("settings.json"),
            r#"{
              "effortLevel": "xhigh",
              "editorMode": "vim",
              "alwaysThinkingEnabled": true,
              "env": { "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "32000" },
              "enabledPlugins": { "superpowers@mkt": true, "warp@mkt": false },
              "permissions": { "defaultMode": "default", "deny": ["Bash(rm:*)"] },
              "mcpServers": {
                "bot-hq": { "type": "http", "url": "http://127.0.0.1:7892/mcp" },
                "stale-only": { "command": "node", "args": ["x.js"] }
              }
            }"#,
        );
        write(
            &home.path().join(".claude.json"),
            r#"{ "mcpServers": {
              "discord": { "command": "npx", "args": ["tsx", "src/index.ts"] },
              "bot-hq": { "command": "./mcp-server.sh" }
            } }"#,
        );
        write(&config.path().join("CLAUDE.md"), "user rules");
        write(
            &config.path().join("skills/note/SKILL.md"),
            "---\nname: note\ndescription: take notes\ndisable-model-invocation: true\n---\nbody",
        );
        write(
            &config.path().join("skills/helper/SKILL.md"),
            "---\nname: helper\ndescription: help\n---\nbody",
        );
        write(
            &config.path().join("projects/proj-a/memory/MEMORY.md"),
            "# mem",
        );
        (config, home)
    }

    fn view() -> ClaudeConfigView {
        let (config, home) = fixture();
        read_at(config.path(), home.path(), "default (~/.claude)", false)
    }

    #[test]
    fn core_knobs_resolve_values_and_unset() {
        let v = view();
        // Effort is surfaced under the env key; with only the legacy
        // `effortLevel` field set, it resolves via the fallback path.
        let effort = v
            .core_knobs
            .iter()
            .find(|k| k.key == "env.CLAUDE_CODE_EFFORT_LEVEL")
            .unwrap();
        assert_eq!(effort.value.as_deref(), Some("xhigh"));
        assert_eq!(effort.source, "~/.claude/settings.json (effortLevel, legacy)");
        let model = v.core_knobs.iter().find(|k| k.key == "model").unwrap();
        assert_eq!(model.value, None);
        assert_eq!(model.source, "unset (default)");
        let tokens = v
            .core_knobs
            .iter()
            .find(|k| k.key == "env.CLAUDE_CODE_MAX_OUTPUT_TOKENS")
            .unwrap();
        assert_eq!(tokens.value.as_deref(), Some("32000"));
    }

    #[test]
    fn effort_env_wins_over_legacy_field() {
        let config = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write(
            &config.path().join("settings.json"),
            r#"{ "effortLevel": "xhigh",
                 "env": { "CLAUDE_CODE_EFFORT_LEVEL": "max" } }"#,
        );
        write(&home.path().join(".claude.json"), r#"{ "mcpServers": {} }"#);
        let v = read_at(config.path(), home.path(), "default (~/.claude)", false);
        let effort = v
            .core_knobs
            .iter()
            .find(|k| k.key == "env.CLAUDE_CODE_EFFORT_LEVEL")
            .unwrap();
        // The env var takes precedence over the legacy `effortLevel` field, and
        // `max` is only expressible via the env path.
        assert_eq!(effort.value.as_deref(), Some("max"));
        assert_eq!(effort.source, "~/.claude/settings.json (env)");
    }

    #[test]
    fn skills_read_frontmatter_disable_flag() {
        let v = view();
        let note = v.skills.iter().find(|s| s.name == "note").unwrap();
        assert!(note.disable_model_invocation);
        assert_eq!(note.kind, "user");
        let helper = v.skills.iter().find(|s| s.name == "helper").unwrap();
        assert!(!helper.disable_model_invocation);
    }

    #[test]
    fn plugins_reflect_enabled_map() {
        let v = view();
        let sp = v
            .plugins
            .iter()
            .find(|p| p.key == "superpowers@mkt")
            .unwrap();
        assert!(sp.enabled);
        let warp = v.plugins.iter().find(|p| p.key == "warp@mkt").unwrap();
        assert!(!warp.enabled);
    }

    #[test]
    fn mcp_flags_ignored_settings_and_reserved_filter() {
        let v = view();
        // discord loads from ~/.claude.json and is forwarded to agents.
        let discord = v.mcp_servers.iter().find(|m| m.name == "discord").unwrap();
        assert!(discord.effective);
        assert!(!discord.reserved_filtered);
        assert_eq!(discord.forwarded_to_agents, vec!["brian"]);
        // bot-hq is reserved → filtered from agents.
        let bothq = v.mcp_servers.iter().find(|m| m.name == "bot-hq").unwrap();
        assert!(bothq.reserved_filtered);
        assert!(bothq.forwarded_to_agents.is_empty());
        // stale-only lives only in settings.json → ignored by claude-code.
        let stale = v
            .mcp_servers
            .iter()
            .find(|m| m.name == "stale-only")
            .unwrap();
        assert!(!stale.effective);
        assert!(stale.loaded_from.contains("ignored"));
        assert!(v.warnings.iter().any(|w| w.contains("settings.json")));
    }

    #[test]
    fn memory_and_permissions_summarized() {
        let v = view();
        assert!(v.memory.user_claude_md.present);
        assert_eq!(v.memory.projects_with_memory, 1);
        assert_eq!(v.permissions.default_mode.as_deref(), Some("default"));
        assert_eq!(v.permissions.deny, 1);
    }

    #[test]
    fn malformed_settings_becomes_warning_not_panic() {
        let config = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write(&config.path().join("settings.json"), "{ not json ]");
        let v = read_at(config.path(), home.path(), "default (~/.claude)", false);
        assert!(v.warnings.iter().any(|w| w.contains("not valid JSON")));
    }

    #[test]
    fn mask_str_redacts_keeping_last_four() {
        assert_eq!(mask_str("abcdef1234"), "••••1234");
        assert_eq!(mask_str("ab"), "••••");
    }
}
