//! Built-in tools the native loop implements itself.
//!
//! ## Role filtering
//!
//! Tools are classified by [`Access`] and filtered per agent by
//! [`ToolPolicy`]. EYES gets the read set and nothing else — not by prompt, by
//! construction: a write tool is absent from the advertised `tools` array AND
//! refused at exec time, so there is no path to it even if the model invents the
//! name.
//!
//! This is a **role** decision, not a limit of the loop. The loop can express any
//! policy; which one an agent gets depends on what that agent is for.
//!
//! ## Read scope
//!
//! Every path is resolved beneath a root and refused if it escapes. This is not
//! defence in depth — it is the *only* read gate that has ever existed for these
//! tools. On the claude-code path `Read`/`Grep`/`Glob` are not `Bash`, so they
//! never reach the Tool Gate, and the Tool Gate's PreToolUse hook is injected in
//! the HANDS branch only; the session's `working_repo_path` is a prompt
//! convention with no mechanism behind it (`issues.md` #1). A native agent
//! implements these tools itself, so the check is free here.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::command::{self, CommandPolicy};
use super::wire::{ToolCall, ToolOutcome};

/// Whether a tool reads or mutates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// Which built-ins an agent may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicy {
    /// Read tools only. The EYES preset.
    ReadOnly,
    /// Read + write. No agent uses this yet; it exists so the role filter has a
    /// second value and is therefore actually a filter rather than a constant.
    ReadWrite,
}

impl ToolPolicy {
    /// The policy for `agent_name`.
    ///
    /// EYES is read-only because EYES reviews. Anything else also gets read-only:
    /// no native HANDS exists, and defaulting an unrecognised role to write access
    /// is the wrong direction to be wrong in.
    pub fn for_agent(agent_name: &str) -> Self {
        match agent_name {
            "rain" => Self::ReadOnly,
            _ => Self::ReadOnly,
        }
    }

    fn allows(self, access: Access) -> bool {
        match self {
            Self::ReadOnly => access == Access::Read,
            Self::ReadWrite => true,
        }
    }
}

/// Every built-in, with its access class.
const TOOLS: &[(&str, Access)] = &[
    ("read_file", Access::Read),
    ("list_files", Access::Read),
    ("search_files", Access::Read),
    ("run_command", Access::Read),
    ("write_file", Access::Write),
];

/// Cap on `search_files` hits.
pub const MAX_SEARCH_HITS: usize = 200;
/// Cap on `list_files` results.
pub const MAX_GLOB_HITS: usize = 500;

/// Cap tool output so one read cannot blow the context window.
pub const MAX_FILE_BYTES: usize = 256 * 1024;

/// Anthropic `tools` entries for the built-ins, ready to concatenate with the
/// converted MCP tool list.
///
/// **Empty when there is no read root.** A session with no working repo has no
/// directory this agent is entitled to read, so the tool is not offered at all
/// rather than pointed somewhere arbitrary — see [`exec`].
pub fn tool_defs_for(root: Option<&Path>, policy: ToolPolicy) -> Vec<Value> {
    if root.is_none() {
        return Vec::new();
    }
    tool_defs()
        .into_iter()
        .filter(|d| {
            d["name"]
                .as_str()
                .and_then(access_of)
                .is_some_and(|a| policy.allows(a))
        })
        .collect()
}

/// The access class of a built-in, or `None` if it isn't one.
pub fn access_of(name: &str) -> Option<Access> {
    TOOLS.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
}

/// Every built-in definition, unfiltered. Prefer [`tool_defs_for`].
pub fn tool_defs() -> Vec<Value> {
    vec![
        json!({
            "name": "read_file",
            "description": "Read a UTF-8 text file from the working repository. \
                            Paths are relative to the repository root.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to the repository root, e.g. \"Cargo.toml\"." }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "list_files",
            "description": "List repository files matching a glob. Use this instead of \
                            shelling out to `find` or `ls`.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob relative to the repository root, e.g. \"src/**/*.rs\"." }
                },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "search_files",
            "description": "Search file CONTENTS with a regular expression and return \
                            matching lines as `path:line: text`. Use this instead of \
                            shelling out to `grep`.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Rust regex, e.g. \"fn spawn_\\\\w+\"." },
                    "glob": { "type": "string", "description": "Optional glob limiting which files are searched, e.g. \"src/**/*.rs\"." }
                },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "run_command",
            "description": "Run ONE read-only command in the repository root. No shell: no \
                            pipes, chaining or redirection. Only an allow-listed set of \
                            reporting commands is permitted (git/gh read subcommands, cat, \
                            ls, wc, head, tail, find, which, file, stat, du, npm ls, \
                            composer show, cargo tree). Anything that could change state is \
                            refused — ask your peer to run it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command, e.g. \"git log --oneline -5\"." }
                },
                "required": ["command"]
            }
        }),
        json!({
            "name": "write_file",
            "description": "Write a UTF-8 text file in the working repository, creating or \
                            replacing it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to the repository root." },
                    "content": { "type": "string", "description": "Full new file contents." }
                },
                "required": ["path", "content"]
            }
        }),
    ]
}

