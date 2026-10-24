//! `session_tray` table: durable mirror of the in-chat tray. Every
//! `ask_user_choice` / `mark_awaiting_user` / phase-request writes a row here
//! so the tray + dashboard counter survive restart, and answers/withdrawals/
//! supersedes flip the row's status.

use super::*;

/// Full column projection for a `SessionTrayEntry` row — shared by
/// `tray_entries_for_session` and `get_tray_entry` so the two can't drift.
const TRAY_COLUMNS: &str = "id, session_id, choice_id, agent, kind, prompt, \
     options_json, status, picked_option, asked_at, answered_at, supersedes_id, command_text, \
     body_row_id, body_sha256, run_refusal, result_row_id, exit_code, ran_ms, live_body_sha256, exec_kind, \
     run_timeout_secs, pinned_scripts, urgent_reason, run_not_before, run_not_after, run_started_at, \
     approval_required, reviewed_at";

/// The statuses a tray row passes through. `queued` (0080) is the one that is
/// NOT a user-facing item: an outward publish waiting for the reviewer to read
/// its body. It renders nowhere, latches nothing, and cannot be answered
/// (`answer_tray_entry` flips `pending` only); settlement promotes it.
pub const TRAY_STATUS_QUEUED: &str = "queued";

/// The `picked_option` a queued gate carries when a reviewer's blocking
/// finding withdrew it — written by settlement, read by
/// [`Storage::latest_gate_withdrawn_by_finding`]. One constant so the two
/// cannot drift apart (a drift would silently re-open the re-issue bypass).
pub const FINDING_WITHDRAWAL_REASON: &str = "withdrawn: the reviewer filed a blocking finding";

/// The statuses [`Storage::tray_entries_for_session`] returns — the rows the
/// UI may render. An explicit allow-list, in SQL, so "a queued row is
/// invisible" is a guarantee of the read and not of eight frontend filters
/// comparing `status === "pending"` (EYES P8): a future `!== "answered"` there
/// would otherwise surface a queued row as an approvable card, and approving
/// one would publish before the reviewer had been dealt.
const TRAY_VISIBLE_STATUSES: &str = "'pending', 'answered', 'superseded', 'withdrawn', 'auto_running', 'auto_resolved'";

/// The options an approval gate parks with, verbatim — the discriminator that
/// tells a GATE (action gate, push gate) from an ordinary parked question.
///
/// One definition, because it is compared in four places across three layers
/// (this file's seeding query, the bridge's withdraw and resolve paths, the
/// app layer's tray-answer classification) and a fifth in the frontend. Four
/// hand-written copies of a JSON literal is how a gate stops being recognised
/// on one path only — which reads as a stuck latch, not as a typo.
pub const GATE_OPTIONS_JSON: &str = r#"["Approve","Reject"]"#;

/// Is this row's `options_json` a gate's?
///
/// Takes the column as stored (`Option<&str>`) so every caller asks the same
/// question of the same shape. Prefer [`is_gate_row`], which also reads the
/// `kind` written at insert since round 8; this is its fallback half.
pub fn is_gate_options(options_json: Option<&str>) -> bool {
    options_json == Some(GATE_OPTIONS_JSON)
}

/// Is this tray row a GATE (answered in the gate slot, latching the ring)?
///
/// `kind = 'approval'` since round 8 — the backend knows at insert whether a
/// row is policy-initiated, so it says so instead of leaving every reader to
/// re-derive it from the options string. (Round 11 made an agent's
/// `request_approval` a gate too; round 12 undid that: it parks as
/// `kind = 'request'`, a tray item that latches nothing — only the host's
/// gates are `approval`. See `bridge/tray.rs`'s insert.) The options check
/// stays as the fallback for rows parked before that (`kind = 'choice'` with
/// the gate menu). Every reader — the ring's gate reseed, the resolve path's
/// latch release, the app layer's tray-answer classification, the frontend's
/// slot choice — goes through this pair, so a gate cannot be recognised on one
/// path and missed on another (which reads as a stuck latch).
pub fn is_gate_row(kind: &str, options_json: Option<&str>) -> bool {
    // The menu fallback is for the LEGACY kind only (round 12, EYES F15): a
    // `request` row may carry the canonical ["Approve","Reject"] menu — the
    // shape the descriptor's convention produces — and is still a tray item;
    // testing the menu independently of kind dragged it into the gate slot
    // as a gate that blocked nothing.
    // A `close` card (0081) is a gate too: it latches the ring while the user
    // decides and lifts it on either answer — the resolve path handles the
    // Approve (a close request) after the shared lift.
    kind == QuestionKind::Approval.as_str()
        || kind == QuestionKind::Close.as_str()
        || (kind == QuestionKind::Choice.as_str() && is_gate_options(options_json))
}

impl Storage {
    /// Insert a fresh tray-entry row in `pending` status. Returns the row id.
    /// `options` is required when kind=Choice (encoded to JSON); ignored
    /// otherwise. `supersedes_id` links to the entry this one replaces
    /// (when an agent rephrases via `supersede_tray_entry`).
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_tray_entry(
        &self,
        session_id: &str,
        choice_id: &str,
        agent: &str,
        kind: QuestionKind,
        prompt: &str,
        options: Option<&[String]>,
        supersedes_id: Option<i64>,
        command_text: Option<&str>,
    ) -> Result<i64> {
        // F10: the prompt and every option are redacted before they are stored
        // — each option BEFORE the JSON is built, never the JSON text (EYES
        // 8eea5190). The bridge's park already redacted them so its in-memory
        // copy agrees; this is the backstop for every other way into the tray
        // (the close card). `command_text` is stored as written: it runs as
        // written, and five queries match it exactly.
        let prompt = crate::policy::secret_scan::redact(prompt);
        let options_json = options
            .filter(|_| matches!(kind, QuestionKind::Choice | QuestionKind::Approval | QuestionKind::Close))
            .map(|opts| {
                let opts: Vec<std::borrow::Cow<'_, str>> =
                    opts.iter().map(|o| crate::policy::secret_scan::redact(o)).collect();
                serde_json::to_string(&opts).unwrap_or_else(|_| "[]".into())
            });
        let res = sqlx::query(
            "INSERT INTO session_tray \
                (session_id, choice_id, agent, kind, prompt, options_json, supersedes_id, command_text, asked_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(choice_id)
        .bind(agent)
        .bind(kind.as_str())
        .bind(prompt.as_ref())
        .bind(options_json)
        .bind(supersedes_id)
        .bind(command_text)
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .with_context(|| format!("inserting tray entry {choice_id} for session {session_id}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Mark a tray entry as answered + record the picked option. Idempotent on
    /// already-answered:
    /// returns Ok with 0 rows affected so callers don't have to guard.
    pub async fn answer_tray_entry(&self, choice_id: &str, picked: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray \
             SET status = 'answered', picked_option = ?, answered_at = ? \
             WHERE choice_id = ? AND status = 'pending'",
        )
        .bind(picked)
        .bind(now_utc())
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("answering tray entry {choice_id}"))?;
        Ok(res.rows_affected())
    }

    // `clear_pending_halts` lived here until rc3 D35 moved the halt off the
    // tray entirely — it is a session-state slot now (`clear_session_halt`,
    // storage/sessions.rs). Nothing writes kind='halt' rows any more (round 11
    // removed the `QuestionKind::Halt` variant with them); the ones in the
    // archive are legacy DATA, matched as the literal `"halt"` by readers
    // (`HaltBanner.tsx::isTrayItem`) and retired by the boot GC.

    /// Mark a tray entry as withdrawn (agent abandons it; never to be answered).
    pub async fn withdraw_tray_entry(&self, choice_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray \
             SET status = 'withdrawn' \
             WHERE choice_id = ? AND status = 'pending'",
        )
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("withdrawing tray entry {choice_id}"))?;
        Ok(res.rows_affected())
    }

