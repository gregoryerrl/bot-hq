//! Signaling tools that park a tray entry: `ask_user_choice` (non-blocking —
//! parks and returns a `{parked}` ack; the pick arrives out-of-band) and
//! `request_approval` (the agent-facing path parks too and latches the ring;
//! the blocking form awaits the user's oneshot and is what the pre-push hook
//! route uses), plus their
//! supersede + resolve machinery, `mark_awaiting_user`,
//! `request_phase_advance`, and the pending-choice snapshots. This is the
//! biggest slice of the bridge — everything that parks a oneshot, mirrors a
//! tray row, or sets the session's awaiting-halt flag.

use super::util::{gate_verdict, oob_resolution_body, outcome_from_picked, parse_tray_ts};
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
///
/// The two-branch parse is [`parse_tray_ts`], which this file already imports
/// (round-2 audit G4). It used to be hand-rolled here — same two branches, same
/// fallback, same `None` — eight lines from the helper sitting at the top of the
/// same module. Worth naming rather than quietly deleting: the duplicate is what
/// makes a tolerance change land in one of two places, and the tolerance is the
/// thing this pair exists for.
pub fn gate_age_secs(asked_at: &str) -> Option<i64> {
    Some((chrono::Utc::now() - parse_tray_ts(asked_at)?).num_seconds())
}

/// What [`SignalingBridge::withdraw_question_for`] did — the three states an
/// agent's `withdraw_question` answer must keep apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withdrawal {
    /// A pending row (in-memory park or the durable tray row) was withdrawn.
    Withdrawn,
    /// Nothing pending under that id (unknown, or already resolved/withdrawn).
    NotPending,
    /// The row is pending but was parked by another participant — untouched.
    NotYours,
    /// The row's owner could not be READ (a storage error), so the "yours
    /// only" scoping could not be applied — untouched (round 10). This is the
    /// tool's one control (`withdraw_question` is deliberately ungated because
    /// this scoping IS the gate), so a read failure refuses rather than
    /// letting the withdrawal through against a row that may be a peer's.
    Unverifiable,
}

/// The command the app RUNS for an approved gate whose asker can no longer run
/// it: a Tool-Gate command (`action` — the agent's blocked Bash), or a push
/// gate's rebuilt `git push` (`ApprovalContext::command`, round 12). `None` for
/// every other approval — nothing executes on a generic yes.
fn executable_command(ctx: &super::ApprovalContext) -> Option<String> {
    match ctx.kind {
        crate::policy::ViolationKind::ToolBlocklist => Some(ctx.action.clone()),
        // Explicit, not a catch-all (EYES, round 12): a future kind that sets
        // `command` must opt in here — nothing becomes executable by default.
        crate::policy::ViolationKind::PushGate => ctx.command.clone(),
        _ => None,
    }
}

/// Bound for a late-approved push re-run: a network push of a real branch,
/// not a local command — `DEFAULT_TIMEOUT` (120 s) is the action_gate bound
/// and too short for the first network op to ride `command_text` (EYES F11).
/// Stated in the OOB row that carries the output.
const PUSH_RERUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

