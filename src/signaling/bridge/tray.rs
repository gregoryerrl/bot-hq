//! Signaling tools that park a tray entry: `ask_user_choice` (non-blocking —
//! parks and returns a `{parked}` ack; the pick arrives out-of-band) and
//! `request_approval` (blocking — awaits the user's oneshot), plus their
//! supersede + resolve machinery, `mark_awaiting_user`,
//! `request_phase_advance`, and the pending-choice snapshots. This is the
//! biggest slice of the bridge — everything that parks a oneshot, mirrors a
//! tray row, or sets the duo's awaiting-halt flag.

use super::util::{oob_resolution_body, outcome_from_picked, parse_tray_ts};
use super::*;
use crate::storage::MessageKind;
use uuid::Uuid;

/// A pending gated command older than this (seconds) gets a confirm step on
/// approve — the repo context it was asked against has likely moved on. Sized
/// well above the observed median answer latency (minutes) so routine
/// approvals stay one-click.
pub const STALE_GATE_MAX_AGE_SECS: i64 = 900;

/// Age of a tray row in seconds, from its `asked_at` (now_utc RFC3339-Z;
/// sqlite `datetime('now')` shape accepted defensively). None = unparseable.
pub fn gate_age_secs(asked_at: &str) -> Option<i64> {
    let asked = chrono::DateTime::parse_from_rfc3339(asked_at)
        .map(|t| t.with_timezone(&chrono::Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(asked_at, "%Y-%m-%d %H:%M:%S")
                .map(|n| n.and_utc())
        })
        .ok()?;
    Some((chrono::Utc::now() - asked).num_seconds())
}

impl SignalingBridge {
    async fn set_session_awaiting(&self, session_id: &str) {
        if let Some(flag) = self.session_awaiting.lock().await.get(session_id) {
            flag.store(true, Ordering::Release);
        }
        // Reflect the flag flip into the derived activity NOW — emit AwaitingUser
        // immediately instead of waiting for the agent's TurnComplete set_busy
        // (the dot-lag bug). Weak upgrade: the tracker may be gone if the session
        // closed mid-flight → silent no-op. The lock is dropped before refresh().
        let tracker = self
            .session_activity
            .lock()
            .await
            .get(session_id)
            .and_then(Weak::upgrade);
        if let Some(tracker) = tracker {
            tracker.refresh();
        }
    }

    /// Called by the MCP `tools/call` handler for `ask_user_choice`. Parks the
    /// question and returns IMMEDIATELY with `{"status":"parked","choice_id"}`
    /// — it does NOT block waiting for the user. The pick is delivered later
    /// out-of-band (resolve_choice → synthetic user message), so a slow human
    /// no longer ties up the agent's MCP request until it client-side times out.
    ///
    /// Auto-supersedes the most recent pending question from this same
    /// `(session_id, agent)` — the new ask replaces the old one in the tray,
    /// the old gets `status='superseded'`, and the new row's `supersedes_id`
    /// points at the old. This kills the retry-duplicate cascade without
    /// relying on agent discipline.
    pub async fn ask_user_choice(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
    ) -> Result<String> {
        let supersedes_id = self
            .auto_supersede_prior_pending(&session_id, &agent, &question)
            .await;
        self.ask_user_choice_inner(
            session_id, agent, question, options, None, supersedes_id, false,
        )
        .await
    }

    /// Policy-initiated approval request, BLOCKING — holds the call open until
    /// the user picks, so the caller gets the answer in-band.
    ///
    /// **Host-internal callers only**, specifically the pre-push git hook
    /// (`server.rs::handle_pre_push`), which maps the pick onto a process exit
    /// code and so needs a synchronous bool. Agent-facing MCP calls must use
    /// [`Self::request_approval_parked`] — an agent blocking here hits its
    /// client's ~60s timeout while the human is still deciding and cannot tell
    /// "queued" from "failed". That is the ghost state `action_gate` was moved
    /// off in `2ab07b4`; this sibling kept the old contract and it fired live
    /// on a production query (2026-07-28T15:55Z).
    pub async fn request_approval(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        ctx: ApprovalContext,
    ) -> Result<String> {
        self.request_approval_inner(session_id, agent, question, options, ctx, true)
            .await
    }

    /// Policy-initiated approval request, PARKED — the agent-facing path.
    ///
    /// Same violation-recording machinery as [`Self::request_approval`], but
    /// returns the `{"status":"parked","choice_id":…}` acknowledgment at once
    /// and delivers the pick out-of-band: the contract `ask_user_choice` and
    /// `action_gate` already use.
    pub async fn request_approval_parked(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        ctx: ApprovalContext,
    ) -> Result<String> {
        self.request_approval_inner(session_id, agent, question, options, ctx, false)
            .await
    }

    /// Shared body of the two entry points above. `blocking` is the ONLY
    /// difference between them — see their docs for which caller gets which.
    async fn request_approval_inner(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        ctx: ApprovalContext,
        blocking: bool,
    ) -> Result<String> {
        let supersedes_id = self
            .auto_supersede_prior_pending(&session_id, &agent, &question)
            .await;
        self.ask_user_choice_inner(
            session_id,
            agent,
            question,
            options,
            Some(ctx),
            supersedes_id,
            blocking,
        )
        .await
    }

    /// Explicit supersede: agent passes the choice_id of a stale question
    /// they want to replace + the new question text/options. Same effect as
    /// `ask_user_choice` from the user's perspective (parks and returns
    /// immediately; the pick arrives out-of-band) but the linkage to a SPECIFIC
    /// stale row is deterministic (vs the auto-supersede heuristic which only
    /// catches the latest). Returns the parked acknowledgment for the new
    /// question.
    pub async fn supersede_question_with_new(
        &self,
        session_id: String,
        agent: String,
        stale_choice_id: String,
        question: String,
        options: Vec<String>,
    ) -> Result<String> {
        // Look up the stale row by choice_id to capture its internal id (for
        // the new row's supersedes_id FK) BEFORE marking it superseded.
        let stale_internal_id = {
            let storage_guard = self.storage.lock().await;
            match storage_guard.as_ref() {
                Some(storage) => storage
                    .get_tray_entry(&stale_choice_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|q| q.id),
                None => None,
            }
        };
        // Flip status + drop parked oneshot + fire ChoiceResolved for the UI.
        {
            let storage_guard = self.storage.lock().await;
            if let Some(storage) = storage_guard.as_ref() {
                if let Err(e) = storage.supersede_tray_entry(&stale_choice_id).await {
                    tracing::warn!(?e, %stale_choice_id, "supersede (explicit) storage update failed");
                }
            }
        }
        self.pending.lock().await.remove(&stale_choice_id);
        let _ = self.event_tx.send(SignalingEvent::ChoiceResolved {
            choice_id: stale_choice_id,
            picked: "(superseded)".to_string(),
        });
        // Post the new question with the supersedes_id link in place. Like a
        // normal ask_user_choice this is non-blocking — it parks and returns;
        // the pick arrives out-of-band.
        self.ask_user_choice_inner(
            session_id,
            agent,
            question,
            options,
            None,
            stale_internal_id,
            false,
        )
        .await
    }

