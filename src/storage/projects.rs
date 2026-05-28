//! `projects` table: CL-index foreign-key parent + per-project working-repo /
//! cl_path resolution.

use super::*;
use std::path::PathBuf;

impl Storage {
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
}