    /// Mark a tray entry as superseded by another (agent rephrased).
    pub async fn supersede_tray_entry(&self, choice_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray \
             SET status = 'superseded' \
             WHERE choice_id = ? AND status = 'pending'",
        )
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("superseding tray entry {choice_id}"))?;
        Ok(res.rows_affected())
    }

    /// WHICH approval gates are open for this session — the `choice_id` of every
    /// pending row whose options are exactly `Approve`/`Reject`, which is what
    /// both gate kinds (action gate, push gate) ask and no ordinary question
    /// ever has (the same discriminator the UI's `isApproval` uses; verified
    /// exact across every row ever recorded — 31 matches, all gates). Seeds the
    /// ring's gate latch on spawn (rc3 D35), so a respawned session cannot deal
    /// turns under a gate that parked before the restart.
    ///
    /// **Ids, not a count** (C2-2). The latch used to be a `usize` seeded from
    /// here and incremented per `GateOpened`, so the SAME gate could be counted
    /// twice — its row lands before the ring starts, its notify arrives after —
    /// and one resolve could never clear it: the ring then deals nothing for the
    /// life of the process. A set keyed by `choice_id` cannot double-count, and
    /// makes a resolve for an unknown gate a no-op instead of a decrement of
    /// somebody else's.
    pub async fn pending_gate_ids(&self, session_id: &str) -> Result<Vec<String>> {
        let ids: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT choice_id FROM session_tray \
             WHERE session_id = ? AND status = 'pending' \
               AND (kind = 'approval' OR kind = 'close' OR options_json = '{GATE_OPTIONS_JSON}')"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids)
    }

    /// Insert an agent's CLOSE card as a QUEUED row (feedback #73 (2)): the
    /// reviewer reads the recap before the user sees the card. Kind `close`,
    /// no command; promoted or withdrawn by the same settlement as a queued
    /// publish.
    pub async fn insert_queued_close(&self, session_id: &str, choice_id: &str, agent: &str, prompt: &str) -> Result<i64> {
        let prompt = crate::policy::secret_scan::redact(prompt);
        let res = sqlx::query(
            "INSERT INTO session_tray \
                (session_id, choice_id, agent, kind, prompt, options_json, command_text, asked_at, status) \
             VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?)",
        )
        .bind(session_id)
        .bind(choice_id)
        .bind(agent)
        .bind(QuestionKind::Close.as_str())
        .bind(prompt.as_ref())
        .bind(GATE_OPTIONS_JSON)
        .bind(now_utc())
        .bind(TRAY_STATUS_QUEUED)
        .execute(&self.pool)
        .await
        .with_context(|| format!("queueing close card {choice_id} for session {session_id}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Read the RENDERABLE tray entries for a session, ordered oldest-first —
    /// every status in [`TRAY_VISIBLE_STATUSES`], never `queued`. Use for the
    /// in-chat tray (filter to status=pending in the UI) and the dashboard
    /// counter (count where status=pending).
    pub async fn tray_entries_for_session(&self, session_id: &str) -> Result<Vec<SessionTrayEntry>> {
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE session_id = ? AND status IN ({TRAY_VISIBLE_STATUSES}) ORDER BY id ASC"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ---- queued outward publishes (0080) ----------------------------------

    /// Insert an outward gated command as a QUEUED row: the reviewer has not yet
    /// read its body, so it is not the user's to answer. Same prompt, options
    /// and `command_text` a park writes, so promotion changes the status and
    /// nothing else the resolve path reads. `asked_at` is the QUEUE instant
    /// here; promotion moves it to the park instant.
    pub async fn insert_queued_gate(
        &self,
        session_id: &str,
        choice_id: &str,
        agent: &str,
        prompt: &str,
        command: &str,
    ) -> Result<i64> {
        self.insert_queued_gate_with_approval(session_id, choice_id, agent, prompt, command, true).await
    }

    /// The authority is stored in the INSERT, before a reviewer can settle
    /// the row. A later flag flip could race a fast review or downgrade an
    /// existing user gate. This is host state, never a tool-supplied flag.
    pub async fn insert_queued_gate_with_approval(
        &self,
        session_id: &str,
        choice_id: &str,
        agent: &str,
        prompt: &str,
        command: &str,
        approval_required: bool,
    ) -> Result<i64> {
        // F10: the card's prompt is redacted like every tray prompt; the
        // command is stored as written (see `insert_tray_entry`).
        let prompt = crate::policy::secret_scan::redact(prompt);
        let res = sqlx::query(
            "INSERT INTO session_tray \
                (session_id, choice_id, agent, kind, prompt, options_json, command_text, asked_at, status, approval_required) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(choice_id)
        .bind(agent)
        .bind(QuestionKind::Approval.as_str())
        .bind(prompt.as_ref())
        .bind(GATE_OPTIONS_JSON)
        .bind(command)
        .bind(now_utc())
        .bind(TRAY_STATUS_QUEUED)
        .bind(approval_required)
        .execute(&self.pool)
        .await
        .with_context(|| format!("queueing outward gate {choice_id} for session {session_id}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Record which posted body row a queued gate waits on.
    pub async fn set_tray_body_row(&self, choice_id: &str, body_row_id: i64) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray SET body_row_id = ?, reviewed_at = NULL \
             WHERE choice_id = ? AND status = 'queued' AND run_started_at IS NULL",
        )
        .bind(body_row_id)
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording the body row of queued gate {choice_id}"))?;
        Ok(res.rows_affected())
    }

    /// A stronger request can require user approval, never remove it. It
    /// invalidates automatic review authority before a run can be claimed.
    pub async fn require_queued_gate_approval(&self, choice_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray SET approval_required = 1, reviewed_at = NULL \
             WHERE choice_id = ? AND status = 'queued' AND run_started_at IS NULL",
        ).bind(choice_id).execute(&self.pool).await
            .context("requiring user approval for queued gate")?;
        Ok(res.rows_affected())
    }

    /// Revoke the old body BEFORE replacing its hashes/pins and posting its
    /// revision. Clearing just the timestamp would let a stale settlement
    /// re-authorize the old body while the replacement is still being built.
    /// The re-poster must stop when this loses to a run claim or newer revision.
    pub async fn begin_queued_gate_revision(&self, choice_id: &str, body_row_id: i64) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE session_tray SET body_row_id = NULL, reviewed_at = NULL \
             WHERE choice_id = ? AND body_row_id = ? AND status = 'queued' AND run_started_at IS NULL",
        ).bind(choice_id).bind(body_row_id).execute(&self.pool).await
            .context("revoking queued gate review before revision")?;
        Ok(res.rows_affected() == 1)
    }

    /// Record review of exactly the body that the caller checked. A concurrent
    /// revision, stronger approval requirement or second settlement loses.
    pub async fn mark_auto_gate_reviewed(&self, choice_id: &str, body_row_id: i64) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE session_tray SET reviewed_at = ? \
             WHERE choice_id = ? AND status = 'queued' AND kind = 'approval' \
               AND approval_required = 0 AND body_row_id = ? AND reviewed_at IS NULL \
               AND run_started_at IS NULL AND run_refusal IS NULL \
               AND command_text IS NOT NULL AND result_row_id IS NULL \
               AND exec_kind IS NULL AND urgent_reason IS NULL \
               AND run_not_before IS NULL AND run_not_after IS NULL",
        ).bind(now_utc()).bind(choice_id).bind(body_row_id).execute(&self.pool).await
            .context("recording review authority for automatic gate")?;
        Ok(res.rows_affected() == 1)
    }

    pub async fn invalidate_auto_gate_review(&self, choice_id: &str, body_row_id: i64) -> Result<()> {
        sqlx::query("UPDATE session_tray SET reviewed_at = NULL WHERE choice_id = ? AND body_row_id = ? AND status = 'queued' AND approval_required = 0")
            .bind(choice_id).bind(body_row_id).execute(&self.pool).await
            .context("invalidating automatic review after a revision request")?;
        Ok(())
    }

    /// Durable work for the automatic-run worker, including after restart.
    /// No user answer is manufactured; ready rows retain NULL picked_option
    /// and answered_at. Closed sessions are never runnable.
    pub async fn reviewed_auto_gates(&self) -> Result<Vec<SessionTrayEntry>> {
        sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE status = 'queued' AND kind = 'approval' AND approval_required = 0 \
               AND reviewed_at IS NOT NULL AND run_started_at IS NULL \
               AND run_refusal IS NULL AND result_row_id IS NULL \
               AND command_text IS NOT NULL \
               AND exec_kind IS NULL AND urgent_reason IS NULL \
               AND run_not_before IS NULL AND run_not_after IS NULL \
               AND EXISTS (SELECT 1 FROM sessions s WHERE s.id = session_tray.session_id AND s.closed_at IS NULL) \
             ORDER BY id"
        )).fetch_all(&self.pool).await.context("reading reviewed automatic gates")
    }

    /// Every QUEUED outward gate in a session, oldest-first.
    pub async fn queued_gates_for_session(&self, session_id: &str) -> Result<Vec<SessionTrayEntry>> {
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE session_id = ? AND status = 'queued' ORDER BY id ASC"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// The choice_id of a QUEUED gate with this exact command in this session,
    /// if any — the dedupe for the queue, as `pending_gate_for_command` is for
    /// the park: an identical re-issue must not queue twice and summon the
    /// reviewer twice (EYES P8).
    pub async fn queued_gate_for_command(
        &self,
        session_id: &str,
        command: &str,
    ) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT choice_id FROM session_tray \
             WHERE session_id = ? AND status = 'queued' AND command_text = ? \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(command)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id,)| id))
    }

    /// Promote a queued gate to the user's `pending` card. `asked_at` becomes
    /// the park instant so the card's age and the stale-gate check measure the
    /// user's wait, not the reviewer's. Returns rows affected — 1 means THIS
    /// call did the flip, so a racing second settlement cannot double-latch.
    pub async fn promote_queued_gate(&self, choice_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray SET status = 'pending', asked_at = ?, approval_required = 1, reviewed_at = NULL \
             WHERE choice_id = ? AND status = 'queued'",
        )
        .bind(now_utc())
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("promoting queued gate {choice_id}"))?;
        Ok(res.rows_affected())
    }

    /// Bridge promotion is based on a reviewed snapshot. A revision that
    /// changed/cleared its body pointer must not inherit that promotion.
    pub async fn promote_queued_gate_for_body(&self, choice_id: &str, body_row_id: Option<i64>) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray SET status = 'pending', asked_at = ?, approval_required = 1, reviewed_at = NULL \
             WHERE choice_id = ? AND body_row_id IS ? AND status = 'queued' AND run_started_at IS NULL",
        ).bind(now_utc()).bind(choice_id).bind(body_row_id).execute(&self.pool).await
            .context("promoting the reviewed body to user approval")?;
        Ok(res.rows_affected())
    }

    /// Withdraw a queued gate (the reviewer filed a blocking finding, or the
    /// session closed). `picked_option` carries the reason for `gate_status`.
    pub async fn withdraw_queued_gate(&self, choice_id: &str, reason: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray SET status = 'withdrawn', picked_option = ? \
             WHERE choice_id = ? AND status = 'queued'",
        )
        .bind(reason)
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("withdrawing queued gate {choice_id}"))?;
        Ok(res.rows_affected())
    }

    /// Record the SHA-256 of a gated command's body files (0084) — the content
    /// its review covered, re-checked when the user approves it.
    pub async fn set_tray_body_sha(&self, choice_id: &str, sha: &str) -> Result<()> {
        sqlx::query("UPDATE session_tray SET body_sha256 = ? WHERE choice_id = ?")
            .bind(sha)
            .bind(choice_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording the body hash of gate {choice_id}"))?;
        Ok(())
    }

    /// Mark how an approved gate runs (0089): `prod_read` is the one value,
    /// set by the prod_read handler alone — the mark that lets approval hand
    /// the project's database password to psql through its environment.
    pub async fn set_tray_exec_kind(&self, choice_id: &str, kind: &str) -> Result<()> {
        sqlx::query("UPDATE session_tray SET exec_kind = ? WHERE choice_id = ?")
            .bind(kind)
            .bind(choice_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("marking gate {choice_id} as {kind}"))?;
        Ok(())
    }

    /// Record the scripts a gate's command runs that were readable at park,
    /// with their hashes (0096, feedback #55) — `json` is the object the
    /// approval re-checks.
    pub async fn set_tray_pinned_scripts(&self, choice_id: &str, json: &str) -> Result<()> {
        sqlx::query("UPDATE session_tray SET pinned_scripts = ? WHERE choice_id = ?")
            .bind(json)
            .bind(choice_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording gate {choice_id}'s pinned scripts"))?;
        Ok(())
    }

    /// Record the bound an approved gate's command runs under (0093): the
    /// agent's `timeout_secs`, already validated by the caller (1..=600).
    pub async fn set_tray_run_timeout_secs(&self, choice_id: &str, secs: u64) -> Result<()> {
        sqlx::query("UPDATE session_tray SET run_timeout_secs = ? WHERE choice_id = ?")
            .bind(secs as i64)
            .bind(choice_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording gate {choice_id}'s run bound"))?;
        Ok(())
    }

    /// Record a gate's run window (0098, feedback #66a) on a row still waiting.
    pub async fn set_tray_run_window(&self, choice_id: &str, not_before: &str, not_after: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE session_tray SET run_not_before = ?, run_not_after = ? \
             WHERE choice_id = ? AND status IN ('pending', 'queued')",
        )
        .bind(not_before)
        .bind(not_after)
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording gate {choice_id}'s run window"))?;
        Ok(res.rows_affected() == 1)
    }

    /// Claim the run of an approved gate (0100, EYES 2577e626): the ONE
    /// atomic stamp both run paths take before executing. `true` = this caller
    /// owns the run; `false` = it was already claimed (a run is in progress or
    /// done), or the row has a result or a refusal — run nothing.
    pub async fn claim_gate_run(&self, choice_id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE session_tray SET run_started_at = ? \
             WHERE choice_id = ? AND run_started_at IS NULL \
               AND result_row_id IS NULL AND run_refusal IS NULL AND approval_required = 1",
        )
        .bind(now_utc())
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("claiming the run of gate {choice_id}"))?;
        Ok(res.rows_affected() == 1)
    }

    /// A separate claim for policy-authorized work. The authority and exact
    /// reviewed body are rechecked in the UPDATE: a racing stronger request
    /// cannot turn an automatic worker into a user-approved executor.
    pub async fn claim_auto_gate_run(&self, choice_id: &str, body_row_id: i64) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE session_tray SET run_started_at = ?, status = 'auto_running' \
             WHERE choice_id = ? AND body_row_id = ? AND status = 'queued' \
               AND kind = 'approval' AND approval_required = 0 AND reviewed_at IS NOT NULL \
               AND command_text IS NOT NULL AND run_started_at IS NULL \
               AND result_row_id IS NULL AND run_refusal IS NULL \
               AND exec_kind IS NULL AND urgent_reason IS NULL \
               AND run_not_before IS NULL AND run_not_after IS NULL \
               AND EXISTS (SELECT 1 FROM sessions s WHERE s.id = session_tray.session_id AND s.closed_at IS NULL) \
               AND EXISTS (SELECT 1 FROM session_participants p, \
                   json_each(CASE WHEN json_valid(p.capabilities) THEN p.capabilities ELSE '[]' END) cap \
                   WHERE p.session_id = session_tray.session_id AND p.slug = session_tray.agent \
                     AND p.enabled <> 0 AND cap.value = 'edit_files') \
               AND NOT EXISTS (SELECT 1 FROM findings f WHERE f.session_id = session_tray.session_id \
                   AND f.status = 'open' AND ( \
                     (f.severity = 'blocking' AND (f.gate_id = session_tray.choice_id \
                       OR (f.gate_id IS NULL AND f.created_at > session_tray.asked_at))) \
                     OR (f.severity = 'advisory' AND f.gate_id = session_tray.choice_id)))",
        ).bind(now_utc()).bind(choice_id).bind(body_row_id).execute(&self.pool).await
            .context("claiming reviewed automatic gate")?;
        Ok(res.rows_affected() == 1)
    }

    /// The runs a previous bot-hq process was cut off in (0100): claimed, with
    /// no result and no refusal. Reported at boot, never re-run.
    pub async fn interrupted_gate_runs(&self) -> Result<Vec<SessionTrayEntry>> {
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE run_started_at IS NOT NULL AND result_row_id IS NULL AND run_refusal IS NULL \
             ORDER BY id ASC"
        ))
        .fetch_all(&self.pool)
        .await
        .context("reading the interrupted gate runs")?;
        Ok(rows)
    }

    /// The approved, windowed gates whose run is DUE at `now` and has not
    /// started: approved before their window opened (no result row, no
    /// refusal, no claim), with `run_not_before` at or before `now`. The
    /// scheduler runs them; a cancelled one carries a `run_refusal`, a running
    /// or finished one a `run_started_at`, and neither is here.
    pub async fn due_scheduled_gates(&self, now: &str) -> Result<Vec<SessionTrayEntry>> {
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE status = 'answered' AND picked_option = 'Approve' AND command_text IS NOT NULL \
               AND run_not_before IS NOT NULL AND run_not_before <= ? \
               AND result_row_id IS NULL AND run_refusal IS NULL AND run_started_at IS NULL \
             ORDER BY run_not_before ASC, id ASC"
        ))
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .context("reading the due scheduled gates")?;
        Ok(rows)
    }

    /// Record why a gate is URGENT (0097, feedback #52) — on a row still
    /// waiting (pending or queued); an answered row keeps its history as it
    /// was. Returns whether a row was marked.
    pub async fn set_tray_urgent_reason(&self, choice_id: &str, reason: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE session_tray SET urgent_reason = ? \
             WHERE choice_id = ? AND status IN ('pending', 'queued')",
        )
        .bind(reason)
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("marking gate {choice_id} urgent"))?;
        Ok(res.rows_affected() == 1)
    }

    /// Record the SHA-256 of an edit's LIVE body as its reviewer's diff saw
    /// it (0088) — re-read and compared when the user approves the edit.
    pub async fn set_tray_live_body_sha(&self, choice_id: &str, sha: &str) -> Result<()> {
        sqlx::query("UPDATE session_tray SET live_body_sha256 = ? WHERE choice_id = ?")
            .bind(sha)
            .bind(choice_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording the live-body hash of gate {choice_id}"))?;
        Ok(())
    }

    /// Record that an APPROVED gate was refused at run time (0085) — so
    /// `gate_status` says "approved but NOT RUN" instead of "executed".
    pub async fn set_tray_run_refusal(&self, choice_id: &str, refusal: &str) -> Result<()> {
        sqlx::query("UPDATE session_tray SET run_refusal = ?, \
            status = CASE WHEN status = 'auto_running' THEN 'auto_resolved' ELSE status END \
            WHERE choice_id = ?")
            .bind(refusal)
            .bind(choice_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording the run refusal of gate {choice_id}"))?;
        Ok(())
    }

    /// Link an approved gate to what its run produced (0087): the delivery row
    /// that carries the output, the exit code and the duration. `result_row_id`
    /// is `None` when the delivery row could not be stored — the code and the
    /// time are still worth keeping.
    pub async fn set_tray_run_result(
        &self,
        choice_id: &str,
        result_row_id: Option<i64>,
        exit_code: Option<i32>,
        ran_ms: u64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_tray SET result_row_id = ?, exit_code = ?, ran_ms = ?, \
             status = CASE WHEN status = 'auto_running' THEN 'auto_resolved' ELSE status END \
             WHERE choice_id = ?",
        )
        .bind(result_row_id)
        .bind(exit_code)
        .bind(i64::try_from(ran_ms).unwrap_or(i64::MAX))
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording the run result of gate {choice_id}"))?;
        Ok(())
    }

    /// Tray rows of ONE session whose id starts with `prefix`, any status —
    /// for resolving the short id participants quote in chat (feedback #68 /
    /// #93). Session-scoped in the query itself: another session's row is not
    /// a candidate, so a prefix cannot be used to probe for one. A prefix that
    /// is not made of id characters (hex and `-`) matches nothing, which also
    /// keeps `%` and `_` out of the `LIKE`.
    pub async fn tray_entries_by_id_prefix(
        &self,
        session_id: &str,
        prefix: &str,
    ) -> Result<Vec<SessionTrayEntry>> {
        if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE session_id = ? AND choice_id LIKE ? ORDER BY id ASC"
        ))
        .bind(session_id)
        .bind(format!("{}%", prefix.to_ascii_lowercase()))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Was the NEWEST gate for this exact command withdrawn by a reviewer's
    /// blocking finding? A re-issue of such a command must queue for a FRESH
    /// review: its body row was delivered when it first queued, so the
    /// content-keyed coverage check would otherwise park the vetoed publish
    /// straight onto the user's card — the veto undone by re-issuing unchanged.
    pub async fn latest_gate_withdrawn_by_finding(
        &self,
        session_id: &str,
        command: &str,
    ) -> Result<bool> {
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT status, picked_option FROM session_tray \
             WHERE session_id = ? AND command_text = ? \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(command)
        .fetch_optional(&self.pool)
        .await?;
        Ok(matches!(
            row,
            Some((status, Some(reason)))
                if status == "withdrawn" && reason == FINDING_WITHDRAWAL_REASON
        ))
    }

    /// The newest OPEN blocking finding that TARGETED a gate with this exact
    /// command text (0083 `findings.gate_id`) — `(finding_uid, gate_id, the
    /// vetoed gate's body_sha256)`. A
    /// targeted veto is about that content by definition, so it keeps holding
    /// a re-issue of the same command until the finding is fixed or rebutted
    /// (EYES advisory 891e0eb1: a veto that holds once is a veto a re-issue
    /// undoes).
    pub async fn open_targeted_veto_for_command(
        &self,
        session_id: &str,
        command: &str,
    ) -> Result<Option<(String, String, Option<String>)>> {
        let row: Option<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT f.finding_uid, f.gate_id, t.body_sha256 FROM findings f \
             JOIN session_tray t ON t.choice_id = f.gate_id \
             WHERE f.session_id = ? AND f.status = 'open' AND f.severity = 'blocking' \
               AND t.session_id = f.session_id AND t.command_text = ? \
             ORDER BY f.id DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(command)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// The choice_id of a still-PENDING gated command with this exact command
    /// text in this session, if one exists. Backs action_gate's duplicate
    /// suppression: re-parking an identical command while the first prompt is
    /// still unanswered stacks confusable Approve/Reject cards (the 2026-07-23
    /// reset story: two stacked prompts, batch-rejected as a timing accident).
    /// Pending-only on purpose — a re-fire AFTER a reject is an intentional
    /// retry and must get a fresh prompt.
    pub async fn pending_gate_for_command(
        &self,
        session_id: &str,
        command: &str,
    ) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT choice_id FROM session_tray \
             WHERE session_id = ? AND status = 'pending' AND command_text = ? \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(command)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id,)| id))
    }

    /// The most recent REJECTED answer to this exact command in this session:
    /// `(answered_at, picked_option)`, or `None`. Backs the re-park card's
    /// "you rejected this before" line (E6, week 35): a rejected outward
    /// command re-parks fresh by design (coverage is content-keyed), and the
    /// card should say so rather than read as a first ask.
    pub async fn last_rejection_for_command(
        &self,
        session_id: &str,
        command: &str,
    ) -> Result<Option<(String, String)>> {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT answered_at, picked_option FROM session_tray \
             WHERE session_id = ? AND status = 'answered' AND command_text = ? \
               AND picked_option IS NOT NULL AND picked_option != 'Approve' \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(command)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(at, picked)| Some((at?, picked?))))
    }

    /// Every ANSWERED gated command in this session (rows carrying
    /// `command_text`), oldest-first. Backs the OOB replay's "approved since
    /// you asked" block: a question parked before one of these was approved may
    /// have been overtaken by it (issues.md #18 — a staging-push choice sat
    /// through the push it asked about and replayed as live state).
    ///
    /// Deliberately unfiltered on time and outcome, and small by construction:
    /// it is one session's answered gates. The caller parses `answered_at` and
    /// compares instants rather than binding a SQL `>` — a habit from when this
    /// column carried two shapes (sqlite's `datetime('now')` and RFC3339;
    /// migrations 0012/0015 normalised it, and only 0054's one-time close-out
    /// of legacy halt rows wrote the zone-less shape again, on rows that are
    /// long purged). Round 10 kept the parse: it costs nothing here and stays
    /// correct if any writer ever slips again.
    pub async fn answered_gates_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionTrayEntry>> {
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE session_id = ? \
               AND command_text IS NOT NULL \
               AND status = 'answered' \
               AND answered_at IS NOT NULL \
             ORDER BY id ASC"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Every COMMAND-carrying gate of a session (`action_gate`, the hook's
    /// auto-park, `read_gate`, `prod_read`), whatever its status — queued,
    /// pending, answered, withdrawn, superseded — newest first, at most
    /// `limit` rows (feedback #88: the ledger `gate_history` reads, so an
    /// approved shape can be reused verbatim after a compaction and "did that
    /// run?" has one place to look).
    pub async fn gates_for_session(&self, session_id: &str, limit: usize) -> Result<Vec<SessionTrayEntry>> {
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE session_id = ? AND command_text IS NOT NULL \
             ORDER BY id DESC LIMIT ?"
        ))
        .bind(session_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// The gated commands of this session the user answered at or after
    /// `since` (an RFC3339-Z stamp, compared as text; inclusive — see
    /// `session_documents_changed_since`), oldest first — the reviewer's turn
    /// digest (feedback #76, #61) lists them with their run's outcome.
    pub async fn gates_answered_since(&self, session_id: &str, since: &str) -> Result<Vec<SessionTrayEntry>> {
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE session_id = ? AND command_text IS NOT NULL \
               AND status = 'answered' AND answered_at >= ? \
             ORDER BY answered_at ASC, id ASC"
        ))
        .bind(session_id)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("reading gates answered since {since} for {session_id}"))?;
        Ok(rows)
    }

    /// Whether ANY tray row is pending OR queued for this session — question,
    /// gate, or an outward publish waiting on the reviewer (0080). The
    /// idle-unflagged watchdog reads this each poll: such a row means the
    /// session is legitimately waiting, so bare-Idle detection must stay quiet
    /// — a queued publish that did not count here would re-create the F9
    /// nudge-during-a-legitimate-wait the queue exists to remove (EYES P8).
    pub async fn has_pending_tray(&self, session_id: &str) -> Result<bool> {
        let row: (i64,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM session_tray \
             WHERE session_id = ? AND status IN ('pending', 'queued'))",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0 != 0)
    }

    /// Look up a tray entry by its `choice_id`. Returns None if absent.
    pub async fn get_tray_entry(&self, choice_id: &str) -> Result<Option<SessionTrayEntry>> {
        let row = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray WHERE choice_id = ?"
        ))
        .bind(choice_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// All pending tray rows for OPEN sessions, oldest-first. Excludes closed
    /// sessions (leftover pending on a closed session is noise). Powers the
    /// durable per-session notification count — survives restart, unlike the
    /// in-memory pending map.
    ///
    /// `queued` (0080) is deliberately NOT counted here, and no read path hands
    /// a queued row to the UI at all (EYES 18f2e8cb, decided 2026-09-06): the
    /// count is "things the USER must answer", and a queued publish is the
    /// reviewer's to read, not the user's. Its visibility is the chat — the
    /// executor's tool result, the posted body row, and the settlement row —
    /// which is deliberately it for v1. A dedicated queued indicator, if ever
    /// wanted, needs its own read path that cannot reach a card renderer.
    pub async fn pending_tray_open_sessions(&self) -> Result<Vec<SessionTrayEntry>> {
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE status = 'pending' \
               AND session_id IN \
                   (SELECT id FROM sessions WHERE closed_at IS NULL) \
             ORDER BY id ASC"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Withdraw every pending row for a session — called when the session
    /// closes, since its pending questions / approvals / gated commands are
    /// moot once the agents are gone. Returns the number of rows withdrawn.
    /// Prevents closed sessions from leaving dead `pending` rows behind.
    pub async fn withdraw_pending_tray_for_session(&self, session_id: &str) -> Result<u64> {
        // `queued` too (0080): a queued outward publish must not outlive its
        // session as a live row. The caller posts the "dropped at close" rows
        // from `queued_gates_for_session` BEFORE this sweep, which is why the
        // sweep itself stays silent.
        let res = sqlx::query(
            "UPDATE session_tray SET status = 'withdrawn' \
             WHERE session_id = ? AND status IN ('pending', 'queued')",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("withdrawing pending tray for session {session_id}"))?;
        Ok(res.rows_affected())
    }

    /// Boot-time reconciliation: withdraw every pending tray row that belongs to
    /// a CLOSED or non-existent (orphaned) session. `close_session` already
    /// withdraws at close time and migration 0011 did a one-shot backfill — but
    /// a one-shot migration runs once and `close_session` only fires for closes
    /// going forward, so rows orphaned by a pre-fix binary (a session that
    /// closed while an older build was running) survive as cruft. The
    /// notifier's open-session filter already hides them, but they're dead
    /// weight and would show in a closed session's tray. Running this every boot
    /// self-heals: it clears the existing backlog AND any future close-path
    /// miss. Returns the number of rows withdrawn (0 on a clean DB).
    pub async fn withdraw_pending_tray_for_closed_or_orphaned(&self) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray SET status = 'withdrawn' \
             WHERE status IN ('pending', 'queued') \
               AND (session_id NOT IN (SELECT id FROM sessions) \
                    OR session_id IN (SELECT id FROM sessions WHERE closed_at IS NOT NULL))",
        )
        .execute(&self.pool)
        .await
        .context("withdrawing pending tray for closed/orphaned sessions")?;
        Ok(res.rows_affected())
    }

    /// GC: delete resolved tray rows (answered/withdrawn/superseded) older than
    /// `retention_days`. Keeps `session_tray` bounded — resolved rows are never
    /// read again (the in-chat tray + counters only surface `pending`), and
    /// `pending` AND `queued` rows are always kept: a queued outward publish
    /// (0080) is live work whose `answered_at` is NULL, so it would age on
    /// `asked_at` alone and vanish — the relaunch strand reintroduced through
    /// the GC (EYES d78c0466). Uses `COALESCE(answered_at, asked_at)`
    /// because withdraw/supersede flip status WITHOUT setting `answered_at` (it
    /// stays NULL) — falling back to `asked_at` (always set at insert) ensures no
    /// resolved row escapes the cutoff. The cutoff is built in Rust in the same
    /// RFC3339-Z format `now_utc()` writes, so the string `<` is a valid
    /// chronological compare. Returns the number of rows deleted.
    pub async fn purge_resolved_tray(&self, retention_days: i64) -> Result<u64> {
        let cutoff = crate::storage::cutoff_days_ago(retention_days);
        let res = sqlx::query(
            "DELETE FROM session_tray \
             WHERE status NOT IN ('pending', 'queued', 'auto_running') \
               AND COALESCE(answered_at, run_started_at, asked_at) < ?",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await
        .context("purging resolved tray rows")?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn automatic_gate(s: &Storage, id: &str) -> i64 {
        if s.participant_by_slug("s-auto", "hands").await.unwrap().is_none() {
            s.insert_participant("s-auto", "hands", "Fixture", None, None, r#"["edit_files","run_bash"]"#, "active", 0).await.unwrap();
        }
        let body = s.post_to_channel("s-auto", "system", None, "system_notice", "review this body", None)
            .await.unwrap().message_id();
        s.insert_queued_gate_with_approval("s-auto", id, "hands", "fixture", "publish fixture", false)
            .await.unwrap();
        s.set_tray_body_row(id, body).await.unwrap();
        body
    }

    #[tokio::test]
    async fn automatic_review_never_inherits_authority_from_legacy_rows() {
        // Exercise the actual migration over a row that predates its columns.
        let old = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
            .connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql("CREATE TABLE session_tray (id INTEGER PRIMARY KEY); INSERT INTO session_tray (id) VALUES (1);")
            .execute(&old).await.unwrap();
        sqlx::raw_sql(include_str!("../../migrations/0104_tray_review_authority.sql"))
            .execute(&old).await.unwrap();
        let migrated: (bool, Option<String>) = sqlx::query_as("SELECT approval_required, reviewed_at FROM session_tray WHERE id = 1")
            .fetch_one(&old).await.unwrap();
        assert_eq!(migrated, (true, None));
        assert!(sqlx::query("UPDATE session_tray SET approval_required = 2").execute(&old).await.is_err());
        let s = Storage::memory().await.unwrap();
        s.create_session("s-auto", "fixture", None).await.unwrap();
        s.insert_queued_gate("s-auto", "old", "hands", "fixture", "publish fixture").await.unwrap();
        let row = s.get_tray_entry("old").await.unwrap().unwrap();
        assert!(row.approval_required);
        let mut old_json = serde_json::to_value(&row).unwrap();
        old_json.as_object_mut().unwrap().remove("approval_required");
        old_json.as_object_mut().unwrap().remove("reviewed_at");
        let old_row: SessionTrayEntry = serde_json::from_value(old_json).unwrap();
        assert!(old_row.approval_required, "old serialized rows fail closed too");
        assert!(!s.mark_auto_gate_reviewed("old", 1).await.unwrap());
        assert!(s.reviewed_auto_gates().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn automatic_review_is_bound_to_the_body_and_a_stronger_request_wins() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-auto", "fixture", None).await.unwrap();
        let body = automatic_gate(&s, "auto").await;
        assert!(!s.claim_auto_gate_run("auto", body).await.unwrap(), "unread auto publish cannot run");
        assert!(!s.mark_auto_gate_reviewed("auto", body + 1).await.unwrap());
        assert!(s.mark_auto_gate_reviewed("auto", body).await.unwrap());
        assert!(!s.mark_auto_gate_reviewed("auto", body).await.unwrap(), "one settlement wins");
        assert_eq!(s.reviewed_auto_gates().await.unwrap().len(), 1);
        assert!(s.begin_queued_gate_revision("auto", body).await.unwrap());
        assert!(!s.mark_auto_gate_reviewed("auto", body).await.unwrap(), "stale review loses while the revision is being built");
        assert!(!s.claim_auto_gate_run("auto", body).await.unwrap());
        let revision = s.post_to_channel("s-auto", "system", None, "system_notice", "revised body", None)
            .await.unwrap().message_id();
        s.set_tray_body_row("auto", revision).await.unwrap();
        assert!(!s.claim_auto_gate_run("auto", body).await.unwrap(), "revision invalidates review");
        assert!(!s.mark_auto_gate_reviewed("auto", body).await.unwrap(), "stale review cannot authorize a revision");
        assert!(s.mark_auto_gate_reviewed("auto", revision).await.unwrap());
        assert_eq!(s.require_queued_gate_approval("auto").await.unwrap(), 1);
        assert!(s.reviewed_auto_gates().await.unwrap().is_empty());
        assert!(!s.claim_auto_gate_run("auto", revision).await.unwrap(), "a stale automatic worker loses to a stronger request");
        assert!(!s.mark_auto_gate_reviewed("auto", revision).await.unwrap());
        assert_eq!(s.promote_queued_gate("auto").await.unwrap(), 1);
        let row = s.get_tray_entry("auto").await.unwrap().unwrap();
        assert!(row.approval_required && row.reviewed_at.is_none());
        assert_eq!(row.status, "pending");
    }

    #[tokio::test]
    async fn automatic_claim_is_once_and_never_manufactures_a_user_answer() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-auto", "fixture", None).await.unwrap();
        let body = automatic_gate(&s, "auto").await;
        s.mark_auto_gate_reviewed("auto", body).await.unwrap();
        assert!(!s.claim_gate_run("auto").await.unwrap(), "the human-approval executor cannot claim automatic work");
        let (a, b) = tokio::join!(s.claim_auto_gate_run("auto", body), s.claim_auto_gate_run("auto", body));
        assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
        let row = s.get_tray_entry("auto").await.unwrap().unwrap();
        assert_eq!(row.status, "auto_running");
        assert!(!s.begin_queued_gate_revision("auto", body).await.unwrap());
        assert!(row.picked_option.is_none() && row.answered_at.is_none());
        assert_eq!(s.set_tray_body_row("auto", body).await.unwrap(), 0, "a running body is frozen");
        assert_eq!(s.require_queued_gate_approval("auto").await.unwrap(), 0);
        assert!(s.pending_gate_ids("s-auto").await.unwrap().is_empty());
        assert!(s.reviewed_auto_gates().await.unwrap().is_empty());
        s.set_tray_run_result("auto", Some(body), Some(0), 12).await.unwrap();
        let row = s.get_tray_entry("auto").await.unwrap().unwrap();
        assert_eq!(row.status, "auto_resolved");
        assert!(row.picked_option.is_none() && row.answered_at.is_none());
        assert!(!s.has_pending_tray("s-auto").await.unwrap());
        assert!(!s.claim_auto_gate_run("auto", body).await.unwrap());
    }

    #[tokio::test]
    async fn automatic_work_survives_restart_but_never_repeats_an_interrupted_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let s = Storage::open(&path).await.unwrap();
        s.create_session("s-auto", "fixture", None).await.unwrap();
        let body = automatic_gate(&s, "auto").await;
        s.mark_auto_gate_reviewed("auto", body).await.unwrap();
        s.pool().close().await;
        let s = Storage::open(&path).await.unwrap();
        assert_eq!(s.reviewed_auto_gates().await.unwrap().len(), 1);
        assert!(s.claim_auto_gate_run("auto", body).await.unwrap());
        s.pool().close().await;
        let s = Storage::open(&path).await.unwrap();
        assert!(s.reviewed_auto_gates().await.unwrap().is_empty());
        assert_eq!(s.interrupted_gate_runs().await.unwrap().len(), 1);
        s.set_tray_run_refusal("auto", "interrupted; outcome unknown").await.unwrap();
        let row = s.get_tray_entry("auto").await.unwrap().unwrap();
        assert_eq!(row.status, "auto_resolved");
        assert!(row.picked_option.is_none());
        assert!(s.interrupted_gate_runs().await.unwrap().is_empty());
        assert!(!s.has_pending_tray("s-auto").await.unwrap());
    }

    #[tokio::test]
    async fn automatic_claim_rechecks_closure_and_explicit_approval_metadata() {
        for restriction in ["closed", "prod_read", "urgent", "window"] {
            let s = Storage::memory().await.unwrap();
            s.create_session("s-auto", "fixture", None).await.unwrap();
            let body = automatic_gate(&s, "auto").await;
            s.mark_auto_gate_reviewed("auto", body).await.unwrap();
            match restriction {
                "closed" => { sqlx::query("UPDATE sessions SET closed_at = 'now' WHERE id = 's-auto'").execute(s.pool()).await.unwrap(); }
                "prod_read" => { s.set_tray_exec_kind("auto", "prod_read").await.unwrap(); }
                "urgent" => { s.set_tray_urgent_reason("auto", "fixture").await.unwrap(); }
                "window" => { s.set_tray_run_window("auto", "2099-01-01T00:00:00Z", "2099-01-02T00:00:00Z").await.unwrap(); }
                _ => unreachable!(),
            }
            assert!(s.reviewed_auto_gates().await.unwrap().is_empty(), "{restriction}");
            assert!(!s.claim_auto_gate_run("auto", body).await.unwrap(), "{restriction}");
        }
    }

    async fn pending_count(s: &Storage, session_id: &str) -> usize {
        s.tray_entries_for_session(session_id)
            .await
            .unwrap()
            .into_iter()
            .filter(|q| q.status == "pending")
            .count()
    }

    /// 0080 (EYES P8): the `queued` status is invisible to every renderer and
    /// latch reader, visible to the watchdog, retired by both sweeps, and
    /// unanswerable — each guarantee pinned on the SQL, not on a frontend
    /// filter.
    #[tokio::test]
    async fn a_queued_gate_is_invisible_unanswerable_pending_for_the_watchdog_and_swept() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-1", "S", None).await.unwrap();
        let cmd = "gh issue close 599 --comment landed";
        s.insert_queued_gate("s-1", "q-1", "hands", "Run gated command?", cmd)
            .await
            .unwrap();
        s.set_tray_body_row("q-1", 42).await.unwrap();
        let row = s.get_tray_entry("q-1").await.unwrap().unwrap();
        assert_eq!(row.status, TRAY_STATUS_QUEUED);
        assert_eq!(row.kind, "approval");
        assert_eq!(row.body_row_id, Some(42));
        assert_eq!(row.command_text.as_deref(), Some(cmd));

        // Invisible to the renderable read (item 4) and to the latch seed;
        // findable by the queue's own readers.
        assert!(s.tray_entries_for_session("s-1").await.unwrap().is_empty());
        assert!(s.pending_gate_ids("s-1").await.unwrap().is_empty());
        assert!(s.pending_gate_for_command("s-1", cmd).await.unwrap().is_none());
        assert_eq!(s.queued_gate_for_command("s-1", cmd).await.unwrap().as_deref(), Some("q-1"));
        assert_eq!(s.queued_gates_for_session("s-1").await.unwrap().len(), 1);
        // Visible to the watchdog (item 1).
        assert!(s.has_pending_tray("s-1").await.unwrap());
        // Unanswerable while queued: the pending-only flip touches 0 rows.
        assert_eq!(s.answer_tray_entry("q-1", "Approve").await.unwrap(), 0);
        assert_eq!(s.get_tray_entry("q-1").await.unwrap().unwrap().status, TRAY_STATUS_QUEUED);

        // Promotion: exactly one flip, then it is an ordinary pending gate.
        assert_eq!(s.promote_queued_gate("q-1").await.unwrap(), 1);
        assert_eq!(s.promote_queued_gate("q-1").await.unwrap(), 0, "second flip finds nothing");
        assert_eq!(s.pending_gate_ids("s-1").await.unwrap(), vec!["q-1".to_string()]);
        assert_eq!(pending_count(&s, "s-1").await, 1);
        assert_eq!(s.answer_tray_entry("q-1", "Reject").await.unwrap(), 1);

        // The close sweep (item 2) and the boot GC both retire a queued row.
        s.insert_queued_gate("s-1", "q-2", "hands", "Run gated command?", "gh pr merge 1")
            .await
            .unwrap();
        assert_eq!(s.withdraw_pending_tray_for_session("s-1").await.unwrap(), 1);
        assert_eq!(s.get_tray_entry("q-2").await.unwrap().unwrap().status, "withdrawn");
        assert!(!s.has_pending_tray("s-1").await.unwrap());
        s.create_session("s-closed", "C", None).await.unwrap();
        s.insert_queued_gate("s-closed", "q-3", "hands", "Run gated command?", "gh pr merge 2")
            .await
            .unwrap();
        s.close_session("s-closed", false).await.unwrap();
        assert_eq!(s.withdraw_pending_tray_for_closed_or_orphaned().await.unwrap(), 1);
        assert_eq!(s.get_tray_entry("q-3").await.unwrap().unwrap().status, "withdrawn");
        // A withdrawn queued row carries its reason for gate_status.
        s.insert_queued_gate("s-1", "q-4", "hands", "Run gated command?", "gh pr merge 3")
            .await
            .unwrap();
        assert_eq!(s.withdraw_queued_gate("q-4", "withdrawn: blocking finding").await.unwrap(), 1);
        let w = s.get_tray_entry("q-4").await.unwrap().unwrap();
        assert_eq!(w.status, "withdrawn");
        assert_eq!(w.picked_option.as_deref(), Some("withdrawn: blocking finding"));
    }

    #[tokio::test]
    async fn answered_gates_returns_only_answered_command_rows() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-1", "S", None).await.unwrap();
        s.create_session("s-2", "Other", None).await.unwrap();
        async fn insert(s: &Storage, sid: &str, cid: &str, cmd: Option<&str>) {
            let opts = vec!["Approve".to_string(), "Reject".to_string()];
            s.insert_tray_entry(
                sid,
                cid,
                "hands",
                QuestionKind::Choice,
                "Run gated command?",
                Some(&opts),
                None,
                cmd,
            )
            .await
            .unwrap();
        }
        insert(&s, "s-1", "g-answered", Some("git push origin staging")).await;
        insert(&s, "s-1", "g-pending", Some("git commit -F /tmp/msg")).await;
        insert(&s, "s-1", "c-plain", None).await;
        insert(&s, "s-2", "g-other-session", Some("git push")).await;
        s.answer_tray_entry("g-answered", "Approve").await.unwrap();
        s.answer_tray_entry("c-plain", "yes").await.unwrap();
        s.answer_tray_entry("g-other-session", "Approve")
            .await
            .unwrap();

        let rows = s.answered_gates_for_session("s-1").await.unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.choice_id.as_str()).collect();
        // Answered + command-bearing + this session only. `g-pending` is still
        // awaiting a pick, `c-plain` carries no command, `g-other-session` is a
        // different session.
        assert_eq!(ids, vec!["g-answered"]);
        assert!(rows[0].answered_at.is_some());
        assert_eq!(
            rows[0].command_text.as_deref(),
            Some("git push origin staging")
        );
    }

    /// **A gate is a gate on both shapes** (round 8). `kind = 'approval'` is
    /// written at insert now; the rows parked before that carry `choice` with
    /// the gate menu. `is_gate_row` and the ring's reseed query accept both,
    /// and a plain question with the same kind but another menu is neither.
    /// Kill-tested: drop the `kind = 'approval'` disjunct from
    /// `pending_gate_ids` and the approval-kind row below vanishes from it.
    #[tokio::test]
    async fn a_gate_is_recognised_by_kind_or_by_menu() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-1", "S", None).await.unwrap();
        let gate = vec!["Approve".to_string(), "Reject".to_string()];
        let other = vec!["A".to_string(), "B".to_string()];
        // The round-8 shape: kind says it, menu agrees.
        s.insert_tray_entry("s-1", "c-kind", "hands", QuestionKind::Approval, "run?", Some(&gate), None, Some("git push"))
            .await
            .unwrap();
        // The pre-round-8 shape: kind is choice, only the menu says gate.
        s.insert_tray_entry("s-1", "c-menu", "hands", QuestionKind::Choice, "run?", Some(&gate), None, Some("git push"))
            .await
            .unwrap();
        // A question: same kind as the legacy shape, another menu.
        s.insert_tray_entry("s-1", "c-question", "hands", QuestionKind::Choice, "which?", Some(&other), None, None)
            .await
            .unwrap();
        // The kind ALONE (no menu stored) — not a shape the insert path
        // produces, but the one that proves the reader keys on the kind and
        // not only on the menu.
        s.insert_tray_entry("s-1", "c-kind-only", "hands", QuestionKind::Approval, "run?", None, None, None)
            .await
            .unwrap();
        // And an approval-kind row must keep its options (the insert used to
        // store options for Choice only).
        let kind_row = s.get_tray_entry("c-kind").await.unwrap().unwrap();
        assert_eq!(kind_row.kind, "approval");
        assert_eq!(kind_row.options_json.as_deref(), Some(GATE_OPTIONS_JSON));

        assert!(is_gate_row("approval", None), "the kind alone is enough");
        assert!(is_gate_row("choice", Some(GATE_OPTIONS_JSON)), "the legacy menu on a legacy row is enough");
        assert!(!is_gate_row("choice", Some(r#"["A","B"]"#)), "a question is neither");
        // Round 12 (EYES F15): an agent's REQUEST is a tray item whatever its
        // menu — the canonical pair on a `request` row must not read as a gate.
        assert!(!is_gate_row("request", Some(GATE_OPTIONS_JSON)), "a request with the canonical menu is not a gate");
        assert!(!is_gate_row("request", Some(r#"["Approve — run it","Deny — wait"]"#)));
        assert_eq!(QuestionKind::Request.as_str(), "request");

        let mut ids = s.pending_gate_ids("s-1").await.unwrap();
        ids.sort();
        assert_eq!(
            ids,
            vec!["c-kind".to_string(), "c-kind-only".to_string(), "c-menu".to_string()]
        );
    }

    #[tokio::test]
    async fn boot_sweep_withdraws_closed_keeps_open() {
        let s = Storage::memory().await.unwrap();
        s.create_session("open-1", "Open", None).await.unwrap();
        s.create_session("closed-1", "Closed", None).await.unwrap();
        let opts = vec!["A".to_string(), "B".to_string()];
        s.insert_tray_entry(
            "open-1",
            "c-open",
            "hands",
            QuestionKind::Choice,
            "q?",
            Some(&opts),
            None,
            None,
        )
        .await
        .unwrap();
        s.insert_tray_entry(
            "closed-1",
            "c-closed",
            "hands",
            QuestionKind::Choice,
            "q?",
            Some(&opts),
            None,
            None,
        )
        .await
        .unwrap();
        s.close_session("closed-1", false).await.unwrap();

        let withdrawn = s
            .withdraw_pending_tray_for_closed_or_orphaned()
            .await
            .unwrap();
        assert_eq!(
            withdrawn, 1,
            "only the closed session's pending row is swept"
        );
        assert_eq!(
            pending_count(&s, "open-1").await,
            1,
            "open session untouched"
        );
        assert_eq!(
            pending_count(&s, "closed-1").await,
            0,
            "closed session swept"
        );

        // Idempotent: a second run withdraws nothing more.
        assert_eq!(
            s.withdraw_pending_tray_for_closed_or_orphaned()
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn purge_resolved_tray_drops_resolved_keeps_pending() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        let opts = vec!["A".to_string(), "B".to_string()];
        for cid in ["c-pending", "c-answered", "c-withdrawn"] {
            s.insert_tray_entry(
                "s1",
                cid,
                "hands",
                QuestionKind::Choice,
                "q?",
                Some(&opts),
                None,
                None,
            )
            .await
            .unwrap();
        }
        // answered sets answered_at; withdrawn flips status but leaves answered_at
        // NULL → exercises the COALESCE(answered_at, asked_at) fallback.
        s.answer_tray_entry("c-answered", "A").await.unwrap();
        s.withdraw_tray_entry("c-withdrawn").await.unwrap();

        // A real retention window keeps freshly-resolved rows.
        assert_eq!(
            s.purge_resolved_tray(90).await.unwrap(),
            0,
            "recent resolved rows are within the window"
        );

        // A QUEUED outward publish (0080) has answered_at NULL and would age on
        // asked_at alone — the GC must keep it like a pending row (EYES
        // d78c0466), or a relaunch after the window finds its card gone.
        s.insert_queued_gate("s1", "c-queued", "hands", "Run gated command?", "gh pr merge 1")
            .await
            .unwrap();

        // Future-dated cutoff (negative retention) purges every resolved row,
        // incl. the withdrawn one whose answered_at is NULL — and nothing live.
        let purged = s.purge_resolved_tray(-1).await.unwrap();
        assert_eq!(purged, 2, "answered + withdrawn purged; pending and queued untouched");

        let rows = s.tray_entries_for_session("s1").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].choice_id, "c-pending");
        assert_eq!(rows[0].status, "pending");
        let queued = s.queued_gates_for_session("s1").await.unwrap();
        assert_eq!(queued.len(), 1, "the queued row survived the GC");
        assert_eq!(queued[0].choice_id, "c-queued");
    }

    #[tokio::test]
    async fn tray_retention_keeps_running_auto_work_and_dates_results_by_the_run() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-auto", "fixture", None).await.unwrap();
        for id in ["running", "finished"] {
            let body = automatic_gate(&s, id).await;
            s.mark_auto_gate_reviewed(id, body).await.unwrap();
            assert!(s.claim_auto_gate_run(id, body).await.unwrap());
            sqlx::query("UPDATE session_tray SET asked_at = '2000-01-01T00:00:00Z' WHERE choice_id = ?")
                .bind(id).execute(s.pool()).await.unwrap();
            if id == "finished" { s.set_tray_run_result(id, Some(body), Some(0), 1).await.unwrap(); }
        }
        assert_eq!(s.purge_resolved_tray(90).await.unwrap(), 0, "a recent run of an old request is still recent");
        assert_eq!(s.purge_resolved_tray(-1).await.unwrap(), 1, "only the finished run may be purged");
        assert_eq!(s.get_tray_entry("running").await.unwrap().unwrap().status, "auto_running");
    }

    /// F10: the store redacts a tray row's prompt and each option (before the
    /// options JSON is built) on every way into the tray — including the close
    /// card, which never passes the bridge's park — while a gate's menu and the
    /// command are stored exactly as written.
    #[tokio::test]
    async fn a_tray_rows_prompt_and_options_are_redacted_but_its_command_is_not() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-1", "t", None).await.unwrap();
        let token = format!("{}{}", "ghp_", "1234567890abcdefghijABCDEF");
        let marker = "[redacted: a GitHub access token]";
        let cmd = format!("curl -H 'Authorization: Bearer {token}' https://api.example.com");
        let gate = ["Approve".to_string(), "Reject".to_string()];
        s.insert_tray_entry("s-1", "c-1", "hands", QuestionKind::Approval, &format!("Run with {token}?"), Some(&gate), None, Some(&cmd))
            .await
            .unwrap();
        let menu = [format!("use {token}"), "skip".to_string()];
        s.insert_tray_entry("s-1", "c-2", "hands", QuestionKind::Close, "Close?", Some(&menu), None, None)
            .await
            .unwrap();
        s.insert_queued_gate("s-1", "q-1", "hands", &format!("Publish with {token}?"), &cmd)
            .await
            .unwrap();
        let row = s.get_tray_entry("c-1").await.unwrap().unwrap();
        assert_eq!(row.prompt, format!("Run with {marker}?"));
        assert_eq!(row.options_json.as_deref(), Some(GATE_OPTIONS_JSON), "the gate menu is untouched");
        assert_eq!(row.command_text.as_deref(), Some(cmd.as_str()), "the command runs as written");
        let row = s.get_tray_entry("c-2").await.unwrap().unwrap();
        assert_eq!(row.options_json, Some(format!(r#"["use {marker}","skip"]"#)));
        let row = s.get_tray_entry("q-1").await.unwrap().unwrap();
        assert_eq!(row.prompt, format!("Publish with {marker}?"));
        assert_eq!(row.command_text.as_deref(), Some(cmd.as_str()));
    }
}
