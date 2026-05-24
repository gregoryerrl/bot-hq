//! sqlite layer: messages, sessions, agent_configs.
//!
//! `Storage` owns a `SqlitePool`. All queries are async via sqlx.

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub mod model;

pub use model::{
    AgentConfig, Author, ClFolder, ClIndexEntry, ClRead, Message, MessageKind, Project,
    QuestionKind, QuestionStatus, Session, SessionDocument, SessionQuestion,
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
        let rows = sqlx::query_as::<_, Session>(
            "SELECT id, title, working_repo_path, created_at, closed_at, archived, \
                    brian_model_at_spawn, rain_model_at_spawn, \
                    brian_claude_session_id, rain_claude_session_id \
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

    // ---- Projects (cl_index foreign key) ---------------------------------

    /// Upsert a project. Used by Emma's registration flow and by the
    /// startup backfill (which auto-creates a row for every projects/<name>
    /// directory it scans).
    ///
    /// `None` for `working_repo_path`, `description`, or `cl_path` PRESERVES
    /// the existing value on conflict (via COALESCE). Pass `Some("")` to
    /// explicitly clear (except for `cl_path` where `Some("")` is also treated
    /// as "use the default convention" by readers — see [`cl_path_for_project`]).
    /// `display_name` is always overwritten because the startup loop passes
    /// the directory name and that's the truth post-rename.
    pub async fn upsert_project(
        &self,
        name: &str,
        display_name: &str,
        working_repo_path: Option<&str>,
        description: Option<&str>,
        cl_path: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO projects (name, display_name, working_repo_path, description, cl_path) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(name) DO UPDATE SET \
                display_name = excluded.display_name, \
                working_repo_path = COALESCE(excluded.working_repo_path, projects.working_repo_path), \
                description = COALESCE(excluded.description, projects.description), \
                cl_path = COALESCE(excluded.cl_path, projects.cl_path)",
        )
        .bind(name)
        .bind(display_name)
        .bind(working_repo_path)
        .bind(description)
        .bind(cl_path)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upserting project {name}"))?;
        Ok(())
    }

    /// Set or clear `cl_path` on an existing project row. Pass `None` to
    /// revert to the default convention (`<data_dir>/projects/<name>/`).
    pub async fn set_project_cl_path(
        &self,
        name: &str,
        cl_path: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE projects SET cl_path = ? WHERE name = ?")
            .bind(cl_path)
            .bind(name)
            .execute(&self.pool)
            .await
            .with_context(|| format!("setting cl_path for {name}"))?;
        Ok(())
    }

    /// Soft-unregister: clear `cl_path` + `working_repo_path` but KEEP the
    /// projects row and all child rows (cl_index, cl_folders, cl_reads). The
    /// folder reverts to "just a folder" in the tree but its descriptions
    /// stay so the user can re-register without losing context.
    pub async fn unregister_project(&self, name: &str) -> Result<()> {
        sqlx::query(
            "UPDATE projects SET cl_path = NULL, working_repo_path = NULL WHERE name = ?",
        )
        .bind(name)
        .execute(&self.pool)
        .await
        .with_context(|| format!("unregistering project {name}"))?;
        Ok(())
    }

    /// Resolve a project's CL root. `_globals` always maps to `data_dir`.
    /// Otherwise: uses `cl_path` from the projects row if set, else falls
    /// back to the default convention `<data_dir>/projects/<name>/`. Missing
    /// rows also fall back to the convention so callers can use the helper
    /// uniformly without pre-checking existence.
    pub async fn cl_path_for_project(
        &self,
        data_dir: &Path,
        project: &str,
    ) -> Result<PathBuf> {
        if project == Project::GLOBALS {
            return Ok(data_dir.to_path_buf());
        }
        let row = self.get_project(project).await?;
        let convention = || data_dir.join("projects").join(project);
        match row.and_then(|r| r.cl_path) {
            Some(p) if !p.is_empty() => Ok(PathBuf::from(p)),
            _ => Ok(convention()),
        }
    }

    /// Set `working_repo_path` for a project ONLY if it's currently NULL or
    /// empty. Used by startup backfill so the convention (`~/Projects/<name>`)
    /// can populate previously-unset projects without clobbering a value the
    /// user (or a future UI editor) deliberately set to something else.
    pub async fn set_project_working_repo_path_if_unset(
        &self,
        name: &str,
        path: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE projects SET working_repo_path = ? \
             WHERE name = ? AND (working_repo_path IS NULL OR working_repo_path = '')",
        )
        .bind(path)
        .bind(name)
        .execute(&self.pool)
        .await
        .with_context(|| format!("setting working_repo_path for {name}"))?;
        Ok(())
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>> {
        let rows = sqlx::query_as::<_, Project>(
            "SELECT name, display_name, working_repo_path, description, created_at, cl_path \
             FROM projects ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_project(&self, name: &str) -> Result<Option<Project>> {
        let row = sqlx::query_as::<_, Project>(
            "SELECT name, display_name, working_repo_path, description, created_at, cl_path \
             FROM projects WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // ---- cl_index --------------------------------------------------------

    /// Upsert a CL index entry. Used by the backfill scan AND by the UI's
    /// "save metadata" flow. Bumps updated_at on conflict.
    pub async fn upsert_cl_index(
        &self,
        project_id: &str,
        file_path: &str,
        description: &str,
        tags: Option<&str>,
    ) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO cl_index (project_id, file_path, description, tags) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(project_id, file_path) DO UPDATE SET \
                description = excluded.description, \
                tags = excluded.tags, \
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(project_id)
        .bind(file_path)
        .bind(description)
        .bind(tags)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upserting cl_index {project_id}/{file_path}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Update only the updated_at timestamp — used by lazy stat sync when
    /// disk mtime is newer than stored. Does not touch description/tags.
    pub async fn touch_cl_index(
        &self,
        project_id: &str,
        file_path: &str,
        updated_at: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE cl_index SET updated_at = ? \
             WHERE project_id = ? AND file_path = ?",
        )
        .bind(updated_at)
        .bind(project_id)
        .bind(file_path)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_cl_index(&self, project_id: &str, file_path: &str) -> Result<u64> {
        let res = sqlx::query(
            "DELETE FROM cl_index WHERE project_id = ? AND file_path = ?",
        )
        .bind(project_id)
        .bind(file_path)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
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
        let like = query.map(|q| format!("%{}%", q.to_lowercase()));
        let select = format!(
            "SELECT id, project_id, {path_column}, description, tags, created_at, updated_at \
             FROM {table}"
        );
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

    /// List index entries for a project (or all if project_id is None).
    /// Optional `query`: when present, returned rows must contain it (case-
    /// insensitive) in any of file_path / description / tags.
    pub async fn cl_index_search(
        &self,
        project_id: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ClIndexEntry>> {
        self.cl_search_table("cl_index", "file_path", project_id, query)
            .await
    }

    /// Lookup by (project, path) — used for sync/touch and audit linking.
    pub async fn get_cl_index(
        &self,
        project_id: &str,
        file_path: &str,
    ) -> Result<Option<ClIndexEntry>> {
        let row = sqlx::query_as::<_, ClIndexEntry>(
            "SELECT id, project_id, file_path, description, tags, created_at, updated_at \
             FROM cl_index WHERE project_id = ? AND file_path = ?",
        )
        .bind(project_id)
        .bind(file_path)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // ---- cl_reads (audit) ------------------------------------------------

    /// Record that an agent read a CL file. Fire-and-forget; failures are
    /// logged but don't bubble up so a flaky audit write can't break the
    /// agent's read flow.
    pub async fn record_cl_read(
        &self,
        cl_index_id: i64,
        session_id: Option<&str>,
        agent: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO cl_reads (cl_index_id, session_id, agent) VALUES (?, ?, ?)",
        )
        .bind(cl_index_id)
        .bind(session_id)
        .bind(agent)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---- cl_folders ------------------------------------------------------

    /// Upsert a folder description. `folder_path = ""` is the project root
    /// description; otherwise it's relative to the project's CL root and
    /// mirrors `cl_index.file_path`'s pattern.
    pub async fn upsert_folder_description(
        &self,
        project: &str,
        folder_path: &str,
        description: &str,
        tags: Option<&str>,
    ) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO cl_folders (project_id, folder_path, description, tags) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(project_id, folder_path) DO UPDATE SET \
                description = excluded.description, \
                tags = excluded.tags, \
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(project)
        .bind(folder_path)
        .bind(description)
        .bind(tags)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upserting cl_folders {project}/{folder_path}"))?;
        Ok(res.last_insert_rowid())
    }

    pub async fn delete_folder_description(
        &self,
        project: &str,
        folder_path: &str,
    ) -> Result<u64> {
        let res = sqlx::query(
            "DELETE FROM cl_folders WHERE project_id = ? AND folder_path = ?",
        )
        .bind(project)
        .bind(folder_path)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    pub async fn get_folder(
        &self,
        project: &str,
        folder_path: &str,
    ) -> Result<Option<ClFolder>> {
        let row = sqlx::query_as::<_, ClFolder>(
            "SELECT id, project_id, folder_path, description, tags, created_at, updated_at \
             FROM cl_folders WHERE project_id = ? AND folder_path = ?",
        )
        .bind(project)
        .bind(folder_path)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Parallel to [`cl_index_search`]. `project=None` searches all projects;
    /// optional `query` is a case-insensitive substring across folder_path /
    /// description / tags.
    pub async fn cl_folder_search(
        &self,
        project: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ClFolder>> {
        self.cl_search_table("cl_folders", "folder_path", project, query)
            .await
    }

    // ---- session_documents ---------------------------------------------

    /// Upsert a per-session document by (session_id, slug). On conflict the
    /// body is overwritten, `phase` is replaced, and `updated_at` is refreshed;
    /// `created_at` is preserved. `phase` is the IPAV phase tag — one of
    /// `investigate` / `plan` / `apply` / `verify` — used by the session view's
    /// document tabs and by phase-filtered searches. Untagged docs (`None`)
    /// are session-scoped scratch invisible to tabs and phase searches.
    /// Returns the row id.
    pub async fn upsert_session_document(
        &self,
        session_id: &str,
        slug: &str,
        body: &str,
        phase: Option<&str>,
    ) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let res = sqlx::query(
            "INSERT INTO session_documents (session_id, slug, body, created_at, updated_at, phase) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(session_id, slug) DO UPDATE SET \
               body = excluded.body, \
               updated_at = excluded.updated_at, \
               phase = excluded.phase",
        )
        .bind(session_id)
        .bind(slug)
        .bind(body)
        .bind(&now)
        .bind(&now)
        .bind(phase)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upsert session_documents session={session_id} slug={slug}"))?;
        // last_insert_rowid is 0 on the UPDATE branch; re-fetch when needed.
        let id = if res.last_insert_rowid() != 0 {
            res.last_insert_rowid()
        } else {
            let row: (i64,) = sqlx::query_as(
                "SELECT id FROM session_documents WHERE session_id = ? AND slug = ?",
            )
            .bind(session_id)
            .bind(slug)
            .fetch_one(&self.pool)
            .await?;
            row.0
        };
        Ok(id)
    }

    /// Search a session's documents. Optional `query` is a case-insensitive
    /// substring filter across slug + body. Optional `phase` filters to a
    /// specific IPAV phase tag. Ordered newest-first.
    pub async fn session_documents_for(
        &self,
        session_id: &str,
        query: Option<&str>,
        phase: Option<&str>,
    ) -> Result<Vec<SessionDocument>> {
        let like = query.map(|q| format!("%{}%", q.to_lowercase()));
        let mut sql = String::from(
            "SELECT id, session_id, slug, body, created_at, updated_at, phase \
             FROM session_documents WHERE session_id = ?",
        );
        if like.is_some() {
            sql.push_str(" AND (LOWER(slug) LIKE ? OR LOWER(body) LIKE ?)");
        }
        if phase.is_some() {
            sql.push_str(" AND phase = ?");
        }
        sql.push_str(" ORDER BY updated_at DESC");

        let mut q = sqlx::query_as::<_, SessionDocument>(&sql).bind(session_id);
        if let Some(l) = like.as_deref() {
            q = q.bind(l).bind(l);
        }
        if let Some(p) = phase {
            q = q.bind(p);
        }
        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// Convenience wrapper: all docs tagged with `phase` for `session_id`,
    /// newest-first. Used by the session view's IPAV document tabs.
    pub async fn session_documents_for_phase(
        &self,
        session_id: &str,
        phase: &str,
    ) -> Result<Vec<SessionDocument>> {
        self.session_documents_for(session_id, None, Some(phase))
            .await
    }

    /// Fetch one document by (session_id, slug). None when not found.
    pub async fn session_document_by_slug(
        &self,
        session_id: &str,
        slug: &str,
    ) -> Result<Option<SessionDocument>> {
        let row = sqlx::query_as::<_, SessionDocument>(
            "SELECT id, session_id, slug, body, created_at, updated_at, phase \
             FROM session_documents \
             WHERE session_id = ? AND slug = ?",
        )
        .bind(session_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}

#[cfg(test)]
mod session_doc_tests {
    use super::*;

    async fn seeded() -> (Storage, &'static str, &'static str) {
        let s = Storage::memory().await.unwrap();
        s.create_session("sess-a", "a", None).await.unwrap();
        s.create_session("sess-b", "b", None).await.unwrap();
        (s, "sess-a", "sess-b")
    }

    #[tokio::test]
    async fn upsert_then_read_by_slug() {
        let (s, a, _) = seeded().await;
        let id = s
            .upsert_session_document(a, "plan-v1", "first body", None)
            .await
            .unwrap();
        assert!(id > 0);
        let doc = s
            .session_document_by_slug(a, "plan-v1")
            .await
            .unwrap()
            .expect("doc should exist");
        assert_eq!(doc.slug, "plan-v1");
        assert_eq!(doc.body, "first body");
        assert_eq!(doc.session_id, "sess-a");
    }

    #[tokio::test]
    async fn upsert_is_idempotent_overwrites_body() {
        let (s, a, _) = seeded().await;
        let id1 = s
            .upsert_session_document(a, "findings", "v1", None)
            .await
            .unwrap();
        let id2 = s
            .upsert_session_document(a, "findings", "v2", None)
            .await
            .unwrap();
        assert_eq!(id1, id2, "same slug should return same row id");
        let doc = s
            .session_document_by_slug(a, "findings")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.body, "v2");
        // Only one row total for this slug.
        let all = s.session_documents_for(a, None, None).await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn search_filters_by_query_across_slug_and_body() {
        let (s, a, _) = seeded().await;
        s.upsert_session_document(a, "plan-v1", "rewrites broadcast.rs", None)
            .await
            .unwrap();
        s.upsert_session_document(a, "findings-perf", "irrelevant", None)
            .await
            .unwrap();
        let hits_slug = s
            .session_documents_for(a, Some("plan"), None)
            .await
            .unwrap();
        assert_eq!(hits_slug.len(), 1);
        assert_eq!(hits_slug[0].slug, "plan-v1");
        let hits_body = s
            .session_documents_for(a, Some("broadcast"), None)
            .await
            .unwrap();
        assert_eq!(hits_body.len(), 1);
        assert_eq!(hits_body[0].slug, "plan-v1");
        let no_query = s.session_documents_for(a, None, None).await.unwrap();
        assert_eq!(no_query.len(), 2);
    }

    #[tokio::test]
    async fn docs_are_isolated_per_session() {
        let (s, a, b) = seeded().await;
        s.upsert_session_document(a, "plan", "for a", None)
            .await
            .unwrap();
        let in_b = s.session_documents_for(b, None, None).await.unwrap();
        assert!(in_b.is_empty(), "session B sees no docs from A: {in_b:?}");
        let read_in_b = s.session_document_by_slug(b, "plan").await.unwrap();
        assert!(read_in_b.is_none(), "session B can't read A's slug");
    }

    #[tokio::test]
    async fn unknown_slug_returns_none() {
        let (s, a, _) = seeded().await;
        let row = s
            .session_document_by_slug(a, "nope")
            .await
            .unwrap();
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn phase_filter_returns_only_matching_docs() {
        let (s, a, _) = seeded().await;
        s.upsert_session_document(a, "plan-v1", "x", Some("plan"))
            .await
            .unwrap();
        s.upsert_session_document(a, "find-1", "y", Some("investigate"))
            .await
            .unwrap();
        s.upsert_session_document(a, "scratch", "z", None)
            .await
            .unwrap();
        let plans = s.session_documents_for_phase(a, "plan").await.unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].slug, "plan-v1");
        assert_eq!(plans[0].phase.as_deref(), Some("plan"));
        let all = s.session_documents_for(a, None, None).await.unwrap();
        assert_eq!(all.len(), 3, "no filter returns all docs including untagged");
    }

    #[tokio::test]
    async fn upsert_overwrites_phase() {
        let (s, a, _) = seeded().await;
        s.upsert_session_document(a, "doc", "v1", Some("plan"))
            .await
            .unwrap();
        s.upsert_session_document(a, "doc", "v2", Some("apply"))
            .await
            .unwrap();
        let doc = s
            .session_document_by_slug(a, "doc")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.body, "v2");
        assert_eq!(doc.phase.as_deref(), Some("apply"));
    }
}

