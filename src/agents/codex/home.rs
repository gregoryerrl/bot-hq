//! The Codex home bot-hq manages for a participant (`CODEX_HOME`): the one
//! hooks file it writes there and the trust it grants that file per spawn.
//!
//! On Codex 0.163.0 a hook runs under `codex app-server` only when it is
//! FILE-resident in the home (`<home>/hooks.json`; hooks passed as `-c`
//! session flags or in `thread/start.config` are listed but never fire —
//! measured, s-de77e2f0 M8/M9) AND trusted: Codex keeps a trust hash per hook
//! and skips an untrusted one silently. bot-hq grants that trust per spawn
//! through `-c hooks.state={…}` with the hash `hooks/list` reports for the
//! entry — and only after the listed entry's event, matcher, command and path
//! equal what bot-hq rendered, exactly (plan-eyes P5): a hash is never
//! trusted for a hook bot-hq did not write.
//!
//! The home is bot-hq's: a `hooks.json` that is not bot-hq's own is refused,
//! never merged or overwritten (P5), and the hook fails CLOSED without a
//! session identity, so the file does nothing useful in a human's own Codex
//! session — which is why the Model dialog refuses `~/.codex` as the home.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// The hook's position in bot-hq's file: Codex keys trust by
/// `<source path>:<event>:<group index>:<handler index>`.
const HOOK_KEY_SUFFIX: &str = ":pre_tool_use:0:0";
/// The matcher: every tool, so `apply_patch` and the MCP tools reach the gate
/// too (a read-only role is refused `apply_patch` by it).
const MATCHER: &str = ".*";
/// Seconds before Codex gives up on the hook (its default is 600). The gate
/// does one local HTTP round trip; 30 s is generous and bounds a wedged app.
const TIMEOUT_SECS: u64 = 30;

/// `"<exe>" policy-check codex-gate --data-dir "<dd>"` — the POSIX form Codex
/// runs through `/bin/zsh -lc` (measured), both paths quoted.
fn gate_command(exe: &Path, data_dir: &Path) -> String {
    format!(
        "\"{}\" policy-check codex-gate --data-dir \"{}\"",
        exe.display(),
        data_dir.display()
    )
}

/// The same for Windows (`commandWindows`, PowerShell): `&` runs a quoted
/// program path.
fn gate_command_windows(exe: &Path, data_dir: &Path) -> String {
    format!(
        "& \"{}\" policy-check codex-gate --data-dir \"{}\"",
        exe.display(),
        data_dir.display()
    )
}

/// The hooks file, rendered: one `PreToolUse` group, one command handler.
pub fn hooks_json(exe: &Path, data_dir: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": MATCHER,
                "hooks": [{
                    "type": "command",
                    "command": gate_command(exe, data_dir),
                    "commandWindows": gate_command_windows(exe, data_dir),
                    "timeout": TIMEOUT_SECS,
                }]
            }]
        }
    })
}

/// Whether a hooks file is bot-hq's own: every handler under every event is a
/// command running `policy-check codex-gate`. A file with any other handler —
/// a human's hook, or one added beside ours — is not ours to touch.
pub fn is_bot_hq_hooks_file(v: &serde_json::Value) -> bool {
    let Some(events) = v.get("hooks").and_then(|h| h.as_object()) else {
        return false;
    };
    let mut seen = 0;
    for groups in events.values() {
        let Some(groups) = groups.as_array() else { return false };
        for group in groups {
            let Some(handlers) = group.get("hooks").and_then(|h| h.as_array()) else {
                return false;
            };
            for handler in handlers {
                let is_ours = handler.get("type").and_then(|t| t.as_str()) == Some("command")
                    && handler
                        .get("command")
                        .and_then(|c| c.as_str())
                        .is_some_and(|c| c.contains("policy-check codex-gate"));
                if !is_ours {
                    return false;
                }
                seen += 1;
            }
        }
    }
    seen > 0
}

