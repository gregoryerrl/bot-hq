//! `action_gate` — the execute-on-approve half of the Tool Gate.
//!
//! The PreToolUse hook blocks a `gate`-mode Bash command (exit 2) and tells the
//! agent to call `action_gate(command)`. This module classifies the command
//! against the GLOBAL keyword list and:
//!   - `auto_allow` / no-match → runs it immediately in the session's repo,
//!   - `gate`                  → surfaces Approve/Reject; on approve, runs it.
//!
//! Either way bot-hq EXECUTES the command server-side (in the session's
//! `working_repo_path`, resolved from storage) and returns combined output to
//! the agent — it's an ACTION request, not a permission request. The agent does
//! NOT re-run the command; the returned output IS the result.

use super::util::outcome_from_picked;
use super::*;
use crate::policy::tool_gate::{self, GateMode};
use crate::policy::ViolationOutcome;

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

    /// Resolve the session's working repo, then run the command and format the
    /// combined output.
    ///
    /// `pub(super)` so `resolve_choice` (sibling module `bridge::tray`) can
    /// run an approved gated command on the receiver-dropped path — when the
    /// agent's `action_gate` tool call timed out client-side, its request future
    /// (which would have called this in-band) was already cancelled.
    pub(super) async fn execute_gated(&self, session_id: &str, command: &str) -> Result<String> {
        let cwd = self.session_working_repo(session_id).await.ok_or_else(|| {
            anyhow::anyhow!(
                "action_gate: session {session_id} has no working_repo_path — cannot execute `{command}`"
            )
        })?;

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
    async fn timed_out_action_gate_still_executes_on_approve() {
        // Regression for the client-timeout gap: the agent's `action_gate` request
        // future is cancelled (here: aborted) before the user approves — simulating
        // claude-code's MCP client giving up. The parked receiver is dropped, but the
        // command must NOT be lost: resolve_choice runs `execute_gated` on the
        // fallback path and delivers the output via the OOB body.
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
        // Client timeout: abort the request future → drops the parked receiver
        // (the PendingChoice stays in `pending`). Await the handle so the cancel lands.
        call.abort();
        let _ = call.await;
        tokio::task::yield_now().await;

        let outcome = bridge.resolve_choice(&cid, "Approve".into()).await.unwrap();
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { body, .. } => assert!(
                body.contains("exit 0"),
                "OOB body must carry the executed command output: {body}"
            ),
            other => panic!("expected AgentReceiverDroppedFellBack, got {other:?}"),
        }
        assert!(
            marker.exists(),
            "approved command must execute on the dropped-receiver (timeout) path"
        );
    }

    #[tokio::test]
    async fn post_restart_action_gate_executes_from_durable_row() {
        // Durability case: an action_gate approval persisted before a restart —
        // command_text on the row, NO in-memory Parked. Resolving Approve must
        // execute from the durable row (the `None` branch). This is the
        // "approve hours/days later / after a restart and it still runs" guarantee.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let marker = repo.path().join("ran.txt");
        let cmd = format!("touch {}", marker.display());
        let bridge =
            SignalingBridge::with_policy(ViolationsLog::new(data.path()), data.path().to_path_buf());
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage
            .create_session("s1", "t", Some(&repo.path().display().to_string()))
            .await
            .unwrap();
        let opts = vec!["Approve".to_string(), "Reject".to_string()];
        storage
            .insert_question(
                "s1",
                "cid-1",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Run gated command in this session's repo?",
                Some(&opts),
                None,
                Some(&cmd), // command_text — the durable execution context
            )
            .await
            .unwrap();

        // No in-memory Parked for cid-1 → resolve hits the None (post-restart) arm.
        let outcome = bridge.resolve_choice("cid-1", "Approve".into()).await.unwrap();
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { body, .. } => assert!(
                body.contains("exit 0"),
                "durable row must execute + carry output via OOB: {body}"
            ),
            other => panic!("expected AgentReceiverDroppedFellBack, got {other:?}"),
        }
        assert!(
            marker.exists(),
            "command must execute from the durable row (post-restart path)"
        );
    }

    #[tokio::test]
    async fn resolve_twice_executes_gated_command_once() {
        // Durable exactly-once: a duplicate/stale resolve must not re-run the
        // command. The first resolve wins the pending→answered flip and executes;
        // the second sees `flipped == false` and is a no-op.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let marker = repo.path().join("ran.txt");
        let cmd = format!("touch {}", marker.display());
        let bridge =
            SignalingBridge::with_policy(ViolationsLog::new(data.path()), data.path().to_path_buf());
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage
            .create_session("s1", "t", Some(&repo.path().display().to_string()))
            .await
            .unwrap();
        let opts = vec!["Approve".to_string(), "Reject".to_string()];
        storage
            .insert_question(
                "s1",
                "cid-2",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Run?",
                Some(&opts),
                None,
                Some(&cmd),
            )
            .await
            .unwrap();

        let body_of = |o| match o {
            ResolveOutcome::AgentReceiverDroppedFellBack { body, .. } => body,
            other => panic!("expected AgentReceiverDroppedFellBack, got {other:?}"),
        };
        let first = body_of(bridge.resolve_choice("cid-2", "Approve".into()).await.unwrap());
        let second = body_of(bridge.resolve_choice("cid-2", "Approve".into()).await.unwrap());
        assert!(first.contains("output below"), "first resolve must execute: {first}");
        assert!(
            !second.contains("output below"),
            "second resolve must NOT re-execute (exactly-once): {second}"
        );
    }
}
