//! Global **Tool Gate** keyword config + matcher + executor.
//!
//! A single, GLOBAL keyword list (one for every session/project) that decides
//! how an agent's Bash tool calls are handled. Each entry is a `{keyword,
//! mode}`: a `gate` keyword makes the command require an Approve/Reject
//! round-trip (surfaced via the `action_gate` MCP tool, which then EXECUTES
//! the command on approval), while an `auto_allow` keyword lets a matching
//! command run with no prompt (the frictionless path for `git commit` /
//! `git push`).
//!
//! Stored as `<data_dir>/config/tool-gate.json` — bot-hq-side, NEVER written into a
//! working repo. The same `load`
//! is read by THREE callers: the Tauri Settings commands (in-process), the
//! `action_gate` bridge method (in-process), and the PreToolUse hook
//! subprocess (which gets `--data-dir` on its command line). They must all
//! agree, so the matching logic lives here once.
//!
//! Matching (per the locked design): **case-insensitive substring** of the
//! keyword against the tool name (`bash` → gates the whole Bash tool) OR the
//! command string (`gh`/`git`/`push` → those commands). This is deliberately
//! NOT the case-sensitive prefix matching the legacy per-project
//! `tool_blocklist` used (that matcher has since been removed).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// How a matching Bash command is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum GateMode {
    /// Block the agent's direct Bash call (PreToolUse exit 2) and route it to
    /// the `action_gate` MCP tool, which surfaces Approve/Reject and — on
    /// approve — executes the command server-side.
    Gate,
    /// Let the command run normally (PreToolUse exit 0). Used to make
    /// `git commit` / `git push` frictionless.
    AutoAllow,
}

/// One global keyword entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct GatedKeyword {
    /// Case-insensitive substring matched against the tool name OR command.
    pub keyword: String,
    pub mode: GateMode,
}

/// Default execution timeout for a gate-approved / auto-allowed command run by
/// `action_gate`. Generous enough for `git push` / `gh` round-trips, short
/// enough that a command waiting on stdin can't hang the session forever.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Combined result of executing a command in the session's working repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// `<data_dir>/config/tool-gate.json`.
pub fn config_path(data_dir: &Path) -> PathBuf {
    crate::paths::config_dir_path(data_dir).join("tool-gate.json")
}

/// Load the global keyword list. **FAIL-OPEN**: a missing file, an unreadable
/// file, or malformed JSON all resolve to an empty list (logged) rather than
/// an error — a config glitch must never brick every Bash call through the
/// PreToolUse hook, which mirrors `run_tool_gate`'s fail-open posture.
pub fn load(data_dir: &Path) -> Vec<GatedKeyword> {
    let path = config_path(data_dir);
    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(?e, path = %path.display(), "tool-gate.json read failed; treating as empty");
            return Vec::new();
        }
    };
    match serde_json::from_str::<Vec<GatedKeyword>>(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(?e, path = %path.display(), "tool-gate.json parse failed; treating as empty");
            Vec::new()
        }
    }
}

