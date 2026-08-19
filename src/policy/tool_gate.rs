//! Global **Tool Gate** keyword config + matcher + executor.
//!
//! A single, GLOBAL keyword list (one for every session/project; a session
//! snapshot may override it — `resolve_keywords`) that decides how an
//! edit-capable participant's Bash tool calls are handled (the PreToolUse hook
//! is injected only for roles holding `edit_files`; a read-only participant's
//! Bash is bounded by `--disallowedTools` instead). Each entry is a `{keyword,
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

/// Whether `shell` (a bare name or a path) runs `-c true` from here. Memoised
/// per NAME for the life of the process — the answer can't change mid-run in
/// any way we care about. (Round 9 found the earlier `bash_present()` memoised
/// into ONE unkeyed cell under a `name` parameter; keyed now, because the
/// shell choice below asks about more than one.)
fn shell_present(shell: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static PRESENT: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = PRESENT.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(&known) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(shell) {
        return known;
    }
    let present = std::process::Command::new(shell)
        .arg("-c")
        .arg("true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(shell.to_string(), present);
    present
}

/// Shells `$SHELL` is allowed to nominate for a gated command: POSIX-family
/// only. A fish / nu / csh login shell would break strictly more of the
/// `$(…)` / heredoc shapes agents write than any of these do.
const GATE_SHELL_ALLOWLIST: [&str; 5] = ["zsh", "bash", "sh", "dash", "ksh"];

/// The shell an approved gated command runs under — **the shell the agent
/// validated the command in**, as far as it can be known (round 10, B2).
///
/// claude-code's own Bash tool runs each command through the user's LOGIN
/// shell (`/bin/zsh -c … eval '<cmd>'` on this Mac), so an agent's command has
/// only ever been proven under `$SHELL`. Round 7 moved this from `sh` to `bash`
/// for exactly the failure that came back in `s-766f4ab9`: a heredoc inside
/// `$(…)` whose body carries an apostrophe (`Dependabot's`) is fine under zsh
/// and dies under macOS's `/bin/bash` **3.2** with "unexpected EOF while
/// looking for matching `''" — two APPROVED PR-creating gates ran, exited 2,
/// created nothing, and their approvals were spent.
///
/// Order: `$SHELL` when its basename is on [`GATE_SHELL_ALLOWLIST`] and it
/// runs from here; else `zsh`, `bash`, `sh` by presence. `SHELL` may be unset
/// (a Finder-launched .app) or non-POSIX (fish), which is why the fallback
/// chain and the allowlist are both here rather than a bare `$SHELL`.
pub(crate) fn gate_shell() -> String {
    gate_shell_for(std::env::var_os("SHELL").as_deref(), shell_present)
}

/// [`gate_shell`] with its two inputs injected, so the choice is a pure
/// function a test can drive through every branch.
fn gate_shell_for(
    env_shell: Option<&std::ffi::OsStr>,
    present: impl Fn(&str) -> bool,
) -> String {
    if let Some(shell) = env_shell.and_then(|s| s.to_str()) {
        let allowed = std::path::Path::new(shell)
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| GATE_SHELL_ALLOWLIST.contains(&name));
        if allowed && present(shell) {
            return shell.to_string();
        }
    }
    for candidate in ["zsh", "bash", "sh"] {
        if present(candidate) {
            return candidate.to_string();
        }
    }
    "sh".to_string()
}

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

/// Two-tier keyword resolution shared by every in-process gate check: the
/// session's FROZEN Tool-Gate list from its policy snapshot first (seeded at
/// spawn, gear-tab-editable), the global `tool-gate.json` only as fallback
/// (no session id / no snapshot / read error — fail-open like `load`). This
/// is the same resolution order the PreToolUse hook (`run_tool_gate`) uses;
/// keeping it here once means `action_gate`, `terminal_exec`, and the hook
/// can't drift apart on which list they enforce.
pub fn resolve_keywords(data_dir: &Path, session_id: Option<&str>) -> Vec<GatedKeyword> {
    match session_id.and_then(|sid| {
        crate::policy::session_policy::read_session_policy(data_dir, sid)
            .ok()
            .flatten()
    }) {
        Some(sp) => sp.tool_gate,
        None => load(data_dir),
    }
}

