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

/// L (feedback #50): the built hook still stops a command that RUNS a
/// keyword, and passes one that only quotes it as a grep pattern.
#[test]
fn the_tool_gate_hook_passes_a_keyword_quoted_to_grep() {
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("config")).unwrap();
    std::fs::write(data.path().join("config/tool-gate.json"), r#"[{"keyword":"gh api","mode":"gate"}]"#).unwrap();
    let (code, out, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "gh api repos/o/r -f title=x");
    assert_eq!((code, out.as_str()), (2, ""), "{err}");
    // Feedback #83, the user's decision: a pure `gh api` GET passes the built
    // hook even with the `gh api` keyword gated; a write beside it does not.
    let (code, out, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "gh api repos/o/r/issues --paginate | jq '.[].title'");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "a GET read runs unapproved");
    let (code, out, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "gh api repos/o/r | rg --no-config -n 'def|class'");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "safe rg filter reaches the built hook");
    let (code, out, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "gh api repos/o/r | command grep -n route");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "plain command wrapper resolves the real filter");
    #[cfg(unix)]
    {
        let (code, out, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "gh api 'repos/o/r/main.py?ref=abc' | /usr/bin/grep -n -E '^@app\\.|route'");
        assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "trusted system filter stays exempt");
    }
    for command in [
        "gh api repos/o/r | rg -n x",
        "gh api repos/o/r | rg --no-config --pre=script x file",
        "gh api repos/o/r | rg --no-config -nz x file.gz",
        "gh api repos/o/r | rg --no-config *",
        "gh api repos/o/r * | rg --no-config x",
        "gh api repos/o/r | ./rg --no-config x",
        "gh api repos/o/r | /tmp/x/head -1",
        "./gh api repos/o/r",
        // Feedback #128: a filter handed a file reads that file, not the GET.
        "gh api repos/o/r | grep . /path/prod.env",
        "gh api repos/o/r | jq --rawfile p /path/prod.env -n '$p'",
        "gh api repos/o/r | rg --no-config x /path/prod.env",
    ] {
        let (code, _, err) = hook_in(data.path(), "tool-gate", "/bin/bash", command);
        assert_eq!(code, 2, "unsafe/configurable rg stays gated: {command}: {err}");
    }
    let (code, _, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "gh api repos/o/r && gh issue comment 5 -b x");
    assert_eq!(code, 2, "a write on the same line keeps the stop: {err}");
    let (code, out, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "grep -rn 'gh api' src | head -5");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "a quoted pattern is not a run");
    let (code, _, err) = hook_in(data.path(), "tool-gate", "/bin/bash", "grep -rn 'gh api' src | sh");
    assert_eq!(code, 2, "a shell in the line keeps the whole-line match: {err}");
}

/// The Codex hook has no permission layer behind it, so it FAILS CLOSED: with
/// no session identity in the environment, or with the app not running to
/// say whether the caller may edit, every call is refused with the reason on
/// stderr and nothing on stdout. (The decision table itself is pinned in
/// `policy::hooks` unit tests; the server's posture route in
/// `signaling::server` tests.)
#[test]
fn the_codex_gate_fails_closed_without_a_session_or_the_app() {
    let data = tempfile::tempdir().unwrap();
    let run = |sid: Option<&str>, tool: &str, command: &str| {
        let payload = serde_json::json!({"tool_name": tool, "tool_input": {"command": command}}).to_string();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_bot-hq"));
        cmd.args(["policy-check", "codex-gate", "--data-dir", data.path().to_str().unwrap()])
            .env("SHELL", "/bin/bash")
            .env_remove("CLAUDE_CODE_SHELL")
            .env_remove("BOT_HQ_SESSION_ID")
            .env("BOT_HQ_AGENT", "eyes")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(sid) = sid {
            cmd.env("BOT_HQ_SESSION_ID", sid);
        }
        let mut child = cmd.spawn().expect("the built binary runs");
        child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    // No session identity: refused, even for a read.
    let (code, out, err) = run(None, "Bash", "git status");
    assert_eq!((code, out.as_str()), (2, ""), "{err}");
    assert!(err.contains("BOT_HQ_SESSION_ID"), "names what is missing: {err}");
    // A session id but no app to ask: refused, naming the app.
    let (code, out, err) = run(Some("s-nowhere"), "Bash", "git status");
    assert_eq!((code, out.as_str()), (2, ""), "{err}");
    assert!(err.contains("bot-hq") && err.contains("running"), "names the app: {err}");
    // The zsh trap is caught before anything else, like the other two hooks.
    let payload_trap = run(Some("s-nowhere"), "Bash", r#"git show "$R:app/x""#);
    assert_eq!(payload_trap.0, 2);
    // Feedback #121: every exit above left no breadcrumb record behind — the
    // directory was created for the session that had an id, and is empty.
    let records = data.path().join(".local/hook-phases/s-nowhere/eyes");
    assert!(records.is_dir(), "the record directory is made on the first invocation");
    assert_eq!(std::fs::read_dir(&records).unwrap().count(), 0, "a hook that exits removes its record");
    assert!(!data.path().join(".local/hook-phases").read_dir().unwrap().any(|e| e.unwrap().file_name() != "s-nowhere"),
        "no session id, no record anywhere");
    // An invocation the hook cannot even parse (bot-hq's own hooks.json names
    // --data-dir; a damaged one does not) still fails CLOSED: exit 2, not the
    // other hooks' soft exit 0.
    let out = Command::new(env!("CARGO_BIN_EXE_bot-hq"))
        .args(["policy-check", "codex-gate"])
        .env("BOT_HQ_SESSION_ID", "s-nowhere").env("BOT_HQ_AGENT", "eyes")
        .stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("BLOCKED") && String::from_utf8_lossy(&out.stderr).contains("--data-dir"));
    let out = Command::new(env!("CARGO_BIN_EXE_bot-hq"))
        .args(["policy-check", "shell-lint"])
        .stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "the other hooks keep their soft exit");
}

