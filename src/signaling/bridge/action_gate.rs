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
        // Two-tier resolve (session snapshot → global fallback) — previously
        // this read only the global list, so a gear-tab session override was
        // invisible to a direct action_gate call.
        let keywords = match self.data_dir.as_ref() {
            Some(d) => tool_gate::resolve_keywords(d, Some(&session_id)),
            None => Vec::new(),
        };
        match tool_gate::match_keyword("Bash", &command, &keywords) {
            // No keyword, or an explicit auto_allow → run with no prompt. (In
            // normal flow the hook only routes `gate` commands here; auto_allow
            // / no-match are handled defensively so a direct call still works.)
            None | Some(GateMode::AutoAllow) => self.execute_gated(&session_id, &command).await,
            Some(GateMode::Gate) => {
                let (gate_id, existing) = self
                    .park_gated_command(&session_id, &agent, &command)
                    .await?;
                Ok(parked_gate_text(&gate_id, &command, existing))
            }
        }
    }

    /// Park a gated command for the user's approval and return
    /// `(gate_id, already_pending)`.
    ///
    /// **Parks only — never matches keywords and never executes.** That is the
    /// difference from [`Self::action_gate`], and it is why the PreToolUse
    /// hook's `/hooks/tool-gate` route calls THIS: `action_gate` runs the
    /// command outright on an `auto_allow`/no-match resolve, so a route wired to
    /// it would execute without approval whenever its resolve disagreed with the
    /// hook's (e.g. the session's keyword list edited between the two) — an
    /// unapproved execution triggered by a call that was just blocked.
    ///
    /// Execution happens later, at resolve time, through the tray's
    /// exactly-once flip (`resolve_choice` → `execute_gated`), so parking alone
    /// is the whole job here.
    pub(crate) async fn park_gated_command(
        &self,
        session_id: &str,
        agent: &str,
        command: &str,
    ) -> Result<(String, bool)> {
        // Duplicate suppression: an identical command already awaiting
        // approval gets the existing gate back instead of stacking a
        // second confusable prompt. PENDING rows only — a re-fire after
        // a reject is an intentional retry and parks fresh.
        if let Some(storage) = self.storage.lock().await.clone() {
            if let Ok(Some(existing)) = storage.pending_gate_for_command(session_id, command).await {
                return Ok((existing, true));
            }
        }
        // Park and return IMMEDIATELY (same contract as ask_user_choice). The
        // old design held the RPC open and the MCP client timed out at ~60s
        // while the human was still deciding — the agent saw "The operation
        // timed out" and could not tell queued from failed (six such ghosts in
        // the archive study).
        let parked = self
            .ask_user_choice_inner(
                session_id.to_string(),
                agent.to_string(),
                format!("Run gated command in this session's repo?\n\n`{command}`"),
                vec!["Approve".to_string(), "Reject".to_string()],
                Some(ApprovalContext {
                    kind: ViolationKind::ToolBlocklist,
                    action: command.to_string(),
                    detail: Some("tool-gate".to_string()),
                }),
                None,
                false,
            )
            .await?;
        let gate_id = serde_json::from_str::<serde_json::Value>(&parked)
            .ok()
            .and_then(|v| {
                v.get("choice_id")
                    .and_then(|c| c.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        Ok((gate_id, false))
    }

    /// The `gate_status` MCP tool: current state of a parked gate by id.
    /// Read-only, either agent. Exists so an agent never has to guess whether
    /// a parked command ran — the archive study's ghost states ("did the merge
    /// happen?") each burned a user round-trip to resolve.
    pub async fn gate_status(&self, gate_id: &str) -> Result<String> {
        let storage = self
            .storage
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("storage not configured"))?;
        let Some(row) = storage.get_tray_entry(gate_id).await? else {
            return Ok(format!("gate_status: no gate with id {gate_id}"));
        };
        // Only a ToolBlocklist (action_gate) approval carries a command —
        // `ask_user_choice_inner` sets `command_text` for that kind alone. A
        // parked `request_approval` (push_gate / per_action) has none, so the
        // command-shaped wording would assert an execution that never happened.
        let Some(command) = row.command_text.as_deref() else {
            return Ok(Self::approval_status_text(&row));
        };
        Ok(match row.status.as_str() {
            "pending" => format!(
                "pending — `{command}` is still awaiting the user's approval. Do not \
                 re-issue it; the outcome will arrive as an out-of-band message."
            ),
            "answered" => {
                let picked = row.picked_option.as_deref().unwrap_or("");
                if matches!(outcome_from_picked(picked), ViolationOutcome::Approved) {
                    format!(
                        "approved — bot-hq executed `{command}` at approval time; the \
                         output was delivered as an out-of-band message (check your \
                         recent messages). Do not re-run it."
                    )
                } else {
                    format!(
                        "rejected — `{command}` was NOT run. User's answer: \"{picked}\". \
                         Anything beyond the word itself is the user's reasoning — read it \
                         before deciding whether to retry."
                    )
                }
            }
            other => format!("{other} — `{command}` did not run (gate is no longer pending)."),
        })
    }

    /// `gate_status` wording for a command-less approval — a parked
    /// `request_approval` (push_gate / per_action). Nothing executes on
    /// approve here; the pick itself is the outcome, so the text must not
    /// claim bot-hq ran anything.
    fn approval_status_text(row: &crate::storage::SessionTrayEntry) -> String {
        match row.status.as_str() {
            "pending" => "pending — the approval request is still awaiting the user's \
                 pick. Do not re-issue it; the outcome will arrive as an \
                 out-of-band message."
                .to_string(),
            "answered" => {
                let picked = row.picked_option.as_deref().unwrap_or("");
                format!(
                    "resolved — the user answered \"{picked}\". No command was attached \
                     (this was a policy approval, not a gated command), so nothing ran \
                     on bot-hq's side; acting on the answer is yours. Anything beyond \
                     the leading word is the user's reasoning — read it."
                )
            }
            other => format!(
                "{other} — the approval request is no longer pending and was never \
                 answered."
            ),
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

/// The parked-gate response text. `existing` distinguishes a fresh park from a
/// dedupe hit on an already-pending identical command.
fn parked_gate_text(gate_id: &str, command: &str, existing: bool) -> String {
    let lead = if existing {
        "action_gate: an identical command is ALREADY parked for approval"
    } else {
        "action_gate: parked for the user's approval"
    };
    format!(
        "{lead} (gate_id: {gate_id}).\n\
         `{command}` runs when the user approves; its output arrives as an \
         out-of-band message. On reject you get a rejection notice instead. Do \
         NOT re-issue the command or assume it ran — call gate_status(\"{gate_id}\") \
         if you need the current state before continuing."
    )
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
        // Park contract: the call returns immediately with a gate_id.
        let parked = bridge
            .action_gate("s1".into(), "brian".into(), cmd)
            .await
            .unwrap();
        assert!(parked.contains("parked"), "got: {parked}");
        let cid = parked
            .split("gate_id: ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap()
            .to_string();
        bridge.resolve_choice(&cid, "Reject".into()).await.unwrap();
        assert!(!marker.exists(), "rejected command must NOT have run");
        let status = bridge.gate_status(&cid).await.unwrap();
        assert!(status.starts_with("rejected"), "got: {status}");
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
        // Park contract: the call returns immediately with a gate_id.
        let parked = bridge
            .action_gate("s1".into(), "brian".into(), cmd)
            .await
            .unwrap();
        assert!(parked.contains("parked"), "got: {parked}");
        let cid = parked
            .split("gate_id: ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap()
            .to_string();
        let outcome = bridge.resolve_choice(&cid, "Approve".into()).await.unwrap();
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { body, .. } => {
                assert!(body.contains("exit 0"), "approve delivers output OOB: {body}")
            }
            other => panic!("expected OOB delivery, got {other:?}"),
        }
        assert!(marker.exists(), "approved command should have run");
        let status = bridge.gate_status(&cid).await.unwrap();
        assert!(status.starts_with("approved"), "got: {status}");
    }

    #[tokio::test]
    async fn gate_status_on_a_command_less_approval_claims_no_execution() {
        // `ask_user_choice_inner` sets command_text for ToolBlocklist rows only,
        // so a parked `request_approval` (push_gate / per_action) has none. The
        // command-shaped wording would then report that bot-hq "executed
        // `(no command attached)`" — a false execution claim to the one caller
        // that exists to avoid guessing whether something ran.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let bridge = bridge_with(data.path(), &[], "s1", repo.path()).await;
        let ack = bridge
            .request_approval_parked(
                "s1".into(),
                "brian".into(),
                "Query prod?".into(),
                vec!["Approve".into(), "Deny".into()],
                ApprovalContext {
                    kind: ViolationKind::PerAction,
                    action: "bq query --project_id=prod ...".into(),
                    detail: None,
                },
            )
            .await
            .unwrap();
        let cid = serde_json::from_str::<serde_json::Value>(&ack).unwrap()["choice_id"]
            .as_str()
            .unwrap()
            .to_string();

        let pending = bridge.gate_status(&cid).await.unwrap();
        assert!(pending.starts_with("pending"), "got: {pending}");
        assert!(
            !pending.contains("no command attached"),
            "placeholder leaked into agent-facing text: {pending}"
        );

        bridge.resolve_choice(&cid, "Approve".into()).await.unwrap();
        let done = bridge.gate_status(&cid).await.unwrap();
        assert!(done.starts_with("resolved"), "got: {done}");
        assert!(
            !done.contains("executed"),
            "must not claim bot-hq ran anything: {done}"
        );
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

        // confirm_stale = true: the user has acknowledged the agent moved on, so
        // the durable command still executes (the safety gate is for UNconfirmed
        // approves — see stale_gate_needs_confirm_before_executing).
        let outcome = bridge
            .resolve_choice_confirmable(&cid, "Approve".into(), true)
            .await
            .unwrap();
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
            .insert_tray_entry(
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
        // confirm_stale = true (post-restart is inherently "agent moved on").
        let outcome = bridge
            .resolve_choice_confirmable("cid-1", "Approve".into(), true)
            .await
            .unwrap();
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
            .insert_tray_entry(
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
        // confirm_stale = true on both (no in-memory parked → stale path).
        let first = body_of(
            bridge
                .resolve_choice_confirmable("cid-2", "Approve".into(), true)
                .await
                .unwrap(),
        );
        let second = body_of(
            bridge
                .resolve_choice_confirmable("cid-2", "Approve".into(), true)
                .await
                .unwrap(),
        );
        assert!(first.contains("output below"), "first resolve must execute: {first}");
        assert!(
            !second.contains("output below"),
            "second resolve must NOT re-execute (exactly-once): {second}"
        );
    }

    #[tokio::test]
    async fn stale_gate_needs_confirm_before_executing() {
        // SAFETY: a gated command whose agent has moved on must NOT run on a
        // plain (unconfirmed) approve — the user could be approving a command
        // that's now invalid/destructive. confirm_stale=false → NeedsConfirm,
        // nothing runs, the row stays pending; confirm_stale=true → it executes.
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
            .insert_tray_entry(
                "s1",
                "cid-stale",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Run gated command in this session's repo?",
                Some(&opts),
                None,
                Some(&cmd), // command_text → it's a gated command
            )
            .await
            .unwrap();
        // Age the row past the stale window (staleness is age-based now).
        let old = (chrono::Utc::now()
            - chrono::Duration::seconds(crate::signaling::STALE_GATE_MAX_AGE_SECS + 60))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query("UPDATE session_tray SET asked_at = ? WHERE choice_id = 'cid-stale'")
            .bind(&old)
            .execute(storage.pool())
            .await
            .unwrap();

        // Unconfirmed approve of a stale gate → NeedsConfirm, no execution.
        let outcome = bridge
            .resolve_choice("cid-stale", "Approve".into())
            .await
            .unwrap();
        match outcome {
            ResolveOutcome::StaleGateNeedsConfirm { command, .. } => assert_eq!(command, cmd),
            other => panic!("expected StaleGateNeedsConfirm, got {other:?}"),
        }
        assert!(!marker.exists(), "stale command must NOT run without confirm");
        let row = storage.get_tray_entry("cid-stale").await.unwrap().unwrap();
        assert_eq!(
            row.status, "pending",
            "unconfirmed stale resolve must not flip the row (confirmed retry needs it)"
        );

        // A Reject is always safe — no confirm needed, nothing executes.
        // (Use a fresh row so the exactly-once flip doesn't interfere.)
        storage
            .insert_tray_entry(
                "s1",
                "cid-reject",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Run gated command in this session's repo?",
                Some(&opts),
                None,
                Some(&cmd),
            )
            .await
            .unwrap();
        let outcome = bridge
            .resolve_choice("cid-reject", "Reject".into())
            .await
            .unwrap();
        assert!(
            !matches!(outcome, ResolveOutcome::StaleGateNeedsConfirm { .. }),
            "Reject must never require stale-confirm"
        );
        assert!(!marker.exists(), "Reject must not run the command");

        // Confirmed approve → executes, delivers output OOB.
        let outcome = bridge
            .resolve_choice_confirmable("cid-stale", "Approve".into(), true)
            .await
            .unwrap();
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { body, .. } => {
                assert!(body.contains("exit 0"), "confirmed run carries output: {body}")
            }
            other => panic!("expected AgentReceiverDroppedFellBack, got {other:?}"),
        }
        assert!(marker.exists(), "confirmed stale command must execute");
    }

    #[tokio::test]
    async fn park_gated_command_parks_dedupes_and_never_executes() {
        // #29(ii): the hook's route calls THIS, not action_gate. Two properties
        // matter. (1) It parks + dedupes like the agent-facing path. (2) It
        // does NOT resolve keywords, so it cannot reach action_gate's
        // auto_allow/no-match EXECUTE branch — a route wired to that would run
        // a command with no approval whenever its resolve disagreed with the
        // hook's. Proven here by parking a command that is NOT gated at all:
        // action_gate would run it; this must merely park it.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let marker = repo.path().join("ran.txt");
        let bridge = bridge_with(data.path(), &[gk("echo", GateMode::Gate)], "s1", repo.path()).await;
        let cmd = format!("touch {}", marker.display()); // matches no keyword

        let (gate_id, existing) = bridge
            .park_gated_command("s1", "brian", &cmd)
            .await
            .unwrap();
        assert!(!gate_id.is_empty());
        assert!(!existing, "first park is not a dedupe hit");
        assert!(
            !marker.exists(),
            "park_gated_command must never execute — that is the whole point of \
             not routing the hook through action_gate"
        );
        assert!(bridge.gate_status(&gate_id).await.unwrap().starts_with("pending"));

        // Identical command while pending → same gate, flagged existing, so a
        // retried Bash call can't stack a second card.
        let (dup_id, dup_existing) = bridge
            .park_gated_command("s1", "brian", &cmd)
            .await
            .unwrap();
        assert_eq!(dup_id, gate_id);
        assert!(dup_existing);

        // Approval still executes at resolve time, through the normal path.
        bridge.resolve_choice(&gate_id, "Approve".into()).await.unwrap();
        assert!(marker.exists(), "approve runs the parked command");
    }

    #[tokio::test]
    async fn gated_command_parks_immediately_and_dedupes_pending() {
        // The park contract: a Gate-mode command returns AT ONCE with a gate_id
        // (no held RPC → nothing to client-timeout), and re-issuing the same
        // command while the first is pending returns the existing gate instead
        // of stacking a duplicate Approve/Reject card.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let bridge = bridge_with(
            data.path(),
            &[gk("echo", GateMode::Gate)],
            "s1",
            repo.path(),
        )
        .await;

        let first = bridge
            .action_gate("s1".into(), "brian".into(), "echo hi".into())
            .await
            .unwrap();
        assert!(first.contains("parked for the user's approval"), "got: {first}");
        let gate_id = first
            .split("gate_id: ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap()
            .to_string();
        assert!(!gate_id.is_empty());

        // gate_status while pending.
        let status = bridge.gate_status(&gate_id).await.unwrap();
        assert!(status.starts_with("pending"), "got: {status}");

        // Identical command re-parked → the SAME gate, flagged as existing.
        let dup = bridge
            .action_gate("s1".into(), "brian".into(), "echo hi".into())
            .await
            .unwrap();
        assert!(dup.contains("ALREADY parked"), "got: {dup}");
        assert!(dup.contains(&gate_id), "dedupe returns the original gate id");

        // Approve executes exactly once and delivers output OOB; gate_status
        // then reports approved.
        let outcome = bridge
            .resolve_choice(&gate_id, "Approve".into())
            .await
            .unwrap();
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { body, .. } => {
                assert!(body.contains("exit 0"), "approve carries output: {body}")
            }
            other => panic!("expected OOB delivery, got {other:?}"),
        }
        let status = bridge.gate_status(&gate_id).await.unwrap();
        assert!(status.starts_with("approved"), "got: {status}");

        // A rejected re-fire parks FRESH (pending-only dedupe): reject the new
        // gate and confirm its status carries the user's reasoning.
        let refire = bridge
            .action_gate("s1".into(), "brian".into(), "echo hi".into())
            .await
            .unwrap();
        assert!(refire.contains("parked for the user's approval"), "post-resolve re-fire is a fresh gate: {refire}");
        let refire_id = refire
            .split("gate_id: ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap()
            .to_string();
        assert_ne!(refire_id, gate_id);
        bridge
            .resolve_choice(&refire_id, "Reject — wrong branch, retarget first".into())
            .await
            .unwrap();
        let status = bridge.gate_status(&refire_id).await.unwrap();
        assert!(status.starts_with("rejected"), "got: {status}");
        assert!(status.contains("wrong branch"), "reject reason surfaces: {status}");
    }

    #[tokio::test]
    async fn fresh_gates_execute_on_plain_approve_only_old_ones_need_confirm() {
        // Age-based staleness: a just-parked gate approves one-click; a gate
        // older than STALE_GATE_MAX_AGE_SECS needs the confirm step.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let marker = repo.path().join("fresh.txt");
        let cmd = format!("touch {}", marker.display());
        let bridge = bridge_with(
            data.path(),
            &[gk("touch", GateMode::Gate)],
            "s1",
            repo.path(),
        )
        .await;

        let parked = bridge
            .action_gate("s1".into(), "brian".into(), cmd.clone())
            .await
            .unwrap();
        let gate_id = parked
            .split("gate_id: ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap()
            .to_string();

        // Fresh (just asked): plain approve executes, no confirm round-trip.
        let outcome = bridge
            .resolve_choice(&gate_id, "Approve".into())
            .await
            .unwrap();
        assert!(
            matches!(outcome, ResolveOutcome::AgentReceiverDroppedFellBack { .. }),
            "fresh gate approves one-click, got {outcome:?}"
        );
        assert!(marker.exists(), "fresh approve executes");

        // Age a second gate past the window by rewriting its asked_at.
        let marker2 = repo.path().join("old.txt");
        let cmd2 = format!("touch {}", marker2.display());
        let parked2 = bridge
            .action_gate("s1".into(), "brian".into(), cmd2.clone())
            .await
            .unwrap();
        let gate_id2 = parked2
            .split("gate_id: ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap()
            .to_string();
        {
            let storage = bridge.storage.lock().await.clone().unwrap();
            let old = (chrono::Utc::now()
                - chrono::Duration::seconds(crate::signaling::STALE_GATE_MAX_AGE_SECS + 60))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            sqlx::query("UPDATE session_tray SET asked_at = ? WHERE choice_id = ?")
                .bind(&old)
                .bind(&gate_id2)
                .execute(storage.pool())
                .await
                .unwrap();
        }
        let outcome = bridge
            .resolve_choice(&gate_id2, "Approve".into())
            .await
            .unwrap();
        match outcome {
            ResolveOutcome::StaleGateNeedsConfirm { command, .. } => assert_eq!(command, cmd2),
            other => panic!("old gate needs confirm, got {other:?}"),
        }
        assert!(!marker2.exists(), "old gate must not run without confirm");
    }
}
