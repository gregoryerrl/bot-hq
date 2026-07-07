//! `cl_proposals` table: durable project-scoped Context Library edit proposals.
//! Agents can propose CL changes without directly mutating CL files; host approval
//! performs any eventual write-back.

use super::*;

/// Full column projection for a `ClProposal` — shared by every read so they
/// can't drift.
const CL_PROPOSAL_COLUMNS: &str = "id, proposal_uid, project_id, file_path, kind, \
     target_excerpt, proposed_body, evidence, status, proposed_by, session_id, \
     base_hash, created_at, updated_at";

impl Storage {
    /// Insert a fresh project-scoped CL proposal in `open` status. `session_id` is
    /// audit-only and may be absent; proposal lifecycle follows the project row.
    pub async fn create_cl_proposal(
        &self,
        proposal_uid: &str,
        project_id: &str,
        file_path: &str,
        kind: &str,
        target_excerpt: Option<&str>,
        proposed_body: &str,
        evidence: &str,
        proposed_by: &str,
        session_id: Option<&str>,
        base_hash: Option<&str>,
    ) -> Result<i64> {
        let now = now_utc();
        let row_id: i64 = sqlx::query_scalar(
            "INSERT INTO cl_proposals \
                (proposal_uid, project_id, file_path, kind, target_excerpt, proposed_body, evidence, \
                 status, proposed_by, session_id, base_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'open', ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(proposal_uid)
        .bind(project_id)
        .bind(file_path)
        .bind(kind)
        .bind(target_excerpt)
        .bind(proposed_body)
        .bind(evidence)
        .bind(proposed_by)
        .bind(session_id)
        .bind(base_hash)
        .bind(&now)
        .bind(&now)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("creating CL proposal {proposal_uid} for {project_id}/{file_path}"))?;
        Ok(row_id)
    }

