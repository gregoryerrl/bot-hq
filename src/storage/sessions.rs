//! `sessions` table: lifecycle (create/get/close/list) + per-session spawn
//! metadata (frozen model names, claude-code resume UUIDs).

use super::*;

impl Storage {
    pub async fn create_session(
        &self,
        id: &str,
        title: &str,
        working_repo_path: Option<&str>,
    ) -> Result<Session> {
        sqlx::query("INSERT INTO sessions (id, title, working_repo_path) VALUES (?, ?, ?)")
            .bind(id)
            .bind(title)
            .bind(working_repo_path)
            .execute(&self.pool)
            .await
            .with_context(|| format!("creating session {id}"))?;
        self.get_session(id)
            .await?
            .context("session row vanished immediately after insert")
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let row = sqlx::query_as::<_, Session>(
            "SELECT id, title, working_repo_path, created_at, closed_at, archived, \
                    brian_model_at_spawn, rain_model_at_spawn, \
                    brian_claude_session_id, rain_claude_session_id \
             FROM sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn close_session(&self, id: &str, archive: bool) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET closed_at = datetime('now'), archived = ? \
             WHERE id = ? AND closed_at IS NULL",
        )
        .bind(if archive { 1 } else { 0 })
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("closing session {id}"))?;
        Ok(())
    }

    /// Active sessions: not archived, not closed. Ordered most-recent first.
    /// Emma is included (she's always active). `id ASC` is the tiebreaker —
    /// `datetime('now')` has 1-second granularity, so sessions created in the
    /// same second tied on `created_at` alone and SQLite returned them in
    /// non-deterministic order, causing dashboard tiles to swap places on
    /// every refresh.
    pub async fn list_active_sessions(&self) -> Result<Vec<Session>> {
        // Exclude the special `emma` singleton — she's a chat overlay, not a
        // duo session. She'd otherwise satisfy `archived=0 AND closed_at IS
        // NULL` and surface as a phantom tile on the Dashboard.
        let rows = sqlx::query_as::<_, Session>(
            "SELECT id, title, working_repo_path, created_at, closed_at, archived, \
                    brian_model_at_spawn, rain_model_at_spawn, \
                    brian_claude_session_id, rain_claude_session_id \
             FROM sessions \
             WHERE archived = 0 AND closed_at IS NULL AND id != 'emma' \
             ORDER BY created_at DESC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Record which model each agent was spawned with for this session. Called
    /// by `spawn_session_handle` right after the configs are fetched so the
    /// chat header can display the frozen model name.
    pub async fn set_session_spawn_models(
        &self,
        session_id: &str,
        brian_model: &str,
        rain_model: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET brian_model_at_spawn = ?, rain_model_at_spawn = ? \
             WHERE id = ?",
        )
        .bind(brian_model)
        .bind(rain_model)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording spawn models on session {session_id}"))?;
        Ok(())
    }

    /// Persist the claude-code session UUID for one agent in a bot-hq session.
    /// Called by `core/duo.rs::pump_agent` when the agent's `init` stream-json
    /// event fires. The next time the bot-hq session is reopened, the spawn
    /// path reads this column and passes `--resume <uuid>` to claude.
    /// `agent` must be `"brian"` or `"rain"`; other values return Err.
    pub async fn set_session_claude_id(
        &self,
        session_id: &str,
        agent: &str,
        claude_session_id: &str,
    ) -> Result<()> {
        let column = match agent {
            "brian" => "brian_claude_session_id",
            "rain" => "rain_claude_session_id",
            other => anyhow::bail!("set_session_claude_id: unsupported agent {other:?}"),
        };
        let sql = format!("UPDATE sessions SET {column} = ? WHERE id = ?");
        sqlx::query(&sql)
            .bind(claude_session_id)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| {
                format!("recording {agent} claude session id on session {session_id}")
            })?;
        Ok(())
    }
}
