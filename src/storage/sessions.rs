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
        sqlx::query(
            "INSERT INTO sessions (id, title, working_repo_path, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(title)
        .bind(working_repo_path)
        .bind(now_utc())
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
                    brian_claude_session_id, rain_claude_session_id, \
                    rain_enabled, brian_model_id, rain_model_id \
             FROM sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn close_session(&self, id: &str, archive: bool) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET closed_at = ?, archived = ? \
             WHERE id = ? AND closed_at IS NULL",
        )
        .bind(now_utc())
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
                    brian_claude_session_id, rain_claude_session_id, \
                    rain_enabled, brian_model_id, rain_model_id \
             FROM sessions \
             WHERE archived = 0 AND closed_at IS NULL AND id != 'emma' \
             ORDER BY created_at DESC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Closed sessions (both just-closed and archived), most-recently-closed
    /// first. Surfaces in the Settings → Archive tab. Like
    /// [`list_active_sessions`] it excludes the `emma` singleton; the
    /// complement predicate (`closed_at IS NOT NULL`) covers everything the
    /// active list omits except emma. `id ASC` tiebreaks the 1-second
    /// `datetime('now')` granularity for stable ordering.
    pub async fn list_closed_sessions(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query_as::<_, Session>(
            "SELECT id, title, working_repo_path, created_at, closed_at, archived, \
                    brian_model_at_spawn, rain_model_at_spawn, \
                    brian_claude_session_id, rain_claude_session_id, \
                    rain_enabled, brian_model_id, rain_model_id \
             FROM sessions \
             WHERE closed_at IS NOT NULL AND id != 'emma' \
             ORDER BY closed_at DESC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Record which model each agent was spawned with for this session. Called
    /// by `spawn_session_handle` right after the configs are fetched so the
    /// chat header can display the frozen model name. `rain_model` is `None` for
    /// a solo-Brian session, stored as SQL NULL (not an empty string).
    pub async fn set_session_spawn_models(
        &self,
        session_id: &str,
        brian_model: &str,
        rain_model: Option<&str>,
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

    /// Record a session's Rain toggle + per-agent model selections, chosen in
    /// the create dialog. Called once right after `create_session`, BEFORE
    /// spawn, so `spawn_session_handle` reads the chosen models off the row.
    pub async fn set_session_spawn_config(
        &self,
        session_id: &str,
        rain_enabled: bool,
        brian_model_id: Option<&str>,
        rain_model_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions \
             SET rain_enabled = ?, brian_model_id = ?, rain_model_id = ? \
             WHERE id = ?",
        )
        .bind(if rain_enabled { 1 } else { 0 })
        .bind(brian_model_id)
        .bind(rain_model_id)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording spawn config on session {session_id}"))?;
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

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[tokio::test]
    async fn active_and_closed_lists_partition_sessions() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-a", "Active one", None).await.unwrap();
        s.create_session("s-b", "Closed one", None).await.unwrap();
        s.close_session("s-b", false).await.unwrap();
        s.create_session("s-c", "Archived one", None).await.unwrap();
        s.close_session("s-c", true).await.unwrap();

        // Active list: only the never-closed session.
        let active: Vec<String> = s
            .list_active_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(active, vec!["s-a"]);

        // Closed list: both the plain-closed and the archived session.
        let closed = s.list_closed_sessions().await.unwrap();
        let closed_ids: Vec<&str> = closed.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(closed.len(), 2);
        assert!(closed_ids.contains(&"s-b"));
        assert!(closed_ids.contains(&"s-c"));
        // Archived flag preserved so the UI can badge it.
        assert_eq!(closed.iter().find(|x| x.id == "s-c").unwrap().archived, 1);
        assert_eq!(closed.iter().find(|x| x.id == "s-b").unwrap().archived, 0);
    }
}