/// Write bot-hq's hooks file into `home`, creating the home (private) when it
/// is missing. Refuses to touch a `hooks.json` bot-hq did not write.
pub fn write_hooks_json(home: &Path, exe: &Path, data_dir: &Path) -> Result<PathBuf> {
    if !home.exists() {
        std::fs::create_dir_all(home).with_context(|| format!("creating the Codex home {}", home.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700));
    }
    let path = home.join("hooks.json");
    if path.exists() {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let existing: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            anyhow!(
                "{} is not bot-hq's hooks file (it is not JSON: {e}); bot-hq only writes its own \
                 hooks into a Codex home it manages — give it a home of its own",
                path.display()
            )
        })?;
        if !is_bot_hq_hooks_file(&existing) {
            return Err(anyhow!(
                "{} is not bot-hq's hooks file; bot-hq only writes its own hooks into a Codex \
                 home it manages and never merges or replaces someone else's — give it a home \
                 of its own",
                path.display()
            ));
        }
    }
    let body = serde_json::to_string_pretty(&hooks_json(exe, data_dir))? + "\n";
    let tmp = home.join(".hooks.json.tmp");
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(path)
}

/// The key Codex files our hook's trust under.
pub fn hook_key(home: &Path) -> String {
    format!("{}{HOOK_KEY_SUFFIX}", home.join("hooks.json").display())
}

/// A trust grant: the key and the hash `hooks/list` reported for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookTrust {
    pub key: String,
    pub hash: String,
}

