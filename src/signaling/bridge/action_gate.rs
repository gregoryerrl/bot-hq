//! `action_gate` — the execute-on-approve half of the Tool Gate.
//!
//! The PreToolUse hook blocks a `gate`-mode Bash command (exit 2) and tells the
//! agent to call `action_gate(command)`. This module classifies the command
//! against the session's resolved keyword list (`tool_gate::resolve_keywords`:
//! the session snapshot first, the global list as fallback) and:
//!   - `auto_allow` / no-match → runs it immediately in the session's repo,
//!   - `gate`                  → surfaces Approve/Reject; on approve, runs it.
//!
//! Either way bot-hq EXECUTES the command server-side (in the session's
//! `working_repo_path`, resolved from storage) and returns combined output to
//! the agent — it's an ACTION request, not a permission request. The agent does
//! NOT re-run the command; the returned output IS the result.

use super::util::gate_verdict;
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
        require_approval: bool,
    ) -> Result<String> {
        // **`require_approval` parks unconditionally** (round 12, EYES F19):
        // the keyword resolve below runs an unmatched / auto_allow command
        // outright, which is right for a command the Tool Gate blocked and
        // wrong for one the AGENT decided must not run unapproved — a prod
        // query on a machine with no prod-host keyword configured would have
        // executed with no approval at all. This is the same unconditional
        // park the `/hooks/tool-gate` route uses, for the same reason.
        if require_approval {
            let outcome = self.park_gated_command(&session_id, &agent, &command).await?;
            return Ok(park_outcome_text(&outcome, &command));
        }
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
                let outcome = self.park_gated_command(&session_id, &agent, &command).await?;
                Ok(park_outcome_text(&outcome, &command))
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
    ) -> Result<ParkOutcome> {
        // **Outward-review precondition (batch 2 C, 2026-08-27).** An OUTWARD
        // command — one that publishes under the user's identity — may park
        // only after the session's reviewer has been DELIVERED its content.
        // Both eras' escapes landed in this hole: the morning's two false
        // claims went out through gates while the reviewer was starved, and
        // the afternoon's empty-bodied PR raced its own retraction. Until 0080
        // the check REFUSED (teaching a two-turn ritual); it now QUEUES the
        // park durably and settles it after the reviewer's turn — nothing is
        // timed, the row survives restart, and a rejected gate still re-parks
        // without re-review when the content is unchanged, because coverage is
        // keyed on the content itself. Correctness refusals (reviewer down,
        // empty or unextractable body) still refuse.
        //
        // Ceiling, stated plainly: delivery is provable, review is not — a
        // reviewer that passed its turn satisfies this check. It is the honest
        // limit of a mechanical precondition.
        //
        // Dedupe for the QUEUE runs first: an identical command already queued
        // must not queue twice and summon the reviewer twice (EYES P8) — and
        // must not re-run the check, which would find the body unread and
        // queue again.
        //
        // A DOWN reviewer is reported before the dedupe (feedback #32): "already
        // queued, nothing to do" would leave the executor waiting on a read
        // that can never happen.
        if outward_command(command) && self.reviewer_override_reason(session_id).is_none() {
            if let Some((reviewer, health)) = self.session_reviewer_down(session_id, agent) {
                anyhow::bail!(
                    "outward publish held: the reviewer ({reviewer}) is {health} and not \
                     recently active. Respawn it, or ask the user to approve \
                     override_reviewer_block — approving it also releases any publish already \
                     queued for review to the user, marked unreviewed."
                );
            }
        }
        {
            let storage = self.storage.lock().await.clone();
            if let Some(storage) = storage {
                if let Ok(Some(existing)) = storage.queued_gate_for_command(session_id, command).await {
                    return Ok(ParkOutcome::Queued { gate_id: existing, existing: true });
                }
            }
        }
        let (note, covered) = match self.outward_review_check(session_id, agent, command).await? {
            OutwardReview::Refuse(text) => return Err(anyhow::anyhow!(text)),
            OutwardReview::Queued { reviewer_id } => {
                let (gate_id, existing) =
                    self.queue_outward_park(session_id, agent, command, reviewer_id).await?;
                return Ok(ParkOutcome::Queued { gate_id, existing });
            }
            OutwardReview::Proceed(note) => {
                if let Some(n) = &note {
                    tracing::warn!(session_id, agent, note = %n, "outward park proceeding with review precondition skipped");
                }
                (note, None)
            }
            OutwardReview::Covered { reviewer, rows } => {
                let cited = covered_rows_text(&rows);
                (
                    Some(format!(
                        "coverage: the reviewer ({reviewer}) already received this exact body \
                         ({cited}), so it parks without a new review turn."
                    )),
                    Some((reviewer, cited)),
                )
            }
        };
        let (gate_id, existing) = self.park_reviewed_command(session_id, agent, command).await?;
        if !existing {
            self.record_body_digest(session_id, &gate_id, command).await;
        }
        // Feedback #40: a publish that parks on PRIOR review used to leave no
        // row at all — the reviewer could not tell it had gone to the user.
        // Say it in the channel, once per fresh park.
        if let (Some((reviewer, cited)), false) = (covered, existing) {
            let storage = self.storage.lock().await.clone();
            if let Some(storage) = storage {
                let _ = crate::core::post_system_notice(
                    &storage,
                    Some(self),
                    session_id,
                    crate::storage::MessageKind::SystemNotice,
                    format!(
                        "📨 Outward publish {gate_id} parked for the user on PRIOR review — the \
                         reviewer ({reviewer}) already received this exact body ({cited}): \
                         `{command}`"
                    ),
                    None,
                )
                .await;
            }
        }
        Ok(ParkOutcome::Parked { gate_id, existing, note })
    }

    /// The park itself, AFTER the outward-review precondition. Split out so
    /// the check cannot be skipped by a new caller reaching for "just park":
    /// this fn is private, `park_gated_command` is the only route in.
    async fn park_reviewed_command(
        &self,
        session_id: &str,
        agent: &str,
        command: &str,
    ) -> Result<(String, bool)> {
        // Duplicate suppression: an identical command already awaiting
        // approval gets the existing gate back instead of stacking a
        // second confusable prompt. PENDING rows only — a re-fire after
        // a reject is an intentional retry and parks fresh.
        // Bound first: an `if let` scrutinee's temporaries — the mutex guard —
        // live to the end of the statement in edition 2021, i.e. across the
        // await below. `let` drops the guard before the body runs.
        let storage = self.storage.lock().await.clone();
        let mut prior_rejection: Option<(String, String)> = None;
        if let Some(storage) = storage {
            if let Ok(Some(existing)) = storage.pending_gate_for_command(session_id, command).await {
                return Ok((existing, true));
            }
            prior_rejection = storage
                .last_rejection_for_command(session_id, command)
                .await
                .ok()
                .flatten();
        }
        // What the user is approving, in one glance (F6, feedback #29): the
        // keyword the Tool Gate matched and where — a destructive literal
        // inside a grep pattern reads as one — and, when this exact command was
        // rejected before, that verdict (E6): a re-park is a fresh card by
        // design, and must not read as a first ask.
        let why = self
            .data_dir
            .as_ref()
            .map(|d| tool_gate::resolve_keywords(d, Some(session_id)))
            .and_then(|kws| tool_gate::gate_match_detail("Bash", command, &kws))
            .map(|m| format!("\n\n{}.", m.describe()))
            .unwrap_or_default();
        let rejected = prior_rejection
            .map(|(at, picked)| {
                format!("\n\n⚠ You REJECTED this identical command at {at}: \"{picked}\"")
            })
            .unwrap_or_default();
        // Park and return IMMEDIATELY (same contract as ask_user_choice). The
        // old design held the RPC open and the MCP client timed out at ~60s
        // while the human was still deciding — the agent saw "The operation
        // timed out" and could not tell queued from failed (six such ghosts in
        // the archive study).
        let parked = self
            .ask_user_choice_inner(
                session_id.to_string(),
                agent.to_string(),
                format!("Run gated command in this session's repo?\n\n`{command}`{why}{rejected}"),
                vec!["Approve".to_string(), "Reject".to_string()],
                Some(ApprovalContext {
                    kind: ViolationKind::ToolBlocklist,
                    action: command.to_string(),
                    detail: Some("tool-gate".to_string()),
                    command: None,
                }),
                None,
                false,
                true,
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
        self.gate_status_for(gate_id, None).await
    }

    /// [`gate_status`](Self::gate_status) scoped to the caller's session
    /// (round 11): a gate row carries the user's answer text and the exact
    /// command, and the tool is deliberately ungated, so a participant holding
    /// another session's id could read that session's gate. Another session's
    /// gate answers exactly like a missing one — no oracle. `None` = unscoped
    /// (host / tests).
    pub async fn gate_status_for(&self, gate_id: &str, session_id: Option<&str>) -> Result<String> {
        let storage = self
            .storage
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("storage not configured"))?;
        let Some(row) = storage.get_tray_entry(gate_id).await? else {
            return Ok(format!("gate_status: no gate with id {gate_id}"));
        };
        if session_id.is_some_and(|sid| sid != row.session_id) {
            return Ok(format!("gate_status: no gate with id {gate_id}"));
        }
        // Only a ToolBlocklist (action_gate) approval carries a command —
        // `ask_user_choice_inner` sets `command_text` for that kind alone. A
        // parked `request_approval` (push_gate / per_action) has none, so the
        // command-shaped wording would assert an execution that never happened.
        let Some(command) = row.command_text.as_deref() else {
            return Ok(Self::approval_status_text(&row));
        };
        Ok(match row.status.as_str() {
            "queued" => format!(
                "queued — `{command}` is an outward publish waiting for the reviewer to read \
                 its body (posted to the channel; the reviewer is summoned). It parks for the \
                 user on its own after their turn, or is withdrawn if they file a blocking \
                 finding. Do not re-issue it."
            ),
            "pending" => format!(
                "pending — `{command}` is still awaiting the user's approval. Do not \
                 re-issue it; the outcome will arrive as an out-of-band message."
            ),
            "answered" => {
                let picked = row.picked_option.as_deref().unwrap_or("");
                if matches!(gate_verdict(picked), ViolationOutcome::Approved) {
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
        self.execute_gated_with(session_id, command, tool_gate::DEFAULT_TIMEOUT, &[])
            .await
    }

    /// [`execute_gated`] with the caller's bound and extra env pairs — the
    /// push re-run (round 12) needs both: a network-sized timeout and the
    /// single-use nonce its own pre-push hook redeems.
    pub(super) async fn execute_gated_with(
        &self,
        session_id: &str,
        command: &str,
        timeout: std::time::Duration,
        extra_envs: &[(&str, &str)],
    ) -> Result<String> {
        let cwd = self.session_working_repo(session_id).await.ok_or_else(|| {
            anyhow::anyhow!(
                "action_gate: session {session_id} has no working_repo_path — cannot execute `{command}`"
            )
        })?;

        // The child carries the session's identity (round 12): the git hooks
        // inside a gated `git commit` / `git push` read `BOT_HQ_SESSION_ID`.
        let session = tool_gate::session_envs(session_id);
        let mut envs: Vec<(&str, &str)> = session.iter().map(|(k, v)| (*k, v.as_str())).collect();
        envs.extend_from_slice(extra_envs);
        let out = tool_gate::run_in_repo(command, &cwd, timeout, &envs).await;
        Ok(format_command_output(&out))
    }

    /// The session's `working_repo_path` from storage — the source of truth on
    /// the session row (no parallel bridge map to keep in sync). None when the
    /// session is unknown, storage isn't wired, or the row has no repo path.
    /// `(reviewer slug, health)` when this session's reviewer (anyone but
    /// `agent`) is stalled or dead AND has made no recent MCP call — the
    /// liveness test the commit gate's reviewer-down branch uses.
    fn session_reviewer_down(&self, session_id: &str, agent: &str) -> Option<(String, String)> {
        let reviewer = self
            .session_reviewers(session_id)
            .into_iter()
            .find(|slug| slug != agent)?;
        let health = self.current_agent_health(session_id, &reviewer);
        let recent =
            self.agent_rpc_recent(session_id, &reviewer, super::findings::REVIEWER_LIVENESS_WINDOW);
        match health.as_deref() {
            Some(h @ ("stalled" | "dead")) if !recent => Some((reviewer, h.to_string())),
            _ => None,
        }
    }

    /// SHA-256 over the command's body FILES as they are on disk now —
    /// `(path, NUL, bytes, NUL)` per file, in the command's order — or the
    /// first path that cannot be read. `None` when the command publishes no
    /// body file (the inline ones are in the command text itself, which
    /// cannot change after review). Feedback #22.
    pub(crate) async fn body_files_digest(
        &self,
        session_id: &str,
        command: &str,
    ) -> Option<Result<String, String>> {
        use sha2::{Digest, Sha256};
        let files = super::outward_body::extract(command).ok()?.files;
        if files.is_empty() {
            return None;
        }
        let mut hasher = Sha256::new();
        for p in &files {
            let resolved = self.resolve_body_path(session_id, p).await;
            match std::fs::read(&resolved) {
                Ok(bytes) => {
                    hasher.update(p.as_bytes());
                    hasher.update([0u8]);
                    hasher.update(&bytes);
                    hasher.update([0u8]);
                }
                Err(_) => return Some(Err(p.clone())),
            }
        }
        Some(Ok(format!("{:x}", hasher.finalize())))
    }

    /// Record the body hash on a freshly parked or queued gate (0084).
    async fn record_body_digest(&self, session_id: &str, gate_id: &str, command: &str) {
        let Some(Ok(sha)) = self.body_files_digest(session_id, command).await else { return };
        let storage = self.storage.lock().await.clone();
        if let Some(storage) = storage {
            if let Err(e) = storage.set_tray_body_sha(gate_id, &sha).await {
                tracing::warn!(%gate_id, error = %e, "could not record the gate's body hash");
            }
        }
    }

    /// A body path as the command will read it: relative paths resolve
    /// against the session's working repo — where `execute_gated` runs it.
    pub(super) async fn resolve_body_path(&self, session_id: &str, p: &str) -> PathBuf {
        let path = std::path::Path::new(p);
        if path.is_relative() {
            if let Some(repo) = self.session_working_repo(session_id).await {
                return repo.join(path);
            }
        }
        path.to_path_buf()
    }

    async fn session_working_repo(&self, session_id: &str) -> Option<PathBuf> {
        let storage = self.storage.lock().await.clone()?;
        let session = storage.get_session(session_id).await.ok()??;
        session.working_repo_path.map(PathBuf::from)
    }
}

/// The outward-review verdict: park may proceed (with an optional LOUD note
/// for the ack — a guard that quietly isn't watching is indistinguishable
/// from one with nothing to report), is refused with teaching text (a
/// correctness refusal: reviewer down, empty or unextractable body), or —
/// since 0080 — is QUEUED because the reviewer has simply not read the content
/// yet: bot-hq delivers it, summons the reviewer and parks after their turn.
pub(crate) enum OutwardReview {
    Proceed(Option<String>),
    Refuse(String),
    Queued { reviewer_id: i64 },
    /// Every body was already RECEIVED by the reviewer — `rows` are the
    /// covering message ids, oldest first. Parks straight for the user, and
    /// says so in the channel (feedback #40: a publish that parked on prior
    /// review left no trace the reviewer could see).
    Covered { reviewer: String, rows: Vec<i64> },
}

/// What [`SignalingBridge::park_gated_command`] did with a command. `Parked` is
/// the user's card (fresh, or a dedupe hit on an identical pending one);
/// `Queued` is a row the reviewer must read first (0080) — fresh, or a dedupe
/// hit on an identical queued one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParkOutcome {
    Parked { gate_id: String, existing: bool, note: Option<String> },
    Queued { gate_id: String, existing: bool },
}

impl ParkOutcome {
    pub(crate) fn gate_id(&self) -> &str {
        match self {
            Self::Parked { gate_id, .. } | Self::Queued { gate_id, .. } => gate_id,
        }
    }
}

/// The agent-facing text for a QUEUED outward publish. Says what happens next
/// and — load-bearing — that the executor may halt: the refusal this replaces
/// told it to end its turn bare so the ring would deal the reviewer, which
/// collided with "every stop declares itself" and produced the #32 deadlock.
pub(crate) fn queued_gate_text(gate_id: &str, command: &str, existing: bool) -> String {
    let lead = if existing {
        "action_gate: an identical outward command is ALREADY queued for review"
    } else {
        "action_gate: QUEUED for the reviewer"
    };
    format!(
        "{lead} (gate_id: {gate_id}): `{command}`.\n\
         This is an OUTWARD publish and the reviewer has not read its content yet. \
         bot-hq has posted the body to the channel for them and summoned them for the \
         next turn; after that turn the gate parks for the user's approval on its own \
         (same gate id) — unless they file a blocking finding, in which case it is \
         withdrawn and a system row says so. Nothing for you to do: do NOT re-issue, do \
         NOT post the body yourself, and halting is fine — the queued row keeps the \
         session from reading as idle. `gate_status(\"{gate_id}\")` reports queued / \
         pending / answered / withdrawn."
    )
}

/// OUTWARD classifier — see [`super::outward_body::is_outward`]: a simple
/// command running `gh` or `curl`, seen through quotes, env assignments,
/// wrappers (`env`, `command`, `sudo`) and `sh -c` / `eval` strings.
fn outward_command(command: &str) -> bool {
    super::outward_body::is_outward(command)
}

impl SignalingBridge {
    /// The C precondition (plan, batch 2): outward parks require the
    /// reviewer to have been DELIVERED the content. Full-body match — raw or
    /// JSON-escaped form (a `session_doc_write`'s tool_use row carries the
    /// body escaped) — against rows at or below the reviewer's cursor.
    /// Head/tail sampling was rejected in review: a mid-body edit after
    /// review is this morning's exact escape shape. Content-free outward
    /// commands (merge/close/label) get the timeline check instead — there
    /// is no payload to cover, and the PR body a merge lands was itself
    /// coverage-checked at creation.
    pub(crate) async fn outward_review_check(
        &self,
        session_id: &str,
        agent: &str,
        command: &str,
    ) -> Result<OutwardReview> {
        if !outward_command(command) {
            return Ok(OutwardReview::Proceed(None));
        }
        let reviewers: Vec<String> = self
            .session_reviewers(session_id)
            .into_iter()
            .filter(|slug| slug != agent)
            .collect();
        let Some(reviewer_slug) = reviewers.first() else {
            return Ok(OutwardReview::Proceed(Some(
                "note: no reviewer in this roster — outward review precondition skipped".into(),
            )));
        };
        // Reviewer down → the same escape hatch the commit gate has: a
        // user-approved override, never a timer.
        if let Some((_, health)) = self.session_reviewer_down(session_id, agent) {
            return Ok(match self.reviewer_override_reason(session_id) {
                Some(_) => OutwardReview::Proceed(Some(
                    "note: reviewer down — user-approved override in effect; outward review \
                     precondition skipped"
                        .into(),
                )),
                None => OutwardReview::Refuse(format!(
                    "outward publish held: the reviewer ({reviewer_slug}) is \
                     {health} and not recently active. Respawn it, or ask the user to \
                     approve override_reviewer_block."
                )),
            });
        }
        let storage = self
            .storage
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no storage wired"))?;
        let Some(reviewer) = storage.participant_by_slug(session_id, reviewer_slug).await? else {
            return Ok(OutwardReview::Proceed(Some(
                "note: reviewer not in this session's roster rows — outward review \
                 precondition skipped"
                    .into(),
            )));
        };
        // WHAT publishes (feedback #35): the per-subcommand extractor, which
        // REFUSES every form whose content cannot be read ahead of the run —
        // shell-computed, stdin, written by the same command, or carried by a
        // file flag it does not know.
        let extracted = match super::outward_body::extract(command) {
            Ok(b) => b,
            Err(why) => return Ok(OutwardReview::Refuse(format!("outward publish held: {why}"))),
        };
        let mut bodies = extracted.inline;
        for p in extracted.files {
            let resolved = self.resolve_body_path(session_id, &p).await;
            match std::fs::read_to_string(&resolved) {
                Ok(s) if s.len() <= 256 * 1024 => bodies.push(s),
                Ok(_) => {
                    return Ok(OutwardReview::Refuse(format!(
                        "outward publish held: {p} is too large to coverage-check — \
                         post the body in the channel or a session doc first."
                    )))
                }
                Err(e) => {
                    return Ok(OutwardReview::Refuse(format!(
                        "outward publish held: cannot read {p} to check review \
                         coverage ({e}) — post the body in the channel or a session \
                         doc first."
                    )))
                }
            }
        }
        // Defence in depth (review round 2): the extractor is the authority on
        // body forms, so a body-carrying flag it returned NOTHING for means a
        // form it mis-parsed — refuse rather than downgrade to the weaker
        // timeline check while looking armed.
        let mentions_body = command.split_whitespace().any(|t| {
            let t = t.trim_start_matches(['"', '\'']);
            ["--body", "--notes", "--input", "--data", "--field", "--raw-field", "--form"]
                .iter()
                .any(|f| t == *f || t.starts_with(&format!("{f}=")) || t.starts_with(&format!("{f}-file")))
        });
        if bodies.is_empty() && mentions_body {
            return Ok(OutwardReview::Refuse(
                "outward publish held: this command carries a body in a form the \
                 coverage check cannot extract. Use --body-file <path> (or --notes-file) \
                 so the reviewer-delivered content can be verified."
                    .into(),
            ));
        }
        let cursor = storage.cursor_for(reviewer.id).await?;
        // A TARGETED veto holds its content until the finding is answered: a
        // re-issue of the exact command it withdrew is refused while that
        // finding is open. Fix it (disposition `fixed`) or rebut it first —
        // either is on the record the user reads.
        let veto = storage.open_targeted_veto_for_command(session_id, command).await?;
        // A body FILE that changed since the vetoed version is new content —
        // it goes to a fresh review instead (0084).
        let body_changed = match veto.as_ref().and_then(|(_, _, sha)| sha.as_deref()) {
            Some(vetoed_sha) => matches!(
                self.body_files_digest(session_id, command).await,
                Some(Ok(now)) if now != vetoed_sha
            ),
            None => false,
        };
        let veto = if body_changed { None } else { veto };
        if let Some((finding, gate, _)) = veto {
            return Ok(OutwardReview::Refuse(format!(
                "outward publish held: open blocking finding {} vetoed this exact publish (gate \
                 {}). Fix what it raises (a changed body file goes to a fresh review) and \
                 disposition it `fixed`, or rebut it, then re-issue.",
                &finding[..finding.len().min(8)],
                &gate[..gate.len().min(8)]
            )));
        }
        // A publish a blocking finding WITHDREW is re-reviewed, never re-parked
        // on its old coverage: its body row was delivered when it first
        // queued, so coverage (or the timeline check, for a content-free
        // command) would put the vetoed publish straight on the user's card.
        if storage.latest_gate_withdrawn_by_finding(session_id, command).await? {
            return Ok(OutwardReview::Queued { reviewer_id: reviewer.id });
        }
        if bodies.is_empty() {
            // Content-free outward: timeline check — a reviewer deal strictly
            // between the caller's previous and current deals.
            let Some(caller) = storage.participant_by_slug(session_id, agent).await? else {
                return Ok(OutwardReview::Proceed(Some(
                    "note: caller not in roster rows — outward review precondition skipped".into(),
                )));
            };
            let deals = storage.deal_instants(caller.id, 2).await?;
            let current = deals.first().cloned().unwrap_or_default();
            let prev = deals.get(1).cloned().unwrap_or_default();
            // Not yet dealt → QUEUE (0080), never refuse: the two-turn ritual
            // this replaced cost 50 refusals in one week and deadlocked once
            // (#32 — every typed user message restarted the rotation at the
            // executor). The summons the queue sends is what a restart cannot
            // undo, and the row is what a relaunch cannot lose.
            return Ok(
                if storage.has_delivery_between(reviewer.id, &prev, &current).await? {
                    OutwardReview::Proceed(None)
                } else {
                    OutwardReview::Queued { reviewer_id: reviewer.id }
                },
            );
        }
        // Coverage window: the newest 500 rows at or below the cursor that the
        // reviewer RECEIVED (its backlog's own filter, clamped as delivered).
        // A body older than that reads as uncovered — failing closed, documented.
        let haystack = storage
            .reviewer_received_bodies_upto(session_id, reviewer.id, cursor, 500)
            .await?;
        let mut covering: Vec<i64> = Vec::new();
        for body in &bodies {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                // Fail CLOSED (review round 2): an empty body is this
                // afternoon's actual escape — PR #559 went out empty-bodied
                // from `--body-file /dev/stdin` with nothing piped, and the
                // old `continue` here would have parked it as if reviewed.
                return Ok(OutwardReview::Refuse(
                    "outward publish held: the body resolves to EMPTY — a \
                     --body-file reading an empty file or unpiped stdin \
                     produces this. Write the real body to a file first."
                        .into(),
                ));
            }
            let escaped = serde_json::to_string(trimmed).unwrap_or_default();
            let escaped = escaped.trim_matches('"');
            // Newest first, so the id recorded is the latest delivery.
            let covered = haystack
                .iter()
                .find(|(_, row)| row.contains(trimmed) || row.contains(escaped))
                .map(|(id, _)| *id);
            if let Some(id) = covered {
                if !covering.contains(&id) {
                    covering.push(id);
                }
            } else {
                // Not read yet → QUEUE (0080): bot-hq posts THIS body as a row
                // and parks once the reviewer's cursor has passed it — the
                // executor no longer re-emits a 25 KB `--body-file` into a
                // session doc to make it "delivered". Coverage stays
                // content-keyed: an unchanged body the reviewer already read
                // proceeds straight to the park above this branch.
                return Ok(OutwardReview::Queued { reviewer_id: reviewer.id });
            }
        }
        covering.sort_unstable();
        Ok(OutwardReview::Covered { reviewer: reviewer_slug.clone(), rows: covering })
    }

    /// The body a queued outward command carries, as the reviewer should read
    /// it: every `--body-file` resolved (relative to the session repo), inline
    /// `--body` strings verbatim, and — for a content-free command such as a
    /// merge or close — the command line itself, which IS the content. Read
    /// failures degrade to a note rather than an error: the check above
    /// already refused unreadable and oversized files before we got here.
    async fn queued_body_text(&self, session_id: &str, command: &str) -> String {
        // The check above already refused every form `extract` rejects; an
        // error here degrades to the command line, which is still content.
        let super::outward_body::OutwardBodies { files, inline } =
            super::outward_body::extract(command).unwrap_or_default();
        let mut parts: Vec<String> = Vec::new();
        for p in files {
            let resolved = self.resolve_body_path(session_id, &p).await;
            match std::fs::read_to_string(&resolved) {
                Ok(s) => parts.push(format!("--- {p} ---\n{}", s.trim_end())),
                Err(e) => parts.push(format!("--- {p} --- (unreadable at queue time: {e})")),
            }
        }
        for s in inline {
            parts.push(format!("--- inline ---\n{s}"));
        }
        if parts.is_empty() {
            format!("(content-free command — the command line is the content)\n{command}")
        } else {
            parts.join("\n\n")
        }
    }

    /// Queue an outward park (0080): ONE durable row, in this order, each step
    /// awaited before the next so nothing observable runs ahead of its record —
    /// (1) the `queued` tray row (dedupe first: an identical queued command
    /// returns its existing id and summons nobody twice), (2) the body posted
    /// as a system row the reviewer's backlog will carry, (3) the row pointed
    /// at that body, (4) the reviewer summoned. No `GateOpened`: D35's latch
    /// closes only when settlement promotes the row.
    async fn queue_outward_park(
        &self,
        session_id: &str,
        agent: &str,
        command: &str,
        reviewer_id: i64,
    ) -> Result<(String, bool)> {
        let storage = self
            .storage
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("storage not configured"))?;
        if let Some(existing) = storage.queued_gate_for_command(session_id, command).await? {
            return Ok((existing, true));
        }
        let gate_id = uuid::Uuid::new_v4().to_string();
        let prompt = format!("Run gated command in this session's repo?\n\n`{command}`");
        storage
            .insert_queued_gate(session_id, &gate_id, agent, &prompt, command)
            .await?;
        self.record_body_digest(session_id, &gate_id, command).await;
        let body = self.queued_body_text(session_id, command).await;
        let notice = format!(
            "📨 Outward publish queued for review (gate {gate_id}) — {agent} wants to run:\n\
             `{command}`\n\n{body}\n\n\
             [Reviewer: this content publishes under the user's identity. Read it on this \
             turn. To stop THIS publish, file a `blocking` finding with \
             gate_id \"{gate_id}\". A `blocking` finding with NO gate_id withdraws EVERY \
             queued publish, so a blocking finding about something else should pass \
             gate_id \"none\". Anything else lets it park for the user after your turn.]"
        );
        let Some(row) = crate::core::post_system_notice(
            &storage,
            Some(self),
            session_id,
            crate::storage::MessageKind::SystemNotice,
            notice,
            None,
        )
        .await
        else {
            // The body did not land: withdraw the row rather than leave a
            // queued gate that can never settle, and tell the caller.
            let _ = storage
                .withdraw_queued_gate(&gate_id, "body row could not be posted")
                .await;
            anyhow::bail!("outward publish could not be queued: the body row was not posted");
        };
        storage.set_tray_body_row(&gate_id, row.message_id()).await?;
        self.summon_participant(session_id, reviewer_id).await;
        Ok((gate_id, false))
    }

    /// Ask the ring to deal `participant_id` next, without resetting the
    /// rotation — the same `SequencerCommand::Summon` an `@mention` and the
    /// retry ladder send. `try_send` like every ring notify (never block inside
    /// a tool call); a miss is logged and healed by the relaunch re-arm.
    pub(crate) async fn summon_participant(&self, session_id: &str, participant_id: i64) {
        let seq = self.session_sequencer.lock().await.get(session_id).cloned();
        match seq {
            Some(tx) => {
                if tx
                    .try_send(crate::core::sequencer::SequencerCommand::Summon { participant_id })
                    .is_err()
                {
                    tracing::warn!(
                        session_id,
                        participant_id,
                        "a reviewer summons for a queued outward publish did not reach the ring"
                    );
                }
            }
            None => tracing::warn!(
                session_id,
                participant_id,
                "no ring registered — the queued outward publish waits for the re-arm"
            ),
        }
    }

    /// The reviewer a queued outward publish waits on: the first roster
    /// participant holding the finding capability who is not the executor.
    async fn queued_reviewer(
        &self,
        session_id: &str,
        agent: &str,
    ) -> Option<crate::storage::Participant> {
        let slug = self
            .session_reviewers(session_id)
            .into_iter()
            .find(|s| s != agent)?;
        let storage = self.storage.lock().await.clone()?;
        storage.participant_by_slug(session_id, &slug).await.ok().flatten()
    }

    /// Settle the session's queued outward publishes after `participant_id`
    /// finished a turn (0080): for each queued row whose reviewer is that
    /// participant and whose body row is at or below the reviewer's CURSOR —
    /// the row was read, not merely delivered (E1) — either withdraw it (a
    /// `blocking` finding was filed after it was queued) or promote it to the
    /// user's `pending` card with every side effect a fresh park has: the
    /// awaiting flag, the D35 latch, the UI event. Idempotent: the promote is
    /// an atomic `queued → pending` flip, so a second call finds nothing.
    pub async fn settle_queued_outward(&self, session_id: &str, participant_id: i64) {
        let Some(storage) = self.storage.lock().await.clone() else { return };
        let Ok(queued) = storage.queued_gates_for_session(session_id).await else { return };
        if queued.is_empty() {
            return;
        }
        let Ok(cursor) = storage.cursor_for(participant_id).await else { return };
        let findings = storage.findings_for_session(session_id).await.unwrap_or_default();
        for row in queued {
            let Some(reviewer) = self.queued_reviewer(session_id, &row.agent).await else {
                continue;
            };
            if reviewer.id != participant_id {
                continue;
            }
            let Some(body_row) = row.body_row_id else { continue };
            if body_row > cursor {
                // Delivered, not yet read — stays queued until the reviewer's
                // cursor passes it (E1: delivery alone never satisfies).
                continue;
            }
            let command = row.command_text.clone().unwrap_or_default();
            if let Some(veto) = findings.iter().find(|f| vetoes_queued_gate(f, &row)) {
                self.withdraw_vetoed_row(&storage, session_id, &row, veto).await;
                continue;
            }
            self.promote_queued_row(
                &storage,
                session_id,
                &row,
                format!(
                    "✅ Queued outward publish {} is now PARKED for the user's approval (the \
                     reviewer read it and filed nothing blocking): `{command}`",
                    row.choice_id
                ),
            )
            .await;
        }
    }

    /// Withdraw a queued row a blocking finding vetoes, and say why in the
    /// channel — the ONE withdrawal both settlement and the reviewer-down
    /// release use, so a vetoed publish can never reach the user as
    /// "unreviewed" (EYES 12c739ec).
    async fn withdraw_vetoed_row(
        &self,
        storage: &crate::storage::Storage,
        session_id: &str,
        row: &crate::storage::SessionTrayEntry,
        veto: &crate::storage::Finding,
    ) {
        if storage
            .withdraw_queued_gate(&row.choice_id, crate::storage::FINDING_WITHDRAWAL_REASON)
            .await
            .unwrap_or(0)
            != 1
        {
            return;
        }
        let command = row.command_text.clone().unwrap_or_default();
        let finding = &veto.finding_uid[..veto.finding_uid.len().min(8)];
        let why = if veto.gate_id.is_some() {
            format!(
                "the reviewer's blocking finding {finding} names this publish. Fix what it \
                 raises and disposition the finding `fixed` (or rebut it), then re-issue — a \
                 re-issue is refused while the finding is open, and after that it queues for a \
                 fresh review"
            )
        } else {
            format!(
                "the reviewer filed blocking finding {finding} naming no gate, which withdraws \
                 every queued publish. If it is about this content, fix it and re-issue; if it \
                 is about something else, re-issue unchanged — the re-issue queues for a fresh \
                 review and the earlier finding does not veto it, so there is no need to \
                 disposition it first"
            )
        };
        let _ = crate::core::post_system_notice(
            storage,
            Some(self),
            session_id,
            crate::storage::MessageKind::SystemNotice,
            format!("⛔ Queued outward publish {} withdrawn — {why}: `{command}`", row.choice_id),
            None,
        )
        .await;
    }

    /// Flip one queued row to a PENDING user card with the side effects a
    /// fresh park has (`ask_user_choice_inner`), minus the in-memory oneshot —
    /// a pending row with no parked oneshot is the post-restart shape
    /// `resolve_choice` already handles — then post `notice`. A lost race
    /// (another settlement already flipped it) does nothing.
    async fn promote_queued_row(
        &self,
        storage: &crate::storage::Storage,
        session_id: &str,
        row: &crate::storage::SessionTrayEntry,
        notice: String,
    ) -> bool {
        if storage.promote_queued_gate(&row.choice_id).await.unwrap_or(0) != 1 {
            return false;
        }
        let command = row.command_text.clone().unwrap_or_default();
        self.set_session_awaiting(session_id, &row.agent, false).await;
        self.notify_ring_gate(session_id, &row.choice_id, true).await;
        let _ = self.event_tx.send(SignalingEvent::PendingChoice(PendingChoice {
            choice_id: row.choice_id.clone(),
            session_id: session_id.to_string(),
            agent: row.agent.clone(),
            question: row.prompt.clone(),
            options: vec!["Approve".to_string(), "Reject".to_string()],
            approval: Some(ApprovalContext {
                kind: ViolationKind::ToolBlocklist,
                action: command,
                detail: Some("tool-gate".to_string()),
                command: None,
            }),
        }));
        let _ = crate::core::post_system_notice(
            storage,
            Some(self),
            session_id,
            crate::storage::MessageKind::SystemNotice,
            notice,
            None,
        )
        .await;
        true
    }

    /// Feedback #32 / D36: the user approved a reviewer-down override, so the
    /// publishes queued for the downed reviewer's read can no longer settle —
    /// settlement runs only on the reviewer's own TurnComplete. Release each
    /// to the user as its OWN Approve card, loudly marked unreviewed. Never a
    /// timer: this runs only on the user's Approve of the override.
    pub(crate) async fn release_queued_outward_unreviewed(&self, session_id: &str) -> usize {
        let Some(storage) = self.storage.lock().await.clone() else { return 0 };
        let Ok(queued) = storage.queued_gates_for_session(session_id).await else { return 0 };
        let findings = storage.findings_for_session(session_id).await.unwrap_or_default();
        let mut released = 0;
        for row in queued {
            // A publish the reviewer VETOED before it went down was reviewed —
            // withdraw it exactly as settlement would, never release it as
            // "unreviewed" (EYES 12c739ec).
            if let Some(veto) = findings.iter().find(|f| vetoes_queued_gate(f, &row)) {
                self.withdraw_vetoed_row(&storage, session_id, &row, veto).await;
                continue;
            }
            let command = row.command_text.clone().unwrap_or_default();
            let notice = format!(
                "⚠ Queued outward publish {} released to the user UNREVIEWED — the user approved \
                 a reviewer-down override, so the reviewer will not read it. It parks as its own \
                 Approve card: `{command}`",
                row.choice_id
            );
            if self.promote_queued_row(&storage, session_id, &row, notice).await {
                released += 1;
            }
        }
        released
    }

    /// Relaunch / respawn re-arm (0080): a queued row's summons lived in the
    /// ring that just died. Re-send one per queued row's reviewer so the queue
    /// is never stranded on a restart — the in-memory design this replaced
    /// would have left the executor halted, waiting for a card that never comes.
    pub(crate) async fn rearm_queued_outward_for(&self, session_id: &str) {
        let Some(storage) = self.storage.lock().await.clone() else { return };
        let Ok(queued) = storage.queued_gates_for_session(session_id).await else { return };
        let mut summoned: Vec<i64> = Vec::new();
        for row in queued {
            if let Some(reviewer) = self.queued_reviewer(session_id, &row.agent).await {
                if !summoned.contains(&reviewer.id) {
                    summoned.push(reviewer.id);
                    self.summon_participant(session_id, reviewer.id).await;
                }
            }
        }
    }

    /// Close-time sweep for queued outward publishes (0080): say which ones
    /// died with the session, then the caller's `withdraw_pending_tray_for_session`
    /// (widened to `queued`) retires the rows. Silent loss is the one outcome
    /// the durable design exists to prevent.
    pub async fn announce_queued_dropped_at_close(&self, session_id: &str) {
        let Some(storage) = self.storage.lock().await.clone() else { return };
        let Ok(queued) = storage.queued_gates_for_session(session_id).await else { return };
        for row in queued {
            let _ = crate::core::post_system_notice(
                &storage,
                Some(self),
                session_id,
                crate::storage::MessageKind::SystemNotice,
                format!(
                    "Outward publish {} dropped at close — it was queued for review and never \
                     parked: `{}`",
                    row.choice_id,
                    row.command_text.unwrap_or_default()
                ),
                None,
            )
            .await;
        }
    }
}