#[tokio::test]
async fn the_compaction_gate_hook_reads_the_live_mirror_and_clears_after_delivery() {
    use bot_hq::{agents::handoff, storage::{Storage, QuestionKind}};
    let data = tempfile::tempdir().unwrap();
    let storage = Storage::memory().await.unwrap().with_handoff_dir(handoff::dir_under(&data.path().join(".local")));
    storage.create_session("s-hook-results", "hook", None).await.unwrap();
    storage.ensure_session_roster("s-hook-results", 2).await.unwrap();
    let hands = storage.participant_by_slug("s-hook-results", "hands").await.unwrap().unwrap().id;
    storage.insert_tray_entry("s-hook-results", "fixture-gate", "hands", QuestionKind::Approval,
        "fixture", None, None, Some("printf fixture")).await.unwrap();
    let result = storage.post_to_channel("s-hook-results", "system", None, "system_notice", "NEW_RESULT_OLDER_THAN_HANDOFF", None).await.unwrap();
    storage.set_tray_run_result("fixture-gate", Some(result.message_id()), Some(0), 1).await.unwrap();
    storage.upsert_session_document("s-hook-results", "handoff-hands", "The gate is still pending.", None).await.unwrap();
    let run = || Command::new(env!("CARGO_BIN_EXE_bot-hq"))
        .args(["policy-check", "compact-gate-results", "--data-dir", data.path().to_str().unwrap(), "--session", "s-hook-results"])
        .env("BOT_HQ_AGENT", "hands").env_remove("BOT_HQ_SESSION_ID").output().unwrap();
    let out = run();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("NEW_RESULT_OLDER_THAN_HANDOFF"));
    assert!(out.stdout.len() <= handoff::GATE_RESULTS_BUDGET_BYTES);
    storage.commit_delivery(hands, &[(result.message_id(), None)]).await.unwrap();
    let out = run();
    assert!(out.status.success() && out.stdout.is_empty());
}

