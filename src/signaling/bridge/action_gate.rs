//! `action_gate` — the execute-on-approve half of the Tool Gate.
//!
//! The PreToolUse hook blocks a `gate`-mode Bash command (exit 2) and tells the
//! agent to call `action_gate(command)`. This module classifies the command
//! against the GLOBAL keyword list and:
//!   - `auto_allow` / no-match → runs it immediately in the session's repo,
//!   - `gate`                  → surfaces Approve/Reject; on approve, runs it.
//! Either way bot-hq EXECUTES the command server-side (in the session's
//! `working_repo_path`, resolved from storage) and returns combined output to
//! the agent — it's an ACTION request, not a permission request. The agent does
//! NOT re-run the command; the returned output IS the result.
//!
//! Push reconcile (hand-off spec §3.5): a gate-run `git push` still trips the
//! pre-push git hook, so before executing a push we record a session-level push
//! grant for the repo's CURRENT branch (the branch the hook checks via
//! `git symbolic-ref`), reusing `add_branch_to_session_grant`. Without this the
//! gate-run push would be double-gated and blocked.

use super::util::outcome_from_picked;
use super::*;
use crate::policy::tool_gate::{self, GateMode};
use crate::policy::{PermissionAction, ViolationOutcome};

impl SignalingBridge {
    /// Entry point for the `action_gate` MCP tool. `command` is the exact Bash
    /// string the gate blocked. Returns combined output text (executed) or a
    /// "not run" message (rejected). Errs only when the session has no
    /// `working_repo_path` to execute in.
    pub async fn action_gate(
        &self,
        session_id: String,
        agent: String,
        command: String,
    ) -> Result<String> {
        let keywords = match self.data_dir.as_ref() {
            Some(d) => tool_gate::load(d),
            None => Vec::new(),
        };
        match tool_gate::match_keyword("Bash", &command, &keywords) {
            // No keyword, or an explicit auto_allow → run with no prompt. (In
            // normal flow the hook only routes `gate` commands here; auto_allow
            // / no-match are handled defensively so a direct call still works.)
            None | Some(GateMode::AutoAllow) => self.execute_gated(&session_id, &command).await,
            Some(GateMode::Gate) => {
                let picked = self
                    .request_approval(
                        session_id.clone(),
                        agent,
                        format!("Run gated command in this session's repo?\n\n`{command}`"),
                        vec!["Approve".to_string(), "Reject".to_string()],
                        ApprovalContext {
                            kind: ViolationKind::ToolBlocklist,
                            action: command.clone(),
                            detail: Some("tool-gate".to_string()),
                        },
                    )
                    .await?;
                if matches!(outcome_from_picked(&picked), ViolationOutcome::Approved) {
                    self.execute_gated(&session_id, &command).await
                } else {
                    Ok(format!(
                        "action_gate: user rejected — `{command}` was not run."
                    ))
                }
            }
        }
    }

    /// Resolve the session's working repo, pre-record a push grant if needed,
    /// then run the command and format the combined output.
    async fn execute_gated(&self, session_id: &str, command: &str) -> Result<String> {
        let cwd = self.session_working_repo(session_id).await.ok_or_else(|| {
            anyhow::anyhow!(
                "action_gate: session {session_id} has no working_repo_path — cannot execute `{command}`"
            )
        })?;

        // Gate-run push reconcile: pre-record a session push grant for the
        // repo's current branch BEFORE the push subprocess runs, so the
        // pre-push git hook (which reads the mirrored grant file) passes
        // instead of double-gating.
        if command_is_git_push(command) {
            if let Some(branch) = current_branch(&cwd).await {
                if let Err(e) = self
                    .add_branch_to_session_grant(session_id, PermissionAction::Push, branch.clone())
                    .await
                {
                    tracing::warn!(
                        ?e,
                        %branch,
                        session_id,
                        "action_gate: push-grant pre-record failed; pre-push hook may re-block"
                    );
                }
            }
        }

        let out = tool_gate::run_in_repo(command, &cwd, tool_gate::DEFAULT_TIMEOUT).await;
        Ok(format_command_output(command, &out))
    }

    /// The session's `working_repo_path` from storage — the source of truth on
    /// the session row (no parallel bridge map to keep in sync). None when the
    /// session is unknown, storage isn't wired, or the row has no repo path.
    async fn session_working_repo(&self, session_id: &str) -> Option<PathBuf> {
        let storage = self.storage.lock().await.clone()?;
        let session = storage.get_session(session_id).await.ok()??;
        session.working_repo_path.map(PathBuf::from)
    }
}

/// True if the command invokes `git push` (case-insensitive substring). Biased
/// to over-detect: a false positive only records a harmless extra push grant,
/// while a miss would let a real gate-run push be double-gated by the pre-push
/// hook.
fn command_is_git_push(command: &str) -> bool {
    command.to_lowercase().contains("git push")
}

