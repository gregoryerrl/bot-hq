//! `sessions` table: lifecycle (create/get/close/list) + per-session spawn
//! metadata (frozen model names, claude-code resume UUIDs).

use super::*;

/// The `sessions` columns every `query_as::<_, Session>` SELECT must list, in
/// `Session` field order (also the flattened prefix for `SessionWithPreview`).
/// Centralized so adding a column is one edit, not four — a missing column
/// fails `query_as` at runtime, not compile time.
const SESSION_COLUMNS: &str = "id, title, working_repo_path, created_at, closed_at, \
    archived, brian_model_at_spawn, rain_model_at_spawn, brian_claude_session_id, \
    rain_claude_session_id, rain_enabled, brian_model_id, rain_model_id, brian_effort, \
    rain_effort, brian_ultracode, rain_ultracode, base_repo_path";

impl Storage {
    pub async fn create_session(
        &self,
        id: &str,
        title: &str,
        working_repo_path: Option<&str>,
    ) -> Result<Session> {
        // Blank-but-present paths ('' from a repo-less project row) must store
        // as NULL: every consumer treats Some as "has a repo", and a phantom
        // path hard-errors action_gate / hook install. Migration 0019 repaired
        // pre-guard rows.
        let working_repo_path = working_repo_path.filter(|p| !p.trim().is_empty());
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
        let row = sqlx::query_as::<_, Session>(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Rename a session. The live `SessionHandle.title` snapshot is NOT
    /// touched (it only feeds spawn-time logs); the UI re-reads the row.
    pub async fn rename_session(&self, id: &str, title: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET title = ? WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("renaming session {id}"))?;
        Ok(())
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

    /// Active sessions: not archived, not closed. Ordered by LAST ACTIVITY —
    /// the newest message timestamp, falling back to `created_at` for
    /// message-less (just-created) sessions — so the dashboard surfaces the
    /// session you (or an agent) touched most recently first. `id ASC` is the
    /// tiebreaker so equal timestamps can't make tiles swap places between
    /// refreshes.
    pub async fn list_active_sessions(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query_as::<_, Session>(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions \
             WHERE archived = 0 AND closed_at IS NULL \
             ORDER BY COALESCE(\
                 (SELECT MAX(m.created_at) FROM messages m WHERE m.session_id = sessions.id), \
                 created_at) DESC, \
                 id ASC"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Like `list_active_sessions` but each row also carries a cheap preview of
    /// its latest `kind='text'` message (content capped at 200 chars + author),
    /// for the dashboard Quickview. The two preview subqueries hit the same
    /// `idx_messages_session_id` index the ORDER BY already uses, so there are
    /// no extra per-tile round-trips. Dashboard-only consumer (`list_sessions`).
    pub async fn list_active_sessions_with_preview(&self) -> Result<Vec<SessionWithPreview>> {
        let rows = sqlx::query_as::<_, SessionWithPreview>(&format!(
            "SELECT {SESSION_COLUMNS}, \
                    (SELECT substr(m.content, 1, 200) FROM messages m \
                       WHERE m.session_id = sessions.id AND m.kind = 'text' \
                       ORDER BY m.id DESC LIMIT 1) AS last_message, \
                    (SELECT m.author FROM messages m \
                       WHERE m.session_id = sessions.id AND m.kind = 'text' \
                       ORDER BY m.id DESC LIMIT 1) AS last_author \
             FROM sessions \
             WHERE archived = 0 AND closed_at IS NULL \
             ORDER BY COALESCE(\
                 (SELECT MAX(m.created_at) FROM messages m WHERE m.session_id = sessions.id), \
                 created_at) DESC, \
                 id ASC"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Closed sessions (both just-closed and archived), most-recently-closed
    /// first. Surfaces in the Settings → Archive tab. `id ASC` tiebreaks the
    /// 1-second `datetime('now')` granularity for stable ordering.
    pub async fn list_closed_sessions(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query_as::<_, Session>(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions \
             WHERE closed_at IS NOT NULL \
             ORDER BY closed_at DESC, id ASC"
        ))
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

    /// Record a session's per-agent effort/ultracode overrides, chosen in the
    /// create dialog. Separate from `set_session_spawn_config` to avoid an
    /// 8-positional-param method; `create_session` calls both back-to-back
    /// before spawn. `None` = inherit the Settings defaults (column left NULL).
    pub async fn set_session_effort_config(
        &self,
        session_id: &str,
        brian_effort: Option<&str>,
        rain_effort: Option<&str>,
        brian_ultracode: Option<bool>,
        rain_ultracode: Option<bool>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions \
             SET brian_effort = ?, rain_effort = ?, brian_ultracode = ?, rain_ultracode = ? \
             WHERE id = ?",
        )
        .bind(brian_effort)
        .bind(rain_effort)
        .bind(brian_ultracode)
        .bind(rain_ultracode)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording effort config on session {session_id}"))?;
        Ok(())
    }

    /// Record the user's main repo for a worktree-isolated session. Called at
    /// create time (before spawn) together with the worktree placement —
    /// `working_repo_path` then carries the worktree path and this column the
    /// repo the worktree was carved from. `None` clears (direct mode).
    pub async fn set_session_base_repo(
        &self,
        session_id: &str,
        base_repo_path: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE sessions SET base_repo_path = ? WHERE id = ?")
            .bind(base_repo_path)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording base repo on session {session_id}"))?;
        Ok(())
    }

    /// Convert a worktree-isolated session to direct mode: point
    /// `working_repo_path` back at the base repo and clear `base_repo_path`.
    /// Used when the worktree can't be materialized at spawn — the row must
    /// follow the fallback or row-readers (action_gate) and the live session
    /// would disagree about where the session runs.
    pub async fn convert_session_to_direct(&self, session_id: &str, repo: &str) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET working_repo_path = ?, base_repo_path = NULL WHERE id = ?",
        )
        .bind(repo)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("converting session {session_id} to direct mode"))?;
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

    #[tokio::test]
    async fn active_sessions_order_by_last_activity() {
        use crate::storage::{Author, MessageKind};
        let s = Storage::memory().await.unwrap();
        s.create_session("s-old", "Older", None).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        s.create_session("s-new", "Newer", None).await.unwrap();

        // No messages anywhere → creation order, newest first.
        let ids: Vec<String> = s
            .list_active_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(ids, vec!["s-new", "s-old"]);

        // Activity on the older session bumps it to the top.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        s.insert_message("s-old", Author::User, MessageKind::Text, "hi")
            .await
            .unwrap();
        let ids: Vec<String> = s
            .list_active_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(ids, vec!["s-old", "s-new"]);
    }

    #[tokio::test]
    async fn preview_carries_latest_text_message() {
        use crate::storage::{Author, MessageKind};
        let s = Storage::memory().await.unwrap();
        s.create_session("s-msg", "Has messages", None)
            .await
            .unwrap();
        s.create_session("s-empty", "No messages", None)
            .await
            .unwrap();

        // Newest TEXT message wins; a later tool_use must NOT shadow it.
        s.insert_message("s-msg", Author::User, MessageKind::Text, "first prompt")
            .await
            .unwrap();
        s.insert_message("s-msg", Author::Brian, MessageKind::Text, "brian reply")
            .await
            .unwrap();
        s.insert_message("s-msg", Author::Brian, MessageKind::ToolUse, "{\"tool\":\"x\"}")
            .await
            .unwrap();

        let rows = s.list_active_sessions_with_preview().await.unwrap();
        let msg = rows.iter().find(|r| r.session.id == "s-msg").unwrap();
        assert_eq!(msg.last_message.as_deref(), Some("brian reply"));
        assert_eq!(msg.last_author.as_deref(), Some("brian"));

        // A session with no text messages → None preview, not an error.
        let empty = rows.iter().find(|r| r.session.id == "s-empty").unwrap();
        assert!(empty.last_message.is_none());
        assert!(empty.last_author.is_none());
    }

    #[tokio::test]
    async fn create_session_normalizes_blank_repo_path_to_null() {
        let s = Storage::memory().await.unwrap();
        let created = s.create_session("s-blank", "T", Some("")).await.unwrap();
        assert!(created.working_repo_path.is_none());
        let ws = s.create_session("s-ws", "T", Some("  ")).await.unwrap();
        assert!(ws.working_repo_path.is_none());
        // A real path still round-trips.
        let real = s
            .create_session("s-real", "T", Some("/tmp/repo"))
            .await
            .unwrap();
        assert_eq!(real.working_repo_path.as_deref(), Some("/tmp/repo"));
    }

    #[tokio::test]
    async fn migration_0017_purges_emma_seed() {
        // 0001 seeds an 'emma' session + agent_config; 0017 deletes both. A
        // freshly migrated DB must come up Emma-free.
        let s = Storage::memory().await.unwrap();
        assert!(s.get_session("emma").await.unwrap().is_none());
        assert!(s.get_agent_config("emma").await.unwrap().is_none());
    }
}
