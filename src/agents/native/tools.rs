//! Built-in tools the native loop implements itself.
//!
//! v1 ships `read_file` only — deliberately. The point of B3 is to prove the
//! loop survives the supervisor, the duo pump and a live session; an agent with
//! no `Bash` has nothing dangerous to get wrong while that is being established.
//! `Grep` / `Glob` / `Bash` (and the write-verb deny matcher `Bash` requires)
//! land in B5.
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

use super::wire::{ToolCall, ToolOutcome};

/// Cap tool output so one read cannot blow the context window.
pub const MAX_FILE_BYTES: usize = 256 * 1024;

/// Anthropic `tools` entries for the built-ins, ready to concatenate with the
/// converted MCP tool list.
///
/// **Empty when there is no read root.** A session with no working repo has no
/// directory this agent is entitled to read, so the tool is not offered at all
/// rather than pointed somewhere arbitrary — see [`exec`].
pub fn tool_defs_for(root: Option<&Path>) -> Vec<Value> {
    if root.is_none() {
        return Vec::new();
    }
    tool_defs()
}

/// The built-in definitions, unconditionally. Prefer [`tool_defs_for`].
pub fn tool_defs() -> Vec<Value> {
    vec![json!({
        "name": "read_file",
        "description": "Read a UTF-8 text file from the working repository. \
                        Paths are relative to the repository root.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path relative to the repository root, e.g. \"Cargo.toml\"."
                }
            },
            "required": ["path"]
        }
    })]
}

/// Is `name` one of the built-ins this module handles?
pub fn handles(name: &str) -> bool {
    name == "read_file"
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
pub fn exec(call: &ToolCall, root: Option<&Path>) -> ToolOutcome {
    let outcome = |content: String, is_error: bool| ToolOutcome {
        tool_use_id: call.id.clone(),
        content,
        is_error,
    };

    let Some(root) = root else {
        return outcome(
            "this session has no working repository, so there is no directory this \
             agent may read. Ask your peer to paste the contents you need."
                .to_string(),
            true,
        );
    };

    match run(call, root) {
        Ok(text) => outcome(text, false),
        Err(msg) => outcome(msg, true),
    }
}

fn run(call: &ToolCall, root: &Path) -> Result<String, String> {
    match call.name.as_str() {
        "read_file" => {
            let path = call
                .input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing required string field \"path\"".to_string())?;
            let target = resolve_in_root(root, path)?;

            let bytes = std::fs::read(&target).map_err(|e| format!("read failed: {e}"))?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err(format!(
                    "file is {} bytes; reads are capped at {MAX_FILE_BYTES}",
                    bytes.len()
                ));
            }
            String::from_utf8(bytes).map_err(|_| "file is not valid UTF-8".to_string())
        }
        other => Err(format!("unknown tool {other:?}")),
    }
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

    #[test]
    fn reads_a_file_inside_the_root() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "a.txt"), Some(&root));
        assert!(!out.is_error);
        assert_eq!(out.content, "hello");
        assert_eq!(out.tool_use_id, "tu_1");
    }

    #[test]
    fn refuses_a_dotdot_escape() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "../../etc/hosts"), Some(&root));
        assert!(out.is_error, "`..` must not escape the root");
    }

    #[test]
    fn refuses_an_absolute_path() {
        // `Path::join` lets an absolute component replace the base outright —
        // the canonicalized-prefix check is what catches it.
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("read_file", "/etc/hosts"), Some(&root));
        assert!(out.is_error, "an absolute path must not replace the root");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_pointing_outside_the_root() {
        // A string prefix check passes this; canonicalizing the candidate is
        // what makes the gate real.
        let (dir, root) = root_with_file("a.txt", "hello");
        let outside = TempDir::new().unwrap();
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "classified").unwrap();
        std::os::unix::fs::symlink(&secret, dir.path().join("link.txt")).unwrap();

        let out = exec(&call("read_file", "link.txt"), Some(&root));
        assert!(out.is_error, "a symlink out of the root must be refused");
        assert!(!out.content.contains("classified"));
    }

    #[test]
    fn missing_path_argument_is_a_readable_error_not_a_panic() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(
            &ToolCall {
                id: "tu_1".into(),
                name: "read_file".into(),
                input: json!({}),
            },
            Some(&root),
        );
        assert!(out.is_error);
        assert!(out.content.contains("path"));
    }

    #[test]
    fn no_root_refuses_every_read_rather_than_falling_back() {
        // The B4 defect: a repo-less session used to fall back to the process
        // cwd, which is bot-hq's own data dir — `.local/mcp-token`,
        // `bot-hq.db` with every auth token, and the whole Context Library.
        let out = exec(&call("read_file", "Cargo.toml"), None);
        assert!(out.is_error);
        assert!(out.content.contains("no working repository"));
    }

    #[test]
    fn no_root_means_the_tool_is_not_even_offered() {
        // Belt and braces: refusing at exec time is the guarantee, but the model
        // should not be told the tool exists in the first place.
        assert!(tool_defs_for(None).is_empty());
        let dir = TempDir::new().unwrap();
        assert_eq!(tool_defs_for(Some(dir.path())).len(), 1);
    }

    #[test]
    fn oversized_file_is_refused_rather_than_truncated() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("big.txt"), vec![b'x'; MAX_FILE_BYTES + 1]).unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(&call("read_file", "big.txt"), Some(&root));
        assert!(out.is_error);
        // Silent truncation would hand the model a file it thinks it read whole.
        assert!(out.content.contains("capped"));
    }

    #[test]
    fn non_utf8_is_reported_not_lossily_decoded() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bin"), [0xff, 0xfe, 0x00]).unwrap();
        let root = dir.path().canonicalize().unwrap();

        let out = exec(&call("read_file", "bin"), Some(&root));
        assert!(out.is_error);
        assert!(out.content.contains("UTF-8"));
    }

    #[test]
    fn unknown_tool_is_an_error_outcome() {
        let (_d, root) = root_with_file("a.txt", "hello");
        let out = exec(&call("write_file", "a.txt"), Some(&root));
        assert!(out.is_error);
        assert!(out.content.contains("unknown tool"));
    }

    #[test]
    fn handles_reports_only_what_exec_implements() {
        assert!(handles("read_file"));
        assert!(!handles("Bash"));
        assert!(!handles("cl_index_search"));
    }

    #[test]
    fn tool_defs_use_the_messages_api_schema_key() {
        let defs = tool_defs();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0]["name"], "read_file");
        assert!(defs[0]["input_schema"].is_object());
        assert!(defs[0].get("inputSchema").is_none());
    }
}
