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
            let (gate_id, existing, note) = self
                .park_gated_command(&session_id, &agent, &command)
                .await?;
            return Ok(parked_gate_text(&gate_id, &command, existing, note.as_deref()));
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
                let (gate_id, existing, note) = self
                    .park_gated_command(&session_id, &agent, &command)
                    .await?;
                Ok(parked_gate_text(&gate_id, &command, existing, note.as_deref()))
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
    ) -> Result<(String, bool, Option<String>)> {
        // **Outward-review precondition (batch 2 C, 2026-08-27).** An OUTWARD
        // command — one that publishes under the user's identity — may park
        // only after the session's reviewer has been DELIVERED its content.
        // Both eras' escapes landed in this hole: the morning's two false
        // claims went out through gates while the reviewer was starved, and
        // the afternoon's empty-bodied PR raced its own retraction. The check
        // refuses (teaching the two-turn ritual) instead of holding: nothing
        // is timed, nothing is stranded on restart, and a rejected gate
        // re-parks without re-review when the content is unchanged, because
        // coverage is keyed on the content itself.
        //
        // Ceiling, stated plainly: delivery is provable, review is not — a
        // reviewer that passed its turn satisfies this check. It is the honest
        // limit of a mechanical precondition.
        let note = match self.outward_review_check(session_id, agent, command).await? {
            OutwardReview::Refuse(text) => return Err(anyhow::anyhow!(text)),
            OutwardReview::Proceed(note) => note,
        };
        if let Some(n) = &note {
            tracing::warn!(session_id, agent, note = %n, "outward park proceeding with review precondition skipped");
        }
        let (gate_id, existing) = self.park_reviewed_command(session_id, agent, command).await?;
        Ok((gate_id, existing, note))
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
        if let Some(storage) = storage {
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
    async fn session_working_repo(&self, session_id: &str) -> Option<PathBuf> {
        let storage = self.storage.lock().await.clone()?;
        let session = storage.get_session(session_id).await.ok()??;
        session.working_repo_path.map(PathBuf::from)
    }
}

/// The parked-gate response text. `existing` distinguishes a fresh park from a
/// dedupe hit on an already-pending identical command.
/// The outward-review verdict: park may proceed (with an optional LOUD note
/// for the ack — a guard that quietly isn't watching is indistinguishable
/// from one with nothing to report), or is refused with teaching text.
pub(crate) enum OutwardReview {
    Proceed(Option<String>),
    Refuse(String),
}

/// OUTWARD classifier, v1: a command publishes under the user's identity when
/// any segment's FIRST WORD is `gh` or `curl`. Segment-anchored both ways (the
/// FileViewerDialog over-match lesson): `echo "gh issue"` is not outward, and
/// `true && gh issue edit …` is. `git push` is deliberately absent — the
/// pre-push hook owns it end to end.
fn outward_command(command: &str) -> bool {
    command
        .split(['\n', ';', '|'])
        .flat_map(|s| s.split("&&"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|seg| {
            matches!(seg.split_whitespace().next().unwrap_or(""), "gh" | "curl")
        })
}

/// Body payloads an outward command carries: `--body-file <p>` /
/// `--body-file=<p>` file references, and inline `--body "…"` / `--body '…'`
/// strings. v1 covers the forms every real gate this week used.
fn outward_bodies(command: &str) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut inline = Vec::new();
    let toks: Vec<&str> = command.split_whitespace().collect();
    for (i, t) in toks.iter().enumerate() {
        if let Some(p) = t.strip_prefix("--body-file=") {
            files.push(p.trim_matches(['"', '\'']).to_string());
        } else if *t == "--body-file" {
            if let Some(p) = toks.get(i + 1) {
                files.push(p.trim_matches(['"', '\'']).to_string());
            }
        }
    }
    // Inline bodies keep their spaces, so they need the raw string, not the
    // token walk: match the quoted span after `--body`.
    for marker in ["--body \"", "--body '"] {
        let quote = marker.chars().last().unwrap();
        let mut rest = command;
        while let Some(pos) = rest.find(marker) {
            let after = &rest[pos + marker.len()..];
            if let Some(end) = after.find(quote) {
                inline.push(after[..end].to_string());
                rest = &after[end..];
            } else {
                break;
            }
        }
    }
    (files, inline)
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
        let health = self.current_agent_health(session_id, reviewer_slug);
        let recent =
            self.agent_rpc_recent(session_id, reviewer_slug, super::findings::REVIEWER_LIVENESS_WINDOW);
        if matches!(health.as_deref(), Some("stalled") | Some("dead")) && !recent {
            return Ok(match self.reviewer_override_reason(session_id) {
                Some(_) => OutwardReview::Proceed(Some(
                    "note: reviewer down — user-approved override in effect; outward review \
                     precondition skipped"
                        .into(),
                )),
                None => OutwardReview::Refuse(format!(
                    "outward publish held: the reviewer ({reviewer_slug}) is \
                     {} and not recently active. Respawn it, or ask the user to \
                     approve override_reviewer_block.",
                    health.as_deref().unwrap_or("down")
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
        let (files, mut bodies) = outward_bodies(command);
        for p in files {
            let path = std::path::Path::new(&p);
            let resolved = if path.is_relative() {
                match self.session_working_repo(session_id).await {
                    Some(repo) => repo.join(path),
                    None => path.to_path_buf(),
                }
            } else {
                path.to_path_buf()
            };
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
        // Fail CLOSED on the forms the extractor cannot evaluate (review
        // round 2): a `--body`/`-b` the parser does not recognise must not
        // silently downgrade to the timeline check while looking armed.
        let mentions_body = command.contains("--body") // covers --body, --body=, --body-file…
            || command.split_whitespace().any(|t| t == "-b");
        if bodies.is_empty() && mentions_body {
            return Ok(OutwardReview::Refuse(
                "outward publish held: this command carries a body in a form the \
                 coverage check cannot extract (-b, --body=…, or unquoted --body). \
                 Use --body-file <path> or --body \"…\" so the reviewer-delivered \
                 content can be verified."
                    .into(),
            ));
        }
        let cursor = storage.cursor_for(reviewer.id).await?;
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
            return Ok(
                if storage.has_delivery_between(reviewer.id, &prev, &current).await? {
                    OutwardReview::Proceed(None)
                } else {
                    OutwardReview::Refuse(
                        "outward publish held for review: the reviewer has not been \
                         dealt a turn since your previous one. End your turn — the \
                         ring deals the reviewer next — then park on your following \
                         turn."
                            .into(),
                    )
                },
            );
        }
        // Coverage window: the newest 500 rows at or below the cursor. A body
        // older than that reads as uncovered — failing closed, documented.
        let haystack = storage.recent_row_bodies_upto(session_id, cursor, 500).await?;
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
            let covered = haystack
                .iter()
                .any(|row| row.contains(trimmed) || row.contains(escaped));
            if !covered {
                return Ok(OutwardReview::Refuse(
                    "outward publish held for review: the reviewer has not been \
                     delivered this content. Post the body to the channel or a \
                     session doc (the write itself is a delivered row), end your \
                     turn — the ring deals the reviewer next — then park on your \
                     following turn. Coverage is content-keyed: an unchanged body \
                     re-parks without re-review."
                        .into(),
                ));
            }
        }
        Ok(OutwardReview::Proceed(None))
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
    // The outward-review skip note rides the ack LOUDLY (batch 2 C): a solo
    // roster or an overridden-down reviewer parks without the precondition,
    // and the agent must see that the guard was not watching.
    let note_line = note.map(|n| format!("\n{n}")).unwrap_or_default();
    format!(
        "{lead} (gate_id: {gate_id}).{note_line}\n\
         `{command}` runs when the user approves; its output arrives as an \
         out-of-band message. On reject you get a rejection notice instead. Do \
         NOT re-issue the command or assume it ran — call gate_status(\"{gate_id}\") \
         if you need the current state before continuing."
    )
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
        bridge.resolve_choice(&cid, "Reject".into()).await.unwrap();
        assert!(!marker.exists(), "rejected command must NOT have run");
        let status = bridge.gate_status(&cid).await.unwrap();
        assert!(status.starts_with("rejected"), "got: {status}");
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

        let (gate_id, existing, _note) = bridge
            .park_gated_command("s1", "hands", &cmd)
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
        let (dup_id, dup_existing, _note) = bridge
            .park_gated_command("s1", "hands", &cmd)
            .await
            .unwrap();
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

    #[tokio::test]
    async fn an_outward_body_never_delivered_is_refused_with_the_ritual() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, _s, _eyes, path, _body) = outward_fixture(&data, &repo).await;
        let err = bridge
            .park_gated_command("s1", "hands", &format!("gh issue edit 5 --body-file {path}"))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Post the body to the channel or a session doc"),
            "the refusal teaches the two-turn ritual; got: {err}"
        );
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
            bridge.park_gated_command("s1", "hands", &cmd).await.unwrap();
        assert!(!existing);
        assert!(note.is_none(), "a real review pass carries no skip note");

        // Reject, then re-park the UNCHANGED content: coverage is keyed on
        // the content, so no re-review is demanded (a later refactor must not
        // turn reject-and-retry into a loop).
        bridge.resolve_choice(&gate_id, "Reject".into()).await.unwrap();
        let (gate2, existing2, _n) =
            bridge.park_gated_command("s1", "hands", &cmd).await.unwrap();
        assert_ne!(gate2, gate_id, "a post-reject re-fire parks fresh");
        assert!(!existing2);
    }

    #[tokio::test]
    async fn an_edited_body_after_review_is_refused_again() {
        // The morning's exact shape: review a version, edit the MIDDLE, park.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, path, body) = outward_fixture(&data, &repo).await;
        let m = storage
            .post_to_channel("s1", "participant", Some("hands"), "text", format!("Draft:\n{body}"), None)
            .await
            .unwrap();
        storage.commit_delivery(eyes, &[(m.message_id(), None)]).await.unwrap();
        std::fs::write(&path, body.replace("conditional", "settled")).unwrap();
        let err = bridge
            .park_gated_command("s1", "hands", &format!("gh issue edit 5 --body-file {path}"))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("has not been delivered this content"), "got: {err}");
    }

    #[tokio::test]
    async fn a_solo_roster_parks_with_the_loud_skip_note() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let bridge = bridge_with(data.path(), &[], "s1", repo.path()).await;
        // No reviewers registered at all.
        let path = repo.path().join("b.md");
        std::fs::write(&path, "solo body").unwrap();
        let (_gid, _existing, note) = bridge
            .park_gated_command(
                "s1",
                "hands",
                &format!("gh issue comment 1 --body-file {}", path.display()),
            )
            .await
            .unwrap();
        assert!(
            note.as_deref().unwrap_or("").contains("skipped"),
            "the skip must be LOUD, not silent; note: {note:?}"
        );
    }

    #[tokio::test]
    async fn a_content_free_outward_needs_a_reviewer_deal_between_the_callers_deals() {
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, storage, eyes, _path, _body) = outward_fixture(&data, &repo).await;
        let hands = storage.participant_by_slug("s1", "hands").await.unwrap().unwrap().id;
        // Caller dealt twice with NO reviewer deal between → refused.
        let m1 = storage.post_to_channel("s1", "user", None, "text", "one", None).await.unwrap();
        storage.commit_delivery(hands, &[(m1.message_id(), None)]).await.unwrap();
        let m2 = storage.post_to_channel("s1", "user", None, "text", "two", None).await.unwrap();
        storage.commit_delivery(hands, &[(m2.message_id(), None)]).await.unwrap();
        let err = bridge
            .park_gated_command("s1", "hands", "gh pr merge 559 --merge")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("has not been dealt a turn"), "got: {err}");
        // A reviewer deal after the caller's latest → the next park passes.
        let m3 = storage.post_to_channel("s1", "user", None, "text", "three", None).await.unwrap();
        storage.commit_delivery(eyes, &[(m3.message_id(), None)]).await.unwrap();
        let m4 = storage.post_to_channel("s1", "user", None, "text", "four", None).await.unwrap();
        storage.commit_delivery(hands, &[(m4.message_id(), None)]).await.unwrap();
        let (gid, _e, note) = bridge
            .park_gated_command("s1", "hands", "gh pr merge 559 --merge")
            .await
            .unwrap();
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
        // `-b`, `--body=…` and unquoted `--body text` are body-carrying forms
        // the extractor does not parse; they must refuse, not silently fall to
        // the weaker timeline check while looking armed.
        let data = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let (bridge, _s, _eyes, _path, _body) = outward_fixture(&data, &repo).await;
        for cmd in [
            "gh issue comment 5 -b \"quick note\"",
            "gh issue comment 5 --body=inline",
            "gh issue comment 5 --body unquoted words",
        ] {
            let err = bridge
                .park_gated_command("s1", "hands", cmd)
                .await
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("cannot extract"),
                "{cmd} must refuse, not downgrade; got: {err}"
            );
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
        let (_gid, _e, note) = bridge
            .park_gated_command("s1", "hands", &format!("gh issue edit 5 --body-file {path}"))
            .await
            .unwrap();
        assert!(note.as_deref().unwrap_or("").contains("override"), "note: {note:?}");
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
