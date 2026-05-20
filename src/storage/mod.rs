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
    QuestionKind, QuestionStatus, Session, SessionQuestion,
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

    /// List index entries for a project (or all if project_id is None).
    /// Optional `query`: when present, returned rows must contain it (case-
    /// insensitive) in any of file_path / description / tags.
    pub async fn cl_index_search(
        &self,
        project_id: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ClIndexEntry>> {
        let like = query.map(|q| format!("%{}%", q.to_lowercase()));
        let rows: Vec<ClIndexEntry> = match (project_id, like) {
            (Some(pid), Some(q)) => sqlx::query_as::<_, ClIndexEntry>(
                "SELECT id, project_id, file_path, description, tags, created_at, updated_at \
                 FROM cl_index \
                 WHERE project_id = ? AND ( \
                    LOWER(file_path) LIKE ? \
                    OR LOWER(description) LIKE ? \
                    OR LOWER(IFNULL(tags, '')) LIKE ?) \
                 ORDER BY updated_at DESC",
            )
            .bind(pid)
            .bind(&q)
            .bind(&q)
            .bind(&q)
            .fetch_all(&self.pool)
            .await?,
            (Some(pid), None) => sqlx::query_as::<_, ClIndexEntry>(
                "SELECT id, project_id, file_path, description, tags, created_at, updated_at \
                 FROM cl_index WHERE project_id = ? ORDER BY updated_at DESC",
            )
            .bind(pid)
            .fetch_all(&self.pool)
            .await?,
            (None, Some(q)) => sqlx::query_as::<_, ClIndexEntry>(
                "SELECT id, project_id, file_path, description, tags, created_at, updated_at \
                 FROM cl_index \
                 WHERE LOWER(file_path) LIKE ? \
                    OR LOWER(description) LIKE ? \
                    OR LOWER(IFNULL(tags, '')) LIKE ? \
                 ORDER BY updated_at DESC",
            )
            .bind(&q)
            .bind(&q)
            .bind(&q)
            .fetch_all(&self.pool)
            .await?,
            (None, None) => sqlx::query_as::<_, ClIndexEntry>(
                "SELECT id, project_id, file_path, description, tags, created_at, updated_at \
                 FROM cl_index ORDER BY updated_at DESC",
            )
            .fetch_all(&self.pool)
            .await?,
        };
        Ok(rows)
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
        let like = query.map(|q| format!("%{}%", q.to_lowercase()));
        let rows: Vec<ClFolder> = match (project, like) {
            (Some(pid), Some(q)) => sqlx::query_as::<_, ClFolder>(
                "SELECT id, project_id, folder_path, description, tags, created_at, updated_at \
                 FROM cl_folders \
                 WHERE project_id = ? AND ( \
                    LOWER(folder_path) LIKE ? \
                    OR LOWER(description) LIKE ? \
                    OR LOWER(IFNULL(tags, '')) LIKE ?) \
                 ORDER BY updated_at DESC",
            )
            .bind(pid)
            .bind(&q)
            .bind(&q)
            .bind(&q)
            .fetch_all(&self.pool)
            .await?,
            (Some(pid), None) => sqlx::query_as::<_, ClFolder>(
                "SELECT id, project_id, folder_path, description, tags, created_at, updated_at \
                 FROM cl_folders WHERE project_id = ? ORDER BY updated_at DESC",
            )
            .bind(pid)
            .fetch_all(&self.pool)
            .await?,
            (None, Some(q)) => sqlx::query_as::<_, ClFolder>(
                "SELECT id, project_id, folder_path, description, tags, created_at, updated_at \
                 FROM cl_folders \
                 WHERE LOWER(folder_path) LIKE ? \
                    OR LOWER(description) LIKE ? \
                    OR LOWER(IFNULL(tags, '')) LIKE ? \
                 ORDER BY updated_at DESC",
            )
            .bind(&q)
            .bind(&q)
            .bind(&q)
            .fetch_all(&self.pool)
            .await?,
            (None, None) => sqlx::query_as::<_, ClFolder>(
                "SELECT id, project_id, folder_path, description, tags, created_at, updated_at \
                 FROM cl_folders ORDER BY updated_at DESC",
            )
            .fetch_all(&self.pool)
            .await?,
        };
        Ok(rows)
    }
}