/// Is `name` one of the built-ins this module handles?
pub fn handles(name: &str) -> bool {
    access_of(name).is_some()
}

/// Resolve `rel` beneath `root`, refusing anything that escapes.
///
/// `root` MUST already be canonicalized. Canonicalizing the candidate too is
/// what makes this hold against `..` *and* symlinks — a plain string prefix
/// check passes both. Note `Path::join` lets an absolute `rel` replace the base
/// entirely, so `/etc/passwd` lands outside `root` and is caught here.
pub fn resolve_in_root(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let canonical = root
        .join(rel)
        .canonicalize()
        .map_err(|e| format!("cannot resolve {rel:?}: {e}"))?;

    if !canonical.starts_with(root) {
        return Err(format!("{rel:?} resolves outside the repository root — refused"));
    }
    Ok(canonical)
}

/// Execute a built-in. Returns an `is_error` outcome rather than failing the
/// turn: errors are inputs to the loop, and the model recovers from them.
///
/// `root` is `None` for a session with no working repo. **There is no fallback
/// root, deliberately.** Defaulting to the process's current directory scopes
/// the agent to wherever bot-hq happens to have been launched from — which in
/// practice is the bot-hq data directory, containing `.local/mcp-token` (a
/// UUID: `0600`, but the agent runs as the same user, and it is valid UTF-8 so
/// it reads cleanly), `.local/bot-hq.db` with every `models.auth_token` in
/// plaintext, and the whole Context Library. A read gate aimed at the secrets
/// is worse than no read gate, because it looks like protection.
pub async fn exec(call: &ToolCall, root: Option<&Path>, policy: ToolPolicy) -> ToolOutcome {
    let outcome = |content: String, is_error: bool| ToolOutcome {
        tool_use_id: call.id.clone(),
        content,
        is_error,
    };

    // Role gate, enforced independently of the advertised tool list. A model that
    // invents a tool name it was never offered still cannot reach a write.
    match access_of(&call.name) {
        None => return outcome(format!("unknown tool {:?}", call.name), true),
        Some(access) if !policy.allows(access) => {
            return outcome(
                format!(
                    "`{}` mutates state and is not available to this agent — that is a role \
                     boundary, not a missing feature. Ask your peer to do it.",
                    call.name
                ),
                true,
            );
        }
        Some(_) => {}
    }

    let Some(root) = root else {
        return outcome(
            "this session has no working repository, so there is no directory this \
             agent may read. Ask your peer to paste the contents you need."
                .to_string(),
            true,
        );
    };

    match run(call, root, policy).await {
        Ok(text) => outcome(text, false),
        Err(msg) => outcome(msg, true),
    }
}

fn str_arg<'a>(call: &'a ToolCall, key: &str) -> Result<&'a str, String> {
    call.input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required string field {key:?}"))
}

