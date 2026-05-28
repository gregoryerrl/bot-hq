//! `session_questions` table: durable mirror of the in-chat tray. Every
//! `ask_user_choice` / `mark_awaiting_user` / phase-request writes a row here
//! so the tray + dashboard counter survive restart, and answers/withdrawals/
//! supersedes flip the row's status.

use super::*;

impl Storage {
    /// Insert a fresh question row in `pending` status. Returns the row id.
    /// `options` is required when kind=Choice (encoded to JSON); ignored
    /// otherwise. `supersedes_id` links to the question this one replaces
    /// (when an agent rephrases via `update_question`).
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_question(
        &self,
        session_id: &str,
        choice_id: &str,
        agent: &str,
        kind: QuestionKind,
        prompt: &str,
        options: Option<&[String]>,
        supersedes_id: Option<i64>,
    ) -> Result<i64> {
        let options_json = options
            .filter(|_| matches!(kind, QuestionKind::Choice))
            .map(|opts| serde_json::to_string(opts).unwrap_or_else(|_| "[]".into()));
        let res = sqlx::query(
            "INSERT INTO session_questions \
                (session_id, choice_id, agent, kind, prompt, options_json, supersedes_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(choice_id)
        .bind(agent)
        .bind(kind.as_str())
        .bind(prompt)
        .bind(options_json)
        .bind(supersedes_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("inserting question {choice_id} for session {session_id}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Mark a question as answered + record the picked option (for choices)
    /// or the typed reply (for open_ask). Idempotent on already-answered:
    /// returns Ok with 0 rows affected so callers don't have to guard.
    pub async fn answer_question(
        &self,
        choice_id: &str,
        picked: &str,
    ) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_questions \
             SET status = 'answered', picked_option = ?, answered_at = datetime('now') \
             WHERE choice_id = ? AND status = 'pending'",
        )
        .bind(picked)
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("answering question {choice_id}"))?;
        Ok(res.rows_affected())
    }

    /// Mark every pending `kind='halt'` row for this session as answered.
    /// Called when the user broadcasts a message to the session — the message
    /// IS the answer to a `mark_awaiting_user` halt, so the tray should clear.
    /// `choice` and other kinds are NOT touched (they wait on a real pick).
    pub async fn clear_pending_halts(&self, session_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_questions \
             SET status = 'answered', \
                 answered_at = datetime('now'), \
                 picked_option = '(user replied)' \
             WHERE session_id = ? AND status = 'pending' AND kind = 'halt'",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("clearing halts for session {session_id}"))?;
        Ok(res.rows_affected())
    }

    /// Mark a question as withdrawn (agent abandons it; never to be answered).
    pub async fn withdraw_question(&self, choice_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_questions \
             SET status = 'withdrawn' \
             WHERE choice_id = ? AND status = 'pending'",
        )
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("withdrawing question {choice_id}"))?;
        Ok(res.rows_affected())
    }

    /// Mark a question as superseded by another (agent rephrased).
    pub async fn supersede_question(&self, choice_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE session_questions \
             SET status = 'superseded' \
             WHERE choice_id = ? AND status = 'pending'",
        )
        .bind(choice_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("superseding question {choice_id}"))?;
        Ok(res.rows_affected())
    }

    /// Read all questions for a session, ordered oldest-first. Use for the
    /// in-chat tray (filter to status=pending in the UI) and the dashboard
    /// counter (count where status=pending).
    pub async fn questions_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionQuestion>> {
        let rows = sqlx::query_as::<_, SessionQuestion>(
            "SELECT id, session_id, choice_id, agent, kind, prompt, options_json, \
                    status, picked_option, asked_at, answered_at, supersedes_id \
             FROM session_questions WHERE session_id = ? ORDER BY id ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Count pending questions per session. Convenience for the dashboard
    /// card badge — `[Need User Input] · N`.
    pub async fn pending_question_count(&self, session_id: &str) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM session_questions WHERE session_id = ? AND status = 'pending'",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Look up a question by its `choice_id`. Returns None if absent.
    pub async fn get_question(&self, choice_id: &str) -> Result<Option<SessionQuestion>> {
        let row = sqlx::query_as::<_, SessionQuestion>(
            "SELECT id, session_id, choice_id, agent, kind, prompt, options_json, \
                    status, picked_option, asked_at, answered_at, supersedes_id \
             FROM session_questions WHERE choice_id = ?",
        )
        .bind(choice_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}