/// The repo's current branch via `git symbolic-ref --short HEAD` (works on an
/// unborn branch, so it's reliable even pre-first-commit). None on detached
/// HEAD / non-repo / git error.
async fn current_branch(cwd: &Path) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(cwd)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Format combined output roughly the way the agent would have seen it from its
/// own Bash call, plus an exit-code footer so a non-zero result is unambiguous.
fn format_command_output(command: &str, out: &tool_gate::CommandOutput) -> String {
    let mut s = String::new();
    if !out.stdout.is_empty() {
        s.push_str(&out.stdout);
        if !out.stdout.ends_with('\n') {
            s.push('\n');
        }
    }
    if !out.stderr.is_empty() {
        s.push_str(&out.stderr);
        if !out.stderr.ends_with('\n') {
            s.push('\n');
        }
    }
    s.push_str(&format!(
        "[action_gate executed `{command}` → exit {}]",
        out.code
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::tool_gate::{GateMode, GatedKeyword};
    use crate::policy::ViolationsLog;
    use crate::storage::Storage;
    use std::process::Command as StdCommand;
    use tempfile::tempdir;

    fn gk(keyword: &str, mode: GateMode) -> GatedKeyword {
        GatedKeyword {
            keyword: keyword.into(),
            mode,
        }
    }

    /// Bridge with data_dir (keywords saved) + storage + a session whose
    /// working_repo_path points at `repo`.
    async fn bridge_with(
        data_dir: &Path,
        keywords: &[GatedKeyword],
        session: &str,
        repo: &Path,
    ) -> Arc<SignalingBridge> {
        tool_gate::save(data_dir, keywords).unwrap();
        let log = ViolationsLog::new(data_dir);
        let bridge = SignalingBridge::with_policy(log, data_dir.to_path_buf());
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage
            .create_session(session, "t", Some(&repo.display().to_string()))
            .await
            .unwrap();
        bridge
    }

    fn init_git_repo(dir: &Path) {
        StdCommand::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir)
            .status()
            .unwrap();
        // Force the branch name deterministically (works pre-first-commit).
        StdCommand::new("git")
            .args(["symbolic-ref", "HEAD", "refs/heads/main"])
            .current_dir(dir)
            .status()
            .unwrap();
    }

    #[tokio::test]
    async fn auto_allow_executes_without_prompt() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let bridge = bridge_with(
            data.path(),
            &[gk("echo", GateMode::AutoAllow)],
            "s1",
            repo.path(),
        )
        .await;
        let out = bridge
            .action_gate("s1".into(), "brian".into(), "echo hi-there".into())
            .await
            .unwrap();
        assert!(out.contains("hi-there"), "out: {out}");
        assert!(out.contains("exit 0"), "out: {out}");
    }

    #[tokio::test]
    async fn no_match_executes() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let bridge = bridge_with(data.path(), &[], "s1", repo.path()).await;
        let out = bridge
            .action_gate("s1".into(), "brian".into(), "echo loose".into())
            .await
            .unwrap();
        assert!(out.contains("loose"), "out: {out}");
    }

    #[tokio::test]
    async fn no_working_repo_errors() {
        let data = tempdir().unwrap();
        tool_gate::save(data.path(), &[]).unwrap();
        let log = ViolationsLog::new(data.path());
        let bridge = SignalingBridge::with_policy(log, data.path().to_path_buf());
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s-norepo", "t", None).await.unwrap();
        let err = bridge
            .action_gate("s-norepo".into(), "brian".into(), "echo x".into())
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("working_repo_path"),
            "err: {err}"
        );
    }

    #[tokio::test]
    async fn gate_reject_does_not_run_the_command() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let marker = repo.path().join("ran.txt");
        let cmd = format!("touch {}", marker.display());
        let bridge = bridge_with(
            data.path(),
            &[gk("touch", GateMode::Gate)],
            "s1",
            repo.path(),
        )
        .await;
        let mut sub = bridge.subscribe();
        let b2 = Arc::clone(&bridge);
        let call = tokio::spawn(async move { b2.action_gate("s1".into(), "brian".into(), cmd).await });
        let cid = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        bridge.resolve_choice(&cid, "Reject".into()).await.unwrap();
        let out = call.await.unwrap().unwrap();
        assert!(out.contains("rejected"), "out: {out}");
        assert!(!marker.exists(), "rejected command must NOT have run");
    }

    #[tokio::test]
    async fn gate_approve_executes_the_command() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let marker = repo.path().join("ran.txt");
        let cmd = format!("touch {}", marker.display());
        let bridge = bridge_with(
            data.path(),
            &[gk("touch", GateMode::Gate)],
            "s1",
            repo.path(),
        )
        .await;
        let mut sub = bridge.subscribe();
        let b2 = Arc::clone(&bridge);
        let call = tokio::spawn(async move { b2.action_gate("s1".into(), "brian".into(), cmd).await });
        let cid = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        bridge.resolve_choice(&cid, "Approve".into()).await.unwrap();
        let out = call.await.unwrap().unwrap();
        assert!(out.contains("exit 0"), "out: {out}");
        assert!(marker.exists(), "approved command should have run");
    }

    #[tokio::test]
    async fn push_records_session_grant_before_exec() {
        // The pre-push git hook checks the CURRENT branch. action_gate must
        // record a push grant for it BEFORE running the push, or the gate-run
        // push is double-gated. We use auto_allow (no prompt); the push itself
        // fails (no remote) — we assert the grant, not the push success.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        init_git_repo(repo.path());
        let bridge = bridge_with(
            data.path(),
            &[gk("git push", GateMode::AutoAllow)],
            "s1",
            repo.path(),
        )
        .await;
        let _ = bridge
            .action_gate("s1".into(), "brian".into(), "git push origin main".into())
            .await
            .unwrap();
        let perm = bridge.list_session_permissions("s1").await;
        assert!(
            perm.allows_push("main"),
            "push grant for the current branch must be recorded before exec"
        );
    }
}
