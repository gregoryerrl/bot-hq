//! `projects` table: CL-index foreign-key parent + per-project working-repo /
//! cl_path resolution.

use super::*;
use std::path::PathBuf;

impl Storage {
    /// Upsert a project. Used by the project-registration flow and by the
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
            "INSERT INTO projects (name, display_name, working_repo_path, description, cl_path, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
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
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .with_context(|| format!("upserting project {name}"))?;
        Ok(())
    }

    /// Set or clear `cl_path` on an existing project row. Pass `None` to
    /// revert to the default convention (`<data_dir>/projects/<name>/`).
    pub async fn set_project_cl_path(&self, name: &str, cl_path: Option<&str>) -> Result<()> {
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
        sqlx::query("UPDATE projects SET cl_path = NULL, working_repo_path = NULL WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await
            .with_context(|| format!("unregistering project {name}"))?;
        Ok(())
    }

    /// Resolve a project's CL root. `_globals` maps to the CL dir
    /// (`<data_dir>/library/`). Otherwise: uses `cl_path` from the projects row
    /// if set, else falls back to the default convention
    /// `<data_dir>/library/projects/<name>/` (via [`crate::paths::Paths::project_dir`]).
    /// Missing rows also fall back to the convention so callers can use the
    /// helper uniformly without pre-checking existence.
    pub async fn cl_path_for_project(&self, data_dir: &Path, project: &str) -> Result<PathBuf> {
        let paths = crate::paths::Paths::for_data_dir(data_dir.to_path_buf());
        if project == Project::GLOBALS {
            return Ok(paths.cl_dir);
        }
        let row = self.get_project(project).await?;
        match row.and_then(|r| r.cl_path) {
            Some(p) if !p.is_empty() => Ok(PathBuf::from(p)),
            _ => Ok(paths.project_dir(project)),
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

    /// Resolve the registered project whose `working_repo_path` points at
    /// `path`. Paths are canonicalized before comparing (falling back to the
    /// raw path when canonicalization fails, e.g. the dir is gone) so
    /// symlinks and `./`-style variants can't defeat the match. Returns
    /// `None` unless EXACTLY ONE project matches — two projects sharing a
    /// repo is ambiguous, logged, and treated as no-match so the caller's
    /// fallback applies.
    pub async fn project_by_repo_path(&self, path: &Path) -> Result<Option<String>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, working_repo_path FROM projects \
             WHERE working_repo_path IS NOT NULL AND working_repo_path != ''",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing projects with a working repo")?;
        let canon =
            |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let target = canon(path);
        let mut matches = rows.into_iter().filter_map(|(name, wrp)| {
            if name == Project::GLOBALS {
                return None;
            }
            (canon(Path::new(&wrp)) == target).then_some(name)
        });
        match (matches.next(), matches.next()) {
            (Some(only), None) => Ok(Some(only)),
            (Some(a), Some(b)) => {
                tracing::warn!(
                    path = %path.display(),
                    first = %a,
                    second = %b,
                    "multiple projects share this working repo — not inferring one"
                );
                Ok(None)
            }
            _ => Ok(None),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn seeded() -> Storage {
        let s = Storage::memory().await.unwrap();
        s.upsert_project("alpha", "alpha", Some("/repos/alpha-web"), None, None)
            .await
            .unwrap();
        s.upsert_project("beta", "beta", Some("/repos/beta"), None, None)
            .await
            .unwrap();
        s
    }

    #[tokio::test]
    async fn project_by_repo_path_matches_exact() {
        let s = seeded().await;
        let hit = s
            .project_by_repo_path(Path::new("/repos/alpha-web"))
            .await
            .unwrap();
        assert_eq!(hit.as_deref(), Some("alpha"));
    }

    #[tokio::test]
    async fn project_by_repo_path_none_when_unregistered() {
        let s = seeded().await;
        let hit = s
            .project_by_repo_path(Path::new("/repos/unknown"))
            .await
            .unwrap();
        assert_eq!(hit, None);
    }

    #[tokio::test]
    async fn project_by_repo_path_trailing_slash_matches() {
        let s = seeded().await;
        // The dirs don't exist, so canonicalize falls back to the raw path;
        // Path equality is component-based, which normalizes the trailing `/`.
        let hit = s
            .project_by_repo_path(Path::new("/repos/beta/"))
            .await
            .unwrap();
        assert_eq!(hit.as_deref(), Some("beta"));
    }

    #[tokio::test]
    async fn project_by_repo_path_ambiguous_is_none() {
        let s = seeded().await;
        s.upsert_project("alpha-fork", "alpha-fork", Some("/repos/alpha-web"), None, None)
            .await
            .unwrap();
        let hit = s
            .project_by_repo_path(Path::new("/repos/alpha-web"))
            .await
            .unwrap();
        assert_eq!(hit, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn project_by_repo_path_resolves_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real-repo");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link-repo");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let s = Storage::memory().await.unwrap();
        s.upsert_project("gamma", "gamma", Some(real.to_str().unwrap()), None, None)
            .await
            .unwrap();
        let hit = s.project_by_repo_path(&link).await.unwrap();
        assert_eq!(hit.as_deref(), Some("gamma"));
    }
}