    /// Dedupe a true RE-ASK: mark a prior pending question from
    /// `(session_id, agent)` with the SAME `prompt` as superseded + remove it
    /// from `pending`. Returns that row's internal id (for the new row's
    /// `supersedes_id`), or None when there's no matching prior pending.
    ///
    /// Matching on `prompt` is load-bearing: it kills the timeout-retry
    /// duplicate cascade (G2 — the agent re-issues the SAME ask after a
    /// client-side timeout) WITHOUT collapsing DISTINCT questions/gates. Distinct
    /// pending from one agent must accumulate in the tray so the user can answer
    /// them all when they return from AFK — superseding them on every new ask
    /// (the old behavior) defeated that.
    async fn auto_supersede_prior_pending(
        &self,
        session_id: &str,
        agent: &str,
        prompt: &str,
    ) -> Option<i64> {
        let storage_guard = self.storage.lock().await;
        let storage = storage_guard.as_ref()?;
        let rows = storage.tray_entries_for_session(session_id).await.ok()?;
        let latest = rows
            .into_iter()
            .rev()
            .find(|q| q.agent == agent && q.status == "pending" && q.prompt == prompt)?;
        let stale_choice_id = latest.choice_id.clone();
        let stale_internal_id = latest.id;
        // Mark in storage first so the UI tray drops it on its next poll.
        if let Err(e) = storage.supersede_tray_entry(&stale_choice_id).await {
            tracing::warn!(?e, %stale_choice_id, "supersede (auto) storage update failed");
        }
        drop(storage_guard);
        // Drop the parked oneshot so any (rare) still-listening client gets
        // the standard cancellation.
        self.pending.lock().await.remove(&stale_choice_id);
        // Tell the UI to clear its inline state for this choice.
        let _ = self.event_tx.send(SignalingEvent::ChoiceResolved {
            choice_id: stale_choice_id,
            picked: "(superseded)".to_string(),
        });
        Some(stale_internal_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn ask_user_choice_inner(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        approval: Option<ApprovalContext>,
        supersedes_id: Option<i64>,
        // `true` = hold the request open until the UI resolves (request_approval
        // / pre-push gate — a git hook awaits a synchronous bool). `false` =
        // park and return immediately (ask_user_choice / supersede); the answer
        // arrives out-of-band. See the branch at the end of this fn.
        blocking: bool,
    ) -> Result<String> {
        let choice_id = Uuid::new_v4().to_string();
        // Persist the command for an action_gate (ToolBlocklist) approval so it
        // can still execute on approve after the in-memory oneshot is gone
        // (client timeout / restart). Extracted before `approval` moves into
        // PendingChoice below.
        let command_text = approval.as_ref().and_then(|a| {
            matches!(a.kind, crate::policy::ViolationKind::ToolBlocklist).then(|| a.action.clone())
        });
        let (tx, rx) = oneshot::channel::<String>();
        let choice = PendingChoice {
            choice_id: choice_id.clone(),
            session_id: session_id.clone(),
            agent: agent.clone(),
            question: question.clone(),
            options: options.clone(),
            approval,
        };
        self.pending.lock().await.insert(
            choice_id.clone(),
            Parked {
                tx,
                choice: choice.clone(),
            },
        );

        // Mirror into the question cache for durable per-session state. The
        // in-chat tray + dashboard counter read from this table, and the row
        // survives restart even though the parked oneshot in `pending` does
        // not. Best-effort: if storage isn't wired yet (test bridges built
        // via ::new), continue without persisting.
        self.persist_question(
            &session_id,
            &choice_id,
            &agent,
            crate::storage::QuestionKind::Choice,
            &question,
            Some(&options),
            supersedes_id,
            command_text.as_deref(),
        )
        .await;

        // Halt the duo BEFORE emitting the event — the agent's next chunk
        // shouldn't volley to its peer while we wait for the user.
        self.set_session_awaiting(&session_id).await;

        // Best-effort broadcast. If no subscribers, the request still parks
        // until resolve_choice is called (mostly a concern for tests).
        let _ = self.event_tx.send(SignalingEvent::PendingChoice(choice));

        if !blocking {
            // Non-blocking primary path (ask_user_choice / supersede): park and
            // return IMMEDIATELY. `rx` drops here — we never await it — but the
            // Parked{tx} stays in `pending` so the UI snapshot and
            // list_my_pending_questions still see the open question. When
            // resolve_choice lands the pick, its `tx.send` fails (rx gone) and
            // falls through to the existing OOB stdin-injection path, which
            // delivers the answer as a synthetic user message. That is the SAME
            // path that already handled client-side timeouts — here it's primary,
            // so there's no ~30s dead-wait and no timeout-then-poll dance.
            return Ok(serde_json::json!({
                "status": "parked",
                "choice_id": choice_id,
            })
            .to_string());
        }
        // Blocking path (request_approval / pre-push gate): the caller needs a
        // synchronous decision (a git hook awaits a bool), so hold the request
        // open until the UI resolves.
        let picked = rx.await.map_err(|_| {
            anyhow::anyhow!("ask_user_choice canceled before user picked an option")
        })?;
        let _ = self.event_tx.send(SignalingEvent::ChoiceResolved {
            choice_id,
            picked: picked.clone(),
        });
        Ok(picked)
    }

    /// Best-effort write of a question row to storage. The bridge's in-memory
    /// `pending` map is still the source of truth for the blocking oneshot,
    /// but the storage row is what the UI tray reads. Failures are logged
    /// and swallowed so the agent's tool call doesn't fail on a DB hiccup.
    #[allow(clippy::too_many_arguments)]
    async fn persist_question(
        &self,
        session_id: &str,
        choice_id: &str,
        agent: &str,
        kind: crate::storage::QuestionKind,
        prompt: &str,
        options: Option<&[String]>,
        supersedes_id: Option<i64>,
        command_text: Option<&str>,
    ) {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return;
        };
        if let Err(e) = storage
            .insert_tray_entry(
                session_id,
                choice_id,
                agent,
                kind,
                prompt,
                options,
                supersedes_id,
                command_text,
            )
            .await
        {
            tracing::warn!(?e, choice_id, "persist_question failed");
        }
    }

    /// Withdraw a pending question (agent abandons it; no answer will arrive).
    /// Removes the parked oneshot AND updates the storage row to status=withdrawn
    /// so the UI tray drops it. Returns true if a question was actually withdrawn,
    /// false if the choice_id was unknown or already resolved.
    pub async fn withdraw_question(&self, choice_id: &str) -> bool {
        let parked = self.pending.lock().await.remove(choice_id);
        let was_parked = parked.is_some();
        // Drop the oneshot — the agent's blocking caller (if any) gets the
        // standard "canceled" error.
        drop(parked);
        let storage_guard = self.storage.lock().await;
        if let Some(storage) = storage_guard.as_ref() {
            if let Err(e) = storage.withdraw_tray_entry(choice_id).await {
                tracing::warn!(?e, choice_id, "withdraw_question storage update failed");
            }
        }
        was_parked
    }

    /// Snapshot the `session_tray` table for a session. Convenience for the UI
    /// + the agent-facing `list_my_pending_questions` MCP tool.
    pub async fn list_questions_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::storage::SessionTrayEntry>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(Vec::new());
        };
        storage.tray_entries_for_session(session_id).await
    }

    /// All pending tray rows across OPEN sessions (closed sessions are
    /// excluded). Durable source for the header notifier's per-session
    /// "needs your input [N]" counts — survives restart, unlike the in-memory
    /// pending map.
    pub async fn list_pending_tray_open(&self) -> Result<Vec<crate::storage::SessionTrayEntry>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(Vec::new());
        };
        storage.pending_tray_open_sessions().await
    }

    /// Convenience entry (`confirm_stale = false`): the external driver + tests
    /// for fresh gates, rejects, and non-command asks. A STALE gated-command
    /// Approve through this path returns `StaleGateNeedsConfirm` rather than
    /// executing — see [`resolve_choice_confirmable`].
    pub async fn resolve_choice(&self, choice_id: &str, picked: String) -> Result<ResolveOutcome> {
        self.resolve_choice_confirmable(choice_id, picked, false)
            .await
    }

    /// Called by the UI when the user clicks a choice button. `confirm_stale =
    /// true` lets an explicitly-confirmed Approve EXECUTE a stale gated command
    /// (the requesting agent has moved on; the user has acknowledged the repo
    /// state may have changed).
    pub async fn resolve_choice_confirmable(
        &self,
        choice_id: &str,
        picked: String,
        confirm_stale: bool,
    ) -> Result<ResolveOutcome> {
        // SAFETY GATE: a gated command whose requesting agent has moved on
        // (client-side MCP timeout / restart) would run blind on a one-click
        // Approve, against a repo state that may have changed since it was
        // parked. Detect that BEFORE the atomic flip and bail to
        // StaleGateNeedsConfirm so nothing flips or executes until the user
        // confirms. Reject / non-executing picks are always safe and skip this.
        // (A vanishingly-small race exists if the agent's receiver drops in the
        // window between this peek and the flip below; the cost is one execution
        // of a command that was live <1ms ago — current context — so it's
        // acceptable. The real target is long-stale / post-restart commands.)
        if !confirm_stale
            && matches!(
                outcome_from_picked(&picked),
                crate::policy::ViolationOutcome::Approved
            )
        {
            if let Some((command, asked_at)) = self.stale_gated_command(choice_id).await {
                return Ok(ResolveOutcome::StaleGateNeedsConfirm { command, asked_at });
            }
        }

        // Flip the row pending→answered first (so the UI/tray updates even if the
        // in-memory parked entry is gone — e.g. after a restart). `rows == 1`
        // means THIS call won the atomic transition; gate resolve-time
        // gated-command execution on it so a duplicate / stale resolve can't
        // double-run. This is the durable exactly-once that replaces the
        // in-memory oneshot's guarantee.
        let flipped = {
            let storage_guard = self.storage.lock().await;
            match storage_guard.as_ref() {
                Some(storage) => match storage.answer_tray_entry(choice_id, &picked).await {
                    Ok(rows) => rows == 1,
                    Err(e) => {
                        tracing::warn!(?e, choice_id, "answer_question storage update failed");
                        false
                    }
                },
                None => false,
            }
        };
        let parked = self.pending.lock().await.remove(choice_id);
        match parked {
            Some(p) => {
                // Write violation record FIRST (before unblocking the agent)
                // so the audit trail captures the decision even if the agent
                // crashes immediately after receiving the result.
                let outcome = outcome_from_picked(&picked);
                if let (Some(log), Some(ctx)) = (self.violations.as_ref(), &p.choice.approval) {
                    let _ = log
                        .record(
                            p.choice.session_id.clone(),
                            p.choice.agent.clone(),
                            ctx.kind,
                            ctx.action.clone(),
                            outcome,
                            ctx.detail.clone(),
                        )
                        .await;
                }

                // Clear the awaiting halt the matching ask set, BEFORE delivering
                // the pick. The agent's blocking call returns on `p.tx.send`, so
                // the duo pump must already see `awaiting == false` by the time the
                // resumed agent's first chunk arrives — else `router::route_forward`
                // suppresses that chunk (and every later Brian<->Rain peer-forward)
                // until the user types free text or advances a phase (the duo goes
                // silent right after the user answers). The bridge set the flag
                // (set_session_awaiting), so the bridge clears it on resolve. Also
                // covers the Err fall-through below (core then re-clears + wakes
                // stdin — harmlessly redundant).
                self.clear_session_awaiting(&p.choice.session_id).await;
                match p.tx.send(picked) {
                    Ok(()) => Ok(ResolveOutcome::Delivered),
                    Err(picked) => {
                        // The agent's blocking `ask_user_choice` timed out client-side
                        // before we delivered the pick (the answer is still captured —
                        // violations already logged above). Fall back to OOB; the gated
                        // command, if any, comes from the in-memory approval ctx.
                        let command = p.choice.approval.as_ref().and_then(|c| {
                            matches!(c.kind, crate::policy::ViolationKind::ToolBlocklist)
                                .then_some(c.action.as_str())
                        });
                        // Age-stamp from the durable row (the in-memory park
                        // carries no ask-time); a miss just omits the line.
                        let asked_at = {
                            let storage_guard = self.storage.lock().await;
                            match storage_guard.as_ref() {
                                Some(storage) => storage
                                    .get_tray_entry(choice_id)
                                    .await
                                    .ok()
                                    .flatten()
                                    .map(|row| row.asked_at),
                                None => None,
                            }
                        };
                        Ok(self
                            .deliver_oob(
                                choice_id,
                                p.choice.session_id.clone(),
                                &p.choice.agent,
                                &p.choice.question,
                                &p.choice.options,
                                picked,
                                command,
                                flipped,
                                asked_at,
                            )
                            .await)
                    }
                }
            }
            None => {
                // No in-memory parked oneshot (the #2 reopened-session bug: the
                // session was closed — oneshot dropped — then reopened, and the
                // resumed agent re-asked with a NEW choice_id while the user
                // answered the OLD tray row). Reconstruct from the durable
                // session_tray row and fall back to OOB stdin delivery so
                // CoreAppState injects the answer into the live (respawned)
                // session — the only channel to a resumed subprocess.
                let q = {
                    let storage_guard = self.storage.lock().await;
                    match storage_guard.as_ref() {
                        Some(storage) => storage.get_tray_entry(choice_id).await?,
                        None => None,
                    }
                };
                let Some(q) = q else {
                    return Err(anyhow::anyhow!("no pending choice with id {choice_id}"));
                };
                let options: Vec<String> = q
                    .options_json
                    .as_deref()
                    .and_then(|j| serde_json::from_str(j).ok())
                    .unwrap_or_default();
                Ok(self
                    .deliver_oob(
                        choice_id,
                        q.session_id.clone(),
                        &q.agent,
                        &q.prompt,
                        &options,
                        picked,
                        q.command_text.as_deref(),
                        flipped,
                        Some(q.asked_at.clone()),
                    )
                    .await)
            }
        }
    }

    /// Persist + broadcast an out-of-band choice resolution for the two
    /// receiver-gone paths (client-timeout `Err(picked)` and post-restart /
    /// reopened `None`). Builds the synthetic user message, runs any approved
    /// gated command when `flipped` (the atomic exactly-once already won),
    /// invalidates the bell / tray via `ChoiceResolved`, and returns
    /// `AgentReceiverDroppedFellBack` — carrying the RECEIPT — so
    /// `CoreAppState::resolve_choice` can wake the live (respawned) subprocess
    /// via stdin. The callers differ only in where `session_id` / `agent` /
    /// `question` / `command_text` come from.
    #[allow(clippy::too_many_arguments)]
    async fn deliver_oob(
        &self,
        choice_id: &str,
        session_id: String,
        agent: &str,
        question: &str,
        options: &[String],
        picked: String,
        command_text: Option<&str>,
        flipped: bool,
        asked_at: Option<String>,
    ) -> ResolveOutcome {
        let mooting = self
            .gates_approved_since(&session_id, choice_id, asked_at.as_deref())
            .await;
        let mut body = oob_resolution_body(
            agent,
            question,
            options,
            &picked,
            asked_at.as_deref(),
            &mooting,
        );
        if flipped {
            self.maybe_run_gated(&session_id, command_text, &picked, &mut body)
                .await;
        }
        // The phase is read HERE, not in `CoreAppState::resolve_choice` where it
        // used to be. The envelope is part of the row, so it has to be known
        // before the INSERT; reading it after and prepending it to the wire is
        // precisely how this path came to record one thing and deliver another.
        //
        // The alternative — carry the receipt out and let core apply the phase —
        // cannot work: a receipt is immutable, so core would either wire the
        // undecorated body (dropping `[PHASE: X]` from what the agent reads) or
        // write a second row for one answer. So the lookup moves to the bridge
        // instead, which the per-session Weak registry already had a pattern for.
        //
        // It also costs core nothing it was relying on: core read the phase
        // microseconds later under the sessions lock, so the only observable
        // difference is a concurrent `advance_phase` landing in that window,
        // where taking the post-time phase is the correct one — it is the phase
        // the row says the agent was told.
        let envelope = self
            .current_session_phase(&session_id)
            .await
            .map(|phase| crate::storage::Envelope::phase(phase.name()));
        let receipt = {
            let storage_guard = self.storage.lock().await;
            match storage_guard.as_ref() {
                Some(storage) => match storage
                    .post_to_channel(
                        session_id.as_str(),
                        // `origin = "user"` + no slug: the OOB replay is the
                        // user's own answer, not a host injection, and this is
                        // what `insert_message(Author::User, ..)` resolved to
                        // before the envelope forced the direct call.
                        "user",
                        None,
                        MessageKind::Text.as_str(),
                        // Borrowed, not moved: `body` is returned to the caller
                        // too (see the outcome's field doc). One copy, exactly
                        // as the `&body` this replaced.
                        body.as_str(),
                        envelope,
                    )
                    .await
                {
                    Ok(m) => Some(m),
                    Err(e) => {
                        tracing::warn!(
                            ?e,
                            %session_id,
                            "out-of-band choice-resolution message failed to persist"
                        );
                        None
                    }
                },
                None => {
                    tracing::warn!(
                        %session_id,
                        "resolve_choice: agent receiver gone AND no storage wired — \
                         pick recorded but not delivered"
                    );
                    None
                }
            }
        };
        if let Some(receipt) = &receipt {
            self.notify_message_persisted(Arc::from(session_id.as_str()), receipt.message_id());
        }
        // Without this the row flips to `answered` in the DB but the cached
        // pending counts (bell + tray) never invalidate.
        let _ = self.event_tx.send(SignalingEvent::ChoiceResolved {
            choice_id: choice_id.to_string(),
            picked,
        });
        ResolveOutcome::AgentReceiverDroppedFellBack {
            session_id,
            body,
            receipt,
        }
    }

    /// `(command, answered_at)` for gated commands in this session APPROVED
    /// AFTER `asked_at` — the events that may have overtaken the question now
    /// being replayed (issues.md #18). Oldest-first; the row being resolved is
    /// excluded, and a rejected gate never ran so it is not an overtaking event.
    ///
    /// Fail-open in every direction (no `asked_at`, no storage, query error,
    /// unparseable timestamp → empty): this decorates a replay the agent needs
    /// either way, so it must never be able to fail the delivery. Same posture
    /// as the age-stamp it sits beside.
    ///
    /// Scope is this session on purpose. A push from ANOTHER session can moot a
    /// question here too, but nothing in `session_tray` observes that — it would
    /// need a repo-state watcher, which is a different mechanism (filed, not
    /// built).
    async fn gates_approved_since(
        &self,
        session_id: &str,
        resolving_choice_id: &str,
        asked_at: Option<&str>,
    ) -> Vec<(String, String)> {
        let Some(asked) = asked_at.and_then(parse_tray_ts) else {
            return Vec::new();
        };
        let Some(storage) = self.storage.lock().await.clone() else {
            return Vec::new();
        };
        let Ok(rows) = storage.answered_gates_for_session(session_id).await else {
            return Vec::new();
        };
        rows.into_iter()
            .filter(|row| row.choice_id != resolving_choice_id)
            .filter(|row| {
                matches!(
                    outcome_from_picked(row.picked_option.as_deref().unwrap_or("")),
                    crate::policy::ViolationOutcome::Approved
                )
            })
            .filter_map(|row| {
                let answered_at = row.answered_at?;
                let command = row.command_text?;
                let ts = parse_tray_ts(&answered_at)?;
                (ts > asked).then_some((command, answered_at))
            })
            .collect()
    }

    /// Run an approved action_gate (ToolBlocklist) command at resolve time and
    /// append its output to the OOB `body`. Used on the receiver-gone paths
    /// (client timeout / post-restart) where action_gate's own future was
    /// cancelled before it could execute in-band. `command` is None for any
    /// non-executing tray item; a no-op unless the pick is Approved. Callers
    /// gate this on the atomic status flip so it runs exactly once.
    async fn maybe_run_gated(
        &self,
        session_id: &str,
        command: Option<&str>,
        picked: &str,
        body: &mut String,
    ) {
        let Some(command) = command else { return };
        if !matches!(
            outcome_from_picked(picked),
            crate::policy::ViolationOutcome::Approved
        ) {
            return;
        }
        body.push_str(
            "\n\n(Your action_gate request was approved; bot-hq executed it — output below.)\n",
        );
        match self.execute_gated(session_id, command).await {
            Ok(output) => body.push_str(&output),
            Err(e) => body.push_str(&format!("action_gate could not run `{command}`: {e}")),
        }
    }

    /// If `choice_id` is a PENDING gated command (action_gate / ToolBlocklist)
    /// old enough that the repo context it was asked against has likely moved
    /// on, return its `(command, asked_at)` — i.e. it is STALE and approving it
    /// deserves a confirm step. Returns None for fresh gates, non-command
    /// items, or already-resolved / unknown ids.
    ///
    /// Staleness is AGE-based, not receiver-based: since action_gate parks and
    /// returns immediately, the requesting agent is NEVER live-waiting on a
    /// oneshot — the old `tx.is_closed()` key would have marked every pending
    /// gate stale and forced a confirm on every approve. Execute-on-approve is
    /// now the designed path; the confirm is reserved for prompts that sat
    /// unanswered long enough for the tree to change underneath them.
    async fn stale_gated_command(&self, choice_id: &str) -> Option<(String, Option<String>)> {
        let storage = self.storage.lock().await.clone()?;
        let row = storage.get_tray_entry(choice_id).await.ok()??;
        if row.status != "pending" {
            return None;
        }
        let command = row.command_text?;
        if gate_age_secs(&row.asked_at).is_some_and(|age| age <= STALE_GATE_MAX_AGE_SECS) {
            return None; // fresh — approve executes without a confirm step
        }
        // Older than the window, or unparseable timestamp (treat unknown age as
        // stale: the confirm step is cheap, executing blind is not).
        Some((command, Some(row.asked_at)))
    }

    // (live_waiting_gates removed: with action_gate parking immediately, no
    // gate ever has a live-waiting receiver — staleness is age-based now, via
    // `gate_age_secs` / `STALE_GATE_MAX_AGE_SECS`.)

    /// Snapshot the currently-parked choices. Used by the external MCP driver
    /// so it can see what's waiting for user input + the choice_ids needed to
    /// resolve them.
    pub async fn list_pending_choices(&self) -> Vec<PendingChoice> {
        self.pending
            .lock()
            .await
            .values()
            .map(|p| p.choice.clone())
            .collect()
    }

    /// Shared tail of `mark_awaiting_user` + `request_phase_advance`: flag the
    /// session awaiting, write a durable `kind=halt` row to `session_tray` (so
    /// the per-session tray, dashboard tile counter, and header bell reflect the
    /// wait via `list_pending_tray` — NOT the in-memory pending map, which halts
    /// don't populate — and it survives a restart), then emit `AwaitingUser` so
    /// the duo's peer-forward halts until the user acts.
    async fn emit_halt_row(&self, session_id: String, agent: String, text: String) {
        self.set_session_awaiting(&session_id).await;
        let choice_id = Uuid::new_v4().to_string();
        self.persist_question(
            &session_id,
            &choice_id,
            &agent,
            crate::storage::QuestionKind::Halt,
            &text,
            None,
            None,
            None,
        )
        .await;
        let _ = self.event_tx.send(SignalingEvent::AwaitingUser {
            session_id,
            agent,
            reason: text,
        });
    }

    /// Called by the MCP `tools/call` handler for `mark_awaiting_user` and
    /// `halt`. This is async (was previously sync) because we need to set the
    /// halt flag before the agent's next chunk can volley.
    ///
    /// Returns the prompt of an unanswered halt this agent already had parked,
    /// when there is one — the caller turns it into a warning on the ack. A
    /// halt blocks the session exactly as hard as a question, but carries none
    /// of the question discipline, so the post-batch study found agents
    /// satisfying "don't ask delegable questions" by yielding instead: 6.04h of
    /// one session's 8.15h blocked time was halts, including three in a row
    /// restating one unchanged state. Checked BEFORE the new row is persisted,
    /// or it would find itself.
    pub async fn mark_awaiting_user(
        &self,
        session_id: String,
        agent: String,
        reason: String,
    ) -> Option<String> {
        let prior = self.pending_halt_prompt(&session_id, &agent).await;
        self.emit_halt_row(session_id, agent, reason).await;
        prior
    }

    /// Storage lookup behind the repeat-halt check. Best-effort: a bridge built
    /// without storage (tests) or a failed query simply reports "no prior".
    async fn pending_halt_prompt(&self, session_id: &str, agent: &str) -> Option<String> {
        let storage = self.storage.lock().await.clone()?;
        storage
            .pending_halt_for_agent(session_id, agent)
            .await
            .ok()
            .flatten()
    }

    /// Agent-initiated IPAV phase advance request. Persists a chat message
    /// authored by the requesting agent (so the scroll shows the ask inline)
    /// and a halt question (so the tray + dashboard counter reflect it via the
    /// durable `list_pending_tray`, not the in-memory map), then sets the
    /// awaiting flag so the duo's peer-forward halts until the user acts.
    ///
    /// The user has two response paths, both clear the halt:
    ///   1. Click the phase chip → `AppState::advance_phase` (which also
    ///      clears awaiting + answers pending halt rows).
    ///   2. Type a reply → `AppState::broadcast` (which always clears halt
    ///      on user input). Implicit decline — phase stays put.
    pub async fn request_phase_advance(
        &self,
        session_id: String,
        agent: String,
        target: String,
        reason: String,
    ) {
        let body = format!("[PHASE REQUEST -> {target}] {reason}");
        {
            let storage_guard = self.storage.lock().await;
            if let Some(storage) = storage_guard.as_ref() {
                let author =
                    crate::storage::Author::parse(&agent).unwrap_or(crate::storage::Author::User);
                // The receipt is dropped: this row records the agent's request,
                // and the only thing done with `body` afterwards is
                // `emit_halt_row` — a UI event, not a wire into any agent's
                // stdin. There is no send on this path for a receipt to gate.
                match storage
                    .insert_message(
                        session_id.as_str(),
                        author,
                        crate::storage::MessageKind::Text,
                        &body,
                    )
                    .await
                {
                    Ok(m) => self
                        .notify_message_persisted(Arc::from(session_id.as_str()), m.message_id()),
                    Err(e) => {
                        tracing::warn!(?e, "request_phase_advance insert_message failed")
                    }
                }
            }
        }
        self.emit_halt_row(session_id, agent, body).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::ViolationOutcome;

    #[tokio::test]
    async fn ask_user_choice_parks_and_returns_immediately() {
        // ask_user_choice is non-blocking: it parks the question, halts the duo,
        // and returns a `{status:"parked", choice_id}` ack right away — it does
        // NOT wait for the user. The pick is delivered later out-of-band, and
        // resolving clears the awaiting halt.
        use std::sync::atomic::{AtomicBool, Ordering};
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        bridge
            .register_session_awaiting("s1".into(), Arc::clone(&flag))
            .await;

        let mut sub = bridge.subscribe();
        // Inline (not spawned): returns immediately with the parked ack.
        let ack = bridge
            .ask_user_choice(
                "s1".into(),
                "brian".into(),
                "pick".into(),
                vec!["Yes".into(), "No".into()],
            )
            .await
            .unwrap();
        assert!(ack.contains("\"status\":\"parked\""), "ack: {ack}");
        assert!(ack.contains("choice_id"), "ack: {ack}");
        assert!(
            flag.load(Ordering::Acquire),
            "ask_user_choice must halt the duo while parked"
        );

        let choice_id = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };

        // The parked oneshot's rx dropped when ask returned, so resolve lands via
        // the OOB path: a synthetic user message + awaiting cleared.
        let outcome = bridge.resolve_choice(&choice_id, "Yes".into()).await.unwrap();
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { session_id, body, .. } => {
                assert_eq!(session_id, "s1");
                assert!(
                    body.contains("User picked:") && body.contains("Yes"),
                    "body: {body}"
                );
            }
            other => panic!("non-blocking ask should resolve via OOB, got {other:?}"),
        }
        assert!(
            !flag.load(Ordering::Acquire),
            "resolve must clear the awaiting halt so the duo resumes"
        );
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert!(msgs
            .iter()
            .any(|m| m.content.contains("(out-of-band)") && m.content.contains("Yes")));
    }

    #[tokio::test]
    async fn the_oob_answer_records_the_phase_it_will_be_delivered_with() {
        // B5 Task 2 moved the phase lookup here from `CoreAppState::resolve_choice`,
        // which used to prepend `[PHASE: X]` to the wire AFTER this row was
        // written — so the row said one thing and the agent read another.
        //
        // The wire must be unchanged: `[PHASE: Plan]\n<replay body>`, exactly
        // what `with_phase_envelope(phase, body)` produced downstream before.
        use crate::core::ipav::{IpavPhase, IpavState};
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        let ipav = Arc::new(tokio::sync::Mutex::new(IpavState::default()));
        ipav.lock().await.advance(IpavPhase::Plan);
        bridge
            .register_session_phase("s1".into(), Arc::downgrade(&ipav))
            .await;
        storage
            .insert_tray_entry(
                "s1",
                "cid-phase",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Pick something?",
                Some(&["A".to_string(), "B".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();

        let outcome = bridge.resolve_choice("cid-phase", "A".into()).await.unwrap();
        let ResolveOutcome::AgentReceiverDroppedFellBack { body, receipt, .. } = outcome else {
            panic!("expected the OOB fallback path");
        };
        let receipt = receipt.expect("storage is wired, so the answer became a row");
        assert_eq!(receipt.body(), body, "the receipt is for THIS answer");
        assert_eq!(receipt.wire(), format!("[PHASE: Plan]\n{body}"));

        // Unregistering (session closed) is the honest degradation: no phase is
        // known, so the row records — and the agent would read — an untagged
        // body rather than a phase the session may have left.
        drop(ipav);
        storage
            .insert_tray_entry(
                "s1",
                "cid-closed",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Pick again?",
                Some(&["A".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();
        let outcome = bridge.resolve_choice("cid-closed", "A".into()).await.unwrap();
        let ResolveOutcome::AgentReceiverDroppedFellBack { body, receipt, .. } = outcome else {
            panic!("expected the OOB fallback path");
        };
        assert_eq!(receipt.unwrap().wire(), body);
    }

    #[tokio::test]
    async fn repeat_halt_with_no_user_reply_reports_the_prior() {
        // The treadmill the post-batch study found: three halts in a row
        // restating one unchanged state, blocking the session each time. A
        // still-pending halt row means the user hasn't replied (user input
        // clears halts), so the second yield is a repeat.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();

        let first = bridge
            .mark_awaiting_user("s1".into(), "brian".into(), "temp.md ready, awaiting go".into())
            .await;
        assert!(first.is_none(), "the first yield has no prior halt");

        let second = bridge
            .mark_awaiting_user("s1".into(), "brian".into(), "temp.md still ready".into())
            .await;
        assert_eq!(
            second.as_deref(),
            Some("temp.md ready, awaiting go"),
            "a second yield with no user reply must surface the earlier one"
        );
    }

    #[tokio::test]
    async fn halt_after_the_user_replies_is_not_a_repeat() {
        // Answering the halt row is what the user's reply does; once it's no
        // longer pending, the next yield is a fresh state and must stay silent.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();

        bridge
            .mark_awaiting_user("s1".into(), "brian".into(), "first".into())
            .await;
        // The real path a user reply takes (core::state::broadcast), not a
        // stand-in — this is the mechanism the guard's "still pending means
        // unanswered" assumption depends on.
        let cleared = storage.clear_pending_halts("s1").await.unwrap();
        assert_eq!(cleared, 1, "the halt row should have been pending");

        let after = bridge
            .mark_awaiting_user("s1".into(), "brian".into(), "second".into())
            .await;
        assert!(after.is_none(), "a yield after the user acted is not a repeat");
    }

    #[tokio::test]
    async fn repeat_halt_check_is_per_agent() {
        // Rain halting doesn't make Brian's next halt a repeat — the discipline
        // is about one agent yielding twice on its own unchanged state.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();

        bridge
            .mark_awaiting_user("s1".into(), "rain".into(), "rain waits".into())
            .await;
        let brian = bridge
            .mark_awaiting_user("s1".into(), "brian".into(), "brian waits".into())
            .await;
        assert!(brian.is_none(), "another agent's halt is not this agent's repeat");
    }

    #[tokio::test]
    async fn mark_awaiting_user_broadcasts() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        bridge
            .mark_awaiting_user("s1".into(), "brian".into(), "ping".into())
            .await;
        let ev = sub.recv().await.unwrap();
        assert!(
            matches!(ev, SignalingEvent::AwaitingUser { session_id, agent, reason }
            if session_id == "s1" && agent == "brian" && reason == "ping")
        );
    }

    #[tokio::test]
    async fn set_session_awaiting_refreshes_registered_activity_tracker() {
        // Bug B: parking a question must reflect the awaiting flip into the
        // derived activity IMMEDIATELY (emit AwaitingUser) via the registered
        // tracker — not wait for the agent's next set_busy (the dot-lag bug).
        use std::sync::atomic::AtomicBool;
        let bridge = SignalingBridge::new();
        let awaiting = Arc::new(AtomicBool::new(false));
        bridge
            .register_session_awaiting("s1".into(), Arc::clone(&awaiting))
            .await;
        // A real tracker sharing the same awaiting flag, registered as a Weak.
        let tracker = ActivityTracker::new("s1", Arc::clone(&awaiting), bridge.clone());
        bridge
            .register_session_activity("s1".into(), Arc::downgrade(&tracker))
            .await;

        let mut sub = bridge.subscribe();
        bridge
            .mark_awaiting_user("s1".into(), "brian".into(), "ping".into())
            .await;

        // refresh() fires inside set_session_awaiting (before the AwaitingUser
        // event) → a SessionActivity{awaiting_user} must be among the emitted events.
        let mut saw_activity = false;
        while let Ok(ev) = sub.try_recv() {
            if let SignalingEvent::SessionActivity {
                session_id, state, ..
            } = ev
            {
                if session_id == "s1" && state == "awaiting_user" {
                    saw_activity = true;
                }
            }
        }
        assert!(
            saw_activity,
            "set_session_awaiting must refresh the registered tracker → emit SessionActivity awaiting_user"
        );
    }

    #[tokio::test]
    async fn resolve_unknown_choice_errors() {
        // No storage + no parked oneshot → genuinely unknown id → error.
        let bridge = SignalingBridge::new();
        let err = bridge.resolve_choice("nope", "x".into()).await.unwrap_err();
        assert!(err.to_string().contains("no pending choice"));
    }

    #[tokio::test]
    async fn resolve_reopened_session_choice_falls_back_to_oob() {
        // #2: after close+reopen the user may answer a choice_id whose parked
        // oneshot died with the old subprocess. The durable question row still
        // exists. resolve_choice must NOT error — it reconstructs the question
        // and returns the OOB fallback so the answer reaches the live agent.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s-reopen", "t", None).await.unwrap();
        let opts = vec!["Yes".to_string(), "No".to_string()];
        storage
            .insert_tray_entry(
                "s-reopen",
                "old-choice-id",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Ship it?",
                Some(&opts),
                None,
                None,
            )
            .await
            .unwrap();

        // No parked oneshot in the in-memory map (post-reopen state).
        let outcome = bridge
            .resolve_choice("old-choice-id", "Yes".into())
            .await
            .expect("reopened-session resolve should fall back, not error");
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { session_id, body, .. } => {
                assert_eq!(session_id, "s-reopen");
                assert!(body.contains("Ship it?"), "body: {body}");
                assert!(body.contains("Yes"), "body: {body}");
            }
            other => panic!("expected OOB fallback, got {other:?}"),
        }
        // OOB message persisted for the agent to read on its next turn.
        let msgs = storage
            .messages_for_session("s-reopen", None)
            .await
            .unwrap();
        assert!(msgs
            .iter()
            .any(|m| m.content.contains("(out-of-band)") && m.content.contains("Yes")));
        // Question row marked answered so the tray clears.
        let q = storage
            .get_tray_entry("old-choice-id")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(q.status, "answered");
    }

    #[tokio::test]
    async fn resolve_choice_oob_emits_choice_resolved() {
        // Regression: the out-of-band resolve paths (agent timed out, or a
        // post-restart reopened session) must emit ChoiceResolved so the bell /
        // tray caches invalidate. The in-band path emits it via the inner ask
        // future; the OOB branches used to only persist a synthetic message
        // (MessagePersisted → agent:messages:batch, which the UI excludes from
        // tray invalidation), leaving the bell stuck on a stale count.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s-oob", "t", None).await.unwrap();
        let opts = vec!["Yes".to_string(), "No".to_string()];
        storage
            .insert_tray_entry(
                "s-oob",
                "cid-oob",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Ship it?",
                Some(&opts),
                None,
                None,
            )
            .await
            .unwrap();

        // No parked oneshot → exercises the `None` OOB branch.
        let mut sub = bridge.subscribe();
        bridge.resolve_choice("cid-oob", "Yes".into()).await.unwrap();

        // Drain buffered events; one must be ChoiceResolved for our choice.
        let mut saw_resolved = false;
        while let Ok(ev) = sub.try_recv() {
            if let SignalingEvent::ChoiceResolved { choice_id, picked } = ev {
                if choice_id == "cid-oob" && picked == "Yes" {
                    saw_resolved = true;
                    break;
                }
            }
        }
        assert!(
            saw_resolved,
            "OOB resolve must emit ChoiceResolved so the bell/tray invalidate"
        );
    }

    #[tokio::test]
    async fn resolve_after_agent_drop_falls_back_to_message() {
        // Simulates: agent calls ask_user_choice → claude-code MCP client
        // times out → drops the receiver. Some time later the orchestrator
        // calls resolve_choice. We expect Ok + a synthetic user message
        // persisted to storage so the agent learns the answer on next poll.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        // Seed a session row so the FK in messages is satisfied.
        storage
            .create_session("s-fallback", "title", None)
            .await
            .unwrap();

        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let asker = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s-fallback".into(),
                    "brian".into(),
                    "Pick something?".into(),
                    vec!["A".into(), "B".into()],
                )
                .await
        });
        // Grab the choice_id from the broadcast event.
        let choice_id = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        // Simulate client-side timeout: abort the agent's future, then yield
        // so the drop runs and the oneshot::Receiver is gone.
        asker.abort();
        let _ = asker.await; // collect the JoinError; we expect Aborted.
        tokio::task::yield_now().await;

        // Orchestrator resolves the (now-orphaned) choice.
        let outcome = bridge
            .resolve_choice(&choice_id, "A".into())
            .await
            .expect("resolve_choice should succeed even when agent receiver dropped");

        // Verify we surfaced the wake info to the caller so CoreAppState can
        // route the body through input_tx and actually unblock the subprocess.
        match outcome {
            ResolveOutcome::AgentReceiverDroppedFellBack { session_id, body, .. } => {
                assert_eq!(session_id, "s-fallback");
                assert!(body.contains("User picked:"));
                assert!(body.contains("A"));
            }
            other => panic!("expected AgentReceiverDroppedFellBack, got {other:?}"),
        }

        // Verify the out-of-band message also landed in storage (for UI + poll).
        let msgs = storage
            .messages_for_session("s-fallback", None)
            .await
            .unwrap();
        let oob = msgs
            .iter()
            .find(|m| m.content.contains("(out-of-band)"))
            .expect("expected synthetic out-of-band message");
        assert_eq!(oob.author, "user");
        assert!(oob.content.contains("User picked:"));
        assert!(oob.content.contains("A"));

        // The OOB insert must fire MessagePersisted so the chat reflects the
        // answer live (event-driven), not only after a manual tab-switch.
        let mut saw_persisted = false;
        for _ in 0..8 {
            match sub.try_recv() {
                Ok(SignalingEvent::MessagePersisted { session_id, .. })
                    if session_id.as_ref() == "s-fallback" =>
                {
                    saw_persisted = true;
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        assert!(
            saw_persisted,
            "OOB resolve must fire MessagePersisted so the chat live-updates"
        );
    }

    #[tokio::test]
    async fn oob_replay_names_gates_approved_after_the_question_was_parked() {
        // The s-bb938f62 shape (issues.md #18): a question sits parked while the
        // action it asks about is approved through a SEPARATE gate, then the
        // stale answer replays. The replay must name that gate so the agent
        // doesn't adopt the dead premise as current state.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s-moot", "title", None).await.unwrap();
        let opts = vec!["Push".to_string(), "discard".to_string()];

        // Question parked FIRST — no in-memory oneshot, so resolve takes the
        // reconstruct-from-storage path into deliver_oob.
        storage
            .insert_tray_entry(
                "s-moot",
                "cid-q",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Re-push to staging?",
                Some(&opts),
                None,
                None,
            )
            .await
            .unwrap();
        // now_utc() is millisecond-precision; nudge past it so the gate's
        // answered_at is strictly after the question's asked_at even when the
        // in-memory DB answers both writes inside one millisecond.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        for (cid, command, pick) in [
            ("cid-gate-ok", "git push origin staging", "Approve"),
            ("cid-gate-no", "git reset --hard origin/main", "Reject"),
        ] {
            storage
                .insert_tray_entry(
                    "s-moot",
                    cid,
                    "brian",
                    crate::storage::QuestionKind::Choice,
                    "Run gated command?",
                    Some(&opts),
                    None,
                    Some(command),
                )
                .await
                .unwrap();
            storage.answer_tray_entry(cid, pick).await.unwrap();
        }

        let outcome = bridge
            .resolve_choice("cid-q", "discard".into())
            .await
            .expect("stale question resolves through the storage path");
        let ResolveOutcome::AgentReceiverDroppedFellBack { body, .. } = outcome else {
            panic!("expected the OOB fallback path");
        };
        assert!(body.contains("**Approved in this session after you asked:**"));
        assert!(body.contains("git push origin staging"));
        // A REJECTED gate never ran — naming it would invent an event.
        assert!(!body.contains("git reset --hard"));
        // And the block must not claim success, only approval: an
        // approved-but-FAILED command leaves an identical tray row.
        assert!(body.contains("whether it succeeded is not recorded"));
    }

    #[tokio::test]
    async fn oob_replay_omits_the_block_when_nothing_was_approved_after_the_ask() {
        // Guard against the inverse failure: decorating an ordinary replay with
        // an overtaking-event warning that has no event behind it.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s-clean", "title", None).await.unwrap();
        let opts = vec!["A".to_string(), "B".to_string()];

        // Gate approved BEFORE the question was parked — it cannot have
        // overtaken a question that did not exist yet.
        storage
            .insert_tray_entry(
                "s-clean",
                "cid-gate-early",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Run gated command?",
                Some(&opts),
                None,
                Some("git push origin main"),
            )
            .await
            .unwrap();
        storage
            .answer_tray_entry("cid-gate-early", "Approve")
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        storage
            .insert_tray_entry(
                "s-clean",
                "cid-q",
                "brian",
                crate::storage::QuestionKind::Choice,
                "Pick something?",
                Some(&opts),
                None,
                None,
            )
            .await
            .unwrap();

        let outcome = bridge.resolve_choice("cid-q", "A".into()).await.unwrap();
        let ResolveOutcome::AgentReceiverDroppedFellBack { body, .. } = outcome else {
            panic!("expected the OOB fallback path");
        };
        assert!(!body.contains("Approved in this session after you asked"));
        assert!(!body.contains("git push origin main"));
        assert!(body.contains("**User picked:** A"));
    }

    #[tokio::test]
    async fn request_approval_blocks_and_returns_pick_in_band() {
        // Contrast with ask_user_choice: request_approval (and the pre-push gate)
        // BLOCKS — the caller's await returns the user's pick directly, in-band,
        // and resolve_choice reports Delivered. This is the synchronous path a
        // git hook depends on, and it must NOT regress to the parked-ack form.
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    "s1".into(),
                    "brian".into(),
                    "Approve push?".into(),
                    vec!["Approve".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push".into(),
                        detail: None,
                    },
                )
                .await
                .unwrap()
        });
        let choice_id = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        let outcome = bridge
            .resolve_choice(&choice_id, "Approve".into())
            .await
            .unwrap();
        let picked = ask.await.unwrap();
        assert_eq!(picked, "Approve", "blocking call returns the pick in-band");
        assert!(matches!(outcome, ResolveOutcome::Delivered));
    }

    #[tokio::test]
    async fn request_approval_parked_returns_immediately() {
        // The agent-facing twin must NOT hold the call open. Before this split
        // both callers blocked, so an agent's MCP client timed out at ~60s while
        // the human was still deciding and could not tell queued from failed —
        // it fired live on a production query (2026-07-28T15:55Z). No resolve
        // happens here: the await alone has to come back.
        let bridge = SignalingBridge::new();
        let ack = bridge
            .request_approval_parked(
                "s1".into(),
                "brian".into(),
                "Query prod?".into(),
                vec!["Approve".into(), "Deny".into()],
                ApprovalContext {
                    kind: ViolationKind::PerAction,
                    action: "bq query ...".into(),
                    detail: None,
                },
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&ack).expect("parked ack is JSON");
        assert_eq!(
            v.get("status").and_then(|s| s.as_str()),
            Some("parked"),
            "agent path parks instead of blocking"
        );
        assert!(
            v.get("choice_id").and_then(|s| s.as_str()).is_some(),
            "parked ack carries the choice_id so gate_status can find it"
        );
    }

    #[tokio::test]
    async fn resolve_choice_delivered_clears_awaiting() {
        // Regression for "the duo goes silent after the user answers": a Delivered
        // resolve must clear the awaiting halt the gate set, or the router keeps
        // dropping every Brian<->Rain peer-forward (router::route_forward is gated on
        // this flag). Uses request_approval (the blocking path that yields
        // Delivered); the non-blocking ask_user_choice clears awaiting on its OOB
        // resolve too — see ask_user_choice_parks_and_returns_immediately.
        use std::sync::atomic::{AtomicBool, Ordering};
        let bridge = SignalingBridge::new();
        let flag = Arc::new(AtomicBool::new(false));
        bridge
            .register_session_awaiting("s1".into(), Arc::clone(&flag))
            .await;
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    "s1".into(),
                    "brian".into(),
                    "Approve push?".into(),
                    vec!["Approve".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push".into(),
                        detail: None,
                    },
                )
                .await
                .unwrap()
        });
        let choice_id = loop {
            match sub.recv().await.unwrap() {
                SignalingEvent::PendingChoice(p) => break p.choice_id,
                _ => continue,
            }
        };
        // The gate halts the duo; set_session_awaiting runs before the
        // PendingChoice event emits, so this read is race-free.
        assert!(
            flag.load(Ordering::Acquire),
            "request_approval should set the awaiting halt"
        );
        let outcome = bridge
            .resolve_choice(&choice_id, "Approve".into())
            .await
            .unwrap();
        let _ = ask.await.unwrap();
        assert!(matches!(outcome, ResolveOutcome::Delivered));
        assert!(
            !flag.load(Ordering::Acquire),
            "a Delivered resolve must clear the awaiting halt so the duo resumes"
        );
    }

    #[tokio::test]
    async fn request_approval_records_violation_on_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    "s1".into(),
                    "brian".into(),
                    "Approve push?".into(),
                    vec!["Approve once".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push origin main".into(),
                        detail: Some("per_branch_approval".into()),
                    },
                )
                .await
                .unwrap()
        });
        let ev = sub.recv().await.unwrap();
        let choice_id = match ev {
            SignalingEvent::PendingChoice(p) => {
                assert!(p.approval.is_some());
                p.choice_id
            }
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge
            .resolve_choice(&choice_id, "Approve once".into())
            .await
            .unwrap();
        let picked = ask.await.unwrap();
        assert_eq!(picked, "Approve once");

        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, ViolationKind::PushGate);
        assert_eq!(recs[0].outcome, ViolationOutcome::Approved);
        assert_eq!(recs[0].action, "git push origin main");
    }

    #[tokio::test]
    async fn deny_picked_records_denied_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .request_approval(
                    "s1".into(),
                    "brian".into(),
                    "Approve force-push?".into(),
                    vec!["Approve".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::ForcePush,
                        action: "git push --force origin main".into(),
                        detail: None,
                    },
                )
                .await
                .unwrap()
        });
        let ev = sub.recv().await.unwrap();
        let choice_id = match ev {
            SignalingEvent::PendingChoice(p) => p.choice_id,
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge
            .resolve_choice(&choice_id, "Deny".into())
            .await
            .unwrap();
        ask.await.unwrap();
        let recs = log.read_all().unwrap();
        assert_eq!(recs[0].outcome, ViolationOutcome::Denied);
    }

    #[tokio::test]
    async fn ask_user_choice_does_not_write_violation() {
        let dir = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let ask = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s1".into(),
                    "brian".into(),
                    "pick".into(),
                    vec!["a".into(), "b".into()],
                )
                .await
                .unwrap()
        });
        let ev = sub.recv().await.unwrap();
        let choice_id = match ev {
            SignalingEvent::PendingChoice(p) => {
                assert!(p.approval.is_none());
                p.choice_id
            }
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge.resolve_choice(&choice_id, "a".into()).await.unwrap();
        ask.await.unwrap();
        let recs = log.read_all().unwrap();
        assert!(recs.is_empty(), "plain ask_user_choice should not log");
    }

    #[tokio::test]
    async fn ask_user_choice_auto_supersedes_reask_same_prompt() {
        // G2: when the same agent re-asks the SAME question (timeout-retry), the
        // prior pending row flips to 'superseded' and the new row links back via
        // supersedes_id — so a re-issue doesn't duplicate in the tray. Match is
        // on prompt: a re-ask has the same prompt.
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("test.db")).await.unwrap();
        storage.create_session("s1", "test", None).await.unwrap();

        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;

        let bridge_clone = Arc::clone(&bridge);
        let first = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s1".into(),
                    "brian".into(),
                    "same question".into(),
                    vec!["a".into(), "b".into()],
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Re-ask the SAME prompt → supersedes the first.
        let bridge_clone = Arc::clone(&bridge);
        let second = tokio::spawn(async move {
            bridge_clone
                .ask_user_choice(
                    "s1".into(),
                    "brian".into(),
                    "same question".into(),
                    vec!["a".into(), "b".into()],
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let rows = storage.tray_entries_for_session("s1").await.unwrap();
        assert_eq!(rows.len(), 2, "two question rows expected");
        let first_row = &rows[0];
        let second_row = &rows[1];
        assert_eq!(first_row.status, "superseded");
        assert_eq!(second_row.status, "pending");
        assert_eq!(
            second_row.supersedes_id,
            Some(first_row.id),
            "new row should link back to the superseded row"
        );

        bridge
            .resolve_choice(&second_row.choice_id, "a".into())
            .await
            .unwrap();
        let _ = first.await.unwrap();
        let _ = second.await.unwrap();
    }

    #[tokio::test]
    async fn distinct_prompts_accumulate_not_superseded() {
        // The AFK-accumulate goal: two DIFFERENT questions from the same agent
        // both stay pending — auto-supersede only collapses a true re-ask of the
        // same prompt, not distinct questions.
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("test.db")).await.unwrap();
        storage.create_session("s1", "test", None).await.unwrap();

        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;

        let b1 = Arc::clone(&bridge);
        let q1 = tokio::spawn(async move {
            b1.ask_user_choice(
                "s1".into(),
                "brian".into(),
                "question one".into(),
                vec!["a".into(), "b".into()],
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let b2 = Arc::clone(&bridge);
        let q2 = tokio::spawn(async move {
            b2.ask_user_choice(
                "s1".into(),
                "brian".into(),
                "question two".into(),
                vec!["a".into(), "b".into()],
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let rows = storage.tray_entries_for_session("s1").await.unwrap();
        let pending: Vec<_> = rows.iter().filter(|r| r.status == "pending").collect();
        assert_eq!(
            pending.len(),
            2,
            "distinct prompts must both stay pending, got: {rows:?}"
        );

        // Clean up both parked oneshots.
        for r in &rows {
            let _ = bridge.resolve_choice(&r.choice_id, "a".into()).await;
        }
        let _ = q1.await.unwrap();
        let _ = q2.await.unwrap();
    }

    #[tokio::test]
    async fn supersede_question_links_old_to_new() {
        // G1: the explicit supersede tool replaces a SPECIFIC stale by
        // choice_id and links the new row to it via supersedes_id.
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("test.db")).await.unwrap();
        storage.create_session("s1", "test", None).await.unwrap();

        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;

        // Seed a stale question directly via storage (skip the auto-
        // supersede path so we have a clean "stale exists, nothing else
        // pending" state for the explicit tool to target).
        storage
            .insert_tray_entry(
                "s1",
                "stale-cid",
                "brian",
                crate::storage::QuestionKind::Choice,
                "stale prompt",
                Some(&["a".to_string(), "b".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();

        let bridge_clone = Arc::clone(&bridge);
        let supersede = tokio::spawn(async move {
            bridge_clone
                .supersede_question_with_new(
                    "s1".into(),
                    "brian".into(),
                    "stale-cid".into(),
                    "rephrased".into(),
                    vec!["x".into(), "y".into()],
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let rows = storage.tray_entries_for_session("s1").await.unwrap();
        assert_eq!(rows.len(), 2);
        let stale = &rows[0];
        let fresh = &rows[1];
        assert_eq!(stale.choice_id, "stale-cid");
        assert_eq!(stale.status, "superseded");
        assert_eq!(fresh.prompt, "rephrased");
        assert_eq!(fresh.status, "pending");
        assert_eq!(fresh.supersedes_id, Some(stale.id));

        bridge
            .resolve_choice(&fresh.choice_id, "x".into())
            .await
            .unwrap();
        // supersede_question_with_new is non-blocking like ask_user_choice: it
        // returns a parked ack, not the pick (which arrives out-of-band).
        let ack = supersede.await.unwrap().unwrap();
        assert!(ack.contains("\"status\":\"parked\""), "ack: {ack}");
    }
}