/// The `-c` value granting trust: `hooks.state={…}` as ONE table (a key with
/// dots and colons cannot ride the dotted key syntax).
pub fn hooks_state_override(trust: &[HookTrust]) -> String {
    let entries = trust
        .iter()
        .map(|t| {
            format!(
                "{}={{enabled=true,trusted_hash={}}}",
                toml_string(&t.key),
                toml_string(&t.hash)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("hooks.state={{{entries}}}")
}

/// A TOML basic string.
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One entry of `hooks/list`'s answer, the fields the trust decision reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedHook {
    pub key: String,
    pub event_name: String,
    pub matcher: Option<String>,
    pub command: String,
    pub source_path: String,
    pub current_hash: String,
    pub trust_status: String,
    pub enabled: bool,
}

/// What bot-hq rendered into a home, for comparison with what Codex lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedHook {
    pub key: String,
    pub source_path: String,
    pub command: String,
}

impl RenderedHook {
    pub fn new(home: &Path, exe: &Path, data_dir: &Path) -> Self {
        Self {
            key: hook_key(home),
            source_path: home.join("hooks.json").display().to_string(),
            command: gate_command(exe, data_dir),
        }
    }

    /// Whether a listed entry IS this hook: key, event, matcher, command and
    /// file all equal.
    fn is_this_hook(&self, listed: &ListedHook) -> bool {
        listed.key == self.key
            && listed.event_name == "preToolUse"
            && listed.matcher.as_deref() == Some(MATCHER)
            && listed.command == self.command
            && listed.source_path == self.source_path
    }

    /// The trust to grant for a listed entry — only when it is this hook,
    /// exactly (plan-eyes P5: never a hash for a hook bot-hq did not write).
    pub fn trust_for(&self, listed: &ListedHook) -> Option<HookTrust> {
        self.is_this_hook(listed).then(|| HookTrust {
            key: listed.key.clone(),
            hash: listed.current_hash.clone(),
        })
    }

    /// Whether the gate will run: this hook is listed, enabled, and trusted
    /// (or managed). `Err` names what is wrong, for the refusal.
    pub fn armed(&self, listed: &[ListedHook]) -> Result<(), String> {
        let Some(entry) = listed.iter().find(|l| self.is_this_hook(l)) else {
            return Err(format!(
                "bot-hq's gate hook is not listed by Codex at {} — the hooks file was changed \
                 or removed",
                self.source_path
            ));
        };
        if !entry.enabled {
            return Err("bot-hq's gate hook is disabled in this Codex home".to_string());
        }
        match entry.trust_status.as_str() {
            "trusted" | "managed" => Ok(()),
            other => Err(format!("bot-hq's gate hook is {other} in this Codex home (hash {})", entry.current_hash)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// One `PreToolUse` group with a catch-all matcher, one command handler
    /// that runs this binary's `codex-gate` with the data dir — quoted, so a
    /// space in either path survives the shell Codex runs hooks with — and a
    /// PowerShell form for Windows. No other events: compaction and the
    /// handoff reminder are the driver's (plan-eyes P5).
    #[test]
    fn the_hooks_file_is_one_catch_all_gate_hook() {
        let v = hooks_json(Path::new("/Applications/bot hq/bot-hq"), Path::new("/Users/me/.bot-hq"));
        let groups = v["hooks"]["PreToolUse"].as_array().expect("one PreToolUse list");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["matcher"], ".*");
        let handlers = groups[0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0]["type"], "command");
        assert_eq!(
            handlers[0]["command"],
            "\"/Applications/bot hq/bot-hq\" policy-check codex-gate --data-dir \"/Users/me/.bot-hq\""
        );
        assert_eq!(handlers[0]["timeout"], 30);
        assert!(handlers[0]["commandWindows"].as_str().unwrap().contains("policy-check codex-gate"));
        for event in ["SessionStart", "PreCompact", "PostCompact", "Stop", "UserPromptSubmit"] {
            assert!(v["hooks"].get(event).is_none(), "{event} is not bot-hq's to hook");
        }
        assert!(is_bot_hq_hooks_file(&v));
    }

    /// The writer creates a missing home (private), writes the file, rewrites
    /// its own (an older binary path), and REFUSES a file bot-hq did not write
    /// — the user's own hooks are never merged or overwritten.
    #[test]
    fn the_writer_owns_its_file_and_refuses_a_foreign_one() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("codex-home");
        let exe = Path::new("/opt/bot-hq");
        let data = Path::new("/data");
        let written = write_hooks_json(&home, exe, data).unwrap();
        assert_eq!(written, home.join("hooks.json"));
        assert!(home.is_dir());
        let text = std::fs::read_to_string(&written).unwrap();
        assert!(text.contains("policy-check codex-gate"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&home).unwrap().permissions().mode() & 0o777, 0o700);
        }
        // A rewrite from a newer binary path replaces bot-hq's own file.
        write_hooks_json(&home, Path::new("/opt/bot-hq-2"), data).unwrap();
        let text = std::fs::read_to_string(&written).unwrap();
        assert!(text.contains("/opt/bot-hq-2") && !text.contains("\"/opt/bot-hq\""));
        // Someone else's hooks: refused, untouched.
        let foreign = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/usr/local/bin/my-linter"}]}]}}"#;
        std::fs::write(&written, foreign).unwrap();
        let err = write_hooks_json(&home, exe, data).unwrap_err().to_string();
        assert!(err.contains("not bot-hq's"), "{err}");
        assert_eq!(std::fs::read_to_string(&written).unwrap(), foreign, "untouched");
        // So is a file that holds bot-hq's hook AND another one.
        let mixed = r#"{"hooks":{"PreToolUse":[{"matcher":".*","hooks":[{"type":"command","command":"\"/opt/bot-hq\" policy-check codex-gate --data-dir \"/data\""},{"type":"command","command":"/usr/local/bin/my-linter"}]}]}}"#;
        std::fs::write(&written, mixed).unwrap();
        assert!(write_hooks_json(&home, exe, data).is_err());
        // And one that is not JSON at all.
        std::fs::write(&written, "{not json").unwrap();
        assert!(write_hooks_json(&home, exe, data).is_err());
    }

    /// Trust is granted per spawn as one `-c` override holding the whole
    /// `hooks.state` table — the key holds dots and colons, which the dotted
    /// `-c` key syntax cannot carry (measured: the dotted form was ignored,
    /// the table form answered `trusted`).
    #[test]
    fn the_trust_override_is_one_whole_table() {
        let home = Path::new("/Users/me/.codex-bot-hq");
        let key = hook_key(home);
        assert_eq!(key, "/Users/me/.codex-bot-hq/hooks.json:pre_tool_use:0:0");
        let flag = hooks_state_override(&[HookTrust { key: key.clone(), hash: "sha256:abc".into() }]);
        assert_eq!(
            flag,
            r#"hooks.state={"/Users/me/.codex-bot-hq/hooks.json:pre_tool_use:0:0"={enabled=true,trusted_hash="sha256:abc"}}"#
        );
        assert_eq!(hooks_state_override(&[]), "hooks.state={}");
    }

    /// `hooks/list`'s entry is trusted only when it IS the hook bot-hq wrote:
    /// same event, same matcher, same command string, same file. A listed
    /// entry that differs in any of them — someone edited the file, or a
    /// different hook sits at index 0 — gets no trust, whatever hash it shows.
    #[test]
    fn a_listed_entry_earns_trust_only_when_it_matches_what_was_rendered() {
        let home = Path::new("/Users/me/.codex-bot-hq");
        let rendered = RenderedHook::new(home, Path::new("/opt/bot-hq"), Path::new("/data"));
        let listed = |event: &str, matcher: &str, command: &str, path: &str| ListedHook {
            key: hook_key(home),
            event_name: event.into(),
            matcher: Some(matcher.into()),
            command: command.into(),
            source_path: path.into(),
            current_hash: "sha256:abc".into(),
            trust_status: "untrusted".into(),
            enabled: true,
        };
        let ok = listed("preToolUse", ".*", &rendered.command, "/Users/me/.codex-bot-hq/hooks.json");
        assert_eq!(rendered.trust_for(&ok).map(|t| t.hash), Some("sha256:abc".to_string()));
        let mut other = ok.clone();
        other.command = "/usr/local/bin/my-linter".into();
        assert!(rendered.trust_for(&other).is_none(), "another command");
        let mut other = ok.clone();
        other.matcher = Some("Bash".into());
        assert!(rendered.trust_for(&other).is_none(), "another matcher");
        let mut other = ok.clone();
        other.event_name = "postToolUse".into();
        assert!(rendered.trust_for(&other).is_none(), "another event");
        let mut other = ok.clone();
        other.source_path = "/Users/me/.codex/hooks.json".into();
        assert!(rendered.trust_for(&other).is_none(), "another file");
        let mut other = ok.clone();
        other.key = "/Users/me/.codex-bot-hq/hooks.json:pre_tool_use:1:0".into();
        assert!(rendered.trust_for(&other).is_none(), "another position");
    }

    /// What the spawn checks before the first turn and before every turn
    /// after (plan-eyes G7): the rendered hook is listed, enabled and
    /// trusted — anything else is a reason to refuse, spelled out.
    #[test]
    fn the_gate_is_armed_only_when_listed_enabled_and_trusted() {
        let home = Path::new("/Users/me/.codex-bot-hq");
        let rendered = RenderedHook::new(home, Path::new("/opt/bot-hq"), Path::new("/data"));
        let mut entry = ListedHook {
            key: hook_key(home),
            event_name: "preToolUse".into(),
            matcher: Some(".*".into()),
            command: rendered.command.clone(),
            source_path: "/Users/me/.codex-bot-hq/hooks.json".into(),
            current_hash: "sha256:abc".into(),
            trust_status: "trusted".into(),
            enabled: true,
        };
        assert_eq!(rendered.armed(&[entry.clone()]), Ok(()));
        entry.trust_status = "untrusted".into();
        assert!(rendered.armed(&[entry.clone()]).unwrap_err().contains("untrusted"));
        entry.trust_status = "modified".into();
        assert!(rendered.armed(&[entry.clone()]).unwrap_err().contains("modified"));
        entry.trust_status = "trusted".into();
        entry.enabled = false;
        assert!(rendered.armed(&[entry.clone()]).unwrap_err().contains("disabled"));
        assert!(rendered.armed(&[]).unwrap_err().contains("not listed"));
        entry.enabled = true;
        entry.command = "/usr/local/bin/my-linter".into();
        assert!(rendered.armed(&[entry]).unwrap_err().contains("not listed"), "a different hook at the key is not ours");
    }
}
