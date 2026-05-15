//! sqlite layer: messages, sessions, agent_configs.
//!
//! `Storage` owns a `SqlitePool`. All queries are async via sqlx.

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

pub mod model;

pub use model::{AgentConfig, Author, Message, MessageKind, Session};

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
}