/// Persist the global keyword list (pretty JSON). Creates the data dir if
/// needed. Errors are returned (the Settings command surfaces them to the UI).
pub fn save(data_dir: &Path, keywords: &[GatedKeyword]) -> Result<()> {
    let path = config_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating data dir {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(keywords).context("serializing tool-gate keywords")?;
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Decide how a Bash call is handled. Case-insensitive substring of each
/// keyword against `tool_name` OR `command`. **Gate wins over AutoAllow** when
/// a command matches both (fail-safe: prefer asking over silently running).
/// Empty/whitespace-only keywords are ignored (they'd otherwise match
/// everything). `None` = no keyword matched → run normally.
pub fn match_keyword(
    tool_name: &str,
    command: &str,
    keywords: &[GatedKeyword],
) -> Option<GateMode> {
    let tool_lc = tool_name.to_lowercase();
    let cmd_lc = command.to_lowercase();
    let hits = |kw: &str| -> bool {
        let kw_lc = kw.trim().to_lowercase();
        !kw_lc.is_empty() && (tool_lc.contains(&kw_lc) || cmd_lc.contains(&kw_lc))
    };
    if keywords
        .iter()
        .any(|k| k.mode == GateMode::Gate && hits(&k.keyword))
    {
        return Some(GateMode::Gate);
    }
    if keywords
        .iter()
        .any(|k| k.mode == GateMode::AutoAllow && hits(&k.keyword))
    {
        return Some(GateMode::AutoAllow);
    }
    None
}

/// Execute `command` via `sh -c` in `cwd`, capturing combined stdout/stderr +
/// exit code, bounded by `timeout`. stdin is `/dev/null` so a command that
/// expects input (e.g. `gh issue comment` with no `--body`) fails fast instead
/// of hanging. On timeout the child is killed (kill-on-drop) and `code` is 124.
pub async fn run_in_repo(command: &str, cwd: &Path, timeout: Duration) -> CommandOutput {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return CommandOutput {
                stdout: String::new(),
                stderr: format!("failed to spawn `{command}`: {e}"),
                code: -1,
            }
        }
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => CommandOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            code: out.status.code().unwrap_or(-1),
        },
        Ok(Err(e)) => CommandOutput {
            stdout: String::new(),
            stderr: format!("error running `{command}`: {e}"),
            code: -1,
        },
        Err(_) => CommandOutput {
            stdout: String::new(),
            stderr: format!("`{command}` timed out after {}s", timeout.as_secs()),
            code: 124,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn kw(keyword: &str, mode: GateMode) -> GatedKeyword {
        GatedKeyword {
            keyword: keyword.into(),
            mode,
        }
    }

    #[test]
    fn load_missing_returns_empty() {
        let dir = tempdir().unwrap();
        assert!(load(dir.path()).is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempdir().unwrap();
        let kws = vec![
            kw("gh", GateMode::Gate),
            kw("git push", GateMode::AutoAllow),
        ];
        save(dir.path(), &kws).unwrap();
        assert_eq!(load(dir.path()), kws);
    }

    #[test]
    fn load_corrupt_returns_empty_fail_open() {
        // A malformed config must NOT error — the hook fails open so a bad
        // file can't brick every Bash call.
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(config_path(dir.path()).parent().unwrap()).unwrap();
        std::fs::write(config_path(dir.path()), "{ not valid json ]").unwrap();
        assert!(load(dir.path()).is_empty());
    }

    #[test]
    fn mode_serializes_snake_case() {
        let j = serde_json::to_string(&kw("x", GateMode::AutoAllow)).unwrap();
        assert!(j.contains("\"auto_allow\""), "got {j}");
        let g = serde_json::to_string(&GateMode::Gate).unwrap();
        assert_eq!(g, "\"gate\"");
    }

    #[test]
    fn bash_keyword_gates_the_whole_tool() {
        // A `bash` keyword matches the tool name → gates ANY Bash command.
        let kws = vec![kw("bash", GateMode::Gate)];
        assert_eq!(match_keyword("Bash", "ls -la", &kws), Some(GateMode::Gate));
        assert_eq!(
            match_keyword("Bash", "echo hi", &kws),
            Some(GateMode::Gate)
        );
    }

    #[test]
    fn gh_keyword_gates_only_matching_command() {
        let kws = vec![kw("gh issue", GateMode::Gate)];
        assert_eq!(
            match_keyword("Bash", "gh issue comment 41 --body x", &kws),
            Some(GateMode::Gate)
        );
        // Read-only / unrelated commands don't match.
        assert_eq!(match_keyword("Bash", "ls", &kws), None);
        assert_eq!(match_keyword("Bash", "gh pr view 7", &kws), None);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let kws = vec![kw("GH", GateMode::Gate)];
        assert_eq!(
            match_keyword("Bash", "gh issue list", &kws),
            Some(GateMode::Gate)
        );
        let kws = vec![kw("PuSh", GateMode::AutoAllow)];
        assert_eq!(
            match_keyword("Bash", "git push origin main", &kws),
            Some(GateMode::AutoAllow)
        );
    }

    #[test]
    fn gate_wins_over_auto_allow_on_conflict() {
        // "git" gates broadly; "git push" auto-allows. A push matches both —
        // the conservative rule gates it.
        let kws = vec![
            kw("git", GateMode::Gate),
            kw("git push", GateMode::AutoAllow),
        ];
        assert_eq!(
            match_keyword("Bash", "git push origin main", &kws),
            Some(GateMode::Gate)
        );
    }

    #[test]
    fn auto_allow_matches_when_no_gate() {
        let kws = vec![kw("git push", GateMode::AutoAllow)];
        assert_eq!(
            match_keyword("Bash", "git push origin main", &kws),
            Some(GateMode::AutoAllow)
        );
    }

    #[test]
    fn empty_keyword_is_ignored() {
        let kws = vec![kw("   ", GateMode::Gate)];
        assert_eq!(match_keyword("Bash", "anything at all", &kws), None);
    }

    #[test]
    fn no_keywords_means_no_match() {
        assert_eq!(match_keyword("Bash", "gh issue comment", &[]), None);
    }

    #[tokio::test]
    async fn run_in_repo_captures_stdout_and_zero_code() {
        let dir = tempdir().unwrap();
        let out = run_in_repo("echo hello-gate", dir.path(), Duration::from_secs(5)).await;
        assert_eq!(out.code, 0);
        assert!(out.stdout.contains("hello-gate"), "stdout: {:?}", out.stdout);
    }

    #[tokio::test]
    async fn run_in_repo_propagates_nonzero_code() {
        let dir = tempdir().unwrap();
        let out = run_in_repo("exit 3", dir.path(), Duration::from_secs(5)).await;
        assert_eq!(out.code, 3);
    }

    #[tokio::test]
    async fn run_in_repo_captures_stderr() {
        // action_gate returns stderr to the agent, so confirm it's captured
        // independently of stdout (Rain's A1 review gap).
        let dir = tempdir().unwrap();
        let out = run_in_repo("echo oops 1>&2; exit 1", dir.path(), Duration::from_secs(5)).await;
        assert_eq!(out.code, 1);
        assert!(out.stderr.contains("oops"), "stderr: {:?}", out.stderr);
        assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
    }

    #[tokio::test]
    async fn run_in_repo_runs_in_cwd() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();
        let out = run_in_repo("ls", dir.path(), Duration::from_secs(5)).await;
        assert!(out.stdout.contains("marker.txt"), "stdout: {:?}", out.stdout);
    }

    #[tokio::test]
    async fn run_in_repo_times_out() {
        let dir = tempdir().unwrap();
        let out = run_in_repo("sleep 5", dir.path(), Duration::from_millis(150)).await;
        assert_eq!(out.code, 124, "stderr: {:?}", out.stderr);
        assert!(out.stderr.contains("timed out"));
    }
}
