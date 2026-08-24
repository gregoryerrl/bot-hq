//! `cl_index`, `cl_reads`, and `cl_folders` tables: the Context Library
//! discovery surface (file + folder descriptions) and the read-audit log.
//! Searches delegate to the parent module's `cl_search_table` generic.

use super::*;

impl Storage {
    /// Has ANY CL file of `project` (or `_globals`) been index-touched since
    /// `since`? (1.0.0 Batch 9 T5, dissect #10.) The close-out gate's
    /// `cl_written` used to live only in the bridge's per-PROCESS map, so a
    /// respawn — every reopen — forgot a delta that was already on disk and
    /// re-nudged for it. The index row's `updated_at` moves on every
    /// `cl_write_file` and on any `cl_rescan` after an out-of-band edit; a
    /// write another session made since `since` also matches, which for a
    /// nudge SUPPRESSOR is the safe direction. An edit never rescanned stays
    /// invisible — documented, not solved.
    pub async fn cl_written_since(&self, project: Option<&str>, since: &str) -> Result<bool> {
        let (n,): (i64,) = match project {
            Some(p) => {
                sqlx::query_as(
                    "SELECT COUNT(*) FROM cl_index \
                     WHERE (project_id = ? OR project_id = '_globals') AND updated_at >= ?",
                )
                .bind(p)
                .bind(since)
                .fetch_one(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as("SELECT COUNT(*) FROM cl_index WHERE updated_at >= ?")
                    .bind(since)
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(n > 0)
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

    /// Refresh the auto-derived `description` (and `updated_at`) when a file's
    /// body changed on disk — used by the rescan sync when disk mtime is newer
    /// than the stored row. Unlike [`Self::upsert_cl_index`] it leaves user-set
    /// `tags` intact (upsert binds `tags = excluded.tags`, which a `None` would
    /// wipe). Without this the description froze at first-index and drifted from
    /// the file's actual content.
    pub async fn refresh_cl_index_description(
        &self,
        project_id: &str,
        file_path: &str,
        description: &str,
        updated_at: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE cl_index SET description = ?, updated_at = ? \
             WHERE project_id = ? AND file_path = ?",
        )
        .bind(description)
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
        self.cl_search_table("cl_index", "file_path", project_id, query, false)
            .await
    }

    /// Agent-facing variant of [`Self::cl_index_search`]: excludes rows the
    /// user marked `agent_visible = 0` (personal notes / diary). The UI and
    /// rescan/orphan sync MUST keep using the unfiltered variant — hiding a
    /// file from agents must not orphan it from the index.
    pub async fn cl_index_search_agent(
        &self,
        project_id: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ClIndexEntry>> {
        self.cl_search_table("cl_index", "file_path", project_id, query, true)
            .await
    }

    /// Flip a file's agent visibility. Returns false when no such row exists.
    pub async fn set_cl_agent_visibility(
        &self,
        project_id: &str,
        file_path: &str,
        visible: bool,
    ) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE cl_index SET agent_visible = ? WHERE project_id = ? AND file_path = ?",
        )
        .bind(visible as i64)
        .bind(project_id)
        .bind(file_path)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Lookup by (project, path) — used for sync/touch and audit linking.
    pub async fn get_cl_index(
        &self,
        project_id: &str,
        file_path: &str,
    ) -> Result<Option<ClIndexEntry>> {
        let row = sqlx::query_as::<_, ClIndexEntry>(&format!(
            "SELECT {} FROM cl_index WHERE project_id = ? AND file_path = ?",
            cl_index_columns()
        ))
        .bind(project_id)
        .bind(file_path)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // ---- cl_reads (audit) ------------------------------------------------

    /// Record that an agent read a CL file. A single awaited insert whose error
    /// PROPAGATES (the bridge maps an unknown path to a no-op before calling
    /// this; a real DB failure reaches the tool as an error). The old doc here
    /// said "fire-and-forget; failures are logged but don't bubble up" — the
    /// `?` two lines down says otherwise (round 9).
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
    /// Test-only since round 7 (2026-08-17): no production caller — the "what
    /// context did this agent see?" view it would power was never built (the
    /// `cl_register_read` descriptor no longer promises it) — kept as a test
    /// seam, not shipped.
    #[cfg(test)]
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

    /// Parallel to [`cl_index_search`]. `project=None` searches all projects;
    /// optional `query` is a case-insensitive substring across folder_path /
    /// description / tags.
    pub async fn cl_folder_search(
        &self,
        project: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ClFolder>> {
        self.cl_search_table("cl_folders", "folder_path", project, query, false)
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
        s.record_cl_read(cl_id, Some("s1"), "hands").await.unwrap();
        s.record_cl_read(cl_id, Some("s2"), "eyes").await.unwrap();

        let reads = s.cl_reads_for_session("s1").await.unwrap();
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].agent, "hands");
        assert_eq!(reads[0].cl_index_id, cl_id);
        // Scoped by session: s2 has its own row, an unknown session has none.
        assert_eq!(s.cl_reads_for_session("s2").await.unwrap().len(), 1);
        assert!(s.cl_reads_for_session("nope").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn agent_visibility_filters_agent_search_and_survives_upsert() {
        let s = Storage::memory().await.unwrap();
        s.upsert_project("p", "p", None, None, None).await.unwrap();
        s.upsert_cl_index("p", "notes.md", "d", None).await.unwrap();
        s.upsert_cl_index("p", "diary.md", "d", None).await.unwrap();

        // Default: visible everywhere.
        assert_eq!(s.cl_index_search_agent(Some("p"), None).await.unwrap().len(), 2);

        // Hidden: gone from the agent variant, still in the unfiltered one.
        assert!(s.set_cl_agent_visibility("p", "diary.md", false).await.unwrap());
        let agent = s.cl_index_search_agent(Some("p"), None).await.unwrap();
        assert_eq!(agent.len(), 1);
        assert_eq!(agent[0].file_path, "notes.md");
        let all = s.cl_index_search(Some("p"), None).await.unwrap();
        assert_eq!(all.len(), 2, "UI/rescan surface keeps hidden rows");
        assert!(all.iter().any(|r| r.file_path == "diary.md" && !r.agent_visible));

        // Query-filtered agent search excludes it too (LIKE branch).
        assert!(s
            .cl_index_search_agent(Some("p"), Some("diary"))
            .await
            .unwrap()
            .is_empty());

        // A rescan-style upsert must NOT resurrect visibility.
        s.upsert_cl_index("p", "diary.md", "new description", None).await.unwrap();
        assert!(s
            .cl_index_search_agent(Some("p"), None)
            .await
            .unwrap()
            .iter()
            .all(|r| r.file_path != "diary.md"));

        // Unknown row: setter reports false.
        assert!(!s.set_cl_agent_visibility("p", "nope.md", true).await.unwrap());
    }

    #[tokio::test]
    async fn refresh_cl_index_description_updates_desc_but_preserves_tags() {
        let s = Storage::memory().await.unwrap();
        s.upsert_project("p", "p", None, None, None).await.unwrap();
        s.upsert_cl_index("p", "notes.md", "old description", Some("tag-a,tag-b"))
            .await
            .unwrap();

        s.refresh_cl_index_description("p", "notes.md", "fresh description", "2030-01-01T00:00:00+00:00")
            .await
            .unwrap();

        let row = s.get_cl_index("p", "notes.md").await.unwrap().unwrap();
        assert_eq!(row.description, "fresh description"); // re-derived (Problem B fix)
        assert_eq!(row.tags.as_deref(), Some("tag-a,tag-b")); // user tags survive
        assert_eq!(row.updated_at, "2030-01-01T00:00:00+00:00");
    }
}