/// Persist the global keyword list (pretty JSON). Creates the data dir if
/// needed. Errors are returned (the Settings command surfaces them to the UI).
/// Atomic — temp + rename through the shared config writer (round 11): a bare
/// `std::fs::write` truncates before it fills, and `load` fails OPEN on a torn
/// or empty file, which reads as "no keywords" — no gating at all — until the
/// next save.
pub fn save(data_dir: &Path, keywords: &[GatedKeyword]) -> Result<()> {
    let path = config_path(data_dir);
    let body = serde_json::to_string_pretty(keywords).context("serializing tool-gate keywords")?;
    crate::policy::write_config_atomically(&path, &body)
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

/// Execute `command` via the agent's shell (`gate_shell`) in `cwd`, capturing combined stdout/stderr +
/// exit code, bounded by `timeout`. stdin is `/dev/null` so a command that
/// expects input (e.g. `gh issue comment` with no `--body`) fails fast instead
/// of hanging. On timeout the child is killed (kill-on-drop) and `code` is 124.
///
/// `envs` are set on the child on top of the app's own environment. **The
/// session's identity rides here** (round 12): the agent subprocess
/// (`spawn.rs`) and the session PTY (`terminal.rs`) both export
/// `BOT_HQ_SESSION_ID`, and the git hooks read it to find whose session a
/// commit or push belongs to — a gated `git commit` run from here with the
/// APP's bare environment skipped the findings gate (fail-open on no session)
/// and resolved the blueprint policy instead of the session's snapshot, and a
/// gated `git push` was refused as "no session context". Callers pass the
/// session id; the tests pass nothing.
pub async fn run_in_repo(
    command: &str,
    cwd: &Path,
    timeout: Duration,
    envs: &[(&str, &str)],
) -> CommandOutput {
    // The agent's own shell, not a fixed one — see `gate_shell`. (Round 7's
    // sh→bash for s-06a3c60b was the same lesson one shell short: macOS's bash
    // IS 3.2, and the command was proven under zsh.)
    run_in_shell(&gate_shell(), command, cwd, timeout, envs).await
}

/// Env pairs for a gated command run on behalf of `session_id` — the one
/// place the gate runner's child environment is decided, so every caller
/// (`execute_gated`, the late re-run) sets the same identity.
pub fn session_envs(session_id: &str) -> Vec<(&'static str, String)> {
    vec![("BOT_HQ_SESSION_ID", session_id.to_string())]
}

/// [`run_in_repo`] with the shell named by the caller — the seam the
/// heredoc test drives with `zsh` directly, so no test mutates `SHELL`.
async fn run_in_shell(
    shell: &str,
    command: &str,
    cwd: &Path,
    timeout: Duration,
    envs: &[(&str, &str)],
) -> CommandOutput {
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in envs {
        cmd.env(k, v);
    }
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

    /// **`save` is atomic** (round 11): temp + rename through the shared config
    /// writer, no `.tmp` left behind, and a rewrite keeps the file's mode —
    /// `load` fails OPEN on a torn file, so a truncate-then-fill write was a
    /// window in which every Bash call ran ungated.
    #[cfg(unix)]
    #[test]
    fn save_is_atomic_and_keeps_the_files_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        save(dir.path(), &[kw("gh", GateMode::Gate)]).unwrap();
        let path = config_path(dir.path());
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms).unwrap();
        save(dir.path(), &[kw("gh", GateMode::Gate), kw("rm -rf", GateMode::Gate)]).unwrap();
        assert_eq!(load(dir.path()).len(), 2);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600,
            "a rewrite keeps the existing mode (rename replaces the inode)"
        );
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no temp file survives the rename");
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
        let out = run_in_repo("echo hello-gate", dir.path(), Duration::from_secs(5), &[]).await;
        assert_eq!(out.code, 0);
        assert!(out.stdout.contains("hello-gate"), "stdout: {:?}", out.stdout);
    }

    #[tokio::test]
    async fn run_in_repo_propagates_nonzero_code() {
        let dir = tempdir().unwrap();
        let out = run_in_repo("exit 3", dir.path(), Duration::from_secs(5), &[]).await;
        assert_eq!(out.code, 3);
    }

    #[tokio::test]
    async fn run_in_repo_captures_stderr() {
        // action_gate returns stderr to the agent, so confirm it's captured
        // independently of stdout (Rain's A1 review gap).
        let dir = tempdir().unwrap();
        let out = run_in_repo("echo oops 1>&2; exit 1", dir.path(), Duration::from_secs(5), &[]).await;
        assert_eq!(out.code, 1);
        assert!(out.stderr.contains("oops"), "stderr: {:?}", out.stderr);
        assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
    }

    #[tokio::test]
    async fn run_in_repo_runs_in_cwd() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();
        let out = run_in_repo("ls", dir.path(), Duration::from_secs(5), &[]).await;
        assert!(out.stdout.contains("marker.txt"), "stdout: {:?}", out.stdout);
    }

    /// Round 12: the gate runner's child carries the session's identity. Delete
    /// the `cmd.env(k, v)` loop in `run_in_shell` and this goes red — which is
    /// the mutation that reproduces the hole: the git hooks inside a gated
    /// `git commit` / `git push` read `BOT_HQ_SESSION_ID`, and the app's bare
    /// environment has none.
    #[tokio::test]
    async fn run_in_repo_sets_the_envs_it_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let envs = session_envs("s-test-env");
        let envs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let out = run_in_repo(
            "printf 'sid=%s' \"$BOT_HQ_SESSION_ID\"",
            dir.path(),
            Duration::from_secs(5),
            &envs,
        )
        .await;
        assert_eq!(out.code, 0, "stderr: {}", out.stderr);
        assert_eq!(out.stdout, "sid=s-test-env");
    }

    #[tokio::test]
    async fn run_in_repo_times_out() {
        let dir = tempdir().unwrap();
        let out = run_in_repo("sleep 5", dir.path(), Duration::from_millis(150), &[]).await;
        assert_eq!(out.code, 124, "stderr: {:?}", out.stderr);
        assert!(out.stderr.contains("timed out"));
    }

    /// The gate runs the command in the AGENT's shell (round 10, B2). Every
    /// branch of the choice, with presence injected.
    #[test]
    fn gate_shell_prefers_a_posix_login_shell_and_falls_back_by_presence() {
        use std::ffi::OsStr;
        let all = |_: &str| true;
        // A POSIX login shell that runs from here is the shell — the command
        // was proven under it by the agent's own Bash tool.
        assert_eq!(gate_shell_for(Some(OsStr::new("/bin/zsh")), all), "/bin/zsh");
        assert_eq!(gate_shell_for(Some(OsStr::new("/usr/bin/bash")), all), "/usr/bin/bash");
        assert_eq!(gate_shell_for(Some(OsStr::new("dash")), all), "dash");
        // A non-POSIX login shell is not nominated: fish would break strictly
        // more `$()` / heredoc shapes than bash 3.2 does.
        assert_eq!(gate_shell_for(Some(OsStr::new("/opt/homebrew/bin/fish")), all), "zsh");
        assert_eq!(gate_shell_for(Some(OsStr::new("/usr/bin/nu")), all), "zsh");
        // A login shell that is on the list but does not run from here (a
        // stale SHELL after an uninstall) falls through too.
        let no_login = |s: &str| s != "/opt/local/bin/ksh";
        assert_eq!(gate_shell_for(Some(OsStr::new("/opt/local/bin/ksh")), no_login), "zsh");
        // Unset (a Finder-launched .app inherits no login SHELL): by presence.
        assert_eq!(gate_shell_for(None, all), "zsh");
        let no_zsh = |s: &str| s != "zsh";
        assert_eq!(gate_shell_for(None, no_zsh), "bash");
        let only_sh = |s: &str| s == "sh";
        assert_eq!(gate_shell_for(None, only_sh), "sh");
        // Nothing answers — `sh` is still named, so the spawn error says why.
        assert_eq!(gate_shell_for(None, |_: &str| false), "sh");
    }

    /// The exact shape that killed two approved gates in `s-766f4ab9`: a
    /// heredoc inside `$(…)` whose body carries an apostrophe. macOS's
    /// `/bin/bash` 3.2 dies on it ("unexpected EOF while looking for matching
    /// `''"); zsh runs it. Skipped where no zsh is installed — the shell under
    /// test is the one `gate_shell` resolves, and the assertion is that the
    /// gate's shell agrees with the agent's.
    #[tokio::test]
    async fn run_in_repo_survives_a_heredoc_inside_command_substitution() {
        if !shell_present("zsh") {
            eprintln!("skipped: no zsh on this machine");
            return;
        }
        // The shell the agent's Bash tool would have proven the command under.
        let dir = tempdir().unwrap();
        let command = "printf '%s' \"$(cat <<'BODY'\nDependabot's #507 was closed as \"no longer updatable\"\nBODY\n)\"";
        let out = run_in_shell("zsh", command, dir.path(), Duration::from_secs(5), &[]).await;
        assert_eq!(out.code, 0, "stderr: {:?}", out.stderr);
        assert!(
            out.stdout.contains("Dependabot's #507"),
            "the heredoc body must come through intact: {:?}",
            out.stdout
        );
    }

    #[test]
    fn resolve_keywords_falls_back_to_global() {
        let dir = tempdir().unwrap();
        save(dir.path(), &[kw("push", GateMode::Gate)]).unwrap();
        // No session id → global list.
        let global = resolve_keywords(dir.path(), None);
        assert_eq!(global, vec![kw("push", GateMode::Gate)]);
        // Session id without a snapshot on disk → global list too.
        let no_snap = resolve_keywords(dir.path(), Some("nope"));
        assert_eq!(no_snap, vec![kw("push", GateMode::Gate)]);
    }

    #[test]
    fn resolve_keywords_prefers_session_snapshot() {
        let dir = tempdir().unwrap();
        save(dir.path(), &[kw("push", GateMode::Gate)]).unwrap();
        let sp = crate::policy::session_policy::SessionPolicy {
            policy: crate::policy::Policy::default(),
            tool_gate: vec![kw("deploy", GateMode::Gate)],
        };
        crate::policy::session_policy::write_session_policy(dir.path(), "s1", &sp).unwrap();
        // The frozen session list wins over the global file entirely
        // (replace, not merge — matching the gear-tab snapshot semantics).
        let got = resolve_keywords(dir.path(), Some("s1"));
        assert_eq!(got, vec![kw("deploy", GateMode::Gate)]);
        assert_eq!(
            resolve_keywords(dir.path(), None),
            vec![kw("push", GateMode::Gate)]
        );
    }
}
