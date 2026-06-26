//! sqlite layer. `Storage` owns a `SqlitePool`; all queries are async via
//! sqlx. The query methods are split across per-table submodules, each
//! contributing its own `impl Storage` block:
//!
//! - [`sessions`], [`messages`], [`agent_config`], [`tray`],
//!   [`projects`], [`cl_index`], [`session_docs`], [`plugins`]
//!
//! This module keeps the `Storage` struct, the `open`/`memory` constructors,
//! the pool accessor, and the shared `cl_search_table` generic used by the
//! CL index/folder searches.

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

pub mod row_types;

mod agent_config;
mod cl_index;
mod findings;
mod messages;
mod models;
mod plugins;
mod projects;
mod session_docs;
mod sessions;
mod time;
mod tray;

pub use models::{RAIN_DISABLED_DEFAULT_KEY, WORKTREE_DEFAULT_KEY};
pub use row_types::{
    AgentConfig, Author, ClFolder, ClIndexEntry, ClRead, Finding, FindingSeverity, FindingStatus,
    Message, MessageKind, Model, Plugin, Project, QuestionKind, Session,
    SessionDocument, SessionTrayEntry, SessionWithPreview,
};
pub(crate) use time::now_utc;

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

    /// Internal: parameterized 4-way search over cl_index / cl_folders.
    /// `path_column` is the column name varying between tables
    /// (`file_path` for cl_index, `folder_path` for cl_folders). Both
    /// `table` and `path_column` are caller-controlled const strings —
    /// no user input, no injection surface.
    async fn cl_search_table<T>(
        &self,
        table: &str,
        path_column: &str,
        project_id: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<T>>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
    {
        // `table` and `path_column` are interpolated into the SQL below (sqlx
        // can't bind identifiers), so they MUST be compile-time constants, never
        // user input. Both call sites pass literals; this guard trips a debug
        // build if a future caller forgets. The search term IS bound — see
        // `.bind(&q)` — so it carries no injection risk.
        debug_assert!(
            matches!(table, "cl_index" | "cl_folders"),
            "cl_search_table: non-constant table name {table:?} — identifiers must not be dynamic"
        );
        let like = query.map(|q| format!("%{}%", q.to_lowercase()));
        let select = format!("SELECT {} FROM {table}", cl_columns(path_column));
        let rows: Vec<T> = match (project_id, like) {
            (Some(pid), Some(q)) => sqlx::query_as::<_, T>(&format!(
                "{select} WHERE project_id = ? AND ( \
                    LOWER({path_column}) LIKE ? \
                    OR LOWER(description) LIKE ? \
                    OR LOWER(IFNULL(tags, '')) LIKE ?) \
                 ORDER BY updated_at DESC"
            ))
            .bind(pid)
            .bind(&q)
            .bind(&q)
            .bind(&q)
            .fetch_all(&self.pool)
            .await?,
            (Some(pid), None) => sqlx::query_as::<_, T>(&format!(
                "{select} WHERE project_id = ? ORDER BY updated_at DESC"
            ))
            .bind(pid)
            .fetch_all(&self.pool)
            .await?,
            (None, Some(q)) => sqlx::query_as::<_, T>(&format!(
                "{select} WHERE LOWER({path_column}) LIKE ? \
                    OR LOWER(description) LIKE ? \
                    OR LOWER(IFNULL(tags, '')) LIKE ? \
                 ORDER BY updated_at DESC"
            ))
            .bind(&q)
            .bind(&q)
            .bind(&q)
            .fetch_all(&self.pool)
            .await?,
            (None, None) => sqlx::query_as::<_, T>(&format!(
                "{select} ORDER BY updated_at DESC"
            ))
            .fetch_all(&self.pool)
            .await?,
        };
        Ok(rows)
    }
}

/// Column projection for `cl_index` / `cl_folders` reads. The path column differs
/// per table (`file_path` vs `folder_path`); everything else is shared, so
/// `get_cl_index` / `get_folder` and `cl_search_table` build from this and can't
/// drift. `path_column` is a caller-controlled const, never user input.
fn cl_columns(path_column: &str) -> String {
    format!("id, project_id, {path_column}, description, tags, created_at, updated_at")
}
