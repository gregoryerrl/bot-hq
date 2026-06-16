//! `cl_index`, `cl_reads`, and `cl_folders` tables: the Context Library
//! discovery surface (file + folder descriptions) and the read-audit log.
//! Searches delegate to the parent module's `cl_search_table` generic.

use super::*;

impl Storage {
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
        let now = now_utc();
        // RETURNING id yields the real row id on BOTH the INSERT and the DO
        // UPDATE branch; `last_insert_rowid()` can report the bumped (unused)
        // AUTOINCREMENT value on an upsert that took the UPDATE branch (same
        // footgun fixed in session_docs.rs).
        let row: (i64,) = sqlx::query_as(
            "INSERT INTO cl_index (project_id, file_path, description, tags, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(project_id, file_path) DO UPDATE SET \
                description = excluded.description, \
                tags = excluded.tags, \
                updated_at = excluded.updated_at \
             RETURNING id",
        )
        .bind(project_id)
        .bind(file_path)
        .bind(description)
        .bind(tags)
        .bind(&now)
        .bind(&now)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("upserting cl_index {project_id}/{file_path}"))?;
        Ok(row.0)
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
            "INSERT INTO cl_reads (cl_index_id, session_id, agent, read_at) VALUES (?, ?, ?, ?)",
        )
        .bind(cl_index_id)
        .bind(session_id)
        .bind(agent)
        .bind(now_utc())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Read back the CL-read audit trail for a session, most-recent first — the
    /// reader half of [`Self::record_cl_read`]. Each row carries the
    /// `cl_index_id` of the file read; callers join to `cl_index` for the path.
    /// Powers the deferred "what context did this agent see?" view promised by
    /// the `cl_register_read` tool descriptor.
    pub async fn cl_reads_for_session(&self, session_id: &str) -> Result<Vec<ClRead>> {
        let rows = sqlx::query_as::<_, ClRead>(
            "SELECT id, cl_index_id, session_id, agent, read_at FROM cl_reads \
             WHERE session_id = ? ORDER BY read_at DESC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
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
        let now = now_utc();
        let row: (i64,) = sqlx::query_as(
            "INSERT INTO cl_folders (project_id, folder_path, description, tags, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(project_id, folder_path) DO UPDATE SET \
                description = excluded.description, \
                tags = excluded.tags, \
                updated_at = excluded.updated_at \
             RETURNING id",
        )
        .bind(project)
        .bind(folder_path)
        .bind(description)
        .bind(tags)
        .bind(&now)
        .bind(&now)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("upserting cl_folders {project}/{folder_path}"))?;
        Ok(row.0)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cl_reads_for_session_returns_recorded_reads_scoped_by_session() {
        let s = Storage::memory().await.unwrap();
        s.upsert_project("p", "p", None, None, None).await.unwrap();
        let cl_id = s.upsert_cl_index("p", "notes.md", "d", None).await.unwrap();
        s.record_cl_read(cl_id, Some("s1"), "brian").await.unwrap();
        s.record_cl_read(cl_id, Some("s2"), "rain").await.unwrap();

        let reads = s.cl_reads_for_session("s1").await.unwrap();
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].agent, "brian");
        assert_eq!(reads[0].cl_index_id, cl_id);
        // Scoped by session: s2 has its own row, an unknown session has none.
        assert_eq!(s.cl_reads_for_session("s2").await.unwrap().len(), 1);
        assert!(s.cl_reads_for_session("nope").await.unwrap().is_empty());
    }
}
