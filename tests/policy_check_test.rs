//! The Bash hooks as claude-code runs them: the BUILT binary's `policy-check`
//! subcommand, a PreToolUse payload on stdin, and the exit code and both
//! streams read back. Unit tests pin the lint (`policy::shell_lint`); this
//! pins what the agent's CLI actually receives, which is where a hook's
//! contract lives — exit 2 with the refusal on stderr blocks the call, and a
//! pass must exit 0 with NOTHING on stdout: a printed `allow` decision would
//! skip claude-code's permission layer, which is what enforces a read-only
//! participant's deny list (EYES, s-3158eb35).

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `bot-hq policy-check <sub>` on a Bash payload for `command`, with the
/// agent's login shell set to `shell`. Returns (exit code, stdout, stderr).
fn hook(sub: &str, shell: &str, command: &str) -> (i32, String, String) {
    let data = tempfile::tempdir().unwrap();
    hook_in(data.path(), sub, shell, command)
}

/// [`hook`] against a given data dir (one carrying a policy).
fn hook_in(data: &std::path::Path, sub: &str, shell: &str, command: &str) -> (i32, String, String) {
    let payload =
        serde_json::json!({"tool_name": "Bash", "tool_input": {"command": command}}).to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_bot-hq"))
        .args(["policy-check", sub, "--data-dir", data.to_str().unwrap()])
        .env("SHELL", shell)
        // The hook resolves the shell the CLI would use: an override on the
        // test runner's own environment must not decide the case.
        .env_remove("CLAUDE_CODE_SHELL")
        // An agent-run `cargo test` inherits a real session id (round 8).
        .env_remove("BOT_HQ_SESSION_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the built binary runs");
    child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The reviewer's hook (feedback #96): silent on a pass, a blocking refusal
/// with the corrected command on the trap, and nothing at all under bash.
#[test]
fn shell_lint_passes_silently_and_refuses_the_zsh_trap() {
    let (code, out, err) = hook("shell-lint", "/bin/zsh", "git log --oneline -1");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "a pass prints nothing");

    let (code, out, err) = hook("shell-lint", "/bin/zsh", r#"git show "$R:app/x""#);
    assert_eq!(code, 2, "exit 2 blocks the call: {err}");
    assert_eq!(out, "", "the refusal goes to stderr, which the CLI feeds to the agent");
    assert!(err.starts_with("Not a Tool Gate stop"), "{err}");
    assert!(err.contains(r#"git show "${R}:app/x""#), "the corrected command: {err}");

    let (code, out, _) = hook("shell-lint", "/bin/bash", r#"git show "$R:app/x""#);
    assert_eq!((code, out.as_str()), (0, ""), "literal under bash, so not refused");
}

/// The executor's hook keeps its arguments and its behaviour, and lints
/// first — before any keyword match or auto-park.
#[test]
fn the_tool_gate_hook_lints_first_and_still_passes_a_benign_command() {
    let (code, out, err) = hook("tool-gate", "/bin/zsh", r#"git show "$R:app/x""#);
    assert_eq!(code, 2, "{err}");
    assert_eq!(out, "");
    assert!(err.starts_with("Not a Tool Gate stop"), "{err}");

    let (code, out, err) = hook("tool-gate", "/bin/zsh", "echo benign");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));
}

/// Group K: the executor's hook stops a command that RUNS a listed read —
/// with no session to park in, it says to route it through action_gate — and
/// lets text that only mentions one through.
#[test]
fn the_tool_gate_hook_stops_a_listed_read_and_passes_a_mention() {
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("config")).unwrap();
    std::fs::write(
        data.path().join("config/general-policy.yaml"),
        "production_reads:\n  - gcloud logging read\n",
    )
    .unwrap();
    let (code, out, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "gcloud logging read 'severity>=ERROR'");
    assert_eq!(code, 2, "{err}");
    assert_eq!(out, "");
    assert!(err.starts_with("A LISTED production read"), "{err}");
    assert!(err.contains("Call the `action_gate` tool"), "{err}");
    let (code, out, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "grep -rn 'gcloud logging read' notes.md");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "a mention is not a read");
}

/// Group K (tray `ef8fe5de`): the reviewer's Bash hook refuses a listed read
/// and points at `read_gate`; a command that is not listed passes silently.
#[test]
fn the_reviewers_hook_points_a_listed_read_at_read_gate() {
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("config")).unwrap();
    std::fs::write(
        data.path().join("config/general-policy.yaml"),
        "production_reads:\n  - gcloud logging read\n",
    )
    .unwrap();
    let (code, out, err) = hook_in(data.path(), "shell-lint", "/bin/bash", "gcloud logging read x");
    assert_eq!(code, 2, "{err}");
    assert_eq!(out, "");
    assert!(err.contains("Use your `read_gate` tool"), "{err}");
    let (code, out, err) = hook_in(data.path(), "shell-lint", "/bin/bash", "git log -1");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "a pass prints nothing");
}