impl SignalingBridge {
    /// Flag the session as awaiting the user, and — for a park that YIELDS the
    /// session — tell the ring who is now blocked.
    ///
    /// `asker` is the caller's slug. It is resolved to a participant id here
    /// because this is the layer that holds both the session and the roster; the
    /// ring holds neither, and an id it cannot resolve is a state it should not
    /// have to reason about. An unresolvable slug sends `None`, which the ring
    /// treats as "halt outright" — the old behaviour, and the safe direction.
    async fn set_session_awaiting(&self, session_id: &str, asker: &str, halt_ring: bool) {
        if let Some(flag) = self.session_awaiting.lock().await.get(session_id) {
            flag.store(true, Ordering::Release);
        }
        // HALT THE RING, not just the cursors (rc3 D35: a halt is a halt — the
        // ring stops where it stands, no lap).
        //
        // `halt_ring` is false for `request_approval` / the action gate: those
        // park through the GATE latch instead (`notify_ring_gate`), which also
        // parks the ring but is lifted by ANSWERING the gate rather than by a
        // user message. Same outcome the user decreed — "Approval gate halts
        // the session" — different release.
        //
        // `try_send`, not `send`: this runs inside a tool call that must not
        // block on the ring's queue, and a full channel already has a halt or a
        // completion in it. A closed channel means the session is tearing down.
        let seq = if halt_ring {
            self.session_sequencer.lock().await.get(session_id).cloned()
        } else {
            None
        };
        if let Some(tx) = seq {
            // WHO declared it — the holder declaring ends its turn; a
            // non-holder leaves the live turn alone (rc3 D35).
            let participant_id = {
                let storage_guard = self.storage.lock().await;
                let storage = storage_guard.clone();
                drop(storage_guard);
                match storage {
                    Some(s) => s
                        .participant_by_slug(session_id, asker)
                        .await
                        .ok()
                        .flatten()
                        .map(|p| p.id),
                    None => None,
                }
            };
            if participant_id.is_none() {
                tracing::warn!(
                    session_id,
                    asker,
                    "a parked question named a participant the roster does not hold; \
                     the whole cycle halts rather than finishing its lap"
                );
            }
            if tx
                .try_send(crate::core::sequencer::SequencerCommand::HaltDeclared {
                    participant_id,
                })
                .is_err()
            {
                tracing::warn!(
                    session_id,
                    "parked question did not reach the ring — the cycle may keep \
                     handing out turns nobody can use"
                );
            }
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
            session_id, agent, question, options, None, supersedes_id, false, false,
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
        // A host GATE: it blocks the session (latch + gate slot).
        self.request_approval_inner(session_id, agent, question, options, ctx, true, true)
            .await
    }

    /// An AGENT's approval request, PARKED — the agent-facing path, and since
    /// round 12 a TRAY item, not a gate (the user's split: "request_approval
    /// is tray parkable, approval_gates are session blockers"). Same
    /// violation-recording machinery as [`Self::request_approval`], any menu
    /// (the agent's own labels), returns the `{"status":"parked","choice_id":…}`
    /// acknowledgment at once and delivers the pick out-of-band as a user row
    /// — exactly like `ask_user_choice`, which is what it now is plus an audit
    /// record. It latches NOTHING: the session keeps working and the agent
    /// waits for the answer row before acting. The host's gates (`action_gate`,
    /// the push hook, the reviewer-down override) are the session blockers.
    pub async fn request_approval_parked(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        ctx: ApprovalContext,
    ) -> Result<String> {
        self.request_approval_inner(session_id, agent, question, options, ctx, false, false)
            .await
    }

    /// Shared body of the two entry points above. `blocking` and `gate` are
    /// the differences between them — see their docs for which caller gets
    /// which: the hook route blocks AND gates; the agent tool parks a request.
    async fn request_approval_inner(
        &self,
        session_id: String,
        agent: String,
        question: String,
        options: Vec<String>,
        ctx: ApprovalContext,
        blocking: bool,
        gate: bool,
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
            gate,
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
        // ONE read of the stale row: its internal id (the new row's
        // `supersedes_id` FK) and — round 11 — its owner. "Replace a stale
        // question YOU parked" is the tool's promise, and `withdraw_question`
        // already refuses a peer's row; this path retired any row by
        // choice_id, from any participant, in any session. A read ERROR
        // refuses too (the scoping is the only control here); a missing row
        // falls through — nothing to protect, and the new question is still
        // the caller's to park.
        let storage = self.storage.lock().await.clone();
        let stale_internal_id = match &storage {
            Some(storage) => match storage.get_tray_entry(&stale_choice_id).await {
                Ok(Some(row)) => {
                    // The answer-vs-supersede race (Batch 9 T3, dissect #17):
                    // the user's pick for this row landed ten seconds after an
                    // agent superseded it — the answer was delivered against a
                    // retired row (answered_at NULL, the pick stranded) and
                    // the replacement was never seen. A row that is no longer
                    // pending has an answer in flight or landed: REFUSE and
                    // point the agent at reading it instead of replacing it.
                    // (A pick still STAGED client-side is invisible here —
                    // that half of the race closes at delivery, which answers
                    // the original row whatever happened since.)
                    if row.status != "pending" {
                        anyhow::bail!(
                            "not superseded: {stale_choice_id} is '{}' — an answer landed                              or is in flight; read the user's pick instead of replacing                              the question",
                            row.status
                        );
                    }
                    if row.agent != agent || row.session_id != session_id {
                        tracing::warn!(
                            %stale_choice_id,
                            asker = %agent,
                            owner = %row.agent,
                            "refusing to supersede a question parked by another participant or session"
                        );
                        anyhow::bail!(
                            "not superseded: {stale_choice_id} was parked by another participant \
                             (or in another session); you can only supersede your own questions"
                        );
                    }
                    // A GATE is not a question (round 12): it latched the ring
                    // when it opened, and retiring it here would leave that
                    // latch with no lift — the same leak the auto-supersede
                    // path had. An approval is answered in the gate, or
                    // withdrawn with `withdraw_question` (which does release
                    // the latch); it is never replaced by a question.
                    if crate::storage::is_gate_row(&row.kind, row.options_json.as_deref()) {
                        anyhow::bail!(
                            "not superseded: {stale_choice_id} is an approval gate, not a \
                             question — it is answered in the gate (or withdrawn with \
                             withdraw_question); park your new question with ask_user_choice"
                        );
                    }
                    Some(row.id)
                }
                Ok(None) => None,
                Err(e) => {
                    tracing::warn!(?e, %stale_choice_id, "supersede: the stale row could not be read; refusing");
                    anyhow::bail!("not superseded: {stale_choice_id} could not be read; try again");
                }
            },
            None => None,
        };
        // Flip status + drop parked oneshot + fire ChoiceResolved for the UI.
        if let Some(storage) = storage {
            if let Err(e) = storage.supersede_tray_entry(&stale_choice_id).await {
                tracing::warn!(?e, %stale_choice_id, "supersede (explicit) storage update failed");
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
        let storage = self.storage.lock().await.clone()?;
        let rows = storage.tray_entries_for_session(session_id).await.ok()?;
        let latest = rows
            .into_iter()
            .rev()
            .find(|q| q.agent == agent && q.status == "pending" && q.prompt == prompt)?;
        let stale_choice_id = latest.choice_id.clone();
        let stale_internal_id = latest.id;
        let stale_was_gate =
            crate::storage::is_gate_row(&latest.kind, latest.options_json.as_deref());
        // Mark in storage first so the UI tray drops it on its next poll.
        if let Err(e) = storage.supersede_tray_entry(&stale_choice_id).await {
            tracing::warn!(?e, %stale_choice_id, "supersede (auto) storage update failed");
        }
        // Drop the parked oneshot so any (rare) still-listening client gets
        // the standard cancellation.
        self.pending.lock().await.remove(&stale_choice_id);
        // **A retired GATE releases the latch it opened** (round 12). The ring
        // drops an id from `open_gates` only on `GateResolved`, and this path
        // used to retire the row, the oneshot and the UI state without sending
        // one — so a byte-identical re-park from the same agent (the pre-push
        // prompt, re-pushed after a client-timeout kill) left the stale id
        // latched: answering the NEW gate lifted nothing and the session read
        // "dealing is parked" until restart. Sent BEFORE the new row's
        // `GateOpened`, so the ring's set never holds a dead id beside a live
        // one. `superseding_a_pending_approval_releases_its_gate_latch` pins it.
        if stale_was_gate {
            self.notify_ring_gate(session_id, &stale_choice_id, false).await;
        }
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
        // `true` = a session-blocking GATE (the host's: a Tool-Gate park, the
        // push hook, the reviewer-down override): `kind = approval`, the ring
        // latches, the gate slot renders it. `false` with an approval context
        // = an agent's `request_approval` (round 12): `kind = request`, a tray
        // item, audited, latching nothing. Ignored when `approval` is None.
        gate: bool,
    ) -> Result<String> {
        let choice_id = Uuid::new_v4().to_string();
        // Persist the command for an action_gate (ToolBlocklist) approval so it
        // can still execute on approve after the in-memory oneshot is gone
        // (client timeout / restart). Extracted before `approval` moves into
        // PendingChoice below. Round 12: a push gate carries its rebuilt,
        // sha-pinned `git push` the same way (`ApprovalContext::command`), for
        // the late approve whose hook has died.
        let command_text = approval.as_ref().and_then(executable_command);
        // Captured before `approval` moves into the park below: the gate latch
        // keys on this (rc3 D35) — AND on `gate` (round 12): an approval
        // context makes the row AUDITED; `gate` makes it a BLOCKER. The host's
        // gates pass true; an agent's `request_approval` passes false and
        // parks a `request` row in the tray.
        let is_approval = approval.is_some();
        let is_gate = is_approval && gate;
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
                gate: is_gate,
            },
        );

        // Mirror into the question cache for durable per-session state. The
        // in-chat tray + dashboard counter read from this table, and the row
        // survives restart even though the parked oneshot in `pending` does
        // not. Best-effort: if storage isn't wired yet (test bridges built
        // via ::new), continue without persisting.
        // The row says what it is: a policy-initiated ask — an approval
        // context, whatever its menu — is an `approval`; a question is a
        // `choice`. `kind = approval` is the durable GATE MARKER: it is what
        // latches the ring below and what every lift path reads back
        // (`is_gate_row` — resolve, withdraw, the respawn reseed). Round 11:
        // this used to require the exact `["Approve","Reject"]` menu as well,
        // while the latch fired on the context alone — so a `request_approval`
        // with the agent's own labels latched a gate that nothing could lift.
        // The render slot is the frontend's call (`isApproval`): a canonical
        // menu is the one-click Approve/Reject gate, a custom menu shows its
        // own labels there. Readers still accept the pre-round-8 shape
        // (`choice` + gate menu) through `is_gate_row`.
        // Round 12: an agent's `request_approval` is a `request` — audited
        // like an approval, parked like a question, latching nothing.
        let kind = if is_gate {
            crate::storage::QuestionKind::Approval
        } else if is_approval {
            crate::storage::QuestionKind::Request
        } else {
            crate::storage::QuestionKind::Choice
        };
        self.persist_question(
            &session_id,
            &choice_id,
            &agent,
            kind,
            &question,
            Some(&options),
            supersedes_id,
            command_text.as_deref(),
        )
        .await;

        // rc3 **D35** — the user's rule, replacing two earlier regimes:
        //
        // - An ordinary QUESTION parks a row and touches NOTHING else. No
        //   awaiting flag, no ring command. The session keeps working; the
        //   answer travels with the user's next Send (D34).
        // - An APPROVAL halts the session: the gate latch parks the ring until
        //   the user answers. **Keyed on the approval CONTEXT, not on
        //   `blocking`** — the action gate parks non-blocking (the agent gets
        //   "parked, outcome out-of-band" and carries on), and keying the
        //   latch on `blocking` let `s-86a81478` roll straight through a
        //   parked gate: the ring never latched, the session never stopped,
        //   and the user never got the floor. The asker's CURRENT turn is not
        //   cut either way; the latch stops the next deal.
        // - An agent's REQUEST (round 12) parks like a question: no awaiting
        //   flag, no latch — the user's split between "tray parkable" and
        //   "session blockers".
        if is_gate {
            self.set_session_awaiting(&session_id, &agent, false).await;
            self.notify_ring_gate(&session_id, &choice_id, true).await;
        }

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
        let Some(storage) = self.storage.lock().await.clone() else {
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
    /// `asker` scopes the withdrawal to the participant that PARKED the row
    /// (A4). Without it any participant could clear any other's question out of
    /// the user's tray — including a review-only one, which has no way to ask a
    /// question of its own and therefore no reason to be retracting one. `None`
    /// means the host is withdrawing (teardown, tray GC), which is not scoped.
    ///
    /// A mismatch is a no-op reported as "not yours", not an error: the caller
    /// asked for a state that already holds — that row is not theirs to worry
    /// about.
    pub async fn withdraw_question(&self, choice_id: &str, asker: Option<&str>) -> bool {
        matches!(
            self.withdraw_question_for(choice_id, asker).await,
            Withdrawal::Withdrawn
        )
    }

    /// [`Self::withdraw_question`] with the reason a `false` would have hidden:
    /// the tool's answer must not call another participant's still-pending row
    /// "not pending" (round 9) — that told the caller a state that does not
    /// hold, when the bridge knew exactly why it refused.
    pub async fn withdraw_question_for(&self, choice_id: &str, asker: Option<&str>) -> Withdrawal {
        // ONE read of the row serves both questions asked of it (round 11 —
        // it used to be read twice, across three storage locks): whose is it,
        // and is it a gate. No storage at all (tests / bootstrap) reads as
        // "no row".
        let storage = self.storage.lock().await.clone();
        let row = match &storage {
            Some(storage) => match storage.get_tray_entry(choice_id).await {
                Ok(row) => row,
                Err(e) => {
                    if asker.is_some() {
                        // "Yours only" — a read ERROR is not "no owner": it
                        // refuses (round 10). Folding the error into `None`
                        // let the withdrawal go ahead, which for the one
                        // control this ungated tool has is fail-OPEN.
                        tracing::warn!(
                            ?e,
                            choice_id,
                            "withdraw_question: the row's owner could not be read; refusing"
                        );
                        return Withdrawal::Unverifiable;
                    }
                    None
                }
            },
            None => None,
        };
        if let (Some(asker), Some(row)) = (asker, row.as_ref()) {
            // A row that does not exist falls through: there is no owner to
            // protect.
            if row.agent != asker {
                tracing::warn!(
                    choice_id,
                    asker,
                    owner = %row.agent,
                    "refusing to withdraw a question parked by another participant"
                );
                return Withdrawal::NotYours;
            }
        }
        let parked = self.pending.lock().await.remove(choice_id);
        let was_parked = parked.is_some();
        // Drop the oneshot — the agent's blocking caller (if any) gets the
        // standard "canceled" error.
        drop(parked);
        // rc3 D35: a withdrawn approval (agent abandons its gate, or an
        // external caller discards one) must lift the ring's gate latch too, or
        // the session stays halted on a gate nobody can answer any more. Read
        // BEFORE the withdraw flips the row away from `pending` — the latch
        // discriminator matches pending rows only.
        let gate_session = row
            .as_ref()
            .filter(|row| {
                row.status == "pending"
                    && crate::storage::is_gate_row(&row.kind, row.options_json.as_deref())
            })
            .map(|row| row.session_id.clone());
        // **The DURABLE row counts too.** The return value was "was there an
        // in-memory park?", so withdrawing a row that outlived its process — a
        // question parked before a restart, which is the case the durable row
        // exists for — reported "no-op: choice_id was not pending" to the agent
        // while actually withdrawing it. The tool's own answer said the opposite
        // of what happened.
        let mut withdrew_row = false;
        if let Some(storage) = storage {
            match storage.withdraw_tray_entry(choice_id).await {
                Ok(rows) => withdrew_row = rows > 0,
                Err(e) => {
                    tracing::warn!(?e, choice_id, "withdraw_question storage update failed")
                }
            }
        }
        if let Some(session_id) = gate_session {
            self.notify_ring_gate(&session_id, choice_id, false).await;
        }
        // A withdrawn reviewer-override request is consumed unanswered — the
        // reviewer-recovery void routes here, and a stale request must never
        // linger to be approved against a future down-incident.
        self.pending_override_requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(choice_id);
        if was_parked || withdrew_row {
            // Tell the UI (round 12) — the bell and the dashboard badges
            // refresh on this event; the supersede paths always sent it, the
            // withdraw path (agent tool AND the user's Discard) did not, so a
            // discarded question stayed counted until the next unrelated
            // tray event.
            let _ = self.event_tx.send(SignalingEvent::ChoiceResolved {
                choice_id: choice_id.to_string(),
                picked: "(withdrawn)".to_string(),
            });
            Withdrawal::Withdrawn
        } else {
            Withdrawal::NotPending
        }
    }

    /// Snapshot the `session_tray` table for a session. Convenience for the UI
    /// + the agent-facing `list_my_pending_questions` MCP tool.
    pub async fn list_questions_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::storage::SessionTrayEntry>> {
        let Some(storage) = self.storage.lock().await.clone() else {
            return Ok(Vec::new());
        };
        storage.tray_entries_for_session(session_id).await
    }

    /// All pending tray rows across OPEN sessions (closed sessions are
    /// excluded). Durable source for the header notifier's per-session
    /// "needs your input [N]" counts — survives restart, unlike the in-memory
    /// pending map.
    pub async fn list_pending_tray_open(&self) -> Result<Vec<crate::storage::SessionTrayEntry>> {
        let Some(storage) = self.storage.lock().await.clone() else {
            return Ok(Vec::new());
        };
        storage.pending_tray_open_sessions().await
    }

    /// Convenience entry (`confirm_stale = false`): the plugin proxy + tests
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
            && matches!(gate_verdict(&picked), crate::policy::ViolationOutcome::Approved)
        {
            if let Some((command, asked_at)) = self.stale_gated_command(choice_id).await {
                return Ok(ResolveOutcome::StaleGateNeedsConfirm { command, asked_at });
            }
        }

        // **Whose gate is this — asked BEFORE the flip.** After it, the only way
        // to tell is a second read, and that read's failure is indistinguishable
        // from "not a gate": `.ok().flatten()` swallowed it, the lift never
        // fired, and the ring stayed latched for the life of the process. The
        // withdraw path has always read first; this one now does too.
        let gate_session = {
            let storage = self.storage.lock().await.clone();
            match storage {
                Some(storage) => match storage.get_tray_entry(choice_id).await {
                    Ok(row) => row
                        .filter(|row| {
                            row.status == "pending"
                                && crate::storage::is_gate_row(&row.kind, row.options_json.as_deref())
                        })
                        .map(|row| row.session_id),
                    Err(e) => {
                        // Not silent, and not fatal: the flip below still runs,
                        // and the latch reseeds from the durable rows on the
                        // next respawn.
                        tracing::warn!(
                            ?e,
                            choice_id,
                            "gate check before resolve failed; if this was a gate its \
                             latch will not lift until the session respawns"
                        );
                        None
                    }
                },
                None => None,
            }
        };
        // Flip the row pending→answered first (so the UI/tray updates even if the
        // in-memory parked entry is gone — e.g. after a restart). `rows == 1`
        // means THIS call won the atomic transition; gate resolve-time
        // gated-command execution on it so a duplicate / stale resolve can't
        // double-run. This is the durable exactly-once that replaces the
        // in-memory oneshot's guarantee.
        let flipped = {
            let storage = self.storage.lock().await.clone();
            match storage {
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
        // rc3 D35: a resolved approval lifts the ring's gate latch. Keyed on
        // the durable row's options — the same Approve/Reject discriminator
        // that seeds the latch — and gated on `flipped` so a duplicate resolve
        // cannot decrement twice.
        if flipped {
            if let Some(session_id) = gate_session {
                self.notify_ring_gate(&session_id, choice_id, false).await;
            }
            // A reviewer-down override REQUEST resolves here (vision alignment,
            // 2026-08-14): Approve moves the reason into the active override —
            // the commit gate reads it — and Reject drops the request so the
            // block stands. Consumed either way, and only on the flip, so a
            // duplicate resolve cannot re-apply it.
            let pending_override = self
                .pending_override_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(choice_id);
            if let Some((session_id, reason)) = pending_override {
                // An Approve/Reject gate: only the listed Approve lifts a block.
                if matches!(gate_verdict(&picked), crate::policy::ViolationOutcome::Approved) {
                    tracing::warn!(
                        session = %session_id,
                        reason = %reason,
                        "reviewer-down commit block override APPROVED by the user"
                    );
                    self.reviewer_override
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(session_id, reason);
                } else {
                    tracing::info!(
                        session = %session_id,
                        "reviewer-down override request REJECTED by the user; the block stands"
                    );
                }
            }
        }
        let parked = self.pending.lock().await.remove(choice_id);
        match parked {
            Some(p) => {
                // Write violation record FIRST (before unblocking the agent)
                // so the audit trail captures the decision even if the agent
                // crashes immediately after receiving the result.
                let outcome = if p.choice.options.iter().map(String::as_str).eq(["Approve", "Reject"]) {
                    gate_verdict(&picked)
                } else {
                    outcome_from_picked(&picked)
                };
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

                // Clear the awaiting halt the matching ask set (approvals only —
                // an ordinary question sets none), BEFORE delivering the pick, so
                // the input-lock state the flag drives is right by the time the
                // answer is read. The bridge set the flag (set_session_awaiting),
                // so the bridge clears it on resolve. Also covers the Err
                // fall-through below (core then re-clears — harmlessly redundant).
                //
                // **Unless an AGENT-declared halt is standing over a gate answer**
                // (12951cc3, the user's pick, 2026-08-24): the gate ran, but the
                // session stays halted — the awaiting flag must keep deriving
                // `awaiting_user`, so the banner and the input state stay true.
                // Host-declared stops (`declared_by = "system"`: consensus,
                // all-pass, round cap, spin) keep today's clear-and-resume.
                // Read-failure counts as STANDING (review note on the 12951cc3
                // batch): `.ok()` here cleared the awaiting flag on a DB
                // hiccup — the exact behaviour the pick voted out — while the
                // state-side half kept the slot. Both halves now fail the
                // same direction.
                let agent_halt_stands = p.gate
                    && match self.storage.lock().await.clone() {
                        Some(storage) => match storage
                            .session_halt(&p.choice.session_id)
                            .await
                        {
                            Ok(halt) => halt.is_some_and(|(by, _, _)| by != "system"),
                            Err(e) => {
                                tracing::warn!(
                                    ?e,
                                    session_id = %p.choice.session_id,
                                    "halt read failed at gate resolve; keeping the awaiting flag"
                                );
                                true
                            }
                        },
                        None => false,
                    };
                if !agent_halt_stands {
                    self.clear_session_awaiting(&p.choice.session_id).await;
                }
                match p.tx.send(picked) {
                    Ok(()) => Ok(ResolveOutcome::Delivered),
                    Err(picked) => {
                        // Nobody is waiting on the oneshot. For `ask_user_choice`
                        // that is the NORMAL case — the tool is non-blocking (rc3
                        // D35) and dropped its receiver at park time — so this is the
                        // primary path, not a fallback; for a blocking approval it
                        // means the client-side MCP timeout beat the pick. Either
                        // way the answer becomes the user's row; the gated command,
                        // if any, comes from the in-memory approval ctx.
                        // Round 12: a push gate's rebuilt command runs HERE
                        // and only here — the `Ok(())` arm above means the
                        // hook is alive and proceeds with its own push, so
                        // running it there too would push twice (EYES F3).
                        let command = p.choice.approval.as_ref().and_then(executable_command);
                        let command = command.as_deref();
                        // Age-stamp from the durable row (the in-memory park
                        // carries no ask-time); a miss just omits the line.
                        let asked_at = {
                            let storage = self.storage.lock().await.clone();
                            match storage {
                                Some(storage) => storage
                                    .get_tray_entry(choice_id)
                                    .await
                                    .ok()
                                    .flatten()
                                    .map(|row| row.asked_at),
                                None => None,
                            }
                        };
                        let is_gate = p.gate;
                        Ok(self
                            .deliver_oob(
                                choice_id,
                                p.choice.session_id.clone(),
                                &p.choice.question,
                                &p.choice.options,
                                picked,
                                command,
                                flipped,
                                asked_at,
                                is_gate,
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
                    let storage = self.storage.lock().await.clone();
                    match storage {
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
                // **The audit record, on this path too** (round 10). The
                // in-memory branch above writes violations.jsonl from the
                // approval context it parked with; a gate answered after a
                // restart (or after `unregister_session` dropped the park) came
                // through here and was never recorded, while the descriptor
                // told the agent every outcome is. The context is not on the
                // row, so the KIND is reconstructed from the row's shape: a
                // gated command is the Tool Gate's; any other gate row is a
                // generic approval (a push gate resolves in-band with its hook
                // blocked on it, so it does not reach here). **Only when the
                // row FLIPPED** (round 11) — a repeat click on an answered gate
                // flips nothing, runs nothing and lifts nothing, and used to
                // append a second, possibly contradicting record. And the pick
                // is read the way the live branch reads it: the fail-closed
                // `gate_verdict` for the host's canonical menu, the label
                // mapper for an agent's own menu (`request_approval`), which
                // `gate_verdict` audited as Denied on an approving pick.
                let is_request = q.kind == crate::storage::QuestionKind::Request.as_str();
                if flipped
                    && (is_request || crate::storage::is_gate_row(&q.kind, q.options_json.as_deref()))
                {
                    if let Some(log) = self.violations.as_ref() {
                        let (kind, action) = match q.command_text.as_deref() {
                            // A push gate's row carries its rebuilt `git push`
                            // (round 12) — the audit names the gate it is.
                            Some(cmd) if crate::policy::push_rerun_refspecs(cmd).is_some() => {
                                (crate::policy::ViolationKind::PushGate, cmd.to_string())
                            }
                            Some(cmd) => (crate::policy::ViolationKind::ToolBlocklist, cmd.to_string()),
                            None => (crate::policy::ViolationKind::GenericApproval, q.prompt.clone()),
                        };
                        let outcome = if crate::storage::is_gate_options(q.options_json.as_deref()) {
                            gate_verdict(&picked)
                        } else {
                            outcome_from_picked(&picked)
                        };
                        let _ = log
                            .record(
                                q.session_id.clone(),
                                q.agent.clone(),
                                kind,
                                action,
                                outcome,
                                Some("resolved from the durable tray row (no live park)".to_string()),
                            )
                            .await;
                    }
                }
                let is_gate = crate::storage::is_gate_row(&q.kind, q.options_json.as_deref());
                Ok(self
                    .deliver_oob(
                        choice_id,
                        q.session_id.clone(),
                        &q.prompt,
                        &options,
                        picked,
                        q.command_text.as_deref(),
                        flipped,
                        Some(q.asked_at.clone()),
                        is_gate,
                    )
                    .await)
            }
        }
    }

    /// Persist + broadcast a tray answer as the user's row — **the primary
    /// delivery for every `ask_user_choice`** (the tool parks and drops its
    /// receiver at park time, rc3 D35, so `resolve_choice`'s in-memory `send`
    /// always misses and lands here) and the only one for a durable row whose
    /// park predates a restart. Builds the compact user message, runs any
    /// approved gated command when `flipped` (the atomic exactly-once already
    /// won), invalidates the bell / tray via `ChoiceResolved`, and returns
    /// `DeliveredOutOfBand` — carrying the RECEIPT — so
    /// `CoreAppState::resolve_choice` can wake an idle ring. The callers differ
    /// only in where `session_id` / `question` / `command_text` come from.
    /// (This used to be documented as the two "receiver-gone" fallbacks; that
    /// was true until the tool stopped blocking.)
    #[allow(clippy::too_many_arguments)]
    async fn deliver_oob(
        &self,
        choice_id: &str,
        session_id: String,
        question: &str,
        options: &[String],
        picked: String,
        command_text: Option<&str>,
        flipped: bool,
        asked_at: Option<String>,
        is_gate: bool,
    ) -> ResolveOutcome {
        // The "approved since you asked" block decorates QUESTIONS only (round
        // 10, B5). A gate is not a premise that a later gate can overtake — and
        // every approved gate lands as its own answer row with its own output,
        // so on an approval row the block only ever listed the sibling gates of
        // the same batch ("… merge 528 (8m later) … whether it succeeded is not
        // recorded", `s-766f4ab9`), telling the agent nothing it did not already
        // have and warning it about a premise a gate does not carry.
        //
        // `is_gate` comes from the CALLER (round 12): the parked approval
        // context on the live path, `is_gate_row` on the durable-row path —
        // the same predicate every latch path reads. This used to re-derive
        // it from `command_text || canonical menu`, which missed an agent's
        // `request_approval` with its own labels, so that gate got the
        // mooting block its doc excludes.
        let mooting = if is_gate {
            Vec::new()
        } else {
            self.gates_approved_since(&session_id, choice_id, asked_at.as_deref())
                .await
        };
        let mut body = oob_resolution_body(
            choice_id,
            question,
            options,
            &picked,
            asked_at.as_deref(),
            command_text,
            &mooting,
        );
        if flipped {
            self.maybe_run_gated(&session_id, choice_id, command_text, &picked, &mut body)
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
        // instead, following `session_activity` — the one other per-session map
        // here that holds a `Weak` back into a live session's state.
        //
        // It also costs core nothing it was relying on: core read the phase
        // microseconds later under the sessions lock, so the only observable
        // difference is a concurrent `advance_phase` landing in that window,
        // where taking the post-time phase is the correct one — it is the phase
        // the row says the agent was told.
        // The open-blocking banner rides EVERY user-origin delivery, not only
        // the typed-message path (Batch 9 T12, dissect #21): a tray answer
        // landing while a blocking finding was open used to carry a bare
        // phase envelope, so the banner appeared on one delivery and vanished
        // on the next while the finding still gated commits. Same fail-safe-0
        // posture as `broadcast_user_message` — the banner is salience, not
        // the gate.
        let open_blocking = {
            let storage = self.storage.lock().await.clone();
            match storage {
                Some(s) => s
                    .count_open_blocking_findings(&session_id)
                    .await
                    .unwrap_or(0) as usize,
                None => 0,
            }
        };
        let envelope = self
            .current_session_phase(&session_id)
            .await
            .map(|phase| {
                crate::storage::Envelope::phase(phase.name()).with_open_blocking(open_blocking)
            });
        let receipt = {
            let storage = self.storage.lock().await.clone();
            match storage {
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
        ResolveOutcome::DeliveredOutOfBand {
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
                    gate_verdict(row.picked_option.as_deref().unwrap_or("")),
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
    /// append its output to the OOB `body`. This is the ONLY path an approved
    /// PARKED command runs through: `action_gate` parks and returns at once
    /// (rc3 D35; an auto-allowed keyword executes directly and never parks),
    /// so nothing runs "in-band" — every approval lands here through
    /// `deliver_oob`, whether the agent's tool future is still open, timed out
    /// or predates a restart. `command` is None for any non-executing tray
    /// item; a no-op unless the pick is Approved. Callers gate this on the
    /// atomic status flip so it runs exactly once.
    async fn maybe_run_gated(
        &self,
        session_id: &str,
        choice_id: &str,
        command: Option<&str>,
        picked: &str,
        body: &mut String,
    ) {
        let Some(command) = command else { return };
        // Only the LISTED Approve runs a parked command (`gate_verdict`): the
        // user's own words are carried to the agent, never executed as a yes.
        if !matches!(gate_verdict(picked), crate::policy::ViolationOutcome::Approved) {
            return;
        }
        // **A push re-run** (round 12): the command is the sha-pinned `git push`
        // the hook's death left unrun. It gets a single-use nonce its own
        // pre-push hook redeems (instead of parking a second gate), a bound
        // sized for a network push rather than a local command (EYES F11), and
        // one line saying so — the agent reads the OOB row.
        if let Some(refspecs) = crate::policy::push_rerun_refspecs(command) {
            let nonce = self.mint_push_nonce(session_id, choice_id, refspecs);
            body.push_str(
                "Late approval: the hook that asked had already gone (the agent's \
                 `git push` was killed before you answered), so bot-hq re-ran the \
                 push it approved — the same commit, pinned by sha — with a \
                 600 s bound.\nOutput:\n",
            );
            let out = self
                .execute_gated_with(
                    session_id,
                    command,
                    PUSH_RERUN_TIMEOUT,
                    &[("BOT_HQ_PUSH_NONCE", nonce.as_str())],
                )
                .await;
            // Redeemed by the hook, or never presented (no hook, or the run died
            // first): either way nothing may redeem it later.
            self.discard_push_nonce(&nonce);
            match out {
                Ok(output) => body.push_str(&output),
                Err(e) => body.push_str(&format!("bot-hq could not re-run `{command}`: {e}")),
            }
            return;
        }
        // The verdict line above already says "approved" and names the command;
        // this is the output, headed by one short line so an empty stdout is
        // still visibly a result rather than nothing.
        body.push_str("Output:\n");
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


    /// Shared tail of `mark_awaiting_user` + `request_phase_advance`: write
    /// the session's HALT SLOT (rc3 D35 — one slot on the session row; no
    /// tray row of any kind, `kind=halt` is legacy DATA nothing writes), flag
    /// the session awaiting, then emit `AwaitingUser` so the UI shows the
    /// halt; the ring latches on `HaltDeclared` until the user acts.
    /// `halt_ring` is false when the RING ITSELF is the declarer: it has already
    /// stopped where it stands, and telling it again is the phantom-participant
    /// round-trip — `participant_by_slug("system")` finds nobody, warns, and
    /// hands the ring a second `halt()` (epoch + 1) for a stop it just made.
    async fn emit_halt_row(
        &self,
        session_id: String,
        agent: String,
        text: String,
        halt_ring: bool,
        wake_at: Option<String>,
    ) {
        // **A halt is SESSION state, not a tray row (rc3 D35).** The user:
        // "halt should be complete different, and not even remotely close to
        // parkable items in tray. It is now a session channel feature." One
        // slot on the session row — a later declaration replaces the earlier,
        // so "there can never be 2 halts" is schema now, not a display rule.
        //
        // **The WRITE comes first, and its failure is now said out loud.** The
        // flip and the emit used to run ahead of it and ignore its result, so a
        // failed write produced a session that showed a halt banner, stopped its
        // ring, and had no halt anywhere in storage: the banner vanished at the
        // next restart while the reason it existed for did not. The stop still
        // happens either way — an agent that asked to stop must stop, and a
        // session that keeps dealing turns under a failed write is the worse of
        // the two — but a halt nobody can recover is not allowed to look
        // identical to one that persisted.
        let recorded = {
            let storage = self.storage.lock().await.clone();
            match storage {
                Some(storage) => match match wake_at.as_deref() {
                    // A TEMPORARY halt (round 12) carries its wake instant in
                    // the same slot; the banner counts down to it.
                    Some(wake) => {
                        storage
                            .declare_temporary_session_halt(&session_id, &agent, &text, wake)
                            .await
                    }
                    None => storage.declare_session_halt(&session_id, &agent, &text).await,
                } {
                    Ok(true) => true,
                    // `Ok(false)` is the REFUSAL: the session is closed (or
                    // gone) and the declare matched no row (round 13,
                    // 828147ad). Nothing may follow — no awaiting flip, no
                    // ring `HaltDeclared`, no banner event, no fallback row —
                    // or the DB ghost the predicate closed becomes a runtime
                    // ghost: a session stopped and bannered with no halt in
                    // storage. The one caller racing a close (a pump's
                    // died-mid-turn declare, an agent halting into a
                    // concurrent close) simply loses, which is the point.
                    Ok(false) => {
                        tracing::debug!(
                            session_id,
                            "halt declare refused: the session is closed"
                        );
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(?e, session_id, "declare_session_halt failed");
                        false
                    }
                },
                // No storage at all is the test/bootstrap shape, not a failure
                // to report to a user who has no session to read it in.
                None => true,
            }
        };
        self.set_session_awaiting(&session_id, &agent, halt_ring).await;
        if !recorded {
            // Best-effort, and on the same storage that just failed — but the
            // failure modes are not identical (a constraint on one statement, a
            // lock held by one writer), so it is worth the attempt. If this
            // fails too the warning above is the whole record.
            let storage = self.storage.lock().await.clone();
            if let Some(storage) = storage {
                crate::core::post_system_notice(
                    &storage,
                    Some(self),
                    session_id.as_str(),
                    crate::storage::MessageKind::SystemNotice,
                    "[System: this halt could not be recorded — the session has \
                     stopped and the banner is live, but it will not survive a \
                     restart. Re-declare it if the app is relaunched.]",
                    None,
                )
                .await;
            }
        }
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
        self.emit_halt_row(session_id, agent, reason, true, None).await;
        prior
    }

    /// **A TEMPORARY HALT** (round 12, the user's Q2: "add a temporary halt …
    /// TEMPORARY HALT 00:03:57"): the same slot, banner and ring stop as
    /// [`Self::mark_awaiting_user`], plus a wake instant `wake_after` from now.
    /// The banner counts down to it; when it passes, [`Self::fire_temporary_halt`]
    /// clears the halt, posts a row saying so and SUMMONS the declarer for a turn
    /// (the D17 release with a mention) — so an agent waiting on CI, a deploy or
    /// a cron declares the wait, the UI shows the countdown, and the session
    /// wakes itself. A user message before the instant cancels it (the release
    /// clears the slot); a paused session or one with a gate open does not
    /// wake. Timers are in-memory; [`Self::rearm_temporary_halts`] re-arms them
    /// at boot.
    pub async fn mark_temporary_halt(
        self: &Arc<Self>,
        session_id: String,
        agent: String,
        reason: String,
        wake_after: std::time::Duration,
    ) -> Option<String> {
        let prior = self.pending_halt_prompt(&session_id, &agent).await;
        let wake_at = (chrono::Utc::now() + chrono::Duration::from_std(wake_after).unwrap_or_default())
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        self.emit_halt_row(
            session_id.clone(),
            agent.clone(),
            reason,
            true,
            Some(wake_at.clone()),
        )
        .await;
        self.arm_temporary_halt(session_id, agent, wake_at);
        prior
    }

    /// Spawn the timer for a temporary halt: sleep until `wake_at` (or fire now
    /// if it is past), then [`Self::fire_temporary_halt`]. Keyed by the instant,
    /// not the session — a later declaration writes a different instant, and
    /// the fire re-reads the slot and no-ops on a mismatch, so a replaced halt
    /// simply lets its old timer expire into nothing.
    fn arm_temporary_halt(self: &Arc<Self>, session_id: String, agent: String, wake_at: String) {
        let bridge = Arc::clone(self);
        let Some(handle) = tokio::runtime::Handle::try_current().ok() else {
            tracing::warn!(session_id, "temporary halt armed outside a runtime; it will not wake");
            return;
        };
        handle.spawn(async move {
            let delay = chrono::DateTime::parse_from_rfc3339(&wake_at)
                .ok()
                .map(|t| t.with_timezone(&chrono::Utc))
                .and_then(|t| (t - chrono::Utc::now()).to_std().ok())
                .unwrap_or_default();
            tokio::time::sleep(delay).await;
            bridge.fire_temporary_halt(&session_id, &agent, &wake_at).await;
        });
    }

    /// The temporary halt's wake. Re-reads the slot first: if the session's
    /// `halt_wake_at` is no longer THIS instant (the halt was replaced, or the
    /// user's message cleared it) nothing happens. A paused session does not
    /// wake (the pause is the user's). A session with an approval gate pending
    /// does not wake yet either (EYES F18 — clearing the slot would leave a
    /// stopped session bannerless): it re-checks every 30 s, up to ten times,
    /// then leaves the halt standing and says so in a row. Otherwise: a system
    /// row, the slot cleared, the awaiting flag dropped, and the ring released
    /// WITH A SUMMONS for the declarer — the next turn is theirs, the rotation
    /// resumes from where it was (rc3 D17).
    pub async fn fire_temporary_halt(&self, session_id: &str, agent: &str, wake_at: &str) {
        let Some(storage) = self.storage.lock().await.clone() else {
            return;
        };
        // No ring registered = the session is not live (closed, or not yet
        // respawned after a relaunch): nothing to deal to, so the halt stands
        // and `register_session_sequencer` re-arms it when the ring comes up.
        if !self.session_sequencer.lock().await.contains_key(session_id) {
            tracing::debug!(session_id, wake_at, "temporary halt expired on a session with no ring; it re-arms when one registers");
            return;
        }
        let mut gate_waits = 0u32;
        loop {
            match storage.session_halt_wake_at(session_id).await {
                Ok(Some(current)) if current == wake_at => {}
                _ => {
                    tracing::debug!(session_id, wake_at, "temporary halt no longer current; no wake");
                    return;
                }
            }
            let paused = self
                .session_activity
                .lock()
                .await
                .get(session_id)
                .and_then(|w| w.upgrade())
                .is_some_and(|t| t.is_paused());
            if paused {
                tracing::debug!(session_id, "temporary halt expired on a PAUSED session; the pause is the user's — no wake");
                return;
            }
            let gates = storage.pending_gate_ids(session_id).await.unwrap_or_default();
            if gates.is_empty() {
                break;
            }
            gate_waits += 1;
            if gate_waits > 10 {
                crate::core::post_system_notice(
                    &storage,
                    Some(self),
                    session_id,
                    crate::storage::MessageKind::SystemNotice,
                    format!(
                        "[System: {agent}'s temporary halt expired, but an approval gate has been                          pending for five minutes — the wake was skipped; the halt stands until                          the gate is answered.]"
                    ),
                    None,
                )
                .await;
                return;
            }
            tracing::debug!(session_id, gate_waits, "temporary halt expired behind an open gate; re-checking in 30 s");
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
        let declared = storage
            .session_halt(session_id)
            .await
            .ok()
            .flatten()
            .map(|(_, reason, at)| (reason, at))
            .unwrap_or_default();
        let now = crate::storage::now_utc();
        crate::core::post_system_notice(
            &storage,
            Some(self),
            session_id,
            crate::storage::MessageKind::SystemNotice,
            format!(
                "[System: temporary halt ended — {} (declared {}, woke {now}). {agent} takes the next turn.]",
                declared.0, declared.1
            ),
            None,
        )
        .await;
        match storage.clear_session_halt(session_id).await {
            Ok(true) => self.notify_halts_cleared(session_id.to_string()),
            Ok(false) => {}
            Err(e) => tracing::warn!(?e, session_id, "clear_session_halt failed on a temporary halt's wake"),
        }
        self.clear_session_awaiting(session_id).await;
        let mentions = match storage.participant_by_slug(session_id, agent).await {
            Ok(Some(p)) => vec![p.id],
            _ => Vec::new(),
        };
        tracing::info!(session_id, agent, wake_at, "temporary halt ended; waking the declarer");
        // Targeted at the declarer via the mention; `true` keeps the pre-flag
        // reset fallback if the slug lookup missed (see watchdog's wake).
        self.notify_ring_user_message(session_id, mentions, true).await;
    }

    /// Re-arm ONE session's temporary halt when its ring registers (timers are
    /// in-memory; the session may have been relaunched, respawned or
    /// reopened): a past instant fires at once, a future one sleeps, no wake
    /// instant → nothing. Called from `register_session_sequencer`, so a wake
    /// always has a ring to deal to.
    pub async fn rearm_temporary_halt_for(self: &Arc<Self>, session_id: &str) {
        let Some(storage) = self.storage.lock().await.clone() else {
            return;
        };
        let wake_at = match storage.session_halt_wake_at(session_id).await {
            Ok(Some(w)) => w,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(?e, session_id, "could not read the session's temporary halt");
                return;
            }
        };
        let agent = storage
            .session_halt(session_id)
            .await
            .ok()
            .flatten()
            .map(|(by, _, _)| by)
            .unwrap_or_else(|| "system".to_string());
        tracing::info!(session_id, %wake_at, "re-arming a temporary halt for a ring that came up");
        self.arm_temporary_halt(session_id.to_string(), agent, wake_at);
    }

    /// **A halt the HOST declares under an agent's slug** — the provider-limit
    /// and error-streak halts (pump), the spin breaker (sequencer), the idle
    /// watchdog: same slot, banner and durable row as the agent's own
    /// `mark_awaiting_user`, PLUS the rc3 D35 self-interrupt at once via
    /// `HaltAcked` (round 8, A1b). The agent's own tool gets its interrupt from
    /// its pump when the tool RESULT lands in its stream — that is what keeps
    /// the interrupt from racing the tool ack — but a host-declared halt has no
    /// tool call, no ack and no result, so there is nothing to wait for and
    /// firing now is what "if a generation is in flight, stop it" means. Every
    /// host caller goes through here; a source guard keeps `mark_awaiting_user`
    /// itself to the JSON-RPC handler, so a host halt cannot quietly lose its
    /// interrupt again (the reviewer's finding on batch 13).
    pub async fn mark_awaiting_user_for(
        &self,
        session_id: String,
        agent: String,
        reason: String,
    ) -> Option<String> {
        let prior = self.mark_awaiting_user(session_id.clone(), agent.clone(), reason).await;
        self.notify_halt_acked(&session_id, &agent);
        prior
    }

    /// **The ring's own stop, declared without a round-trip.**
    ///
    /// The all-pass yield, the round cap, consensus and an unwound wedge are
    /// halts the RING makes — it has already cleared its holder and bumped its
    /// epoch by the time it says so. They used to be announced through
    /// `mark_awaiting_user(.., "system", ..)`, which resolves the asker against
    /// the roster, finds nobody (there is no `system` participant — 0044 made
    /// host rows `origin = 'system'` with a NULL participant precisely because
    /// there is no such row), warns about it, and then hands the ring a
    /// `HaltDeclared` for the stop it just made: a second `halt()`, epoch + 1,
    /// and a warning line per yield that reads like a roster bug.
    ///
    /// Same slot, same banner, same durable row — without the trip through a
    /// participant that does not exist.
    pub async fn declare_host_halt(&self, session_id: &str, reason: String) {
        self.emit_halt_row(session_id.to_string(), "system".to_string(), reason, false, None)
            .await;
    }

    /// Storage lookup behind the repeat-halt check. Best-effort: a bridge built
    /// without storage (tests) or a failed query simply reports "no prior".
    async fn pending_halt_prompt(&self, session_id: &str, agent: &str) -> Option<String> {
        // The session's ONE halt slot (rc3 D35): a prior halt from this same
        // agent that the user has not acted on yet.
        let storage = self.storage.lock().await.clone()?;
        storage
            .session_halt(session_id)
            .await
            .ok()
            .flatten()
            .filter(|(by, _, _)| by == agent)
            .map(|(_, reason, _)| reason)
    }

    /// Agent-initiated IPAV phase advance request. Persists a chat message
    /// authored by the requesting agent (so the scroll shows the ask inline)
    /// and a halt question (so the tray + dashboard counter reflect it via the
    /// durable `list_pending_tray`, not the in-memory map), then sets the
    /// awaiting flag so the ring holds until the user acts.
    ///
    /// The user has two response paths, both clear the halt:
    ///   1. Pick a phase in the session header → the `advance_session_phase`
    ///      command → `AppState::advance_phase` (which also clears awaiting +
    ///      answers pending halt rows).
    ///
    ///      **This path did not exist until round 4**, and this comment claimed
    ///      it did — no Tauri command wrote the phase, and the chip it named was
    ///      a bare `<span>` on the dashboard tile. So the only reachable answer
    ///      was path 2, the implicit decline: this tool's stated purpose was
    ///      unreachable for as long as it has existed.
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
            let storage = self.storage.lock().await.clone();
            if let Some(storage) = storage {
                // The receipt is dropped: this row records the agent's request,
                // and the only thing done with `body` afterwards is
                // `emit_halt_row` — a UI event, not a wire into any agent's
                // stdin. There is no send on this path for a receipt to gate.
                //
                // **Written as the REQUESTING PARTICIPANT, which it had never
                // been (round-3 F13).** This line used to read
                // `Author::parse(&agent).unwrap_or(Author::User)`, and `parse`
                // knew only `user`/`brian`/`rain` — so for every rc3 role slug
                // it returned `None` and the `unwrap_or` filed the row as
                // `origin = "user", slug = NULL`. An agent asking for a phase
                // advance was recorded, and RENDERED, as something the user
                // said: `ChatMessage.tsx` has no case for `Text`, so the row
                // fell through to ordinary authored prose under the user's
                // label. Agents read the transcript back, so the system was
                // manufacturing a user utterance — the mechanical version of
                // the fabricated-authorization failure the general rules exist
                // to prevent. A rename would have preserved it exactly, since
                // renaming does not teach a parser to see role slugs.
                match storage
                    .post_to_channel(
                        session_id.as_str(),
                        "participant",
                        Some(agent.as_str()),
                        crate::storage::MessageKind::Text.as_str(),
                        &body,
                        None,
                    )
                    .await
                {
                    Ok(m) => self
                        .notify_message_persisted(Arc::from(session_id.as_str()), m.message_id()),
                    Err(e) => {
                        tracing::warn!(?e, "request_phase_advance receipt row failed to persist")
                    }
                }
            }
        }
        self.emit_halt_row(session_id, agent, body, true, None).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::ViolationOutcome;

    /// **A phase request is recorded as the AGENT that asked, not as the user**
    /// (round-3 F13).
    ///
    /// The bug this pins was live, not hypothetical. The receipt used to be
    /// written `Author::parse(&agent).unwrap_or(Author::User)`, and `parse`
    /// knew only `user` / `brian` / `rain` — so for every rc3 role slug it
    /// returned `None` and the fallback filed the row as `origin = "user"`,
    /// `participant_id = NULL`.
    ///
    /// Why that is worse than a wrong label: `ChatMessage.tsx` has no case for
    /// `Text`, so the row rendered as ordinary authored prose under the USER's
    /// name, and agents read the transcript back. The system was manufacturing
    /// a user utterance — the mechanical form of the fabricated-authorization
    /// failure the general rules exist to prevent.
    ///
    /// It also survived every earlier sweep by construction: a rename of the
    /// two variants would have left `parse` exactly as unable to see a role
    /// slug, so the `unwrap_or` would still fire. Only deleting the type
    /// removed the fallback.
    #[tokio::test]
    async fn a_phase_request_is_attributed_to_the_agent_that_asked() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage
            .ensure_session_roster("s1", crate::storage::MAX_SESSION_PARTICIPANTS)
            .await
            .unwrap();

        bridge
            .request_phase_advance("s1".into(), "eyes".into(), "Apply".into(), "ready".into())
            .await;

        let rows = storage.messages_for_session("s1", None).await.unwrap();
        let receipt = rows
            .iter()
            .find(|m| m.content.contains("[PHASE REQUEST -> Apply]"))
            .expect("the receipt row was written");
        assert_eq!(
            receipt.author, "eyes",
            "the requesting participant owns the row; `user` is the bug"
        );
        assert_ne!(
            receipt.author, "user",
            "a phase request must never render as something the user said"
        );
    }

    /// **Parking a question must HALT THE RING, not merely set a flag.**
    ///
    /// The regression this pins was live on 2026-08-12: `ask_user_choice` set
    /// the awaiting flag, which only gates cursor advance, so the ring kept
    /// dealing turns while the session was blocked on a human. Each participant
    /// woke with nothing new delivered and no legal move, and passed — ~15 model
    /// calls in 1m44s with both alternating "standing by".
    ///
    /// It could not self-terminate, which is why the flag was not enough: a pass
    /// casts no vote AND retracts its own, and any prose at all is a substantive
    /// ending that clears the whole tally. So the agents kept resetting the very
    /// consensus that would have stopped them, by saying they had nothing to say.
    ///
    /// `SequencerCommand::HaltDeclared` was written, documented in six places
    /// and covered by two sequencer tests — with no production sender. This is
    /// that sender.
    #[tokio::test]
    async fn a_halt_reaches_the_ring_and_a_question_does_not() {
        // **Changed subject at rc3 D35 — it used to assert both doors halt.**
        // The user's rule split them: *"A halt is a halt. Still working means
        // still working."* `mark_awaiting_user` is a participant declaring the
        // session waits — the ring stops. `ask_user_choice` parks a row and
        // touches NOTHING; the session keeps working and the answer travels
        // with the user's next Send (D34). The old behaviour put peers to work
        // under a ⏸ HALT banner (questions halted after a lap) — both halves
        // of that are gone.
        use crate::core::sequencer::SequencerCommand;
        use std::sync::atomic::AtomicBool;

        for (door, expects_halt) in [("ask_user_choice", false), ("mark_awaiting_user", true)] {
            let bridge = SignalingBridge::new();
            let storage = crate::storage::Storage::memory().await.unwrap();
            bridge.set_storage(storage.clone()).await;
            storage.create_session("s1", "t", None).await.unwrap();
            bridge
                .register_session_awaiting("s1".into(), Arc::new(AtomicBool::new(false)))
                .await;
            let (tx, mut rx) = tokio::sync::mpsc::channel(8);
            bridge.register_session_sequencer("s1".into(), tx).await;

            match door {
                "ask_user_choice" => {
                    bridge
                        .ask_user_choice(
                            "s1".into(),
                            "hands".into(),
                            "close?".into(),
                            vec!["yes".into(), "no".into()],
                        )
                        .await
                        .unwrap();
                }
                _ => {
                    bridge
                        .mark_awaiting_user("s1".into(), "hands".into(), "blocked".into())
                        .await;
                }
            }

            if expects_halt {
                assert!(
                    matches!(rx.try_recv(), Ok(SequencerCommand::HaltDeclared { .. })),
                    "{door} declared a halt and the ring was never told"
                );
            } else {
                assert!(
                    rx.try_recv().is_err(),
                    "{door} is a parked QUESTION — it must not touch the ring at all"
                );
            }
        }
    }

    /// **The RELEASE, which is the half that was missing.**
    ///
    /// A halt with no release is worse than no halt: `HaltDeclared` sets
    /// `holder = None`, and the sequencer's only un-halt is a `UserMessage`. When
    /// this shipped without a release path, the first `mark_awaiting_user` of a
    /// session stopped the cycle permanently — the participants kept their
    /// subprocesses, received zero deliveries, and ran blind on whatever was
    /// already in their stdin. It looked like three agents working; it was three
    /// agents talking past each other.
    ///
    /// So this asserts the PAIR. Halting alone passing a test is exactly what
    /// let the bug through.
    #[tokio::test]
    async fn a_parked_question_halts_the_ring_and_a_user_message_releases_it() {
        use crate::core::sequencer::SequencerCommand;
        use std::sync::atomic::AtomicBool;

        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        bridge
            .register_session_awaiting("s1".into(), Arc::new(AtomicBool::new(false)))
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        bridge.register_session_sequencer("s1".into(), tx).await;

        bridge
            .mark_awaiting_user("s1".into(), "hands".into(), "blocked".into())
            .await;
        assert!(
            matches!(rx.try_recv(), Ok(SequencerCommand::HaltDeclared { .. })),
            "parking must halt the ring"
        );

        bridge.notify_ring_user_message("s1", Vec::new(), true).await;
        assert!(
            matches!(rx.try_recv(), Ok(SequencerCommand::UserMessage { .. })),
            "a user message must RELEASE the halt — without this the cycle never restarts"
        );
    }

    /// A blocking approval is a hook waiting on a bool, not the session yielding.
    /// It must NOT halt the cycle: the holder still holds its turn, the ring is
    /// already waiting on the completion, and halting there stopped a cycle that
    /// was not stuck. Every gated `git commit` goes through this path.
    #[tokio::test]
    async fn any_approval_opens_a_gate_blocking_or_parked() {
        use std::sync::atomic::AtomicBool;
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        bridge
            .register_session_awaiting("s1".into(), Arc::new(AtomicBool::new(false)))
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        bridge.register_session_sequencer("s1".into(), tx).await;

        // **The latch keys on the approval CONTEXT, not on `blocking`** — the
        // defect found live in `s-86a81478`: the action gate parks
        // NON-blocking (the agent gets "parked" and carries on), and a
        // blocking-keyed latch let the session roll straight through a parked
        // gate. Both modes must open one.
        //
        // Blocking (the pre-push hook shape) — holds open, so it is spawned.
        let bridge2 = bridge.clone();
        tokio::spawn(async move {
            let _ = bridge2
                .ask_user_choice_inner(
                    "s1".into(),
                    "hands".into(),
                    "run it?".into(),
                    vec!["Approve".into(), "Reject".into()],
                    Some(super::super::ApprovalContext {
                        kind: crate::policy::ViolationKind::PushGate,
                        action: "git push".into(),
                        detail: None,
                        command: None,
                    }),
                    None,
                    true,
                    true,
                )
                .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert!(
            matches!(
                rx.try_recv(),
                Ok(crate::core::sequencer::SequencerCommand::GateOpened { .. })
            ),
            "a blocking approval must open a gate — the session halts on it"
        );

        // Parked (the action-gate shape): blocking = false, ack returns
        // immediately, and the gate must STILL open.
        let _ = bridge
            .ask_user_choice_inner(
                "s1".into(),
                "hands".into(),
                "Run gated command?".into(),
                vec!["Approve".into(), "Reject".into()],
                Some(super::super::ApprovalContext {
                    kind: crate::policy::ViolationKind::ToolBlocklist,
                    action: "rm -rf build".into(),
                    detail: None,
                    command: None,
                }),
                None,
                false,
                true,
            )
            .await;
        assert!(
            matches!(
                rx.try_recv(),
                Ok(crate::core::sequencer::SequencerCommand::GateOpened { .. })
            ),
            "a PARKED approval must open a gate too — this is the s-86a81478 hole"
        );

        // And an ordinary question (no approval context) must NOT.
        let _ = bridge
            .ask_user_choice(
                "s1".into(),
                "hands".into(),
                "which?".into(),
                vec!["a".into(), "b".into()],
            )
            .await;
        assert!(
            rx.try_recv().is_err(),
            "a question is not a gate; the ring hears nothing"
        );
    }

    /// **An agent's `request_approval` is a TRAY item** (round 12 — the user's
    /// split: "request_approval is tray parkable, approval_gates are session
    /// blockers"). It carries the agent's OWN menu — here the canonical pair,
    /// the shape the descriptor's convention produces — parks a `request` row,
    /// latches NOTHING (no `GateOpened`, no awaiting flag), and the pick comes
    /// back as the agent's label, audited. (Round 11 had made the same call
    /// latch and render as a gate — "a custom-labelled approval latched a gate
    /// nothing could lift" — which is the behaviour the user reported as issue
    /// #1: a question on the input box. The host's gates keep the latch:
    /// `any_approval_opens_a_gate_blocking_or_parked`.)
    #[tokio::test]
    async fn an_agents_request_parks_in_the_tray_and_latches_nothing() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        let awaiting = Arc::new(AtomicBool::new(false));
        bridge
            .register_session_awaiting("s1".into(), Arc::clone(&awaiting))
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        bridge.register_session_sequencer("s1".into(), tx).await;

        for (prompt, menu) in [
            ("Commit the #494 work?", vec!["Approve".to_string(), "Reject".to_string()]),
            (
                "Commit T4 (#516)?",
                vec![
                    "Approve — commit it".to_string(),
                    "Approve, and push + open the PR too".to_string(),
                    "Deny — I want to read the diff first".to_string(),
                    "Deny — change the commit message".to_string(),
                ],
            ),
        ] {
            let ack = bridge
                .request_approval_parked(
                    "s1".into(),
                    "hands".into(),
                    prompt.into(),
                    menu.clone(),
                    super::super::ApprovalContext {
                        kind: crate::policy::ViolationKind::PerAction,
                        action: format!("git commit — {prompt}"),
                        detail: None,
                        command: None,
                    },
                )
                .await
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&ack).unwrap();
            assert_eq!(v["status"], "parked");
            let choice_id = v["choice_id"].as_str().unwrap().to_string();
            assert!(rx.try_recv().is_err(), "a request latches nothing: no ring command for {prompt:?}");
            assert!(!awaiting.load(Ordering::SeqCst), "a request sets no awaiting flag");
            let row = storage.get_tray_entry(&choice_id).await.unwrap().unwrap();
            assert_eq!(row.kind, "request", "the row says what it is");
            assert!(
                !crate::storage::is_gate_row(&row.kind, row.options_json.as_deref()),
                "a request is not a gate — whatever its menu ({menu:?})"
            );
            // The user answers in the tray with one of the agent's own labels;
            // nothing lifts (nothing latched) and the pick is recorded verbatim.
            let pick = menu[0].clone();
            let outcome = bridge.resolve_choice(&choice_id, pick.clone()).await.unwrap();
            assert!(matches!(outcome, ResolveOutcome::DeliveredOutOfBand { .. }), "{outcome:?}");
            assert!(rx.try_recv().is_err(), "no GateResolved for a row that opened no gate");
            let row = storage.get_tray_entry(&choice_id).await.unwrap().unwrap();
            assert_eq!(row.picked_option.as_deref(), Some(pick.as_str()));
        }
    }

    /// Round 12: a re-ask that SUPERSEDES a pending approval must release the
    /// latch the old gate opened. `auto_supersede_prior_pending` retired the old
    /// row (status, oneshot, UI event) but never told the ring, and the ring
    /// drops an id from `open_gates` only on `GateResolved` — so a byte-identical
    /// re-park from the same agent (the pre-push prompt after a client-timeout
    /// kill, re-pushed) left the stale id latched: answering the NEW gate lifted
    /// nothing and the session stayed "dealing is parked" until restart.
    #[tokio::test]
    async fn superseding_a_pending_approval_releases_its_gate_latch() {
        use crate::core::sequencer::SequencerCommand;
        use std::sync::atomic::AtomicBool;
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        bridge
            .register_session_awaiting("s1".into(), Arc::new(AtomicBool::new(false)))
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        bridge.register_session_sequencer("s1".into(), tx).await;
        let ctx = || super::super::ApprovalContext {
            kind: crate::policy::ViolationKind::PushGate,
            action: "git push origin main".into(),
            detail: None,
            command: None,
        };
        let prompt = "Allow `git push` to `main` in this session's repo?";

        // The push-gate shape: a HOST gate (gate = true), parked so the test
        // does not block — the hook route's blocking twin goes through the
        // same `request_approval_inner` and the same auto-supersede.
        let ack = bridge
            .request_approval_inner(
                "s1".into(),
                "hands".into(),
                prompt.into(),
                vec!["Approve".into(), "Reject".into()],
                ctx(),
                false,
                true,
            )
            .await
            .unwrap();
        let first: serde_json::Value = serde_json::from_str(&ack).unwrap();
        let first_id = first["choice_id"].as_str().unwrap().to_string();
        match rx.try_recv() {
            Ok(SequencerCommand::GateOpened { choice_id }) => assert_eq!(choice_id, first_id),
            other => panic!("expected GateOpened for the first gate, got {other:?}"),
        }

        // The same agent re-parks the same prompt while the first is pending:
        // the first is auto-superseded.
        let ack = bridge
            .request_approval_inner(
                "s1".into(),
                "hands".into(),
                prompt.into(),
                vec!["Approve".into(), "Reject".into()],
                ctx(),
                false,
                true,
            )
            .await
            .unwrap();
        let second: serde_json::Value = serde_json::from_str(&ack).unwrap();
        let second_id = second["choice_id"].as_str().unwrap().to_string();
        assert_ne!(first_id, second_id);
        let first_row = storage.get_tray_entry(&first_id).await.unwrap().unwrap();
        assert_eq!(first_row.status, "superseded");

        // The ring must hear the OLD gate close before (or beside) the new one
        // opening — otherwise its `open_gates` keeps the dead id for ever.
        let mut resolved_first = false;
        let mut opened_second = false;
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                SequencerCommand::GateResolved { choice_id } if choice_id == first_id => {
                    resolved_first = true
                }
                SequencerCommand::GateOpened { choice_id } if choice_id == second_id => {
                    opened_second = true
                }
                other => panic!("unexpected ring command {other:?}"),
            }
        }
        assert!(
            resolved_first,
            "superseding a pending APPROVAL must send GateResolved for the retired id"
        );
        assert!(opened_second, "the new gate still opens");
    }

    /// The question-supersede tool cannot retire an approval: a gate is answered
    /// in the gate or withdrawn, never replaced by a question — and an agent that
    /// tried would leave the ring latched on the retired id (same leak as above,
    /// through the explicit path).
    #[tokio::test]
    async fn supersede_question_refuses_to_retire_an_approval_gate() {
        use crate::core::sequencer::SequencerCommand;
        use std::sync::atomic::AtomicBool;
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        bridge
            .register_session_awaiting("s1".into(), Arc::new(AtomicBool::new(false)))
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        bridge.register_session_sequencer("s1".into(), tx).await;
        // A host GATE (the Tool-Gate park shape), parked.
        let ack = bridge
            .ask_user_choice_inner(
                "s1".into(),
                "hands".into(),
                "Run gated command?".into(),
                vec!["Approve".into(), "Reject".into()],
                Some(super::super::ApprovalContext {
                    kind: crate::policy::ViolationKind::ToolBlocklist,
                    action: "psql -h prod …".into(),
                    detail: None,
                    command: None,
                }),
                None,
                false,
                true,
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&ack).unwrap();
        let gate_id = v["choice_id"].as_str().unwrap().to_string();
        assert!(matches!(rx.try_recv(), Ok(SequencerCommand::GateOpened { .. })));

        let err = bridge
            .supersede_question_with_new(
                "s1".into(),
                "hands".into(),
                gate_id.clone(),
                "Which table first?".into(),
                vec!["users".into(), "orders".into()],
            )
            .await
            .expect_err("a gate id must be refused by the question-supersede tool");
        assert!(
            err.to_string().contains("approval gate"),
            "the refusal names what the id is: {err}"
        );
        // Untouched: still pending, still latched, nothing new parked.
        let row = storage.get_tray_entry(&gate_id).await.unwrap().unwrap();
        assert_eq!(row.status, "pending");
        assert!(rx.try_recv().is_err(), "no ring command for a refused supersede");
        assert_eq!(
            storage.tray_entries_for_session("s1").await.unwrap().len(),
            1,
            "no new question row was parked"
        );
    }

    /// Round 12: a push gate answered AFTER its hook died re-runs the push it
    /// approved — sha-pinned — and a push gate answered while its hook is alive
    /// runs nothing (the hook proceeds with its own push). Real git, temp
    /// repos: `work` pushes to the bare `origin`. No bot-hq hook is installed
    /// in `work`, so the re-run's nonce is minted, never presented, and
    /// discarded — the redemption round trip is pinned separately
    /// (`a_push_nonce_redeems_once_for_its_session_and_refspecs`, and the
    /// route test in `server.rs`).
    #[tokio::test]
    async fn a_late_approved_push_gate_re_runs_the_push_it_approved() {
        use std::process::Command;
        fn git(dir: &std::path::Path, args: &[&str]) -> String {
            let out = Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git runs");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("origin.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        git(&bare, &["init", "--bare", "-q", "-b", "main"]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(work.join("a.txt"), "a\n").unwrap();
        git(&work, &["add", "a.txt"]);
        git(&work, &["commit", "-q", "-m", "a"]);
        let sha_a = git(&work, &["rev-parse", "HEAD"]);

        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage
            .create_session("s1", "t", Some(work.to_str().unwrap()))
            .await
            .unwrap();
        bridge
            .register_session_awaiting(
                "s1".into(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )
            .await;
        let refs = vec![crate::policy::PushRef {
            local_ref: "refs/heads/main".into(),
            local_oid: sha_a.clone(),
            remote_ref: "refs/heads/main".into(),
            remote_oid: "0000000000000000000000000000000000000000".into(),
        }];
        let command = crate::policy::push_rerun_command("origin", &refs).expect("rebuildable");
        let ctx = || super::super::ApprovalContext {
            kind: crate::policy::ViolationKind::PushGate,
            action: crate::policy::push_gate_action(Some("main")),
            detail: Some("main".into()),
            command: Some(command.clone()),
        };

        // (1) The hook is ALIVE: `request_approval` (blocking) is what the route
        // awaits; the answer reaches it and the app runs nothing.
        let b2 = Arc::clone(&bridge);
        let c2 = ctx();
        let live = tokio::spawn(async move {
            b2.request_approval(
                "s1".into(),
                "hands".into(),
                "Allow `git push` to `main` in this session's repo?".into(),
                vec!["Approve".into(), "Reject".into()],
                c2,
            )
            .await
            .unwrap()
        });
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        let rows = storage.tray_entries_for_session("s1").await.unwrap();
        let live_id = rows.last().unwrap().choice_id.clone();
        assert_eq!(rows.last().unwrap().command_text.as_deref(), Some(command.as_str()));
        bridge.resolve_choice(&live_id, "Approve".into()).await.unwrap();
        assert_eq!(live.await.unwrap(), "Approve");
        let remote_main = Command::new("git")
            .args(["rev-parse", "--verify", "-q", "refs/heads/main"])
            .current_dir(&bare)
            .output()
            .unwrap();
        assert!(
            !remote_main.status.success(),
            "a live hook means the app pushed NOTHING — the hook does its own push"
        );

        // (2) The hook is DEAD: the parked shape drops its receiver at park
        // time, exactly like a hook killed before the answer. The branch
        // moves on after the park; the approve still ships sha A, pinned.
        //
        // **The WIRE, not the halves** (EYES fd17516b): a plain shell pre-push
        // hook in `work` records what the re-run's child environment carries
        // — the minted nonce and the session id — so mint → env → child →
        // hook is one observation, not three tests that each trust the next.
        let hook_env = tmp.path().join("hook-env.txt");
        let hooks_dir = work.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        let hook = hooks_dir.join("pre-push");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\nprintf '%s %s' \"$BOT_HQ_PUSH_NONCE\" \"$BOT_HQ_SESSION_ID\" > '{}'\nexit 0\n",
                hook_env.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // The push gate is a HOST gate (gate = true), parked here so the test
        // does not block — the shape the hook route leaves behind when its
        // process dies.
        let ack = bridge
            .request_approval_inner(
                "s1".into(),
                "hands".into(),
                "Allow `git push` to `main` in this session's repo?".into(),
                vec!["Approve".into(), "Reject".into()],
                ctx(),
                false,
                true,
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&ack).unwrap();
        let dead_id = v["choice_id"].as_str().unwrap().to_string();
        std::fs::write(work.join("b.txt"), "b\n").unwrap();
        git(&work, &["add", "b.txt"]);
        git(&work, &["commit", "-q", "-m", "b"]);
        let sha_b = git(&work, &["rev-parse", "HEAD"]);
        assert_ne!(sha_a, sha_b);

        let outcome = bridge.resolve_choice(&dead_id, "Approve".into()).await.unwrap();
        let ResolveOutcome::DeliveredOutOfBand { body, .. } = &outcome else {
            panic!("the dead-waiter path delivers out of band: {outcome:?}");
        };
        let remote_main = git(&bare, &["rev-parse", "refs/heads/main"]);
        assert_eq!(remote_main, sha_a, "the approved commit shipped — not the branch tip");
        // The answer row says what happened, names the bound, carries the output.
        assert!(body.contains("Late approval"), "{body}");
        assert!(body.contains("600 s"), "{body}");
        assert!(body.contains("exit") || body.contains("main"), "{body}");
        // The re-run's hook saw the session id AND a nonce — the join pinned.
        let seen = std::fs::read_to_string(&hook_env).expect("the re-run ran the repo's pre-push hook");
        let (nonce, sid) = seen.split_once(' ').expect("nonce and session id");
        assert_eq!(sid, "s1", "the child carried BOT_HQ_SESSION_ID: {seen:?}");
        assert_eq!(nonce.len(), 32, "the child carried the minted nonce: {seen:?}");
        assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()), "{nonce}");
        // And it was discarded after the run: nothing redeems it later.
        let refspecs = crate::policy::push_rerun_refspecs(&command).unwrap();
        assert!(
            bridge.redeem_push_nonce("s1", nonce, &refspecs).is_err(),
            "a nonce the run did not redeem is discarded, not left redeemable"
        );
    }

    /// Round 12 (the user's Q2): a TEMPORARY halt fills the slot with a wake
    /// instant, and when it passes the bridge clears the halt, posts a row and
    /// releases the ring WITH A SUMMONS for the declarer — the session wakes
    /// itself. A halt declared over it, or a user's release before the
    /// instant, makes the timer a no-op; a pending gate defers the wake; a
    /// session with no ring leaves the halt for the ring that comes up.
    #[tokio::test]
    async fn a_temporary_halt_counts_down_and_wakes_its_declarer() {
        use crate::core::sequencer::SequencerCommand;
        use std::sync::atomic::{AtomicBool, Ordering};
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage.ensure_session_roster("s1", 2).await.unwrap();
        let hands_id = storage.participant_by_slug("s1", "hands").await.unwrap().unwrap().id;
        let awaiting = Arc::new(AtomicBool::new(false));
        bridge
            .register_session_awaiting("s1".into(), Arc::clone(&awaiting))
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        bridge.register_session_sequencer("s1".into(), tx).await;
        let mut events = bridge.subscribe();

        bridge
            .mark_temporary_halt(
                "s1".into(),
                "hands".into(),
                "CI on PR #531".into(),
                std::time::Duration::from_millis(200),
            )
            .await;
        // Declared: the slot carries the reason AND the wake instant; the ring
        // was told to halt (the ordinary halt machinery).
        let (by, reason, _) = storage.session_halt("s1").await.unwrap().expect("halted");
        assert_eq!((by.as_str(), reason.as_str()), ("hands", "CI on PR #531"));
        let wake = storage.session_halt_wake_at("s1").await.unwrap().expect("a wake instant");
        assert!(wake.ends_with('Z'), "RFC3339-Z: {wake}");
        assert!(awaiting.load(Ordering::SeqCst), "a temporary halt halts");
        assert!(matches!(rx.try_recv(), Ok(SequencerCommand::HaltDeclared { .. })));

        // It expires: the slot clears, a row says so, the ring is released with
        // the declarer summoned, the awaiting flag drops.
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        assert!(storage.session_halt("s1").await.unwrap().is_none(), "the halt ended on its own");
        assert_eq!(storage.session_halt_wake_at("s1").await.unwrap(), None);
        assert!(!awaiting.load(Ordering::SeqCst));
        let released = loop {
            match rx.try_recv() {
                Ok(SequencerCommand::UserMessage { mentions, .. }) => break mentions,
                Ok(_) => continue,
                Err(_) => panic!("the wake must release the ring"),
            }
        };
        assert_eq!(released, vec![hands_id], "the declarer is summoned for the next turn");
        let rows = storage.messages_for_session("s1", None).await.unwrap();
        assert!(
            rows.iter().any(|m| m.kind == "system_notice" && m.content.contains("temporary halt ended") && m.content.contains("CI on PR #531")),
            "a row records the wake: {:?}",
            rows.iter().map(|m| m.content.as_str()).collect::<Vec<_>>()
        );
        let mut cleared = false;
        while let Ok(ev) = events.try_recv() {
            if matches!(ev, SignalingEvent::HaltsCleared { .. }) {
                cleared = true;
            }
        }
        assert!(cleared, "the UI is told the halt cleared");
    }

    #[tokio::test]
    async fn a_temporary_halt_replaced_or_released_before_its_instant_does_not_wake() {
        use crate::core::sequencer::SequencerCommand;
        use std::sync::atomic::AtomicBool;
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage.ensure_session_roster("s1", 2).await.unwrap();
        bridge
            .register_session_awaiting("s1".into(), Arc::new(AtomicBool::new(false)))
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        bridge.register_session_sequencer("s1".into(), tx).await;

        // Replaced: an ordinary halt declared over the temporary one owns the
        // slot; the old timer finds a different slot and does nothing.
        bridge
            .mark_temporary_halt("s1".into(), "hands".into(), "CI".into(), std::time::Duration::from_millis(150))
            .await;
        bridge.mark_awaiting_user("s1".into(), "hands".into(), "plain halt, yours".into()).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let (_, reason, _) = storage.session_halt("s1").await.unwrap().expect("the ordinary halt stands");
        assert_eq!(reason, "plain halt, yours");
        assert!(storage.session_halt_wake_at("s1").await.unwrap().is_none());
        while let Ok(cmd) = rx.try_recv() {
            assert!(!matches!(cmd, SequencerCommand::UserMessage { .. }), "no wake: {cmd:?}");
        }

        // Released by the user before the instant: the slot is empty when the
        // timer fires — nothing happens.
        storage.clear_session_halt("s1").await.unwrap();
        bridge
            .mark_temporary_halt("s1".into(), "hands".into(), "CI again".into(), std::time::Duration::from_millis(150))
            .await;
        storage.clear_session_halt("s1").await.unwrap(); // the user's message, in effect
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(storage.session_halt("s1").await.unwrap().is_none());
        while let Ok(cmd) = rx.try_recv() {
            assert!(!matches!(cmd, SequencerCommand::UserMessage { .. }), "no wake after a release: {cmd:?}");
        }
        let rows = storage.messages_for_session("s1", None).await.unwrap();
        assert!(
            !rows.iter().any(|m| m.content.contains("temporary halt ended")),
            "no wake row was posted"
        );
    }

    #[tokio::test]
    async fn a_temporary_halt_does_not_wake_behind_an_open_gate_or_without_a_ring() {
        use crate::core::sequencer::SequencerCommand;
        use std::sync::atomic::AtomicBool;
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage.ensure_session_roster("s1", 2).await.unwrap();
        bridge
            .register_session_awaiting("s1".into(), Arc::new(AtomicBool::new(false)))
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        bridge.register_session_sequencer("s1".into(), tx).await;
        // A gate is pending (a parked Tool-Gate command).
        bridge
            .ask_user_choice_inner(
                "s1".into(),
                "hands".into(),
                "Run gated command?".into(),
                vec!["Approve".into(), "Reject".into()],
                Some(super::super::ApprovalContext {
                    kind: crate::policy::ViolationKind::ToolBlocklist,
                    action: "rm -rf build".into(),
                    detail: None,
                    command: None,
                }),
                None,
                false,
                true,
            )
            .await
            .unwrap();
        bridge
            .mark_temporary_halt("s1".into(), "hands".into(), "CI".into(), std::time::Duration::from_millis(100))
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        // Expired behind the gate: the halt stands (the wake is deferred; it
        // re-checks every 30 s, past this test's patience).
        assert!(storage.session_halt("s1").await.unwrap().is_some(), "the halt stands behind an open gate");
        while let Ok(cmd) = rx.try_recv() {
            assert!(!matches!(cmd, SequencerCommand::UserMessage { .. }), "no wake behind a gate: {cmd:?}");
        }

        // No ring at all (a closed / not-yet-respawned session): the halt
        // stands for the ring that comes up.
        let b2 = SignalingBridge::new();
        let s2 = crate::storage::Storage::memory().await.unwrap();
        b2.set_storage(s2.clone()).await;
        s2.create_session("s9", "t", None).await.unwrap();
        b2.register_session_awaiting("s9".into(), Arc::new(AtomicBool::new(false))).await;
        b2.mark_temporary_halt("s9".into(), "hands".into(), "deploy".into(), std::time::Duration::from_millis(100))
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(s2.session_halt("s9").await.unwrap().is_some(), "no ring, no wake — the halt stands");
        assert!(s2.session_halt_wake_at("s9").await.unwrap().is_some());
        // …and registering a ring re-arms it: a past instant fires now.
        let (tx9, mut rx9) = tokio::sync::mpsc::channel(8);
        b2.register_session_sequencer("s9".into(), tx9).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(s2.session_halt("s9").await.unwrap().is_none(), "the ring that came up took the wake");
        assert!(matches!(rx9.try_recv(), Ok(SequencerCommand::UserMessage { .. })));
    }

    /// **The dc99564c wire, end to end** (round 13): a gate answered through
    /// `AppState::resolve_choice` on an idle ring — the real path, stub
    /// subprocesses only — with the halt slot read back from storage after.
    /// Three rows of the 12951cc3 table:
    ///   host-declared halt + gate  → released AND slot NULL (release ⇒ clear);
    ///   agent-declared halt + gate → command path runs, but NO release, slot
    ///                                and awaiting flag stand ("halt wins");
    ///   agent-declared halt + question → a user response releases + clears.
    /// Deleting the clear inside `user_responded`, the suppression predicate,
    /// or the release itself each reddens a different assertion here.
    #[tokio::test]
    async fn a_gate_answer_clears_a_host_halt_and_leaves_an_agents_standing() {
        use crate::core::sequencer::SequencerCommand;
        use std::sync::atomic::Ordering;
        /// `ask_user_choice_inner` returns the parked ACK the agent reads —
        /// `{"choice_id":"…","status":"parked"}` — not the bare id.
        fn cid(ack: &str) -> String {
            serde_json::from_str::<serde_json::Value>(ack).unwrap()["choice_id"]
                .as_str()
                .unwrap()
                .to_string()
        }
        let storage = crate::storage::Storage::memory().await.unwrap();
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        let server = crate::signaling::start_signaling_server(Arc::clone(&bridge))
            .await
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::core::AppState::new(
            crate::paths::Paths::for_data_dir(tmp.path().to_path_buf()),
            storage.clone(),
            server,
        )
        .await;

        let park_gate = |sid: &str| {
            let bridge = Arc::clone(&bridge);
            let sid = sid.to_string();
            async move {
                bridge
                    .ask_user_choice_inner(
                        sid,
                        "hands".into(),
                        "Run gated command?".into(),
                        vec!["Approve".into(), "Reject".into()],
                        Some(super::super::ApprovalContext {
                            kind: crate::policy::ViolationKind::ToolBlocklist,
                            action: "echo hi".into(),
                            detail: None,
                            command: None,
                        }),
                        None,
                        false,
                        true,
                    )
                    .await
                    .unwrap()
            }
        };
        // (park_gate returns the raw ack; every use goes through `cid`.)

        // --- Row 1: host halt (consensus et al.) + gate → release AND clear.
        storage.create_session("s1", "t", None).await.unwrap();
        let (h1, _stdin1) = crate::core::session::stub_session_for_tests("s1", &bridge).await;
        let awaiting1 = Arc::clone(&h1.awaiting);
        bridge.register_session_awaiting("s1".into(), Arc::clone(&awaiting1)).await;
        let (tx1, mut rx1) = tokio::sync::mpsc::channel(16);
        bridge.register_session_sequencer("s1".into(), tx1).await;
        state.sessions.lock().await.insert("s1".into(), h1);
        let gate1 = cid(&park_gate("s1").await);
        bridge.declare_host_halt("s1", "All-pass yield".into()).await;
        awaiting1.store(true, Ordering::Release);
        state.resolve_choice(&gate1, "Approve".into(), false).await.unwrap();
        assert!(
            storage.session_halt("s1").await.unwrap().is_none(),
            "a host halt is cleared by the gate answer's release (release ⇒ clear)"
        );
        let mut released = false;
        while let Ok(cmd) = rx1.try_recv() {
            if let SequencerCommand::UserMessage {
                restarts_rotation, ..
            } = cmd
            {
                released = true;
                // The starvation fix's call-site pin (2026-08-27): a gate/tray
                // ANSWER releases WITHOUT resetting the rotation — flipping
                // resolve_choice's flag back to `true` re-deals the front on
                // every approval and the position-1 participant starves (the
                // 137-event measurement). This is the real path, so the pin
                // covers state.rs → bridge → command, not just the enum.
                assert!(
                    !restarts_rotation,
                    "a gate answer must not reset the rotation to the front"
                );
            }
        }
        assert!(released, "the idle ring is released to drain the answer row");
        assert!(!awaiting1.load(Ordering::Acquire), "the awaiting flag cleared");

        // --- Row 2: AGENT halt + gate → suppressed ("halt wins", 12951cc3).
        storage.create_session("s2", "t", None).await.unwrap();
        let (h2, _stdin2) = crate::core::session::stub_session_for_tests("s2", &bridge).await;
        let awaiting2 = Arc::clone(&h2.awaiting);
        bridge.register_session_awaiting("s2".into(), Arc::clone(&awaiting2)).await;
        let (tx2, mut rx2) = tokio::sync::mpsc::channel(16);
        bridge.register_session_sequencer("s2".into(), tx2).await;
        state.sessions.lock().await.insert("s2".into(), h2);
        let gate2 = cid(&park_gate("s2").await);
        storage
            .declare_session_halt("s2", "hands", "recap: waiting on you")
            .await
            .unwrap();
        awaiting2.store(true, Ordering::Release);
        state.resolve_choice(&gate2, "Approve".into(), false).await.unwrap();
        let halt = storage.session_halt("s2").await.unwrap();
        assert_eq!(
            halt.as_ref().map(|(by, _, _)| by.as_str()),
            Some("hands"),
            "the agent's halt slot stands: {halt:?}"
        );
        while let Ok(cmd) = rx2.try_recv() {
            assert!(
                !matches!(cmd, SequencerCommand::UserMessage { .. }),
                "no release under an agent's halt: {cmd:?}"
            );
        }
        assert!(
            awaiting2.load(Ordering::Acquire),
            "the awaiting flag stands too — banner and input state stay true"
        );

        // --- Row 3: the same agent halt + a QUESTION → a user response, as ever.
        let q = bridge
            .ask_user_choice_inner(
                "s2".into(),
                "hands".into(),
                "Which way?".into(),
                vec!["a".into(), "b".into()],
                None,
                None,
                false,
                false,
            )
            .await
            .unwrap();
        let q = cid(&q);
        state.resolve_choice(&q, "a".into(), false).await.unwrap();
        assert!(
            storage.session_halt("s2").await.unwrap().is_none(),
            "answering a question is a user response — it clears the halt"
        );
        let mut released2 = false;
        while let Ok(cmd) = rx2.try_recv() {
            if matches!(cmd, SequencerCommand::UserMessage { .. }) {
                released2 = true;
            }
        }
        assert!(released2, "and releases the ring");
    }

    /// A session with no ring registered must not panic or block — the bridge is
    /// shared with tests and with sessions torn down mid-flight.
    #[tokio::test]
    async fn parking_a_question_without_a_ring_is_a_silent_no_op() {
        use std::sync::atomic::AtomicBool;
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        bridge
            .register_session_awaiting("s1".into(), Arc::new(AtomicBool::new(false)))
            .await;
        // No `register_session_sequencer`.
        bridge
            .mark_awaiting_user("s1".into(), "hands".into(), "blocked".into())
            .await;
    }

    #[tokio::test]
    async fn ask_user_choice_parks_and_returns_immediately() {
        // ask_user_choice is non-blocking: it parks the question and returns a
        // `{status:"parked", choice_id}` ack right away — it does NOT wait for
        // the user, and (rc3 D35) it halts NOTHING: no awaiting flag, no ring
        // command — the assertions below check exactly that. The pick is
        // delivered later out-of-band.
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
                "hands".into(),
                "pick".into(),
                vec!["Yes".into(), "No".into()],
            )
            .await
            .unwrap();
        assert!(ack.contains("\"status\":\"parked\""), "ack: {ack}");
        assert!(ack.contains("choice_id"), "ack: {ack}");
        // rc3 D35: a parked QUESTION sets nothing — no awaiting flag, no ring
        // command. The session keeps working; only the row exists.
        assert!(
            !flag.load(Ordering::Acquire),
            "a parked question must not flag the session as awaiting"
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
            ResolveOutcome::DeliveredOutOfBand { session_id, body, .. } => {
                assert_eq!(session_id, "s1");
                assert!(
                    body.contains("Picked: Yes"),
                    "body: {body}"
                );
            }
            other => panic!("non-blocking ask should resolve via OOB, got {other:?}"),
        }
        assert!(
            !flag.load(Ordering::Acquire),
            "resolve must clear the awaiting halt so the session resumes"
        );
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert!(msgs
            .iter()
            .any(|m| m.content.starts_with("Tray answer ") && m.content.contains("Picked: Yes")));
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
                "hands",
                crate::storage::QuestionKind::Choice,
                "Pick something?",
                Some(&["A".to_string(), "B".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();

        let outcome = bridge.resolve_choice("cid-phase", "A".into()).await.unwrap();
        let ResolveOutcome::DeliveredOutOfBand { body, receipt, .. } = outcome else {
            panic!("expected the OOB fallback path");
        };
        let receipt = receipt.expect("storage is wired, so the answer became a row");
        assert_eq!(receipt.body(), body, "the receipt is for THIS answer");
        assert_eq!(receipt.wire(), format!("[user] [PHASE: Plan]\n{body}"));

        // Dropping the session's IPAV state (its handle is gone, but nothing
        // called `unregister_session`) leaves a dead `Weak`. That is the honest
        // degradation: no phase is known, so the row records — and the agent
        // would read — an untagged body rather than a phase the session may
        // have left. The unregister path is covered separately, in `mod.rs`.
        drop(ipav);
        storage
            .insert_tray_entry(
                "s1",
                "cid-closed",
                "hands",
                crate::storage::QuestionKind::Choice,
                "Pick again?",
                Some(&["A".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();
        let outcome = bridge.resolve_choice("cid-closed", "A".into()).await.unwrap();
        let ResolveOutcome::DeliveredOutOfBand { body, receipt, .. } = outcome else {
            panic!("expected the OOB fallback path");
        };
        // No phase to envelope (the session's IPAV state is gone), so the wire
        // is the body plus the speaker and nothing else — rc3 D23.
        assert_eq!(receipt.unwrap().wire(), format!("[user] {body}"));
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
            .mark_awaiting_user("s1".into(), "hands".into(), "temp.md ready, awaiting go".into())
            .await;
        assert!(first.is_none(), "the first yield has no prior halt");

        let second = bridge
            .mark_awaiting_user("s1".into(), "hands".into(), "temp.md still ready".into())
            .await;
        assert_eq!(
            second.as_deref(),
            Some("temp.md ready, awaiting go"),
            "a second yield with no user reply must surface the earlier one"
        );
    }

    /// **A question is withdrawn by the participant that parked it, and by
    /// nobody else** (A4).
    ///
    /// `withdraw_question` took a `choice_id` and nothing else, so any
    /// participant could clear any other's question out of the user's tray —
    /// including a review-only one, which has no way to ask a question of its
    /// own and therefore no reason to be retracting one.
    ///
    /// Scoping is not gating: WHO may call the tool is a capability question and
    /// the parity oracle says that one is the user's. This is about WHICH ROW a
    /// caller may act on, which was never expressed anywhere.
    #[tokio::test]
    async fn a_question_is_only_withdrawable_by_its_asker() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage
            .insert_tray_entry(
                "s1",
                "q-1",
                "hands",
                crate::storage::QuestionKind::Choice,
                "Which batch next?",
                Some(&["one".to_string(), "two".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();

        assert!(
            !bridge.withdraw_question("q-1", Some("eyes")).await,
            "a peer cleared a question it did not park"
        );
        let still_there = storage.get_tray_entry("q-1").await.unwrap().unwrap();
        assert_eq!(still_there.status, "pending", "and the row survived");
        // Round 9: the refusal is reported as WHAT it is — the row is pending
        // and someone else's — not as "not pending".
        assert_eq!(
            bridge.withdraw_question_for("q-1", Some("eyes")).await,
            Withdrawal::NotYours
        );
        assert_eq!(
            bridge.withdraw_question_for("nope", Some("eyes")).await,
            Withdrawal::NotPending
        );

        assert!(
            bridge.withdraw_question("q-1", Some("hands")).await,
            "the asker withdraws its own"
        );
        assert_eq!(
            storage.get_tray_entry("q-1").await.unwrap().unwrap().status,
            "withdrawn"
        );
    }

    /// Round 12: a withdrawal tells the UI. The auto-supersede and explicit
    /// supersede paths both emit `ChoiceResolved` for the retired row; the
    /// withdraw path (the agent's `withdraw_question` AND the user's Discard
    /// button → `discard_choice`) emitted nothing, so the bell count and the
    /// dashboard badges — which refresh on `session:choice_resolved` — stayed
    /// stale until some other tray event or the resync sweep.
    #[tokio::test]
    async fn a_withdrawal_emits_choice_resolved_for_the_ui() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage
            .insert_tray_entry(
                "s1",
                "q-9",
                "hands",
                crate::storage::QuestionKind::Choice,
                "Which batch next?",
                Some(&["one".to_string(), "two".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();
        let mut sub = bridge.subscribe();
        // The user's Discard path passes no asker.
        assert_eq!(bridge.withdraw_question_for("q-9", None).await, Withdrawal::Withdrawn);
        let mut saw = false;
        while let Ok(ev) = sub.try_recv() {
            if let SignalingEvent::ChoiceResolved { choice_id, picked } = ev {
                assert_eq!(choice_id, "q-9");
                assert_eq!(picked, "(withdrawn)");
                saw = true;
            }
        }
        assert!(saw, "withdrawing a row must emit ChoiceResolved so the bell and badges refresh");
        // A no-op withdrawal (already gone) emits nothing.
        let mut sub = bridge.subscribe();
        assert_eq!(bridge.withdraw_question_for("q-9", None).await, Withdrawal::NotPending);
        assert!(
            !matches!(sub.try_recv(), Ok(SignalingEvent::ChoiceResolved { .. })),
            "nothing to withdraw, nothing to announce"
        );
    }

    /// **The "yours only" scoping fails CLOSED on a storage error** (round 10).
    /// The owner read used to fold `Err` into "no owner" and the withdrawal
    /// went ahead — for an ungated tool whose only control is this scoping. A
    /// real error is induced by dropping the table the read goes through; the
    /// answer is `Unverifiable`, and nothing is touched.
    #[tokio::test]
    async fn a_withdrawal_whose_owner_cannot_be_read_is_refused() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        // A parked (in-memory) row too, so a fail-open path would have had
        // something to remove.
        let _ = bridge
            .ask_user_choice("s1".into(), "hands".into(), "Which?".into(), vec!["a".into()])
            .await;
        sqlx::query("DROP TABLE session_tray")
            .execute(storage.pool())
            .await
            .unwrap();
        assert_eq!(
            bridge.withdraw_question_for("anything", Some("eyes")).await,
            Withdrawal::Unverifiable,
            "an unreadable owner refuses instead of withdrawing"
        );
        assert!(
            !bridge.pending.lock().await.is_empty(),
            "and the in-memory park was left alone"
        );
    }

    /// **A halt whose write FAILS still stops the session, and says it did not
    /// persist** — the behavioural half, on a real storage error.
    ///
    /// EYES' correction to my own claim that this could not be induced: it can.
    /// `Storage::pool()` is public and this file's tests already run raw SQL
    /// through it, so dropping the column `declare_session_halt` writes makes
    /// the UPDATE return a genuine `Err` while `messages` stays intact for the
    /// notice. That closes what the source pin cannot see — an edit that keeps
    /// the order and swallows the error.
    #[tokio::test]
    async fn a_halt_that_cannot_be_recorded_still_stops_and_says_so() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        let mut events = bridge.subscribe();

        // The one column the halt write needs, gone. Everything else — the
        // session row, `messages` and its foreign key — is untouched.
        sqlx::query("ALTER TABLE sessions DROP COLUMN halt_reason")
            .execute(storage.pool())
            .await
            .expect("the column drops");

        bridge
            .mark_awaiting_user("s1".into(), "hands".into(), "waiting on you".into())
            .await;

        // 1. The session still STOPS. An agent that asked to stop must stop,
        //    whatever storage did.
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match events.recv().await {
                    Ok(SignalingEvent::AwaitingUser { reason, .. }) => return reason,
                    Ok(_) => continue,
                    Err(e) => panic!("event channel closed: {e}"),
                }
            }
        })
        .await
        .expect("the halt still raises its banner when the write fails");
        assert_eq!(stopped, "waiting on you");

        // 2. And the user is TOLD it will not survive a restart, rather than
        //    being shown a banner indistinguishable from a durable one.
        let notices: Vec<String> = storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .filter(|c| c.contains("could not be recorded"))
            .collect();
        assert_eq!(
            notices.len(),
            1,
            "a halt that failed to persist looks exactly like one that did: {notices:?}"
        );
    }

    /// **The halt is written before it is shown, and a failed write says so.**
    ///
    /// The old order flipped the awaiting flag and stopped the ring FIRST and
    /// ignored the write's result, so a failed `declare_session_halt` produced a
    /// session that was stopped, bannered, and had no halt in storage — the
    /// banner disappeared at the next restart while the reason for it did not.
    ///
    /// The ORDER half is asserted over the source because a true write
    /// FAILURE (an `Err`) still cannot be induced from a test. The no-match
    /// case CAN be now — since round 13 the UPDATE carries `closed_at IS
    /// NULL` and reports `Ok(false)`, the REFUSAL — and that path is pinned
    /// behaviorally by `a_halt_on_a_closed_session_is_refused_entirely`
    /// below: refusal is an early return, never `recorded = false` (which
    /// would banner a session that has no halt in storage — 828147ad).
    #[test]
    fn the_halt_is_recorded_before_it_is_shown() {
        let src = include_str!("tray.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        let body = prod
            .split("async fn emit_halt_row")
            .nth(1)
            .expect("emit_halt_row exists")
            .split("\n    /// ")
            .next()
            .expect("a split always yields a first part");
        let write = body
            .find("declare_session_halt(")
            .expect("the halt is written");
        let stop = body
            .find("set_session_awaiting(")
            .expect("the halt stops the session");
        assert!(
            write < stop,
            "the session is stopped and bannered before the halt is recorded — a \
             write failure then leaves a halt that vanishes at the next restart"
        );
        assert!(
            body.contains("if !recorded"),
            "a halt that could not be recorded must not look identical to one that \
             persisted"
        );
        assert!(
            body.contains("Ok(false) => {"),
            "the closed-row REFUSAL must be its own arm — folding it into \
             `recorded = false` banners a session with no halt in storage \
             (828147ad)"
        );
    }

    /// **A halt on a closed session is refused entirely** (round 13,
    /// 828147ad). F1's `closed_at IS NULL` predicate made the no-match UPDATE
    /// reachable; deriving `recorded` from a bare `Ok` then produced the
    /// runtime ghost — awaiting flipped, `AwaitingUser` emitted, the
    /// not-recorded notice SUPPRESSED — for a session that held no halt. The
    /// refusal must leave no trace: flag untouched, no event, no notice row,
    /// slot empty.
    #[tokio::test]
    async fn a_halt_on_a_closed_session_is_refused_entirely() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage.close_session("s1", false).await.unwrap();

        let awaiting = Arc::new(AtomicBool::new(false));
        bridge
            .register_session_awaiting("s1".into(), Arc::clone(&awaiting))
            .await;
        let mut events = bridge.subscribe();

        bridge
            .mark_awaiting_user("s1".into(), "hands".into(), "ghost recap".into())
            .await;

        assert!(
            storage.session_halt("s1").await.unwrap().is_none(),
            "no slot on a closed row"
        );
        assert!(
            !awaiting.load(Ordering::Acquire),
            "the awaiting flag must not flip for a refused halt"
        );
        let mut saw_awaiting = false;
        while let Ok(ev) = events.try_recv() {
            if matches!(ev, SignalingEvent::AwaitingUser { .. }) {
                saw_awaiting = true;
            }
        }
        assert!(!saw_awaiting, "no banner event for a refused halt");
        let notices = storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.content.contains("could not be recorded"))
            .count();
        assert_eq!(notices, 0, "no false not-recorded notice either");
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
            .mark_awaiting_user("s1".into(), "hands".into(), "first".into())
            .await;
        // The real path a user reply takes (core::state::broadcast), not a
        // stand-in — this is the mechanism the guard's "still pending means
        // unanswered" assumption depends on.
        let cleared = storage.clear_session_halt("s1").await.unwrap();
        assert!(cleared, "the session's halt slot should have been set");

        let after = bridge
            .mark_awaiting_user("s1".into(), "hands".into(), "second".into())
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
            .mark_awaiting_user("s1".into(), "eyes".into(), "eyes waits".into())
            .await;
        let hands = bridge
            .mark_awaiting_user("s1".into(), "hands".into(), "hands waits".into())
            .await;
        assert!(hands.is_none(), "another agent's halt is not this agent's repeat");
    }

    #[tokio::test]
    async fn mark_awaiting_user_broadcasts() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        bridge
            .mark_awaiting_user("s1".into(), "hands".into(), "ping".into())
            .await;
        let ev = sub.recv().await.unwrap();
        assert!(
            matches!(ev, SignalingEvent::AwaitingUser { session_id, agent, reason }
            if session_id == "s1" && agent == "hands" && reason == "ping")
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
        let tracker = ActivityTracker::new(
            "s1",
            Arc::clone(&awaiting),
            bridge.clone(),
            vec!["hands".into(), "eyes".into()],
        );
        bridge
            .register_session_activity("s1".into(), Arc::downgrade(&tracker))
            .await;

        let mut sub = bridge.subscribe();
        bridge
            .mark_awaiting_user("s1".into(), "hands".into(), "ping".into())
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
                "hands",
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
            ResolveOutcome::DeliveredOutOfBand { session_id, body, .. } => {
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
            .any(|m| m.content.starts_with("Tray answer ") && m.content.contains("Picked: Yes")));
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
                "hands",
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
                    "hands".into(),
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
            ResolveOutcome::DeliveredOutOfBand { session_id, body, .. } => {
                assert_eq!(session_id, "s-fallback");
                assert!(body.contains("Picked: A"), "body: {body}");
            }
            other => panic!("expected DeliveredOutOfBand, got {other:?}"),
        }

        // Verify the out-of-band message also landed in storage (for UI + poll).
        let msgs = storage
            .messages_for_session("s-fallback", None)
            .await
            .unwrap();
        let oob = msgs
            .iter()
            .find(|m| m.content.starts_with("Tray answer "))
            .expect("expected the tray-answer user row");
        assert_eq!(oob.author, "user");
        assert!(oob.content.contains("Picked: A"));

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
                "hands",
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
                    "hands",
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
        let ResolveOutcome::DeliveredOutOfBand { body, .. } = outcome else {
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
                "hands",
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
                "hands",
                crate::storage::QuestionKind::Choice,
                "Pick something?",
                Some(&opts),
                None,
                None,
            )
            .await
            .unwrap();

        let outcome = bridge.resolve_choice("cid-q", "A".into()).await.unwrap();
        let ResolveOutcome::DeliveredOutOfBand { body, .. } = outcome else {
            panic!("expected the OOB fallback path");
        };
        assert!(!body.contains("Approved in this session after you asked"));
        assert!(!body.contains("git push origin main"));
        assert!(body.contains("Picked: A"));
    }

    /// The block is for QUESTIONS (round 10, B5): an approval row resolved after
    /// a sibling gate was approved carries no "approved since you asked" list —
    /// its siblings each arrive as their own answer row.
    #[tokio::test]
    async fn oob_replay_of_an_approval_carries_no_mooting_block() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s-gates", "title", None).await.unwrap();
        let gate_opts = vec!["Approve".to_string(), "Reject".to_string()];
        // Two gates parked together (the merge pair from s-766f4ab9)…
        for (cid, cmd) in [
            ("cid-merge-528", "merge-tool 528 --squash"),
            ("cid-merge-529", "merge-tool 529 --squash"),
        ] {
            storage
                .insert_tray_entry(
                    "s-gates",
                    cid,
                    "hands",
                    crate::storage::QuestionKind::Approval,
                    "Run gated command in this session's repo?",
                    Some(&gate_opts),
                    None,
                    Some(cmd),
                )
                .await
                .unwrap();
        }
        // …the first approved strictly after both were asked (now_utc() is
        // millisecond-precision; nudge past the inserts), then the second
        // resolved.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        storage.answer_tray_entry("cid-merge-528", "Approve").await.unwrap();
        // No in-memory park (a durable-row resolve), so the body is the OOB one.
        // Reject, so nothing runs.
        let outcome = bridge
            .resolve_choice("cid-merge-529", "Reject".into())
            .await
            .unwrap();
        let ResolveOutcome::DeliveredOutOfBand { body, .. } = outcome else {
            panic!("expected the OOB path");
        };
        assert!(
            !body.contains("Approved in this session after you asked"),
            "an approval's answer must not list its sibling gates as mooting: {body}"
        );
        assert!(!body.contains("merge-tool 528"), "the sibling is not named here: {body}");
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
                    "hands".into(),
                    "Approve push?".into(),
                    vec!["Approve".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push".into(),
                        detail: None,
                        command: None,
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
                "hands".into(),
                "Query prod?".into(),
                vec!["Approve".into(), "Deny".into()],
                ApprovalContext {
                    kind: ViolationKind::PerAction,
                    action: "bq query ...".into(),
                    detail: None,
                    command: None,
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
        // Regression for "the session goes silent after the user answers": a
        // Delivered resolve must clear the awaiting halt the gate set. (The
        // original symptom was the deleted bilateral router dropping every
        // peer-forward while the flag stayed up; today the flag is what the D35
        // gate release and the input lock read, and a stale one still reads as
        // a session waiting on the user.) Uses request_approval (the blocking
        // path that yields Delivered); the non-blocking ask_user_choice sets no
        // flag at all — see ask_user_choice_parks_and_returns_immediately.
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
                    "hands".into(),
                    "Approve push?".into(),
                    vec!["Approve".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push".into(),
                        detail: None,
                        command: None,
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
        // The gate halts the session; set_session_awaiting runs before the
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
            "a Delivered resolve must clear the awaiting halt so the session resumes"
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
                    "hands".into(),
                    "Approve push?".into(),
                    vec!["Approve once".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::PushGate,
                        action: "git push origin main".into(),
                        detail: Some("per_branch_approval".into()),
                        command: None,
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
                    "hands".into(),
                    "Approve force-push?".into(),
                    vec!["Approve".into(), "Deny".into()],
                    ApprovalContext {
                        kind: ViolationKind::ForcePush,
                        action: "git push --force origin main".into(),
                        detail: None,
                        command: None,
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

    /// **A gate answered after a restart is still audited** (round 10). With no
    /// live park — the durable-row branch — the resolve used to write nothing
    /// to violations.jsonl while `request_approval`'s descriptor told the agent
    /// every outcome is recorded. The kind is reconstructed from the row: a
    /// gated command records as the Tool Gate's, a command-less approval as
    /// generic; a plain question still records nothing.
    #[tokio::test]
    async fn a_gate_resolved_from_the_durable_row_is_still_audited() {
        let dir = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        let gate_opts = vec!["Approve".to_string(), "Reject".to_string()];
        // Rows written before "the restart": no in-memory park for either.
        storage
            .insert_tray_entry(
                "s1",
                "g-cmd",
                "hands",
                crate::storage::QuestionKind::Approval,
                "Run gated command in this session's repo?",
                Some(&gate_opts),
                None,
                Some("echo hi"),
            )
            .await
            .unwrap();
        storage
            .insert_tray_entry(
                "s1",
                "g-plain",
                "hands",
                crate::storage::QuestionKind::Approval,
                "Query prod read-only?",
                Some(&gate_opts),
                None,
                None,
            )
            .await
            .unwrap();
        storage
            .insert_tray_entry(
                "s1",
                "q-1",
                "hands",
                crate::storage::QuestionKind::Choice,
                "Which?",
                Some(&["a".to_string(), "b".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();
        // Reject the command (nothing runs), approve the plain gate, answer
        // the question.
        bridge.resolve_choice("g-cmd", "Reject".into()).await.unwrap();
        bridge.resolve_choice("g-plain", "Approve".into()).await.unwrap();
        bridge.resolve_choice("q-1", "a".into()).await.unwrap();

        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 2, "two gates audited, the question not: {recs:?}");
        assert_eq!(recs[0].kind, ViolationKind::ToolBlocklist);
        assert_eq!(recs[0].action, "echo hi");
        assert_eq!(recs[0].outcome, ViolationOutcome::Denied);
        assert_eq!(recs[1].kind, ViolationKind::GenericApproval);
        assert_eq!(recs[1].action, "Query prod read-only?");
        assert_eq!(recs[1].outcome, ViolationOutcome::Approved);
    }

    /// **The durable-row audit is written once, and reads the pick the way the
    /// live path does** (round 11). Two defects in the round-10 block: it was
    /// not gated on `flipped`, so a second `resolve_choice` on an already
    /// answered gate — which flips nothing, runs nothing and lifts nothing —
    /// appended a second, possibly contradicting record; and it classified
    /// every pick with the fail-closed `gate_verdict`, so a `request_approval`
    /// with its own labels (a gate since 76cd7aa) audited an approving pick
    /// as Denied, while the in-memory branch dispatches by menu.
    #[tokio::test]
    async fn the_durable_row_audit_is_written_once_and_reads_a_custom_menu() {
        let dir = tempfile::tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage
            .insert_tray_entry(
                "s1",
                "g-custom",
                "hands",
                crate::storage::QuestionKind::Approval,
                "Query prod read-only?",
                Some(&["Approve — read only".to_string(), "Deny with reason".to_string()]),
                None,
                None,
            )
            .await
            .unwrap();
        bridge
            .resolve_choice("g-custom", "Approve — read only".into())
            .await
            .unwrap();
        // The second answer flips nothing: an already-answered row is not
        // pending, so the UPDATE matches no row.
        let _ = bridge.resolve_choice("g-custom", "Deny with reason".into()).await;

        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1, "one record for one gate, however often it is clicked: {recs:?}");
        assert_eq!(recs[0].kind, ViolationKind::GenericApproval);
        assert_eq!(
            recs[0].outcome,
            ViolationOutcome::Approved,
            "an approving pick from the agent's own menu audits as approved"
        );
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
                    "hands".into(),
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
                    "hands".into(),
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
                    "hands".into(),
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
                "hands".into(),
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
                "hands".into(),
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

    /// **A supersede is scoped to the caller's own rows** (round 11). The
    /// tool says "replace a stale question YOU parked … without disturbing
    /// the others", `withdraw_question` refuses a peer's row (`NotYours`), and
    /// this path — which retires a row and mints a new one — did no check at
    /// all: any participant holding the capability could retire another's
    /// pending question, or one from another session, by choice_id.
    #[tokio::test]
    async fn supersede_refuses_a_peers_row_and_another_sessions_row() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("test.db")).await.unwrap();
        storage.create_session("s1", "test", None).await.unwrap();
        storage.create_session("s2", "test", None).await.unwrap();
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        let opts = ["a".to_string(), "b".to_string()];
        storage
            .insert_tray_entry("s1", "eyes-cid", "eyes", crate::storage::QuestionKind::Choice, "eyes asks", Some(&opts), None, None)
            .await
            .unwrap();
        storage
            .insert_tray_entry("s2", "other-cid", "hands", crate::storage::QuestionKind::Choice, "other session", Some(&opts), None, None)
            .await
            .unwrap();

        // hands (s1) tries to supersede eyes's row.
        let res = bridge
            .supersede_question_with_new(
                "s1".into(),
                "hands".into(),
                "eyes-cid".into(),
                "rephrased".into(),
                vec!["x".into(), "y".into()],
            )
            .await;
        assert!(res.is_err(), "a peer's row is not yours to supersede");
        // hands (s1) tries to supersede its own slug's row in ANOTHER session.
        let res = bridge
            .supersede_question_with_new(
                "s1".into(),
                "hands".into(),
                "other-cid".into(),
                "rephrased".into(),
                vec!["x".into(), "y".into()],
            )
            .await;
        assert!(res.is_err(), "another session's row is not yours to supersede");

        // Both rows are untouched, and no new question was parked anywhere.
        let eyes = storage.get_tray_entry("eyes-cid").await.unwrap().unwrap();
        assert_eq!(eyes.status, "pending");
        let other = storage.get_tray_entry("other-cid").await.unwrap().unwrap();
        assert_eq!(other.status, "pending");
        assert_eq!(storage.tray_entries_for_session("s1").await.unwrap().len(), 1);
        assert_eq!(storage.tray_entries_for_session("s2").await.unwrap().len(), 1);
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
                "hands",
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
                    "hands".into(),
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
