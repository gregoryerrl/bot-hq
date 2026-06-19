//! `findings` table: the EYES-sign-off gate's persistence layer. EYES (rain)
//! files findings via `eyes_flag`; HANDS (brian) resolves them via
//! `disposition_finding`. An `open` `blocking` finding gates `git commit` (the
//! MCP `check_open_findings` tool is the prompted primary; the pre-commit /
//! pre-push hooks are the mechanical backstop). Mirrors `tray.rs`'s shape.

use super::*;

/// Full column projection for a `Finding` — shared by every read so they can't
/// drift (mirrors `tray.rs::TRAY_COLUMNS`).
const FINDING_COLUMNS: &str = "id, session_id, finding_uid, agent, severity, summary, \
     code_ref, status, disposition_reason, disposed_by, created_at, updated_at, \
     raise_count, eyes_approved";

impl Storage {
    /// Insert a fresh finding in `open` status. Returns the row id. The session
    /// must already exist (FK).
    pub async fn insert_finding(
        &self,
        session_id: &str,
        finding_uid: &str,
        agent: &str,
        severity: FindingSeverity,
        summary: &str,
        code_ref: Option<&str>,
    ) -> Result<i64> {
        let now = now_utc();
        let res = sqlx::query(
            "INSERT INTO findings \
                (session_id, finding_uid, agent, severity, summary, code_ref, status, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, 'open', ?, ?)",
        )
        .bind(session_id)
        .bind(finding_uid)
        .bind(agent)
        .bind(severity.as_str())
        .bind(summary)
        .bind(code_ref)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .with_context(|| format!("inserting finding {finding_uid} for session {session_id}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Disposition an OPEN finding (fixed / rebutted / stale). Only flips a row
    /// still in `open` status, so a duplicate/stale call is a no-op. Returns the
    /// number of rows affected (0 = uid unknown or already resolved). `reason`
    /// requiredness for fixed/rebutted is enforced at the bridge layer.
    pub async fn disposition_finding(
        &self,
        finding_uid: &str,
        status: FindingStatus,
        reason: Option<&str>,
        disposed_by: &str,
    ) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE findings \
             SET status = ?, disposition_reason = ?, disposed_by = ?, updated_at = ? \
             WHERE finding_uid = ? AND status = 'open'",
        )
        .bind(status.as_str())
        .bind(reason)
        .bind(disposed_by)
        .bind(now_utc())
        .bind(finding_uid)
        .execute(&self.pool)
        .await
        .with_context(|| format!("dispositioning finding {finding_uid}"))?;
        Ok(res.rows_affected())
    }

    /// Count OPEN BLOCKING findings for a session — the gate's hot path
    /// (`check_open_findings` + the git hooks). `advisory` is excluded by design.
    pub async fn count_open_blocking_findings(&self, session_id: &str) -> Result<i64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM findings \
             WHERE session_id = ? AND status = 'open' AND severity = 'blocking'",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("counting open blocking findings for {session_id}"))?;
        Ok(n)
    }

