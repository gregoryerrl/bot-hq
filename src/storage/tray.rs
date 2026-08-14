//! `session_tray` table: durable mirror of the in-chat tray. Every
//! `ask_user_choice` / `mark_awaiting_user` / phase-request writes a row here
//! so the tray + dashboard counter survive restart, and answers/withdrawals/
//! supersedes flip the row's status.

use super::*;

/// Full column projection for a `SessionTrayEntry` row — shared by
/// `tray_entries_for_session` and `get_tray_entry` so the two can't drift.
const TRAY_COLUMNS: &str = "id, session_id, choice_id, agent, kind, prompt, \
     options_json, status, picked_option, asked_at, answered_at, supersedes_id, command_text";

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
            .filter(|_| matches!(kind, QuestionKind::Choice))
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

    /// Mark a tray entry as answered + record the picked option (for choices)
    /// or the typed reply (for open_ask). Idempotent on already-answered:
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
    // storage/sessions.rs). Nothing writes kind='halt' rows any more; the ones
    // in the archive are history.

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

    /// How many approval gates are open for this session — pending rows whose
    /// options are exactly `Approve`/`Reject`, which is what both gate kinds
    /// (action gate, push gate) ask and no ordinary question ever has (the same
    /// discriminator the UI's `isApproval` uses; verified exact across every
    /// row ever recorded — 31 matches, all gates). Seeds the ring's gate latch
    /// on spawn (rc3 D35), so a respawned session cannot deal turns under a
    /// gate that parked before the restart.
    pub async fn count_pending_gates(&self, session_id: &str) -> Result<usize> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM session_tray \
             WHERE session_id = ? AND status = 'pending' \
               AND options_json = '[\"Approve\",\"Reject\"]'",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as usize)
    }

    /// Read all tray entries for a session, ordered oldest-first. Use for the
    /// in-chat tray (filter to status=pending in the UI) and the dashboard
    /// counter (count where status=pending).
    pub async fn tray_entries_for_session(&self, session_id: &str) -> Result<Vec<SessionTrayEntry>> {
        let rows = sqlx::query_as::<_, SessionTrayEntry>(&format!(
            "SELECT {TRAY_COLUMNS} FROM session_tray \
             WHERE session_id = ? ORDER BY id ASC"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
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

    /// Every ANSWERED gated command in this session (rows carrying
    /// `command_text`), oldest-first. Backs the OOB replay's "approved since
    /// you asked" block: a question parked before one of these was approved may
    /// have been overtaken by it (issues.md #18 — a staging-push choice sat
    /// through the push it asked about and replayed as live state).
    ///
    /// Deliberately unfiltered on time and outcome. `answered_at` has two
    /// historical shapes in this table (RFC3339 and sqlite's
    /// `datetime('now')` — see migrations 0012/0015), which sort differently
    /// as strings for the same instant, so a SQL `answered_at > ?` would
    /// mis-order across them. The caller parses both sides and compares
    /// instants.
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

    /// Whether ANY tray row is pending for this session — question, halt, or
    /// gate. The idle-unflagged watchdog reads this each poll: a pending row
    /// means the session is legitimately waiting on the user, so bare-Idle
    /// detection must stay quiet. Hits the `idx_session_tray_pending` partial
    /// index; effectively free.
    pub async fn has_pending_tray(&self, session_id: &str) -> Result<bool> {
        let row: (i64,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM session_tray \
             WHERE session_id = ? AND status = 'pending')",
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
        let res = sqlx::query(
            "UPDATE session_tray SET status = 'withdrawn' \
             WHERE session_id = ? AND status = 'pending'",
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
             WHERE status = 'pending' \
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
    /// `pending` rows are always kept. Uses `COALESCE(answered_at, asked_at)`
    /// because withdraw/supersede flip status WITHOUT setting `answered_at` (it
    /// stays NULL) — falling back to `asked_at` (always set at insert) ensures no
    /// resolved row escapes the cutoff. The cutoff is built in Rust in the same
    /// RFC3339-Z format `now_utc()` writes, so the string `<` is a valid
    /// chronological compare. Returns the number of rows deleted.
    pub async fn purge_resolved_tray(&self, retention_days: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(retention_days))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let res = sqlx::query(
            "DELETE FROM session_tray \
             WHERE status != 'pending' \
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
                "brian",
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

    #[tokio::test]
    async fn boot_sweep_withdraws_closed_keeps_open() {
        let s = Storage::memory().await.unwrap();
        s.create_session("open-1", "Open", None).await.unwrap();
        s.create_session("closed-1", "Closed", None).await.unwrap();
        let opts = vec!["A".to_string(), "B".to_string()];
        s.insert_tray_entry(
            "open-1",
            "c-open",
            "brian",
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
            "brian",
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
                "brian",
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

        // Future-dated cutoff (negative retention) purges every non-pending row,
        // incl. the withdrawn one whose answered_at is NULL.
        let purged = s.purge_resolved_tray(-1).await.unwrap();
        assert_eq!(purged, 2, "answered + withdrawn purged; pending untouched");

        let rows = s.tray_entries_for_session("s1").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].choice_id, "c-pending");
        assert_eq!(rows[0].status, "pending");
    }
}