fn parked_gate_text(
    gate_id: &str,
    command: &str,
    existing: bool,
    note: Option<&str>,
) -> String {
    let lead = if existing {
        "action_gate: an identical command is ALREADY parked for approval"
    } else {
        "action_gate: parked for the user's approval"
    };
    // The outward-review note rides the ack LOUDLY (batch 2 C): either a SKIP
    // note (a solo roster or an overridden-down reviewer parks without the
    // precondition, and the agent must see that the guard was not watching)
    // or a COVERAGE note naming the rows the reviewer already received.
    let note_line = note.map(|n| format!("\n{n}")).unwrap_or_default();
    format!(
        "{lead} (gate_id: {gate_id}).{note_line}\n\
         `{command}` runs when the user approves; its output arrives as an \
         out-of-band message. On reject you get a rejection notice instead. Do \
         NOT re-issue the command or assume it ran — call gate_status(\"{gate_id}\") \
         if you need the current state before continuing."
    )
}

/// Does this finding withdraw this queued publish at settlement? Only an OPEN
/// BLOCKING finding vetoes (one already dispositioned has been answered). A
/// finding that names a gate (0083) vetoes exactly that gate; `"none"` vetoes
/// nothing; one that names no gate keeps the fail-closed default — it
/// withdraws every publish queued before it was filed (feedback #42/#43).
fn vetoes_queued_gate(
    f: &crate::storage::Finding,
    row: &crate::storage::SessionTrayEntry,
) -> bool {
    if f.severity != "blocking" || f.status != "open" {
        return false;
    }
    match f.gate_id.as_deref() {
        None => f.created_at > row.asked_at,
        Some("none") => false,
        Some(gate) => gate == row.choice_id,
    }
}