    /// All OPEN BLOCKING findings for a session, oldest-first — for the gate's
    /// block message that lists the offenders the agent must disposition.
    pub async fn open_blocking_findings_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<Finding>> {
        let rows = sqlx::query_as::<_, Finding>(&format!(
            "SELECT {FINDING_COLUMNS} FROM findings \
             WHERE session_id = ? AND status = 'open' AND severity = 'blocking' \
             ORDER BY id ASC"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Every finding for a session, oldest-first — the full `check_open_findings`
    /// view and (later) the UI panel.
    pub async fn findings_for_session(&self, session_id: &str) -> Result<Vec<Finding>> {
        let rows = sqlx::query_as::<_, Finding>(&format!(
            "SELECT {FINDING_COLUMNS} FROM findings WHERE session_id = ? ORDER BY id ASC"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Look up a single finding by its public uid. None when absent.
    pub async fn get_finding(&self, finding_uid: &str) -> Result<Option<Finding>> {
        let row = sqlx::query_as::<_, Finding>(&format!(
            "SELECT {FINDING_COLUMNS} FROM findings WHERE finding_uid = ?"
        ))
        .bind(finding_uid)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Most recent OPEN finding with this exact summary — the re-raise dedup
    /// target for `eyes_flag`. None when no open finding matches.
    pub async fn latest_open_finding_by_summary(
        &self,
        session_id: &str,
        summary: &str,
    ) -> Result<Option<Finding>> {
        let row = sqlx::query_as::<_, Finding>(&format!(
            "SELECT {FINDING_COLUMNS} FROM findings \
             WHERE session_id = ? AND summary = ? AND status = 'open' \
             ORDER BY id DESC LIMIT 1"
        ))
        .bind(session_id)
        .bind(summary)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Bump a finding's `raise_count` (a genuine, turn-guarded re-raise) and
    /// touch `updated_at`. Returns rows affected (0 if the uid is unknown).
    pub async fn increment_raise_count(&self, finding_uid: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE findings SET raise_count = raise_count + 1, updated_at = ? \
             WHERE finding_uid = ?",
        )
        .bind(now_utc())
        .bind(finding_uid)
        .execute(&self.pool)
        .await
        .with_context(|| format!("incrementing raise_count for {finding_uid}"))?;
        Ok(res.rows_affected())
    }

    /// EYES confirms a finding's resolution (`approve_finding`): set
    /// `eyes_approved = 1`, clearing the escalation signal. Returns rows
    /// affected (0 if the uid is unknown). Non-gating — purely signal-clearing.
    pub async fn approve_finding(&self, finding_uid: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE findings SET eyes_approved = 1, updated_at = ? WHERE finding_uid = ?",
        )
        .bind(now_utc())
        .bind(finding_uid)
        .execute(&self.pool)
        .await
        .with_context(|| format!("approving finding {finding_uid}"))?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(s: &Storage) {
        s.create_session("s1", "t", None).await.unwrap();
    }

    #[tokio::test]
    async fn insert_count_disposition_flow() {
        let s = Storage::memory().await.unwrap();
        seed(&s).await;
        s.insert_finding(
            "s1",
            "f1",
            "rain",
            FindingSeverity::Blocking,
            "NPE: job reads adAccount->id but command aliased it",
            Some("ReconcileMetaData.php:42"),
        )
        .await
        .unwrap();
        // An advisory finding must NOT count toward the gate.
        s.insert_finding(
            "s1",
            "f2",
            "rain",
            FindingSeverity::Advisory,
            "nit: rename variable",
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            s.count_open_blocking_findings("s1").await.unwrap(),
            1,
            "only the blocking finding counts"
        );

        // Dispositioning the blocking one clears the gate.
        let n = s
            .disposition_finding("f1", FindingStatus::Fixed, Some("fixed in abc123"), "brian")
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            s.count_open_blocking_findings("s1").await.unwrap(),
            0,
            "a disposed finding no longer gates"
        );

        // Idempotent: a second disposition of the same uid is a no-op.
        assert_eq!(
            s.disposition_finding("f1", FindingStatus::Fixed, Some("x"), "brian")
                .await
                .unwrap(),
            0,
            "already-resolved finding flips no rows"
        );

        let one = s.get_finding("f1").await.unwrap().unwrap();
        assert_eq!(one.status, "fixed");
        assert_eq!(one.disposed_by.as_deref(), Some("brian"));
        assert_eq!(one.disposition_reason.as_deref(), Some("fixed in abc123"));
    }

    #[tokio::test]
    async fn open_blocking_list_is_ordered_and_scoped() {
        let s = Storage::memory().await.unwrap();
        seed(&s).await;
        s.create_session("s2", "t", None).await.unwrap();
        s.insert_finding("s1", "f1", "rain", FindingSeverity::Blocking, "bug A", None)
            .await
            .unwrap();
        s.insert_finding("s1", "f2", "rain", FindingSeverity::Blocking, "bug B", None)
            .await
            .unwrap();
        s.insert_finding("s2", "f3", "rain", FindingSeverity::Blocking, "other session", None)
            .await
            .unwrap();

        let open = s.open_blocking_findings_for_session("s1").await.unwrap();
        assert_eq!(open.len(), 2, "scoped to the session");
        assert_eq!(open[0].summary, "bug A", "oldest-first");
        assert_eq!(open[1].summary, "bug B");
        assert_eq!(s.findings_for_session("s1").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn rebutted_clears_the_gate_too() {
        // §6 decision #2: a rebuttal (HANDS disagrees, with a reason) resolves the
        // finding without EYES agreement — no deadlock.
        let s = Storage::memory().await.unwrap();
        seed(&s).await;
        s.insert_finding("s1", "f1", "rain", FindingSeverity::Blocking, "disputed bug", None)
            .await
            .unwrap();
        s.disposition_finding(
            "f1",
            FindingStatus::Rebutted,
            Some("tests prove the path is unreachable"),
            "brian",
        )
        .await
        .unwrap();
        assert_eq!(s.count_open_blocking_findings("s1").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn reraise_target_increment_and_approve() {
        let s = Storage::memory().await.unwrap();
        seed(&s).await;
        s.insert_finding("s1", "f1", "rain", FindingSeverity::Blocking, "dup bug", None)
            .await
            .unwrap();
        // An OPEN same-summary finding is the dedup target.
        let found = s
            .latest_open_finding_by_summary("s1", "dup bug")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.finding_uid, "f1");
        assert_eq!(found.raise_count, 1);
        assert_eq!(found.eyes_approved, 0);
        // A disposed finding is NOT a dedup target (a re-flag becomes a fresh one).
        s.disposition_finding("f1", FindingStatus::Fixed, Some("done"), "brian")
            .await
            .unwrap();
        assert!(s
            .latest_open_finding_by_summary("s1", "dup bug")
            .await
            .unwrap()
            .is_none());
        // increment + approve mechanics.
        assert_eq!(s.increment_raise_count("f1").await.unwrap(), 1);
        assert_eq!(s.get_finding("f1").await.unwrap().unwrap().raise_count, 2);
        assert_eq!(s.approve_finding("f1").await.unwrap(), 1);
        assert_eq!(s.get_finding("f1").await.unwrap().unwrap().eyes_approved, 1);
    }

    #[tokio::test]
    async fn has_message_from_author_since_detects_turn() {
        let s = Storage::memory().await.unwrap();
        seed(&s).await;
        s.insert_message(
            "s1",
            crate::storage::Author::Brian,
            crate::storage::MessageKind::Text,
            "looking",
        )
        .await
        .unwrap();
        // Fixed-string bounds avoid wall-clock flakiness.
        assert!(s
            .has_message_from_author_since("s1", "brian", "2000-01-01T00:00:00.000Z")
            .await
            .unwrap());
        assert!(!s
            .has_message_from_author_since("s1", "brian", "2999-01-01T00:00:00.000Z")
            .await
            .unwrap());
        assert!(!s
            .has_message_from_author_since("s1", "rain", "2000-01-01T00:00:00.000Z")
            .await
            .unwrap());
    }
}
