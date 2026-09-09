//! `session_tray` table: durable mirror of the in-chat tray. Every
//! `ask_user_choice` / `mark_awaiting_user` / phase-request writes a row here
//! so the tray + dashboard counter survive restart, and answers/withdrawals/
//! supersedes flip the row's status.

use super::*;

/// Full column projection for a `SessionTrayEntry` row — shared by
/// `tray_entries_for_session` and `get_tray_entry` so the two can't drift.
const TRAY_COLUMNS: &str = "id, session_id, choice_id, agent, kind, prompt, \
     options_json, status, picked_option, asked_at, answered_at, supersedes_id, command_text, \
     body_row_id";

/// The statuses a tray row passes through. `queued` (0080) is the one that is
/// NOT a user-facing item: an outward publish waiting for the reviewer to read
/// its body. It renders nowhere, latches nothing, and cannot be answered
/// (`answer_tray_entry` flips `pending` only); settlement promotes it.
pub const TRAY_STATUS_QUEUED: &str = "queued";

/// The statuses [`Storage::tray_entries_for_session`] returns — the rows the
/// UI may render. An explicit allow-list, in SQL, so "a queued row is
/// invisible" is a guarantee of the read and not of eight frontend filters
/// comparing `status === "pending"` (EYES P8): a future `!== "answered"` there
/// would otherwise surface a queued row as an approvable card, and approving
/// one would publish before the reviewer had been dealt.
const TRAY_VISIBLE_STATUSES: &str = "'pending', 'answered', 'superseded', 'withdrawn'";

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
/// re-derive it from the options string (round 11 dropped the menu-exact half
/// of that predicate: an agent's `request_approval` with its own labels is a
/// gate too, and stamping it `choice` left the latch it opened with no lift —
/// see `bridge/tray.rs`'s insert). The options check
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
        let options_json = options
            .filter(|_| matches!(kind, QuestionKind::Choice | QuestionKind::Approval | QuestionKind::Close))
            .map(|opts| serde_json::to_string(opts).unwrap_or_else(|_| "[]".into()));
        let res = sqlx::query(
            "INSERT INTO session_tray \
                (session_id, choice_id, agent, kind, prompt, options_json, supersedes_id, command_text, asked_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(choice_id)
        .bind(agent)
        .bind(kind.as_str())
        .bind(prompt)
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
        let res = sqlx::query(
            "INSERT INTO session_tray \
                (session_id, choice_id, agent, kind, prompt, options_json, command_text, asked_at, status) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(choice_id)
        .bind(agent)
        .bind(QuestionKind::Approval.as_str())
        .bind(prompt)
        .bind(GATE_OPTIONS_JSON)
        .bind(command)
        .bind(now_utc())
        .bind(TRAY_STATUS_QUEUED)
        .execute(&self.pool)
        .await
        .with_context(|| format!("queueing outward gate {choice_id} for session {session_id}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Record which posted body row a queued gate waits on.
    pub async fn set_tray_body_row(&self, choice_id: &str, body_row_id: i64) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_tray SET body_row_id = ? WHERE choice_id = ? AND status = 'queued'",
        )
        .bind(body_row_id)
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording the body row of queued gate {choice_id}"))?;
        Ok(res.rows_affected())
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
            "UPDATE session_tray SET status = 'pending', asked_at = ? \
             WHERE choice_id = ? AND status = 'queued'",
        )
        .bind(now_utc())
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("promoting queued gate {choice_id}"))?;
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
             WHERE status NOT IN ('pending', 'queued') \
               AND COALESCE(answered_at, asked_at) < ?",
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
}