async fn run(call: &ToolCall, root: &Path, policy: ToolPolicy) -> Result<String, String> {
    match call.name.as_str() {
        "read_file" => {
            let target = resolve_in_root(root, str_arg(call, "path")?)?;
            let bytes = std::fs::read(&target).map_err(|e| format!("read failed: {e}"))?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err(format!(
                    "file is {} bytes; reads are capped at {MAX_FILE_BYTES}",
                    bytes.len()
                ));
            }
            String::from_utf8(bytes).map_err(|_| "file is not valid UTF-8".to_string())
        }

        "list_files" => list_files(root, str_arg(call, "pattern")?),

        "search_files" => search_files(
            root,
            str_arg(call, "pattern")?,
            call.input.get("glob").and_then(Value::as_str),
        ),

        "run_command" => {
            let cmd = str_arg(call, "command")?;
            let cp = match policy {
                ToolPolicy::ReadOnly => CommandPolicy::ReadOnly,
                ToolPolicy::ReadWrite => CommandPolicy::ReadOnly,
            };
            command::run(cmd, root, cp).await
        }

        "write_file" => {
            let target = root.join(str_arg(call, "path")?);
            // Re-check the parent: `resolve_in_root` needs an existing path, and a
            // new file has none yet.
            let parent = target
                .parent()
                .ok_or_else(|| "path has no parent directory".to_string())?;
            let canonical_parent = parent
                .canonicalize()
                .map_err(|e| format!("cannot resolve parent of the target: {e}"))?;
            if !canonical_parent.starts_with(root) {
                return Err("target resolves outside the repository root — refused".into());
            }
            std::fs::write(&target, str_arg(call, "content")?)
                .map_err(|e| format!("write failed: {e}"))?;
            Ok(format!("wrote {}", target.display()))
        }

        other => Err(format!("unknown tool {other:?}")),
    }
}

/// Glob beneath the root. Every hit is re-checked through [`resolve_in_root`]
/// because the PATTERN is untrusted — `../../**` would otherwise walk out.
fn list_files(root: &Path, pattern: &str) -> Result<String, String> {
    if pattern.starts_with('/') || pattern.split('/').any(|s| s == "..") {
        return Err(format!(
            "{pattern:?} points outside the repository root — refused"
        ));
    }
    let joined = root.join(pattern);
    let entries = glob::glob(&joined.to_string_lossy())
        .map_err(|e| format!("bad glob {pattern:?}: {e}"))?;

    let mut out = Vec::new();
    for entry in entries.flatten() {
        if !entry.canonicalize().is_ok_and(|c| c.starts_with(root)) {
            continue;
        }
        if let Ok(rel) = entry.strip_prefix(root) {
            out.push(rel.to_string_lossy().into_owned());
        }
        if out.len() >= MAX_GLOB_HITS {
            out.push(format!("… capped at {MAX_GLOB_HITS} results"));
            break;
        }
    }
    if out.is_empty() {
        return Ok(format!("no files match {pattern:?}"));
    }
    out.sort();
    Ok(out.join("\n"))
}