    /// List proposals for one project, optionally filtered by lifecycle status,
    /// oldest-first for stable review order.
    pub async fn list_cl_proposals(
        &self,
        project_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<ClProposal>> {
        let rows = if let Some(status) = status {
            sqlx::query_as::<_, ClProposal>(&format!(
                "SELECT {CL_PROPOSAL_COLUMNS} FROM cl_proposals \
                 WHERE project_id = ? AND status = ? ORDER BY id ASC"
            ))
            .bind(project_id)
            .bind(status)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, ClProposal>(&format!(
                "SELECT {CL_PROPOSAL_COLUMNS} FROM cl_proposals \
                 WHERE project_id = ? ORDER BY id ASC"
            ))
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows)
    }

    /// Look up a single proposal by its public uid. None when absent.
    pub async fn get_cl_proposal(&self, proposal_uid: &str) -> Result<Option<ClProposal>> {
        let row = sqlx::query_as::<_, ClProposal>(&format!(
            "SELECT {CL_PROPOSAL_COLUMNS} FROM cl_proposals WHERE proposal_uid = ?"
        ))
        .bind(proposal_uid)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Resolve an OPEN proposal. Returns None when the uid is unknown or already
    /// resolved, so callers can treat stale duplicate approvals as no-ops.
    pub async fn resolve_cl_proposal(
        &self,
        proposal_uid: &str,
        status: &str,
    ) -> Result<Option<ClProposal>> {
        let now = now_utc();
        let row = sqlx::query_as::<_, ClProposal>(&format!(
            "UPDATE cl_proposals SET status = ?, updated_at = ? \
             WHERE proposal_uid = ? AND status = 'open' \
             RETURNING {CL_PROPOSAL_COLUMNS}"
        ))
        .bind(status)
        .bind(&now)
        .bind(proposal_uid)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("resolving CL proposal {proposal_uid}"))?;
        Ok(row)
    }

    /// Open-proposal counts per project in one aggregate — feeds the Context
    /// Manager sidebar badges + the subtab pill sum without N per-project
    /// `list_cl_proposals` round-trips. Projects with zero open proposals are
    /// simply absent from the result.
    pub async fn count_open_cl_proposals_by_project(&self) -> Result<Vec<(String, i64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT project_id, COUNT(*) FROM cl_proposals \
             WHERE status = 'open' GROUP BY project_id ORDER BY project_id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Count OTHER open proposals targeting the same file — the "competing
    /// suggestions" signal surfaced back to the proposing agent and the review
    /// queue. Excludes `exclude_uid` so a fresh insert doesn't count itself.
    pub async fn count_open_cl_proposals_for_file(
        &self,
        project_id: &str,
        file_path: &str,
        exclude_uid: &str,
    ) -> Result<i64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cl_proposals \
             WHERE project_id = ? AND file_path = ? AND status = 'open' AND proposal_uid != ?",
        )
        .bind(project_id)
        .bind(file_path)
        .bind(exclude_uid)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    /// Revert an APPROVED proposal back to `open` — the compensation step when
    /// the approval's file write-back fails after the open→approved claim.
    /// Scoped to `approved` rows so it can never resurrect a rejected proposal.
    /// Returns true when a row was reverted.
    pub async fn reopen_cl_proposal(&self, proposal_uid: &str) -> Result<bool> {
        let now = now_utc();
        let res = sqlx::query(
            "UPDATE cl_proposals SET status = 'open', updated_at = ? \
             WHERE proposal_uid = ? AND status = 'approved'",
        )
        .bind(&now)
        .bind(proposal_uid)
        .execute(&self.pool)
        .await
        .with_context(|| format!("reopening CL proposal {proposal_uid}"))?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(s: &Storage) {
        s.upsert_project("bot-hq", "bot-hq", None, None, None)
            .await
            .unwrap();
        s.create_session("s1", "CL proposals", None).await.unwrap();
        s.create_session("s2", "Other session", None).await.unwrap();
    }

    #[tokio::test]
    async fn create_list_get_resolve_proposal_flow() {
        let s = Storage::memory().await.unwrap();
        seed(&s).await;

        s.create_cl_proposal(
            "p1",
            "bot-hq",
            "notes.md",
            "correct",
            Some("old paragraph"),
            "# bot-hq notes\n\nUpdated full body\n",
            "Observed stale wording during review.",
            "rain",
            Some("s1"),
            None,
        )
        .await
        .unwrap();

        let open = s.list_cl_proposals("bot-hq", Some("open")).await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].proposal_uid, "p1");
        assert_eq!(open[0].project_id, "bot-hq");
        assert_eq!(open[0].file_path, "notes.md");
        assert_eq!(open[0].kind, "correct");
        assert_eq!(open[0].target_excerpt.as_deref(), Some("old paragraph"));
        assert_eq!(open[0].proposed_body, "# bot-hq notes\n\nUpdated full body\n");
        assert_eq!(open[0].evidence, "Observed stale wording during review.");
        assert_eq!(open[0].status, "open");
        assert_eq!(open[0].proposed_by, "rain");
        assert_eq!(open[0].session_id.as_deref(), Some("s1"));

        let one = s.get_cl_proposal("p1").await.unwrap().unwrap();
        assert_eq!(one.proposal_uid, "p1");

        let resolved = s.resolve_cl_proposal("p1", "approved").await.unwrap().unwrap();
        assert_eq!(resolved.status, "approved");
        assert!(resolved.updated_at >= one.updated_at);

        assert!(s.list_cl_proposals("bot-hq", Some("open")).await.unwrap().is_empty());
        assert_eq!(s.list_cl_proposals("bot-hq", Some("approved")).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn proposals_are_project_scoped_and_session_is_audit_only() {
        let s = Storage::memory().await.unwrap();
        seed(&s).await;
        s.upsert_project("other", "other", None, None, None).await.unwrap();

        s.create_cl_proposal(
            "p1",
            "bot-hq",
            "notes.md",
            "add",
            None,
            "new file body",
            "new note",
            "brian",
            Some("s1"),
            None,
        )
        .await
        .unwrap();
        s.create_cl_proposal(
            "p2",
            "other",
            "notes.md",
            "add",
            None,
            "other body",
            "other note",
            "rain",
            Some("s2"),
            None,
        )
        .await
        .unwrap();
        s.create_cl_proposal(
            "p3",
            "bot-hq",
            "decisions.md",
            "correct",
            None,
            "complete body",
            "audit-only session absent",
            "rain",
            None,
            None,
        )
        .await
        .unwrap();

        let ours = s.list_cl_proposals("bot-hq", None).await.unwrap();
        assert_eq!(ours.len(), 2);
        assert_eq!(ours[0].proposal_uid, "p1");
        assert_eq!(ours[1].proposal_uid, "p3");
        assert_eq!(s.list_cl_proposals("other", None).await.unwrap().len(), 1);

        assert_eq!(s.get_cl_proposal("p3").await.unwrap().unwrap().session_id, None);
    }

    #[tokio::test]
    async fn count_open_proposals_groups_by_project_and_excludes_resolved() {
        let s = Storage::memory().await.unwrap();
        seed(&s).await;
        s.upsert_project("other", "other", None, None, None).await.unwrap();

        s.create_cl_proposal("p1", "bot-hq", "notes.md", "add", None, "b", "e", "brian", None, None)
            .await
            .unwrap();
        s.create_cl_proposal("p2", "bot-hq", "decisions.md", "correct", None, "b", "e", "rain", None, None)
            .await
            .unwrap();
        s.create_cl_proposal("p3", "other", "notes.md", "add", None, "b", "e", "brian", None, None)
            .await
            .unwrap();
        s.create_cl_proposal("p4", "other", "ideas.md", "add", None, "b", "e", "rain", None, None)
            .await
            .unwrap();
        // Resolved rows must not count.
        s.resolve_cl_proposal("p4", "rejected").await.unwrap().unwrap();

        let counts = s.count_open_cl_proposals_by_project().await.unwrap();
        assert_eq!(counts, vec![("bot-hq".to_string(), 2), ("other".to_string(), 1)]);

        // A project with zero open proposals disappears from the aggregate.
        s.resolve_cl_proposal("p3", "approved").await.unwrap().unwrap();
        let counts = s.count_open_cl_proposals_by_project().await.unwrap();
        assert_eq!(counts, vec![("bot-hq".to_string(), 2)]);
    }

    #[tokio::test]
    async fn reopen_reverts_approved_but_never_rejected_or_open() {
        let s = Storage::memory().await.unwrap();
        seed(&s).await;
        s.create_cl_proposal("p1", "bot-hq", "notes.md", "add", None, "b", "e", "brian", None, None)
            .await
            .unwrap();

        // Open → reopen is a no-op (nothing to revert).
        assert!(!s.reopen_cl_proposal("p1").await.unwrap());

        // Approved → reopen reverts to open (the failed-write compensation).
        s.resolve_cl_proposal("p1", "approved").await.unwrap().unwrap();
        assert!(s.reopen_cl_proposal("p1").await.unwrap());
        assert_eq!(s.get_cl_proposal("p1").await.unwrap().unwrap().status, "open");

        // Rejected → reopen must NOT resurrect.
        s.resolve_cl_proposal("p1", "rejected").await.unwrap().unwrap();
        assert!(!s.reopen_cl_proposal("p1").await.unwrap());
        assert_eq!(s.get_cl_proposal("p1").await.unwrap().unwrap().status, "rejected");
    }
}