/// "row #12" / "rows #12, #15" — the message ids a coverage hit cites.
fn covered_rows_text(rows: &[i64]) -> String {
    let ids = rows.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ");
    if rows.len() == 1 {
        format!("row {ids}")
    } else {
        format!("rows {ids}")
    }
}

/// The agent-facing text for whatever the park did.
fn park_outcome_text(outcome: &ParkOutcome, command: &str) -> String {
    match outcome {
        ParkOutcome::Parked { gate_id, existing, note } => {
            parked_gate_text(gate_id, command, *existing, note.as_deref())
        }
        ParkOutcome::Queued { gate_id, existing } => queued_gate_text(gate_id, command, *existing),
    }
}

/// Format combined output roughly the way the agent would have seen it from its
/// own Bash call, plus an exit-code footer so a non-zero result is unambiguous.
///
/// The footer does NOT repeat the command (round 7, A5): in-band the agent
/// issued the command it is reading the result of, and out-of-band the
/// tray-answer row names it once on its verdict line — the old footer put a
/// 550-char gated command into the user-voice channel a second time.
fn format_command_output(out: &tool_gate::CommandOutput) -> String {
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
    // Exit code + payload size + the executing shell, always (1.0.0 Batch 8
    // B3, from the Batch-0 evidence): the historic false green was a script
    // authored under bash semantics running under the gate's zsh — every
    // result now says what ran it and how big the answer was, so
    // "suspiciously empty but exit 0" is visible at a glance instead of a
    // forensic finding. The shell is resolved the same way the runner resolves
    // it (`gate_shell`), so the label cannot drift from reality — but printed
    // as a BASENAME (`gate_shell_label`): on Windows the resolved path is
    // absolute, and Scoop / `%LOCALAPPDATA%\Programs` layouts carry the
    // username in it, while this string lands in archived transcripts.
    let bytes = out.stdout.len() + out.stderr.len();
    s.push_str(&format!(
        "[action_gate → exit {} · {} output byte{} · shell {}]",
        out.code,
        bytes,
        if bytes == 1 { "" } else { "s" },
        tool_gate::gate_shell_label(),
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render an ABSOLUTE fixture path in POSIX form for a gated command.
    ///
    /// Real agent commands arrive forward-slashed, and on Windows the gate runs
    /// them through Git-for-Windows' MSYS `sh`, which mangles native `C:\…`
    /// argument paths — so a fixture built with `Path::display()` hands `touch`
    /// something it cannot create.
    ///
    /// **Do NOT unify this with `util::rel_key`.** They look like the same
    /// operation and are not: `rel_key` joins `Component::Normal` because it
    /// builds a RELATIVE database key, and running that over an absolute
    /// tempdir path would silently drop the `C:\` prefix and hand `touch` a
    /// relative path. Different jobs, different correct implementations — a
    /// plain `replace` is right *here* precisely because the input is absolute.
    fn posix_path(p: &std::path::Path) -> String {
        p.display().to_string().replace('\\', "/")
    }
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
            .action_gate("s1".into(), "hands".into(), "echo hi-there".into(), false)
            .await
            .unwrap();
        assert!(out.contains("hi-there"), "out: {out}");
        assert!(out.contains("exit 0"), "out: {out}");
    }

    /// Round 12 (EYES F19): `require_approval` parks whatever the keyword
    /// list says — the agent's own "this must not run unapproved" (the prod
    /// rule). With NO keyword configured an unmatched command would otherwise
    /// run outright; here it parks, latches the ring and executes nothing
    /// until the user's Approve.
    #[tokio::test]
    async fn require_approval_parks_with_no_keyword_and_runs_nothing() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let bridge = bridge_with(data.path(), &[], "s1", repo.path()).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        bridge.register_session_sequencer("s1".into(), tx).await;
        let marker = repo.path().join("ran.txt");
        let cmd = format!("touch {}", posix_path(&marker));
        let out = bridge
            .action_gate("s1".into(), "hands".into(), cmd.clone(), true)
            .await
            .unwrap();
        assert!(out.contains("PARKED") || out.contains("parked"), "parked, not run: {out}");
        assert!(!marker.exists(), "nothing executed before the user's Approve");
        assert!(
            matches!(rx.try_recv(), Ok(crate::core::sequencer::SequencerCommand::GateOpened { .. })),
            "a forced park is a real gate — the ring latches"
        );
        // The same command without the flag (no keyword) runs at once — the
        // default the Tool-Gate route relies on is unchanged.
        let out = bridge
            .action_gate("s1".into(), "hands".into(), cmd, false)
            .await
            .unwrap();
        assert!(out.contains("exit 0"), "{out}");
        assert!(marker.exists());
    }

    #[tokio::test]
    async fn no_match_executes() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let bridge = bridge_with(data.path(), &[], "s1", repo.path()).await;
        let out = bridge
            .action_gate("s1".into(), "hands".into(), "echo loose".into(), false)
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
            .action_gate("s-norepo".into(), "hands".into(), "echo x".into(), false)
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
        let cmd = format!("touch {}", posix_path(&marker));
        let bridge = bridge_with(
            data.path(),
            &[gk("touch", GateMode::Gate)],
            "s1",
            repo.path(),
        )
        .await;
        // Park contract: the call returns immediately with a gate_id.
        let parked = bridge
            .action_gate("s1".into(), "hands".into(), cmd, false)
            .await
            .unwrap();
        assert!(parked.contains("parked"), "got: {parked}");
        let cid = parked
            .split("gate_id: ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap()
            .to_string();
        bridge.resolve_choice(&cid, "Reject — not on a Friday".into()).await.unwrap();
        assert!(!marker.exists(), "rejected command must NOT have run");
        let status = bridge.gate_status(&cid).await.unwrap();
        assert!(status.starts_with("rejected"), "got: {status}");

        // F6 + E6: the card names the keyword the gate matched and where, and a
        // re-park of the SAME command after a reject carries that verdict — a
        // fresh card by design (coverage is content-keyed), but never a card
        // that reads as a first ask.
        let storage = bridge.storage.lock().await.clone().unwrap();
        let first = storage.get_tray_entry(&cid).await.unwrap().unwrap();
        assert!(first.prompt.contains("matched Tool-Gate keyword `touch` at col 0"), "got: {}", first.prompt);
        assert!(!first.prompt.contains("REJECTED"), "a first ask carries no prior verdict");
        let cmd = format!("touch {}", posix_path(&marker));
        let again = bridge
            .action_gate("s1".into(), "hands".into(), cmd, false)
            .await
            .unwrap();
        let cid2 = again
            .split("gate_id: ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap()
            .to_string();
        assert_ne!(cid2, cid, "a post-reject re-fire parks fresh");
        let second = storage.get_tray_entry(&cid2).await.unwrap().unwrap();
        assert!(
            second.prompt.contains("You REJECTED this identical command at")
                && second.prompt.contains("Reject — not on a Friday"),
            "the re-park card carries the prior verdict: {}",
            second.prompt
        );
    }

    /// **Only the LISTED Approve runs a parked command** (round 8, R3). The
    /// menu is exactly Approve/Reject; a typed answer — even one that starts
    /// with "approve" — is the user saying something ELSE, and it is carried to
    /// the agent as words, never executed as a yes. Before this the shared
    /// prefix map ran the original command on `"approve but dry-run first"`
    /// while the tray-answer body told the agent to honor the words.
    /// Kill-tested: route the gate back through `outcome_from_picked` and the
    /// first row below runs the command.
    #[tokio::test]
    async fn a_typed_approval_on_a_gate_is_carried_as_words_and_never_executes() {
        for typed in [
            "approve but dry-run first",
            "approved",
            "approved?",
            "ok",
            "yes",
            "ok, but use --dry-run",
            "sure",
        ] {
            let data = tempdir().unwrap();
            let repo = tempdir().unwrap();
            let marker = repo.path().join("ran.txt");
            let cmd = format!("touch {}", posix_path(&marker));
            let bridge = bridge_with(
                data.path(),
                &[gk("touch", GateMode::Gate)],
                "s1",
                repo.path(),
            )
            .await;
            let parked = bridge
                .action_gate("s1".into(), "hands".into(), cmd, false)
                .await
                .unwrap();
            let cid = parked
                .split("gate_id: ")
                .nth(1)
                .and_then(|s| s.split(')').next())
                .unwrap()
                .to_string();
            let outcome = bridge.resolve_choice(&cid, typed.into()).await.unwrap();
            assert!(
                !marker.exists(),
                "typed pick {typed:?} must NOT run the parked command"
            );
            match outcome {
                ResolveOutcome::DeliveredOutOfBand { body, .. } => {
                    assert!(
                        body.contains(&format!("rejected ({typed})")),
                        "the verdict names the words as a rejection: {body}"
                    );
                    assert!(
                        body.contains("honor the words, not the menu"),
                        "and the words are carried to the agent: {body}"
                    );
                    assert!(!body.contains("Output:"), "nothing ran, so no output block: {body}");
                }
                other => panic!("expected OOB delivery, got {other:?}"),
            }
            let status = bridge.gate_status(&cid).await.unwrap();
            assert!(
                status.starts_with("rejected"),
                "gate_status agrees nothing ran for {typed:?}: {status}"
            );
        }
    }

    /// **The row says what it is at insert** (round 8, T2-2): a parked
    /// action_gate command lands as `kind = 'approval'`, so readers no longer
    /// re-derive gate-ness from the options string. Kill-tested: park with
    /// `QuestionKind::Choice` again and this reads "choice".
    #[tokio::test]
    async fn a_parked_gate_is_written_as_an_approval_row() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let bridge = bridge_with(
            data.path(),
            &[gk("touch", GateMode::Gate)],
            "s1",
            repo.path(),
        )
        .await;
        let parked = bridge
            .action_gate("s1".into(), "hands".into(), "touch nothing".into(), false)
            .await
            .unwrap();
        let cid = parked
            .split("gate_id: ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap()
            .to_string();
        let storage = bridge.storage.lock().await.clone().expect("test bridge has storage");
        let row = storage.get_tray_entry(&cid).await.unwrap().unwrap();
        assert_eq!(row.kind, "approval", "a gate row names itself");
        assert_eq!(
            row.options_json.as_deref(),
            Some(crate::storage::GATE_OPTIONS_JSON),
            "and still carries the menu (the fallback readers key on it)"
        );
    }

    #[tokio::test]
    async fn gate_approve_executes_the_command() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let marker = repo.path().join("ran.txt");
        let cmd = format!("touch {}", posix_path(&marker));
        let bridge = bridge_with(
            data.path(),
            &[gk("touch", GateMode::Gate)],
            "s1",
            repo.path(),
        )
        .await;
        // Park contract: the call returns immediately with a gate_id.
        let parked = bridge
            .action_gate("s1".into(), "hands".into(), cmd, false)
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
            ResolveOutcome::DeliveredOutOfBand { body, .. } => {
                assert!(body.contains("exit 0"), "approve delivers output OOB: {body}")
            }
            other => panic!("expected OOB delivery, got {other:?}"),
        }
        assert!(marker.exists(), "approved command should have run");
        let status = bridge.gate_status(&cid).await.unwrap();
        assert!(status.starts_with("approved"), "got: {status}");
    }

    /// **`gate_status` answers only for the caller's own session** (round 11).
    /// The tool is ungated by design; the scoping is what keeps one session's
    /// gate — its command, the user's answer — out of another's reach, and a
    /// foreign gate reads exactly like a missing one.
    #[tokio::test]
    async fn gate_status_does_not_answer_for_another_sessions_gate() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage.create_session("s2", "t", None).await.unwrap();
        storage
            .insert_tray_entry(
                "s2",
                "gate-in-s2",
                "hands",
                crate::storage::QuestionKind::Approval,
                "Run gated command?",
                Some(&["Approve".to_string(), "Reject".to_string()]),
                None,
                Some("echo secret"),
            )
            .await
            .unwrap();
        let foreign = bridge.gate_status_for("gate-in-s2", Some("s1")).await.unwrap();
        assert_eq!(foreign, "gate_status: no gate with id gate-in-s2");
        assert!(!foreign.contains("echo secret"));
        let own = bridge.gate_status_for("gate-in-s2", Some("s2")).await.unwrap();
        assert!(own.starts_with("pending"), "the owning session reads it: {own}");
        assert!(own.contains("echo secret"));
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
                "hands".into(),
                "Query prod?".into(),
                vec!["Approve".into(), "Deny".into()],
                ApprovalContext {
                    kind: ViolationKind::PerAction,
                    action: "bq query --project_id=prod ...".into(),
                    detail: None,
                    command: None,
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
        let cmd = format!("touch {}", posix_path(&marker));
        let bridge = bridge_with(
            data.path(),
            &[gk("touch", GateMode::Gate)],
            "s1",
            repo.path(),
        )
        .await;
        let mut sub = bridge.subscribe();
        let b2 = Arc::clone(&bridge);
        let call = tokio::spawn(async move { b2.action_gate("s1".into(), "hands".into(), cmd, false).await });
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
            ResolveOutcome::DeliveredOutOfBand { body, .. } => assert!(
                body.contains("exit 0"),
                "OOB body must carry the executed command output: {body}"
            ),
            other => panic!("expected DeliveredOutOfBand, got {other:?}"),
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
        let cmd = format!("touch {}", posix_path(&marker));
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
                "hands",
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
            ResolveOutcome::DeliveredOutOfBand { body, .. } => assert!(
                body.contains("exit 0"),
                "durable row must execute + carry output via OOB: {body}"
            ),
            other => panic!("expected DeliveredOutOfBand, got {other:?}"),
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
        let cmd = format!("touch {}", posix_path(&marker));
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
                "hands",
                crate::storage::QuestionKind::Choice,
                "Run?",
                Some(&opts),
                None,
                Some(&cmd),
            )
            .await
            .unwrap();

        let body_of = |o| match o {
            ResolveOutcome::DeliveredOutOfBand { body, .. } => body,
            other => panic!("expected DeliveredOutOfBand, got {other:?}"),
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
        assert!(first.contains("Output:"), "first resolve must execute: {first}");
        assert!(
            !second.contains("Output:"),
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
        let cmd = format!("touch {}", posix_path(&marker));
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
                "hands",
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
                "hands",
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
            ResolveOutcome::DeliveredOutOfBand { body, .. } => {
                assert!(body.contains("exit 0"), "confirmed run carries output: {body}")
            }
            other => panic!("expected DeliveredOutOfBand, got {other:?}"),
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
        let cmd = format!("touch {}", posix_path(&marker)); // matches no keyword

        let (gate_id, existing, _note) =
            parked(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
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
        let (dup_id, dup_existing, _note) =
            parked(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        assert_eq!(dup_id, gate_id);
        assert!(dup_existing);

        // Approval still executes at resolve time, through the normal path.
        bridge.resolve_choice(&gate_id, "Approve".into()).await.unwrap();
        assert!(marker.exists(), "approve runs the parked command");
    }

    // ---- Outward-review precondition (batch 2 C, 2026-08-27) --------------

    #[test]
    fn the_outward_classifier_is_segment_anchored_both_ways() {
        assert!(super::outward_command("gh issue edit 541 --body-file /tmp/x.md"));
        assert!(super::outward_command("true && gh pr create --base main"));
        assert!(super::outward_command("curl -X POST https://x"));
        assert!(!super::outward_command("echo \"gh issue edit\""));
        assert!(!super::outward_command("git push origin main"));
        assert!(!super::outward_command("cargo test && echo done"));
    }

    /// Roster + reviewer registry + a body file in the repo, shared by the
    /// coverage tests. Returns (bridge, storage, eyes id, body path, body).
    async fn outward_fixture(
        data: &tempfile::TempDir,
        repo: &tempfile::TempDir,
    ) -> (
        std::sync::Arc<SignalingBridge>,
        crate::storage::Storage,
        i64,
        String,
        String,
    ) {
        let bridge = bridge_with(data.path(), &[], "s1", repo.path()).await;
        let storage = bridge.storage.lock().await.clone().unwrap();
        storage
            .ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS)
            .await
            .unwrap();
        bridge.register_session_reviewers("s1".to_string(), vec!["eyes".to_string()]);
        let eyes = storage.participant_by_slug("s1", "eyes").await.unwrap().unwrap().id;
        let body =
            "The deletion rule is conditional on Martin's answer.\nSecond line.".to_string();
        let path = repo.path().join("draft.md");
        std::fs::write(&path, &body).unwrap();
        (bridge, storage, eyes, path.to_string_lossy().to_string(), body)
    }

    /// The message row a queued gate points at.
    async fn message_by_id(storage: &crate::storage::Storage, id: i64) -> crate::storage::Message {
        storage
            .messages_for_session("s1", Some(id - 1))
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.id == id)
            .expect("the body row exists")
    }

    /// Unwrap a park that must be the user's card.
    fn parked(o: ParkOutcome) -> (String, bool, Option<String>) {
        match o {
            ParkOutcome::Parked { gate_id, existing, note } => (gate_id, existing, note),
            ParkOutcome::Queued { gate_id, .. } => panic!("expected a park, got queued {gate_id}"),
        }
    }

    /// Unwrap a park that must have been QUEUED for the reviewer (0080).
    fn queued(o: ParkOutcome) -> (String, bool) {
        match o {
            ParkOutcome::Queued { gate_id, existing } => (gate_id, existing),
            ParkOutcome::Parked { gate_id, .. } => panic!("expected queued, got a park {gate_id}"),
        }
    }

    /// A ring channel the bridge can summon through; returns the receiver.
    async fn ring_for(
        bridge: &std::sync::Arc<SignalingBridge>,
    ) -> tokio::sync::mpsc::Receiver<crate::core::sequencer::SequencerCommand> {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        bridge.register_session_sequencer("s1".into(), tx).await;
        rx
    }

    /// Drain whatever the ring has received so far.
    fn drain(rx: &mut tokio::sync::mpsc::Receiver<crate::core::sequencer::SequencerCommand>)
        -> Vec<crate::core::sequencer::SequencerCommand> {
        let mut out = Vec::new();
        while let Ok(cmd) = rx.try_recv() {
            out.push(cmd);
        }
        out
    }

    fn summons_in(cmds: &[crate::core::sequencer::SequencerCommand]) -> Vec<i64> {
        cmds.iter()
            .filter_map(|c| match c {
                crate::core::sequencer::SequencerCommand::Summon { participant_id } => Some(*participant_id),
                _ => None,
            })
            .collect()
    }

    fn gate_opened_in(cmds: &[crate::core::sequencer::SequencerCommand]) -> Vec<String> {
        cmds.iter()
            .filter_map(|c| match c {
                crate::core::sequencer::SequencerCommand::GateOpened { choice_id } => Some(choice_id.clone()),
                _ => None,
            })
            .collect()
    }

    /// 0080: an outward body the reviewer has not read is QUEUED, not refused —
    /// one durable `queued` row pointing at a posted body row, the reviewer
    /// summoned, NO gate latch, invisible to the tray read, visible to the
    /// watchdog's pending check. Replaces the two-turn ritual (50 timeline +
    /// 33 coverage refusals in the seven client-project sessions of week 35).
    #[tokio::test]
    async fn an_outward_body_never_delivered_is_queued_with_its_body_posted() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, body) = outward_fixture(&data, &repo).await;
        let mut ring = ring_for(&bridge).await;
        drain(&mut ring);
        let cmd = format!("gh issue edit 5 --body-file {path}");

        let (gate_id, existing) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        assert!(!existing);
        let row = storage.get_tray_entry(&gate_id).await.unwrap().unwrap();
        assert_eq!(row.status, "queued");
        assert_eq!(row.kind, "approval");
        assert_eq!(row.command_text.as_deref(), Some(cmd.as_str()));
        // The body row exists, is a system row, carries the body verbatim, and
        // the tray row points at it.
        let body_row_id = row.body_row_id.expect("queued row points at its body row");
        let posted = message_by_id(&storage, body_row_id).await;
        assert_eq!(posted.kind, "system_notice");
        assert!(posted.content.contains(&body), "body row must carry the body: {}", posted.content);
        assert!(posted.content.contains(&gate_id));
        // The reviewer was summoned — and nothing latched.
        let cmds = drain(&mut ring);
        assert_eq!(summons_in(&cmds), vec![eyes], "exactly one summons, for the reviewer");
        assert!(gate_opened_in(&cmds).is_empty(), "a queued row must NOT latch the ring");
        // Invisible to the renderable read, invisible to the latch seed,
        // visible to the watchdog's pending check (P8 items 1 and 4).
        assert!(storage.tray_entries_for_session("s1").await.unwrap().iter().all(|e| e.choice_id != gate_id));
        assert!(storage.pending_gate_ids("s1").await.unwrap().is_empty());
        assert!(storage.has_pending_tray("s1").await.unwrap());
        assert!(bridge.gate_status(&gate_id).await.unwrap().starts_with("queued"));
        // And it cannot be answered while queued: the flip is pending-only.
        assert_eq!(storage.answer_tray_entry(&gate_id, "Approve").await.unwrap(), 0);
    }

    /// E1's guard: delivery alone never settles. The body row DELIVERED but
    /// not READ (cursor short of it) leaves the row queued; once the reviewer's
    /// cursor passes it, settlement promotes it to the user's card with the
    /// latch and the awaiting flag a fresh park has.
    #[tokio::test]
    async fn a_queued_publish_parks_only_after_the_reviewer_reads_the_body() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let mut ring = ring_for(&bridge).await;
        let cmd = format!("gh issue edit 5 --body-file {path}");
        let (gate_id, _) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        let body_row_id = storage.get_tray_entry(&gate_id).await.unwrap().unwrap().body_row_id.unwrap();
        drain(&mut ring);

        // The reviewer finishes a turn WITHOUT having read the body row.
        bridge.settle_queued_outward("s1", eyes).await;
        assert_eq!(storage.get_tray_entry(&gate_id).await.unwrap().unwrap().status, "queued");
        assert!(gate_opened_in(&drain(&mut ring)).is_empty());

        // Now the reviewer's cursor passes the body row.
        storage.commit_delivery(eyes, &[(body_row_id, None)]).await.unwrap();
        bridge.settle_queued_outward("s1", eyes).await;
        let row = storage.get_tray_entry(&gate_id).await.unwrap().unwrap();
        assert_eq!(row.status, "pending", "read → promoted");
        assert_eq!(gate_opened_in(&drain(&mut ring)), vec![gate_id.clone()], "promotion latches the ring");
        assert_eq!(storage.pending_gate_ids("s1").await.unwrap(), vec![gate_id.clone()]);
        assert!(storage.tray_entries_for_session("s1").await.unwrap().iter().any(|e| e.choice_id == gate_id));
        assert!(bridge.gate_status(&gate_id).await.unwrap().starts_with("pending"));
        // Idempotent: a second settlement finds nothing to flip.
        bridge.settle_queued_outward("s1", eyes).await;
        assert!(gate_opened_in(&drain(&mut ring)).is_empty());
        // The card resolves like any park (Reject: nothing to execute).
        bridge.resolve_choice(&gate_id, "Reject".into()).await.unwrap();
        assert_eq!(storage.get_tray_entry(&gate_id).await.unwrap().unwrap().status, "answered");
    }

    #[tokio::test]
    async fn a_blocking_finding_after_queueing_withdraws_the_publish() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let mut ring = ring_for(&bridge).await;
        let cmd = format!("gh issue edit 5 --body-file {path}");
        let (gate_id, _) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        let body_row_id = storage.get_tray_entry(&gate_id).await.unwrap().unwrap().body_row_id.unwrap();
        storage.commit_delivery(eyes, &[(body_row_id, None)]).await.unwrap();
        // The reviewer read it and filed a BLOCKING finding.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        storage
            .insert_finding("s1", "f-veto", "eyes", crate::storage::FindingSeverity::Blocking, "wrong claim in the body", None)
            .await
            .unwrap();
        drain(&mut ring);
        bridge.settle_queued_outward("s1", eyes).await;
        let row = storage.get_tray_entry(&gate_id).await.unwrap().unwrap();
        assert_eq!(row.status, "withdrawn");
        assert!(row.picked_option.as_deref().unwrap_or("").contains("blocking finding"));
        assert!(gate_opened_in(&drain(&mut ring)).is_empty(), "a withdrawn row latches nothing");
        let status = bridge.gate_status(&gate_id).await.unwrap();
        assert!(status.starts_with("withdrawn"), "got: {status}");
        assert!(status.contains("did not run"));
    }

    /// Two publishes queued in one turn, both bodies read by the reviewer —
    /// the #42/#43 shape. Returns `(gate_a, gate_b, cmd_a)`.
    async fn two_queued_and_read(
        bridge: &std::sync::Arc<SignalingBridge>,
        storage: &crate::storage::Storage,
        eyes: i64,
        repo: &tempfile::TempDir,
        path_a: &str,
    ) -> (String, String, String) {
        let path_b = repo.path().join("comment.md");
        std::fs::write(&path_b, "An answer to the stakeholder.\n").unwrap();
        let cmd_a = format!("gh issue create --title t --body-file {path_a}");
        let cmd_b = format!("gh issue comment 749 --body-file {}", path_b.display());
        let (gate_a, _) = queued(bridge.park_gated_command("s1", "hands", &cmd_a).await.unwrap());
        let (gate_b, _) = queued(bridge.park_gated_command("s1", "hands", &cmd_b).await.unwrap());
        for g in [&gate_a, &gate_b] {
            let row = storage.get_tray_entry(g).await.unwrap().unwrap().body_row_id.unwrap();
            storage.commit_delivery(eyes, &[(row, None)]).await.unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        (gate_a, gate_b, cmd_a)
    }

    async fn status_of(storage: &crate::storage::Storage, gate: &str) -> String {
        storage.get_tray_entry(gate).await.unwrap().unwrap().status
    }

    /// #42: a finding that NAMES one queued publish withdraws that one only —
    /// by its full id or a unique 8-char prefix.
    #[tokio::test]
    async fn a_finding_naming_one_gate_withdraws_only_that_gate() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let (gate_a, gate_b, _) = two_queued_and_read(&bridge, &storage, eyes, &repo, &path).await;
        let uid = bridge
            .eyes_flag_for_gate(
                "s1".into(),
                "eyes".into(),
                crate::storage::FindingSeverity::Blocking,
                "date range attributed to the wrong rows".into(),
                None,
                Some(gate_a[..8].to_string()),
            )
            .await
            .unwrap();
        let f = storage.get_finding(&uid).await.unwrap().unwrap();
        assert_eq!(f.gate_id.as_deref(), Some(gate_a.as_str()), "a prefix resolves to the full id");
        bridge.settle_queued_outward("s1", eyes).await;
        assert_eq!(status_of(&storage, &gate_a).await, "withdrawn");
        assert_eq!(status_of(&storage, &gate_b).await, "pending", "the other publish parks");
        let rows = storage.recent_row_bodies_upto("s1", i64::MAX, 20).await.unwrap();
        assert!(
            rows.iter().any(|r| r.contains(&gate_a) && r.contains("names this publish")),
            "the withdrawal says it was targeted: {rows:?}"
        );
    }

    /// #43: a blocking finding about something else entirely, filed with
    /// `gate_id: "none"`, leaves every queued publish to park.
    #[tokio::test]
    async fn a_finding_with_gate_none_withdraws_no_queued_publish() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let (gate_a, gate_b, _) = two_queued_and_read(&bridge, &storage, eyes, &repo, &path).await;
        bridge
            .eyes_flag_for_gate(
                "s1".into(),
                "eyes".into(),
                crate::storage::FindingSeverity::Blocking,
                "data loss in bin/export".into(),
                None,
                Some("none".into()),
            )
            .await
            .unwrap();
        bridge.settle_queued_outward("s1", eyes).await;
        assert_eq!(status_of(&storage, &gate_a).await, "pending");
        assert_eq!(status_of(&storage, &gate_b).await, "pending");
        // The finding still gates commits — `none` scopes the PUBLISH veto only.
        assert_eq!(storage.count_open_blocking_findings("s1").await.unwrap(), 1);
    }

    /// The fail-closed default stays: no `gate_id` withdraws every queued
    /// publish, even when the summary MENTIONS one gate (plan review M2 —
    /// prose never narrows the veto).
    #[tokio::test]
    async fn an_untargeted_finding_withdraws_every_queued_publish_even_if_it_names_one() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let (gate_a, gate_b, _) = two_queued_and_read(&bridge, &storage, eyes, &repo, &path).await;
        bridge
            .eyes_flag_for_gate(
                "s1".into(),
                "eyes".into(),
                crate::storage::FindingSeverity::Blocking,
                format!("gate {} has a wrong date", &gate_a[..8]),
                Some(format!("gate {}", &gate_a[..8])),
                None,
            )
            .await
            .unwrap();
        bridge.settle_queued_outward("s1", eyes).await;
        assert_eq!(status_of(&storage, &gate_a).await, "withdrawn");
        assert_eq!(status_of(&storage, &gate_b).await, "withdrawn");
        let rows = storage.recent_row_bodies_upto("s1", i64::MAX, 20).await.unwrap();
        assert!(
            rows.iter().any(|r| r.contains(&gate_b) && r.contains("no need to disposition")),
            "an untargeted withdrawal must not push the executor to clear the finding: {rows:?}"
        );
    }

    /// Only an OPEN finding vetoes: one already dispositioned was answered.
    #[tokio::test]
    async fn a_dispositioned_finding_no_longer_vetoes() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let (gate_a, gate_b, _) = two_queued_and_read(&bridge, &storage, eyes, &repo, &path).await;
        storage
            .insert_finding("s1", "f-old", "eyes", crate::storage::FindingSeverity::Blocking, "x", None)
            .await
            .unwrap();
        storage
            .disposition_finding("s1", "f-old", crate::storage::FindingStatus::Fixed, Some("abc"), "hands")
            .await
            .unwrap();
        bridge.settle_queued_outward("s1", eyes).await;
        assert_eq!(status_of(&storage, &gate_a).await, "pending");
        assert_eq!(status_of(&storage, &gate_b).await, "pending");
    }

    /// A gate id that is not QUEUED (mistyped, or already parked for the user)
    /// is an error naming what is queued — never a veto that silently missed.
    #[tokio::test]
    async fn a_gate_id_that_is_not_queued_is_an_error() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let (gate_a, _gate_b, _) = two_queued_and_read(&bridge, &storage, eyes, &repo, &path).await;
        let mistyped = gate_a[..8].replace(&gate_a[..1], "z");
        for bad in ["deadbeef-0000", "abc", mistyped.as_str()] {
            let err = bridge
                .eyes_flag_for_gate(
                    "s1".into(),
                    "eyes".into(),
                    crate::storage::FindingSeverity::Blocking,
                    format!("bad target {bad}"),
                    None,
                    Some(bad.to_string()),
                )
                .await
                .expect_err("an unknown gate must not file silently");
            assert!(err.to_string().contains(&gate_a[..8]), "the error lists what is queued: {err}");
        }
        assert_eq!(storage.count_open_blocking_findings("s1").await.unwrap(), 0);
    }

    /// The veto cannot be undone by re-issuing the vetoed command unchanged:
    /// its body row was delivered when it first queued, so coverage alone
    /// would park it on the user's card. It queues for a FRESH review.
    #[tokio::test]
    async fn a_withdrawn_publish_reissued_unchanged_queues_for_a_fresh_review() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let (gate_a, _gate_b, cmd_a) = two_queued_and_read(&bridge, &storage, eyes, &repo, &path).await;
        let finding = bridge
            .eyes_flag_for_gate(
                "s1".into(),
                "eyes".into(),
                crate::storage::FindingSeverity::Blocking,
                "wrong claim".into(),
                None,
                Some(gate_a.clone()),
            )
            .await
            .unwrap();
        bridge.settle_queued_outward("s1", eyes).await;
        assert_eq!(status_of(&storage, &gate_a).await, "withdrawn");
        // EYES 891e0eb1: while the TARGETED finding is open, the same command
        // is refused outright — a veto that holds once is one a re-issue undoes.
        let err = bridge
            .park_gated_command("s1", "hands", &cmd_a)
            .await
            .expect_err("an open targeted veto holds its content");
        assert!(err.to_string().contains("vetoed this exact publish"), "got: {err}");
        storage
            .disposition_finding("s1", &finding, crate::storage::FindingStatus::Fixed, Some("body fixed"), "hands")
            .await
            .unwrap();
        let (again, existing) = queued(bridge.park_gated_command("s1", "hands", &cmd_a).await.unwrap());
        assert!(!existing);
        assert_ne!(again, gate_a);
        // Read again without a new finding → it may park now: the old
        // finding named the OLD gate, and each queued publish gets its own call.
        let row = storage.get_tray_entry(&again).await.unwrap().unwrap().body_row_id.unwrap();
        storage.commit_delivery(eyes, &[(row, None)]).await.unwrap();
        bridge.settle_queued_outward("s1", eyes).await;
        assert_eq!(status_of(&storage, &again).await, "pending");
    }

    #[tokio::test]
    async fn settlement_ignores_a_participant_who_is_not_the_reviewer() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let hands = storage.participant_by_slug("s1", "hands").await.unwrap().unwrap().id;
        let _ring = ring_for(&bridge).await;
        let cmd = format!("gh issue edit 5 --body-file {path}");
        let (gate_id, _) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        let body_row_id = storage.get_tray_entry(&gate_id).await.unwrap().unwrap().body_row_id.unwrap();
        // Even with BOTH cursors past the body, only the reviewer's turn settles.
        storage.commit_delivery(hands, &[(body_row_id, None)]).await.unwrap();
        storage.commit_delivery(eyes, &[(body_row_id, None)]).await.unwrap();
        bridge.settle_queued_outward("s1", hands).await;
        assert_eq!(storage.get_tray_entry(&gate_id).await.unwrap().unwrap().status, "queued");
        bridge.settle_queued_outward("s1", eyes).await;
        assert_eq!(storage.get_tray_entry(&gate_id).await.unwrap().unwrap().status, "pending");
    }

    /// P8 item 3: an identical re-issue while queued returns the same row and
    /// summons nobody twice.
    #[tokio::test]
    async fn an_identical_queued_command_dedupes_and_summons_once() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let mut ring = ring_for(&bridge).await;
        drain(&mut ring);
        let cmd = format!("gh issue edit 5 --body-file {path}");
        let (first, e1) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        let (second, e2) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        assert_eq!(first, second);
        assert!(!e1);
        assert!(e2, "the second is a dedupe hit");
        assert_eq!(summons_in(&drain(&mut ring)), vec![eyes], "one summons, not two");
        assert_eq!(storage.queued_gates_for_session("s1").await.unwrap().len(), 1);
    }

    /// P1's strand path, closed: a queued row with no live ring (relaunch) is
    /// re-summoned the moment a ring registers.
    #[tokio::test]
    async fn queued_rows_re_summon_when_the_ring_registers() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        // No ring registered yet: the queue lands, the summons has nowhere to go.
        let cmd = format!("gh issue edit 5 --body-file {path}");
        let (gate_id, _) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        assert_eq!(storage.get_tray_entry(&gate_id).await.unwrap().unwrap().status, "queued");
        // The relaunch: a ring comes up → the re-arm summons the reviewer.
        let mut ring = ring_for(&bridge).await;
        assert_eq!(summons_in(&drain(&mut ring)), vec![eyes]);
    }

    /// Close: a queued publish is named in the channel, then retired by the
    /// widened sweep — never a live row on a closed session (P8 item 2).
    #[tokio::test]
    async fn queued_rows_are_named_and_withdrawn_at_close() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, _eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let cmd = format!("gh issue edit 5 --body-file {path}");
        let (gate_id, _) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        bridge.announce_queued_dropped_at_close("s1").await;
        let swept = storage.withdraw_pending_tray_for_session("s1").await.unwrap();
        assert_eq!(swept, 1);
        assert_eq!(storage.get_tray_entry(&gate_id).await.unwrap().unwrap().status, "withdrawn");
        let rows = storage.recent_row_bodies_upto("s1", i64::MAX, 50).await.unwrap();
        assert!(
            rows.iter().any(|r| r.contains("dropped at close") && r.contains(&gate_id)),
            "the drop must be said out loud"
        );
        assert!(!storage.has_pending_tray("s1").await.unwrap());
    }

    #[tokio::test]
    async fn an_outward_body_delivered_to_the_reviewer_parks_and_reparks_after_reject() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, body) = outward_fixture(&data, &repo).await;
        // HANDS posts the draft (any row carrying the full body) and the
        // reviewer is DEALT it — its cursor moves past the row.
        let m = storage
            .post_to_channel(
                "s1",
                "participant",
                Some("hands"),
                "text",
                format!("Draft for review:\n{body}"),
                None,
            )
            .await
            .unwrap();
        storage.commit_delivery(eyes, &[(m.message_id(), None)]).await.unwrap();

        let cmd = format!("gh issue edit 5 --body-file {path}");
        let (gate_id, existing, note) =
            parked(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        assert!(!existing);
        let note = note.expect("a coverage park cites what covered it");
        assert!(
            note.starts_with("coverage:") && note.contains(&format!("#{}", m.message_id())),
            "the note names the covering row, never a skip: {note}"
        );
        // #40: the park on prior review is said in the channel, where the
        // reviewer reads it.
        let rows = storage.recent_row_bodies_upto("s1", i64::MAX, 20).await.unwrap();
        assert!(
            rows.iter().any(|r| r.contains("PRIOR review") && r.contains(&gate_id)),
            "a covered park posts its row: {rows:?}"
        );

        // Reject, then re-park the UNCHANGED content: coverage is keyed on
        // the content, so no re-review is demanded (a later refactor must not
        // turn reject-and-retry into a loop).
        bridge.resolve_choice(&gate_id, "Reject".into()).await.unwrap();
        let (gate2, existing2, _n) =
            parked(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        assert_ne!(gate2, gate_id, "a post-reject re-fire parks fresh");
        assert!(!existing2);
    }

    /// The hole beside feedback #40: the body sat only in the EXECUTOR's own
    /// tool row (a `Write`, a `session_doc_write`), which no peer's backlog
    /// ever delivers. That is not review — it queues.
    #[tokio::test]
    async fn a_body_only_in_the_executors_tool_row_is_not_coverage() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let m = storage
            .post_to_channel(
                "s1",
                "participant",
                Some("hands"),
                "tool_use",
                format!("{{\"file_path\":\"{path}\",\"content\":{}}}", serde_json::to_string(&body).unwrap()),
                None,
            )
            .await
            .unwrap();
        // Even with the reviewer's cursor PAST the row, it was never dealt it.
        storage.commit_delivery(eyes, &[(m.message_id(), None)]).await.unwrap();
        let cmd = format!("gh issue edit 5 --body-file {path}");
        queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
    }

    /// The reviewer's OWN tool row is its own context — it read the file.
    #[tokio::test]
    async fn the_reviewers_own_tool_row_counts_as_coverage() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, body) = outward_fixture(&data, &repo).await;
        let m = storage
            .post_to_channel("s1", "participant", Some("eyes"), "tool_result", body.clone(), None)
            .await
            .unwrap();
        storage.commit_delivery(eyes, &[(m.message_id(), None)]).await.unwrap();
        let cmd = format!("gh issue edit 5 --body-file {path}");
        let (_gate, _existing, note) =
            parked(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        assert!(note.is_some_and(|n| n.contains(&format!("#{}", m.message_id()))));
    }

    /// Delivery clamps a row at `WIRE_BODY_CLAMP_BYTES`: text past the cut
    /// never reached the reviewer, so it cannot cover a publish.
    #[tokio::test]
    async fn a_body_past_the_wire_clamp_is_not_coverage() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let padding = "x".repeat(crate::storage::WIRE_BODY_CLAMP_BYTES + 10);
        let m = storage
            .post_to_channel("s1", "participant", Some("hands"), "text", format!("{padding}\n{body}"), None)
            .await
            .unwrap();
        storage.commit_delivery(eyes, &[(m.message_id(), None)]).await.unwrap();
        let cmd = format!("gh issue edit 5 --body-file {path}");
        queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
    }

    #[tokio::test]
    async fn an_edited_body_after_review_is_queued_again() {
        // The morning's exact shape: review a version, edit the MIDDLE, park.
        // Since 0080 the edited body QUEUES for a fresh read instead of
        // refusing — the reviewer sees the new text, never the old.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let m = storage
            .post_to_channel("s1", "participant", Some("hands"), "text", format!("Draft:\n{body}"), None)
            .await
            .unwrap();
        storage.commit_delivery(eyes, &[(m.message_id(), None)]).await.unwrap();
        let edited = body.replace("conditional", "settled");
        std::fs::write(&path, &edited).unwrap();
        let (gate_id, _) = queued(
            bridge
                .park_gated_command("s1", "hands", &format!("gh issue edit 5 --body-file {path}"))
                .await
                .unwrap(),
        );
        let row = storage.get_tray_entry(&gate_id).await.unwrap().unwrap();
        let posted = message_by_id(&storage, row.body_row_id.unwrap()).await;
        assert!(posted.content.contains(&edited), "the reviewer gets the EDITED body");
    }

    #[tokio::test]
    async fn a_solo_roster_parks_with_the_loud_skip_note() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let bridge = bridge_with(data.path(), &[], "s1", repo.path()).await;
        // No reviewers registered at all.
        let path = repo.path().join("b.md");
        std::fs::write(&path, "solo body").unwrap();
        let (_gid, _existing, note) = parked(
            bridge
                .park_gated_command(
                    "s1",
                    "hands",
                    &format!("gh issue comment 1 --body-file {}", path.display()),
                )
                .await
                .unwrap(),
        );
        assert!(
            note.as_deref().unwrap_or("").contains("skipped"),
            "the skip must be LOUD, not silent; note: {note:?}"
        );
    }

    /// The timeline branch (the ENTIRE #32 deadlock class): a content-free
    /// outward command with no reviewer deal since the caller's previous one
    /// QUEUES — the command line is the posted content — and settles after the
    /// reviewer reads that row; with a reviewer deal already between the
    /// caller's deals it parks straight away, as before.
    #[tokio::test]
    async fn a_content_free_outward_queues_until_the_reviewer_is_dealt() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, _path, _body) = outward_fixture(&data, &repo).await;
        let hands = storage.participant_by_slug("s1", "hands").await.unwrap().unwrap().id;
        let mut ring = ring_for(&bridge).await;
        // Caller dealt twice with NO reviewer deal between → queued + summoned.
        let m1 = storage.post_to_channel("s1", "user", None, "text", "one", None).await.unwrap();
        storage.commit_delivery(hands, &[(m1.message_id(), None)]).await.unwrap();
        let m2 = storage.post_to_channel("s1", "user", None, "text", "two", None).await.unwrap();
        storage.commit_delivery(hands, &[(m2.message_id(), None)]).await.unwrap();
        drain(&mut ring);
        let (gate_id, _) = queued(
            bridge.park_gated_command("s1", "hands", "gh pr merge 559 --merge").await.unwrap(),
        );
        assert_eq!(summons_in(&drain(&mut ring)), vec![eyes]);
        let row = storage.get_tray_entry(&gate_id).await.unwrap().unwrap();
        let posted = message_by_id(&storage, row.body_row_id.unwrap()).await;
        assert!(posted.content.contains("gh pr merge 559 --merge"), "the command line IS the content");
        // The reviewer reads it → settlement parks it.
        storage.commit_delivery(eyes, &[(row.body_row_id.unwrap(), None)]).await.unwrap();
        bridge.settle_queued_outward("s1", eyes).await;
        assert_eq!(storage.get_tray_entry(&gate_id).await.unwrap().unwrap().status, "pending");

        // A reviewer deal already between the caller's deals → a fresh park
        // proceeds without queueing, exactly as before 0080.
        bridge.resolve_choice(&gate_id, "Reject".into()).await.unwrap();
        let m3 = storage.post_to_channel("s1", "user", None, "text", "three", None).await.unwrap();
        storage.commit_delivery(eyes, &[(m3.message_id(), None)]).await.unwrap();
        let m4 = storage.post_to_channel("s1", "user", None, "text", "four", None).await.unwrap();
        storage.commit_delivery(hands, &[(m4.message_id(), None)]).await.unwrap();
        let (gid, _e, note) = parked(
            bridge.park_gated_command("s1", "hands", "gh pr merge 559 --merge").await.unwrap(),
        );
        assert!(!gid.is_empty());
        assert!(note.is_none());
    }

    #[tokio::test]
    async fn an_empty_body_is_refused_not_skipped() {
        // The afternoon's actual escape: `--body-file /dev/stdin` with nothing
        // piped shipped an empty-bodied PR. The check must fail CLOSED.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, _s, _eyes, _path, _body) = outward_fixture(&data, &repo).await;
        let empty = repo.path().join("empty.md");
        std::fs::write(&empty, "  \n").unwrap();
        let err = bridge
            .park_gated_command(
                "s1",
                "hands",
                &format!("gh pr create --base main --body-file {}", empty.display()),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("resolves to EMPTY"), "got: {err}");
    }

    #[tokio::test]
    async fn an_unextractable_body_form_refuses_instead_of_downgrading() {
        // Content that cannot be read BEFORE the run — shell-computed, stdin,
        // written by the same command line, or behind a file flag the
        // extractor does not know — refuses; it never falls to the weaker
        // timeline check while looking armed.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, _s, _eyes, path, _body) = outward_fixture(&data, &repo).await;
        for (cmd, why) in [
            ("gh issue comment 5 --body \"$(cat notes.md)\"".to_string(), "computed by the shell"),
            ("gh issue comment 5 --body-file -".to_string(), "stdin"),
            ("gh pr create --title t --fill".to_string(), "commit messages"),
            ("gh issue create --template-file t.md".to_string(), "does not know"),
            (format!("cp other.md {path} && gh issue comment 5 --body-file {path}"), "same command"),
        ] {
            let err = bridge
                .park_gated_command("s1", "hands", &cmd)
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains(why), "{cmd} must refuse ({why}); got: {err}");
        }
    }

    /// #35: forms the old extractor missed now reach the reviewer — a release
    /// body (`--notes-file`), `-b`, `--body=` — queued with their content
    /// posted, never parked as content-free.
    #[tokio::test]
    async fn release_notes_and_short_body_flags_queue_with_their_content() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, _eyes, path, body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        for (cmd, needle) in [
            (format!("gh release edit v1.0.5 --notes-file {path}"), body.as_str()),
            ("gh issue comment 5 -b \"quick note\"".to_string(), "quick note"),
            ("gh issue comment 6 --body=inline-words".to_string(), "inline-words"),
        ] {
            let (gate_id, _) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
            let row = storage.get_tray_entry(&gate_id).await.unwrap().unwrap().body_row_id.unwrap();
            let posted = message_by_id(&storage, row).await;
            assert!(posted.content.contains(needle), "{cmd}: the reviewer gets the content: {}", posted.content);
        }
    }

    #[tokio::test]
    async fn a_downed_reviewer_refuses_and_an_override_lifts_it() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, _s, _eyes, path, _body) = outward_fixture(&data, &repo).await;
        bridge.notify_agent_health("s1".to_string(), "eyes", "dead");
        let err = bridge
            .park_gated_command("s1", "hands", &format!("gh issue edit 5 --body-file {path}"))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("override_reviewer_block"), "got: {err}");
        // Activate the override the way the resolve path would — the same
        // session-scoped slot the commit gate reads.
        bridge
            .reviewer_override
            .lock()
            .unwrap()
            .insert("s1".to_string(), "user approved for the smoke".to_string());
        let (_gid, _e, note) = parked(
            bridge
                .park_gated_command("s1", "hands", &format!("gh issue edit 5 --body-file {path}"))
                .await
                .unwrap(),
        );
        assert!(note.as_deref().unwrap_or("").contains("override"), "note: {note:?}");
    }

    /// Feedback #32: a reviewer that went down AFTER a publish queued is
    /// reported on the re-issue — never "already queued, nothing to do", which
    /// would wait forever on a read that cannot happen.
    #[tokio::test]
    async fn a_down_reviewer_is_reported_before_the_queue_dedupe() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, _storage, _eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let cmd = format!("gh issue edit 5 --body-file {path}");
        queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        bridge.notify_agent_health("s1".to_string(), "eyes", "dead");
        let err = bridge.park_gated_command("s1", "hands", &cmd).await.unwrap_err().to_string();
        assert!(err.contains("override_reviewer_block") && err.contains("releases any publish already"), "got: {err}");
    }

    /// Feedback #32 / D36: the user's Approve of a reviewer-down override
    /// releases every publish stranded in the queue as its OWN Approve card,
    /// loudly marked unreviewed; a Reject releases nothing.
    #[tokio::test]
    async fn an_approved_override_releases_stranded_queued_publishes() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, _eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let cmd = format!("gh issue edit 5 --body-file {path}");
        let (gate_id, _) = queued(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        bridge.notify_agent_health("s1".to_string(), "eyes", "dead");
        let override_card = |bridge: std::sync::Arc<SignalingBridge>| async move {
            bridge
                .list_questions_for_session("s1")
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.status == "pending" && r.prompt.contains("override the review block"))
                .expect("the override parked its card")
        };

        // Reject first: the queued row stays queued.
        bridge.override_reviewer_block("s1", "hands", "reviewer crashed").await;
        let card = override_card(bridge.clone()).await;
        assert!(card.prompt.contains("skip the reviewer's read"), "the prompt says what it waives");
        bridge.resolve_choice_confirmable(&card.choice_id, "Reject".into(), false).await.unwrap();
        assert_eq!(status_of(&storage, &gate_id).await, "queued");

        bridge.override_reviewer_block("s1", "hands", "reviewer crashed").await;
        let card = override_card(bridge.clone()).await;
        bridge.resolve_choice_confirmable(&card.choice_id, "Approve".into(), false).await.unwrap();
        assert_eq!(status_of(&storage, &gate_id).await, "pending", "released to the user");
        let rows = storage.recent_row_bodies_upto("s1", i64::MAX, 20).await.unwrap();
        assert!(
            rows.iter().any(|r| r.contains(&gate_id) && r.contains("UNREVIEWED")),
            "the release is said out loud: {rows:?}"
        );
    }

    /// EYES 12c739ec: the reviewer filed a blocking finding against a queued
    /// publish and died before its settlement ran. The override release must
    /// withdraw that publish (it WAS reviewed — and vetoed), not hand it to the
    /// user as "unreviewed".
    #[tokio::test]
    async fn an_override_release_withdraws_a_publish_the_reviewer_vetoed() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let (gate_a, gate_b, _) = two_queued_and_read(&bridge, &storage, eyes, &repo, &path).await;
        bridge
            .eyes_flag_for_gate(
                "s1".into(),
                "eyes".into(),
                crate::storage::FindingSeverity::Blocking,
                "wrong claim in the body".into(),
                None,
                Some(gate_a.clone()),
            )
            .await
            .unwrap();
        // Dead before its TurnComplete — no settlement ran.
        bridge.notify_agent_health("s1".to_string(), "eyes", "dead");
        assert_eq!(bridge.release_queued_outward_unreviewed("s1").await, 1);
        assert_eq!(status_of(&storage, &gate_a).await, "withdrawn", "the vetoed one is withdrawn");
        assert_eq!(status_of(&storage, &gate_b).await, "pending", "the other is released");
    }

    /// Park an outward command that is COVERED (the reviewer received its
    /// body) and so parks straight for the user with its body hash recorded.
    /// The command is harmless if it ever runs: a `curl` to a closed local
    /// port, then a `touch` of `marker` — so "did it run?" is a file check.
    async fn covered_curl_park(
        bridge: &std::sync::Arc<SignalingBridge>,
        storage: &crate::storage::Storage,
        eyes: i64,
        path: &str,
        body: &str,
        marker: &std::path::Path,
    ) -> String {
        let m = storage
            .post_to_channel("s1", "participant", Some("hands"), "text", format!("Draft:\n{body}"), None)
            .await
            .unwrap();
        storage.commit_delivery(eyes, &[(m.message_id(), None)]).await.unwrap();
        let cmd = format!(
            "curl -s --max-time 1 -d @{path} http://127.0.0.1:9/ ; touch {}",
            posix_path(marker)
        );
        let (gate, _existing, _note) = parked(bridge.park_gated_command("s1", "hands", &cmd).await.unwrap());
        gate
    }

    async fn approve_oob(bridge: &std::sync::Arc<SignalingBridge>, gate: &str) -> String {
        match bridge.resolve_choice(gate, "Approve".into()).await.unwrap() {
            ResolveOutcome::DeliveredOutOfBand { body, .. } => body,
            other => panic!("expected OOB delivery, got {other:?}"),
        }
    }

    /// Feedback #22: the review covered the body FILE as it was at park time.
    /// Unchanged → it runs; edited or deleted after review → NOTHING runs.
    #[tokio::test]
    async fn an_approved_publish_whose_body_file_changed_does_not_run() {
        for case in ["unchanged", "edited", "deleted"] {
            let data = tempdir().unwrap();
            let repo = tempdir().unwrap();
            let (bridge, storage, eyes, path, body) = outward_fixture(&data, &repo).await;
            let marker = repo.path().join("ran.txt");
            let gate = covered_curl_park(&bridge, &storage, eyes, &path, &body, &marker).await;
            assert!(
                storage.get_tray_entry(&gate).await.unwrap().unwrap().body_sha256.is_some(),
                "the park records the reviewed body's hash"
            );
            match case {
                "edited" => std::fs::write(&path, format!("{body}\nA line added after review.")).unwrap(),
                "deleted" => std::fs::remove_file(&path).unwrap(),
                _ => {}
            }
            let out = approve_oob(&bridge, &gate).await;
            if case == "unchanged" {
                assert!(out.contains("Output:") && marker.exists(), "{case}: runs — {out}");
            } else {
                assert!(out.contains("NOT RUN"), "{case}: refused — {out}");
                assert!(!marker.exists(), "{case}: nothing ran");
                let rows = storage.recent_row_bodies_upto("s1", i64::MAX, 10).await.unwrap();
                assert!(rows.iter().any(|r| r.contains("did not run") && r.contains(&gate)));
            }
        }
    }

    /// A gate parked before 0084 has no recorded hash: it runs as before, and
    /// the output says the body was not re-checked.
    #[tokio::test]
    async fn a_pre_upgrade_gate_without_a_hash_runs_with_a_note() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, body) = outward_fixture(&data, &repo).await;
        let marker = repo.path().join("ran.txt");
        let gate = covered_curl_park(&bridge, &storage, eyes, &path, &body, &marker).await;
        sqlx::query("UPDATE session_tray SET body_sha256 = NULL WHERE choice_id = ?")
            .bind(&gate)
            .execute(storage.pool())
            .await
            .unwrap();
        let out = approve_oob(&bridge, &gate).await;
        assert!(out.contains("not re-checked") && marker.exists(), "got: {out}");
    }

    /// EYES 891e0eb1, second half: while a TARGETED veto is open, the same
    /// command is refused — unless its body FILE changed since the vetoed
    /// version, which is new content and goes to a fresh review.
    #[tokio::test]
    async fn a_targeted_veto_lets_a_changed_body_file_through_to_review() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, _body) = outward_fixture(&data, &repo).await;
        let _ring = ring_for(&bridge).await;
        let (gate_a, _gate_b, cmd_a) = two_queued_and_read(&bridge, &storage, eyes, &repo, &path).await;
        bridge
            .eyes_flag_for_gate(
                "s1".into(),
                "eyes".into(),
                crate::storage::FindingSeverity::Blocking,
                "wrong claim".into(),
                None,
                Some(gate_a.clone()),
            )
            .await
            .unwrap();
        bridge.settle_queued_outward("s1", eyes).await;
        assert!(bridge.park_gated_command("s1", "hands", &cmd_a).await.is_err(), "unchanged: refused");
        std::fs::write(&path, "The corrected claim.\n").unwrap();
        queued(bridge.park_gated_command("s1", "hands", &cmd_a).await.unwrap());
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
            .action_gate("s1".into(), "hands".into(), "echo hi".into(), false)
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
            .action_gate("s1".into(), "hands".into(), "echo hi".into(), false)
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
            ResolveOutcome::DeliveredOutOfBand { body, .. } => {
                assert!(body.contains("exit 0"), "approve carries output: {body}")
            }
            other => panic!("expected OOB delivery, got {other:?}"),
        }
        let status = bridge.gate_status(&gate_id).await.unwrap();
        assert!(status.starts_with("approved"), "got: {status}");

        // A rejected re-fire parks FRESH (pending-only dedupe): reject the new
        // gate and confirm its status carries the user's reasoning.
        let refire = bridge
            .action_gate("s1".into(), "hands".into(), "echo hi".into(), false)
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
        let cmd = format!("touch {}", posix_path(&marker));
        let bridge = bridge_with(
            data.path(),
            &[gk("touch", GateMode::Gate)],
            "s1",
            repo.path(),
        )
        .await;

        let parked = bridge
            .action_gate("s1".into(), "hands".into(), cmd.clone(), false)
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
            matches!(outcome, ResolveOutcome::DeliveredOutOfBand { .. }),
            "fresh gate approves one-click, got {outcome:?}"
        );
        assert!(marker.exists(), "fresh approve executes");

        // Age a second gate past the window by rewriting its asked_at.
        let marker2 = repo.path().join("old.txt");
        let cmd2 = format!("touch {}", posix_path(&marker2));
        let parked2 = bridge
            .action_gate("s1".into(), "hands".into(), cmd2.clone(), false)
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

    /// Round 12 (EYES fd17516b): `execute_gated_with` PASSES the session's
    /// identity and the caller's extra envs to the child — the join between
    /// `session_envs` and `run_in_repo`, which `run_in_repo_sets_the_envs_it_is_given`
    /// does not reach. Delete the `session_envs`/`extend_from_slice` lines and
    /// this goes red with every gated command silently losing `BOT_HQ_SESSION_ID`.
    #[tokio::test]
    async fn execute_gated_with_passes_the_session_identity_and_extra_envs() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        storage
            .create_session("s-env", "t", Some(dir.path().to_str().unwrap()))
            .await
            .unwrap();
        let out = bridge
            .execute_gated_with(
                "s-env",
                "printf 'sid=%s extra=%s' \"$BOT_HQ_SESSION_ID\" \"$EXTRA_ENV\"",
                std::time::Duration::from_secs(5),
                &[("EXTRA_ENV", "ride-along")],
            )
            .await
            .unwrap();
        assert!(out.contains("sid=s-env extra=ride-along"), "{out}");
    }

    /// Batch 0 (rc3→1.0.0, dissect items 3/12): the approve path hands the
    /// STORED command to the shell byte-complete — a multi-line script's
    /// line-2 assignment is consumed on its last line and a deep-line marker
    /// executes. The chat row showing only the first line
    /// (`bridge/util.rs::render_answer`) is display, not execution; if this
    /// test holds and the display changes claim otherwise, believe this test.
    #[tokio::test]
    async fn execute_gated_runs_the_whole_multi_line_command_not_its_first_line() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        storage
            .create_session("s-deep", "t", Some(dir.path().to_str().unwrap()))
            .await
            .unwrap();
        let command = "cd .\nMARK=gate-depth-9e1\necho one\necho two\necho three\necho four\necho five\necho six\necho seven\necho eight\necho nine\necho ten\necho eleven\necho twelve\nprintf 'tail:%s\\n' \"$MARK\"";
        let out = bridge
            .execute_gated("s-deep", command)
            .await
            .unwrap();
        assert!(
            out.contains("tail:gate-depth-9e1"),
            "the last line of a 15-line gated command must execute with line-2 state intact: {out}"
        );
    }
}
