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

mod activity_events;
mod agent_config;
mod cancel_events;
mod cl_atoms;
mod cl_index;
mod context_readings;
mod feedback;
mod forward_events;
mod findings;
mod messages;
mod models;
mod participants;
mod plugin_kv;
mod plugins;
mod projects;
mod retrieval_events;
mod session_docs;
mod sessions;
mod time;
mod tray;

pub use cl_atoms::{Atom, RetrievedAtom};
pub use context_readings::ContextReading;
pub(crate) use cl_atoms::estimate_tokens;
pub use models::WORKTREE_DEFAULT_KEY;
pub use cancel_events::CancelEventRecord;
pub use participants::{
    participant_display_name, participant_slug, render_wire, speaker_of, ChannelPage, Envelope,
    Participant,
    ParticipantDraft, PersistedMessage, Role, RoleDraft, MAX_SESSION_PARTICIPANTS,
    PARTICIPATION_MODES, UNREAD_BATCH_LIMIT, WIRE_BODY_CLAMP_BYTES, WIRE_JOIN,
};
pub use feedback::{FEEDBACK_KINDS, FEEDBACK_STATUSES};
pub use tray::{is_gate_options, GATE_OPTIONS_JSON};
pub use row_types::{
    AgentConfig, AgentFeedback, CancelEvent, ClFolder, ForwardEvent, ClIndexEntry, ClRead,
    Finding, FindingSeverity, FindingStatus, Message, MessageKind, Model, Plugin, Project,
    QuestionKind, RetrievalStats, Session, SessionDocument, SessionTrayEntry, SessionWithPreview,
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
            .foreign_keys(true)
            // **WAL, for the reader this app does not own** (F1). The journal
            // mode was `delete`, which takes an exclusive lock for the length of
            // every write — against an 8-connection pool AND the git hooks,
            // which open this same file read-only from a SEPARATE PROCESS
            // (`hooks.rs::check_findings_gate`). Under `delete` a commit's gate
            // check can be locked out by an ordinary write; under WAL readers
            // never block writers and writers never block readers.
            //
            // Set on the connection, persisted in the FILE HEADER: the first
            // connection flips the database and every later opener — including
            // the hook's read-only handle, which cannot set it itself — inherits
            // it. Reverting means opening the file once with `journal_mode` back
            // to delete; nothing in a migration can express it.
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
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
        agent_only: bool,
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
        // `agent_only` filters user-hidden files; only cl_index carries the flag.
        debug_assert!(
            !agent_only || table == "cl_index",
            "agent_only visibility filter is a cl_index concept"
        );
        // Splice shapes for the visibility filter (const strings, no input):
        // after a `WHERE x = ?` clause vs prefixing a bare OR-group.
        let vis_and = if agent_only { " AND agent_visible = 1" } else { "" };
        let vis_pre = if agent_only { "agent_visible = 1 AND " } else { "" };
        let vis_where = if agent_only { " WHERE agent_visible = 1" } else { "" };
        let like = query.map(|q| format!("%{}%", q.to_lowercase()));
        let columns = if table == "cl_index" {
            cl_index_columns()
        } else {
            cl_columns(path_column)
        };
        let select = format!("SELECT {columns} FROM {table}");
        let rows: Vec<T> = match (project_id, like) {
            (Some(pid), Some(q)) => sqlx::query_as::<_, T>(&format!(
                "{select} WHERE project_id = ?{vis_and} AND ( \
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
                "{select} WHERE project_id = ?{vis_and} ORDER BY updated_at DESC"
            ))
            .bind(pid)
            .fetch_all(&self.pool)
            .await?,
            (None, Some(q)) => sqlx::query_as::<_, T>(&format!(
                "{select} WHERE {vis_pre}(LOWER({path_column}) LIKE ? \
                    OR LOWER(description) LIKE ? \
                    OR LOWER(IFNULL(tags, '')) LIKE ?) \
                 ORDER BY updated_at DESC"
            ))
            .bind(&q)
            .bind(&q)
            .bind(&q)
            .fetch_all(&self.pool)
            .await?,
            (None, None) => sqlx::query_as::<_, T>(&format!(
                "{select}{vis_where} ORDER BY updated_at DESC"
            ))
            .fetch_all(&self.pool)
            .await?,
        };
        Ok(rows)
    }
}

/// Column projection for `cl_folders` reads (and the shared base of
/// `cl_index_columns`). The path column differs per table (`file_path` vs
/// `folder_path`), so `get_folder` and `cl_search_table` build from this and
/// can't drift. `path_column` is a caller-controlled const, never user input.
fn cl_columns(path_column: &str) -> String {
    format!("id, project_id, {path_column}, description, tags, created_at, updated_at")
}

/// Column projection for `cl_index` reads — the shared list plus
/// `agent_visible`, which only cl_index carries (selecting it from cl_folders
/// would be a SQL error, hence the per-table split in `cl_search_table`).
fn cl_index_columns() -> String {
    format!("{}, agent_visible", cl_columns("file_path"))
}