/// Regex over file contents beneath the root.
///
/// Skips `.git/` and anything that isn't valid UTF-8 — a binary "match" is noise,
/// and decoding lossily would report line numbers that don't exist.
fn search_files(root: &Path, pattern: &str, file_glob: Option<&str>) -> Result<String, String> {
    let re = regex::Regex::new(pattern).map_err(|e| format!("bad regex {pattern:?}: {e}"))?;
    let listing = list_files(root, file_glob.unwrap_or("**/*"))?;

    let mut hits = Vec::new();
    for rel in listing.lines() {
        if rel.starts_with('.') && rel.split('/').next() == Some(".git") {
            continue;
        }
        let path = root.join(rel);
        if !path.is_file() {
            continue;
        }
        let Ok(body) = std::fs::read(&path) else {
            continue;
        };
        let Ok(text) = String::from_utf8(body) else {
            continue; // binary
        };
        for (n, line) in text.lines().enumerate() {
            if re.is_match(line) {
                let trimmed: String = line.chars().take(300).collect();
                hits.push(format!("{rel}:{}: {}", n + 1, trimmed));
                if hits.len() >= MAX_SEARCH_HITS {
                    hits.push(format!("… capped at {MAX_SEARCH_HITS} matches"));
                    return Ok(hits.join("\n"));
                }
            }
        }
    }
    if hits.is_empty() {
        return Ok(format!("no matches for {pattern:?}"));
    }
    Ok(hits.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn call(name: &str, path: &str) -> ToolCall {
        ToolCall {
            id: "tu_1".into(),
            name: name.into(),
            input: json!({ "path": path }),
        }
    }

    fn root_with_file(name: &str, body: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(name), body).unwrap();
        let root = dir.path().canonicalize().unwrap();
        (dir, root)
    }

    #[tokio::test]
    async     fn reads_a_file_inside_the_root() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "a.txt"), Some(&root), ToolPolicy::ReadOnly).await;
        assert!(!out.is_error);
        assert_eq!(out.content, "hello");
        assert_eq!(out.tool_use_id, "tu_1");
    }

    #[tokio::test]
    async     fn refuses_a_dotdot_escape() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "../../etc/hosts"), Some(&root), ToolPolicy::ReadOnly).await;
        assert!(out.is_error, "`..` must not escape the root");
    }

    #[tokio::test]
    async     fn refuses_an_absolute_path() {
        // `Path::join` lets an absolute component replace the base outright —
        // the canonicalized-prefix check is what catches it.
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "/etc/hosts"), Some(&root), ToolPolicy::ReadOnly).await;
        assert!(out.is_error, "an absolute path must not replace the root");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_a_symlink_pointing_outside_the_root() {
        // A string prefix check passes this; canonicalizing the candidate is
        // what makes the gate real.
        let (dir, root) = root_with_file("a.txt", "hello");
        let outside = TempDir::new().unwrap();
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "classified").unwrap();
        std::os::unix::fs::symlink(&secret, dir.path().join("link.txt")).unwrap();

        let out = exec(&call("read_file", "link.txt"), Some(&root), ToolPolicy::ReadOnly).await;
        assert!(out.is_error, "a symlink out of the root must be refused");
        assert!(!out.content.contains("classified"));
    }

    #[tokio::test]
    async     fn missing_path_argument_is_a_readable_error_not_a_panic() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "tu_1".into(),
                name: "read_file".into(),
                input: json!({}),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
        )
        .await;
        assert!(out.is_error);
        assert!(out.content.contains("path"));
    }

    #[tokio::test]
    async     fn no_root_refuses_every_read_rather_than_falling_back() {
        // The B4 defect: a repo-less session used to fall back to the process
        // cwd, which is bot-hq's own data dir — `.local/mcp-token`,
        // `bot-hq.db` with every auth token, and the whole Context Library.
        let out = exec(&call("read_file", "Cargo.toml"), None, ToolPolicy::ReadOnly).await;
        assert!(out.is_error);
        assert!(out.content.contains("no working repository"));
    }

    #[test]
    fn no_root_means_the_tool_is_not_even_offered() {
        // Belt and braces: refusing at exec time is the guarantee, but the model
        // should not be told the tool exists in the first place.
        assert!(tool_defs_for(None, ToolPolicy::ReadOnly).is_empty());
        let dir = TempDir::new().unwrap();
        assert!(!tool_defs_for(Some(dir.path()), ToolPolicy::ReadOnly).is_empty());
    }

    #[tokio::test]
    async     fn oversized_file_is_refused_rather_than_truncated() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("big.txt"), vec![b'x'; MAX_FILE_BYTES + 1]).unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(&call("read_file", "big.txt"), Some(&root), ToolPolicy::ReadOnly).await;
        assert!(out.is_error);
        // Silent truncation would hand the model a file it thinks it read whole.
        assert!(out.content.contains("capped"));
    }

    #[tokio::test]
    async     fn non_utf8_is_reported_not_lossily_decoded() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bin"), [0xff, 0xfe, 0x00]).unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(&call("read_file", "bin"), Some(&root), ToolPolicy::ReadOnly).await;
        assert!(out.is_error);
        assert!(out.content.contains("UTF-8"));
    }

    #[tokio::test]
    async     fn unknown_tool_is_an_error_outcome() {
        let (_d, root) = root_with_file("a.txt", "hello");
        // A name that is not a built-in at all. `write_file` IS one — it's refused
        // by role, which is a different message and a different test.
        let out = exec(&call("nuke_everything", "a.txt"), Some(&root), ToolPolicy::ReadOnly).await;
        assert!(out.is_error);
        assert!(out.content.contains("unknown tool"));
    }

    // ---- role filtering -------------------------------------------------

    #[test]
    fn eyes_is_never_offered_a_write_tool() {
        let dir = TempDir::new().unwrap();
        let offered: Vec<String> = tool_defs_for(Some(dir.path()), ToolPolicy::ReadOnly)
            .iter()
            .map(|d| d["name"].as_str().unwrap().to_string())
            .collect();

        assert!(offered.contains(&"read_file".to_string()));
        assert!(offered.contains(&"search_files".to_string()));
        assert!(offered.contains(&"list_files".to_string()));
        assert!(offered.contains(&"run_command".to_string()));
        assert!(
            !offered.contains(&"write_file".to_string()),
            "a write tool was advertised to EYES: {offered:?}"
        );

        // Every write tool must be absent, not just the ones named above.
        for (name, access) in TOOLS {
            if *access == Access::Write {
                assert!(!offered.contains(&name.to_string()), "{name} leaked");
            }
        }
    }

    #[tokio::test]
    async fn a_write_tool_is_refused_even_when_invented_by_name() {
        // Filtering the advertised list is not the guarantee — the exec-time gate
        // is. A model that names a tool it was never offered must still be refused.
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "tu_1".into(),
                name: "write_file".into(),
                input: json!({ "path": "a.txt", "content": "overwritten" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
        )
        .await;

        assert!(out.is_error);
        assert!(out.content.contains("role boundary"));
        // And the file is untouched.
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "hello");
    }

    #[tokio::test]
    async fn read_write_policy_can_actually_write() {
        // Proves the filter is a filter, not a constant.
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "tu_1".into(),
                name: "write_file".into(),
                input: json!({ "path": "a.txt", "content": "new" }),
            },
            Some(&root),
            ToolPolicy::ReadWrite,
        )
        .await;

        assert!(!out.is_error, "{}", out.content);
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "new");
    }

    #[tokio::test]
    async fn a_write_cannot_escape_the_root_even_under_read_write() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "tu_1".into(),
                name: "write_file".into(),
                input: json!({ "path": "../escaped.txt", "content": "x" }),
            },
            Some(&root),
            ToolPolicy::ReadWrite,
        )
        .await;
        assert!(out.is_error);
        assert!(!root.parent().unwrap().join("escaped.txt").exists());
    }

    #[test]
    fn every_agent_currently_resolves_to_read_only() {
        for agent in ["rain", "brian", "unknown"] {
            assert_eq!(ToolPolicy::for_agent(agent), ToolPolicy::ReadOnly);
        }
    }

    // ---- search_files / list_files ---------------------------------------

    #[tokio::test]
    async fn search_files_returns_path_line_and_text() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.rs"), "fn one() {}\nfn two() {}\n").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "search_files".into(),
                input: json!({ "pattern": r"fn two" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
        )
        .await;

        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("a.rs:2:"), "{}", out.content);
        assert!(!out.content.contains("fn one"));
    }

    #[tokio::test]
    async fn search_files_reports_a_bad_regex_instead_of_panicking() {
        let (_d, root) = root_with_file("a.txt", "x");
        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "search_files".into(),
                input: json!({ "pattern": "(unclosed" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
        )
        .await;
        assert!(out.is_error);
        assert!(out.content.contains("bad regex"));
    }

    #[tokio::test]
    async fn list_files_matches_a_glob_and_refuses_an_escaping_one() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "").unwrap();
        fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let hit = exec(
            &ToolCall {
                id: "t".into(),
                name: "list_files".into(),
                input: json!({ "pattern": "src/**/*.rs" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
        )
        .await;
        assert!(!hit.is_error, "{}", hit.content);
        assert!(hit.content.contains("src/main.rs"));
        assert!(hit.content.contains("src/lib.rs"));

        for bad in ["../**/*", "/etc/*"] {
            let out = exec(
                &ToolCall {
                    id: "t".into(),
                    name: "list_files".into(),
                    input: json!({ "pattern": bad }),
                },
                Some(&root),
                ToolPolicy::ReadOnly,
            )
            .await;
            assert!(out.is_error, "glob {bad} should be refused");
        }
    }

    #[tokio::test]
    async fn run_command_refuses_a_mutation_through_the_tool_layer() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "t".into(),
                name: "run_command".into(),
                input: json!({ "command": "rm a.txt" }),
            },
            Some(&root),
            ToolPolicy::ReadOnly,
        )
        .await;
        assert!(out.is_error);
        assert!(root.join("a.txt").exists(), "the file was deleted");
    }

    #[test]
    fn handles_reports_only_what_exec_implements() {
        assert!(handles("read_file"));
        assert!(!handles("Bash"));
        assert!(handles("write_file"));
        assert!(!handles("cl_index_search"));
    }

    #[test]
    fn tool_defs_use_the_messages_api_schema_key() {
        for def in tool_defs() {
            assert!(def["name"].is_string());
            assert!(def["input_schema"].is_object(), "{}", def["name"]);
            assert!(def.get("inputSchema").is_none());
        }
    }
}
