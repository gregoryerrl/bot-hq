//! sqlite layer: messages, sessions, agent_configs.
//!
//! `Storage` owns a `SqlitePool`. All queries are async via sqlx.

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

pub mod model;

pub use model::{
    AgentConfig, Author, Message, MessageKind, QuestionKind, QuestionStatus, Session,
    SessionQuestion,
};

#[derive(Clone)]
pub struct Storage {
    pool: SqlitePool,
}

impl Storage {
    /// Open the database at `db_path`, run migrations, return a ready Storage.
    /// Creates the file if missing. The parent directory must already exist.
    pub async fn open(db_path: &Path) -> Result<Self> {
        let dsn = format!("sqlite://{}", db_path.display());
        let opts = SqliteConnectOptions::from_str(&dsn)
            .with_context(|| format!("invalid sqlite dsn: {dsn}"))?
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .with_context(|| format!("opening sqlite at {}", db_path.display()))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("running sqlite migrations")?;
        Ok(Self { pool })
    }

    /// In-memory test backend. Available to integration tests in `tests/`.
    pub async fn memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // ---- sessions ------------------------------------------------------

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
                    brian_model_at_spawn, rain_model_at_spawn \
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
        let rows = sqlx::query_as::<_, Session>(
            "SELECT id, title, working_repo_path, created_at, closed_at, archived, \
                    brian_model_at_spawn, rain_model_at_spawn \
             FROM sessions \
             WHERE archived = 0 AND closed_at IS NULL \
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

    // ---- messages ------------------------------------------------------

    pub async fn insert_message(
        &self,
        session_id: &str,
        author: Author,
        kind: MessageKind,
        content: &str,
    ) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO messages (session_id, author, kind, content) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(author.as_str())
        .bind(kind.as_str())
        .bind(content)
        .execute(&self.pool)
        .await
        .with_context(|| format!("inserting message into session {session_id}"))?;
        Ok(res.last_insert_rowid())
    }

    /// All messages for the session, oldest first.
    /// If `since_id` is provided, returns only messages with id > since_id.
    pub async fn messages_for_session(
        &self,
        session_id: &str,
        since_id: Option<i64>,
    ) -> Result<Vec<Message>> {
        let rows = match since_id {
            Some(sid) => {
                sqlx::query_as::<_, Message>(
                    "SELECT id, session_id, author, kind, content, created_at \
                     FROM messages WHERE session_id = ? AND id > ? ORDER BY id ASC",
                )
                .bind(session_id)
                .bind(sid)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, Message>(
                    "SELECT id, session_id, author, kind, content, created_at \
                     FROM messages WHERE session_id = ? ORDER BY id ASC",
                )
                .bind(session_id)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    // ---- agent_configs -------------------------------------------------

    pub async fn get_agent_config(&self, name: &str) -> Result<Option<AgentConfig>> {
        let row = sqlx::query_as::<_, AgentConfig>(
            "SELECT agent_name, provider, model_name, base_url, auth_token, updated_at \
             FROM agent_configs WHERE agent_name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_agent_configs(&self) -> Result<Vec<AgentConfig>> {
        let rows = sqlx::query_as::<_, AgentConfig>(
            "SELECT agent_name, provider, model_name, base_url, auth_token, updated_at \
             FROM agent_configs ORDER BY agent_name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn upsert_agent_config(&self, cfg: &AgentConfig) -> Result<()> {
        sqlx::query(
            "INSERT INTO agent_configs (agent_name, provider, model_name, base_url, auth_token, updated_at) \
             VALUES (?, ?, ?, ?, ?, datetime('now')) \
             ON CONFLICT(agent_name) DO UPDATE SET \
                 provider = excluded.provider, \
                 model_name = excluded.model_name, \
                 base_url = excluded.base_url, \
                 auth_token = excluded.auth_token, \
                 updated_at = excluded.updated_at",
        )
        .bind(&cfg.agent_name)
        .bind(&cfg.provider)
        .bind(&cfg.model_name)
        .bind(&cfg.base_url)
        .bind(&cfg.auth_token)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upserting agent_config {}", cfg.agent_name))?;
        Ok(())
    }

    // ---- session_questions ---------------------------------------------

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