/// Both built connectors traverse the authenticated HTTP route. The native
/// command is blocked while review is owed; only the host runs the local
/// publisher after settlement. No remote service or model inference is used.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_native_hooks_queue_auto_publications_and_the_host_runs_them_once() {
    use bot_hq::{paths::Paths, policy::ViolationsLog, signaling::{SignalingBridge, start_signaling_server}, storage::Storage};
    let data = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let paths = Paths::for_data_dir(data.path().to_path_buf());
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::create_dir_all(&paths.local_dir).unwrap();
    let policy = paths.config_dir.join("tool-gate.json");
    std::fs::write(&policy, r#"[{"keyword":"gh","mode":"auto_allow"}]"#).unwrap();
    let bridge = SignalingBridge::with_policy(ViolationsLog::new(data.path()), data.path().to_path_buf());
    let token = paths.write_hook_token().unwrap();
    bridge.set_hook_token(&token);
    let storage = Storage::memory().await.unwrap();
    storage.create_session("s-hook", "fixture", Some(repo.path().to_str().unwrap())).await.unwrap();
    storage.ensure_session_roster("s-hook", bot_hq::storage::MAX_SESSION_PARTICIPANTS).await.unwrap();
    let reviewer = storage.participant_by_slug("s-hook", "eyes").await.unwrap().unwrap().id;
    bridge.set_storage(storage.clone()).await;
    bridge.register_session_reviewers("s-hook".into(), vec!["eyes".into()]);
    let server = start_signaling_server(bridge.clone()).await.unwrap();
    paths.write_signaling_addr(server.local_addr).unwrap();
    let quote = |p: &std::path::Path| format!("'{}'", p.to_string_lossy().replace('\\', "/").replace('\'', "'\\''"));
    let marker = repo.path().join("published.txt");
    let draft = repo.path().join("draft.md");
    let run_hook = |sub: &'static str, command: String| {
        let data = data.path().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut child = Command::new(env!("CARGO_BIN_EXE_bot-hq"))
                .args(["policy-check", sub, "--data-dir", data.to_str().unwrap()])
                .env("BOT_HQ_SESSION_ID", "s-hook").env("BOT_HQ_AGENT", "hands")
                .env("BOT_HQ_DATA_DIR", &data).env_remove("BOT_HQ_PROJECT")
                .env("SHELL", "/bin/bash").env_remove("CLAUDE_CODE_SHELL")
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
                .spawn().unwrap();
            let payload = serde_json::json!({"tool_name":"Bash", "tool_input":{"command":command}});
            child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
            child.wait_with_output().unwrap()
        })
    };
    let mut expected = String::new();
    for sub in ["tool-gate", "codex-gate"] {
        let body = format!("Body for {sub}\n");
        std::fs::write(&draft, &body).unwrap();
        let command = format!("gh() {{ cat {} >> {}; printf 'local fixture\\n'; }}; gh issue create --title {sub} --body-file {}", quote(&draft), quote(&marker), quote(&draft));
        for _ in 0..2 {
            let out = run_hook(sub, command.clone()).await.unwrap();
            let err = String::from_utf8_lossy(&out.stderr);
            assert_eq!(out.status.code(), Some(2), "{sub}: {err}");
            assert!(out.stdout.is_empty(), "a hook must not emit an allow JSON override");
            assert!(err.contains("QUEUED") && err.contains("No user approval card"), "{sub}: {err}");
        }
        assert_eq!(std::fs::read_to_string(&marker).unwrap_or_default(), expected, "the hook/route must not run the command");
        let rows = storage.queued_gates_for_session("s-hook").await.unwrap();
        assert_eq!(rows.len(), 1, "hook retry dedupes");
        let row = &rows[0];
        assert!(!row.approval_required);
        assert!(storage.pending_gate_ids("s-hook").await.unwrap().is_empty());
        storage.commit_delivery(reviewer, &[(row.body_row_id.unwrap(), None)]).await.unwrap();
        bridge.settle_queued_outward("s-hook", reviewer).await;
        assert_eq!(bridge.run_reviewed_auto_publications().await, 1);
        assert_eq!(bridge.run_reviewed_auto_publications().await, 0);
        expected.push_str(&body);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), expected);
        let result = storage.get_tray_entry(&row.choice_id).await.unwrap().unwrap();
        assert_eq!(result.status, "auto_resolved");
        assert!(result.picked_option.is_none() && result.answered_at.is_none());
        assert!(result.result_row_id.is_some());
        // The standing pure-GET exception still passes the real binaries.
        let out = run_hook(sub, "gh api repos/o/r --jq '.id'".into()).await.unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.stdout.is_empty() && out.stderr.is_empty());
    }

    // A flag in the request cannot confer authority when live policy changed,
    // when the caller is read-only, or without the launch's authentication.
    std::fs::write(&policy, r#"[{"keyword":"gh","mode":"gate"}]"#).unwrap();
    std::fs::write(&draft, "A new unread body\n").unwrap();
    let command = format!("gh issue create --title changed --body-file {}", quote(&draft));
    let url = format!("http://{}/hooks/tool-gate", server.local_addr);
    let client = reqwest::Client::new();
    for (agent, credential, status) in [("hands", "wrong", 401), ("eyes", token.as_str(), 400), ("hands", token.as_str(), 200)] {
        let response = client.post(&url).header("x-bot-hq-hook-token", credential)
            .header("content-type", "application/json")
            .body(serde_json::json!({"session_id":"s-hook", "agent":agent, "command":command, "review_auto_allow":true}).to_string())
            .send().await.unwrap();
        assert_eq!(response.status().as_u16(), status);
        if status == 200 {
            let response: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
            assert_eq!(response["approval_required"], true);
            assert!(response.get("allow").is_none());
        }
    }
    assert_eq!(std::fs::read_to_string(marker).unwrap(), expected);
    server.shutdown();
}
