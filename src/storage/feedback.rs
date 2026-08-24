//! `agent_feedback` queries — agents' issues/ideas about bot-hq itself.
//!
//! Cross-session by design: a session working on any project files here, and a
//! later bot-hq session reads the queue. That is the whole point, so none of
//! these reads are scoped to a session.

use super::{time::now_utc, Storage};
use crate::storage::row_types::AgentFeedback;
use anyhow::{Context, Result};

/// The kinds a caller may file. Anything else is rejected at the tool layer
/// rather than silently stored, so the reader's filters stay meaningful.
pub const FEEDBACK_KINDS: &[&str] = &["issue", "idea"];
/// Lifecycle states a reader may move a row through.
pub const FEEDBACK_STATUSES: &[&str] = &["open", "done", "dismissed"];

impl Storage {
    /// File one piece of feedback. Returns the new row id.
    pub async fn insert_feedback(
        &self,
        session_id: &str,
        project: Option<&str>,
        agent: &str,
        kind: &str,
        title: &str,
        body: &str,
    ) -> Result<i64> {
        let now = now_utc();
        let res = sqlx::query(
            "INSERT INTO agent_feedback \
             (session_id, project, agent, kind, title, body, status, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, 'open', ?, ?)",
        )
        .bind(session_id)
        .bind(project)
        .bind(agent)
        .bind(kind)
        .bind(title)
        .bind(body)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .with_context(|| format!("filing {kind} feedback from {agent}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Feedback rows, newest first. `status = None` returns every row.
    pub async fn list_feedback(&self, status: Option<&str>) -> Result<Vec<AgentFeedback>> {
        let rows = match status {
            Some(s) => {
                sqlx::query_as::<_, AgentFeedback>(
                    "SELECT id, session_id, project, agent, kind, title, body, status, \
                            created_at, updated_at \
                     FROM agent_feedback WHERE status = ? ORDER BY id DESC",
                )
                .bind(s)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as::<_, AgentFeedback>(
                    "SELECT id, session_id, project, agent, kind, title, body, status, \
                            created_at, updated_at \
                     FROM agent_feedback ORDER BY id DESC",
                )
                .fetch_all(&self.pool)
                .await
            }
        }
        .context("listing agent feedback")?;
        Ok(rows)
    }

    /// Move a row through its lifecycle. Returns rows affected (0 = unknown id).
    pub async fn set_feedback_status(&self, id: i64, status: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE agent_feedback SET status = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status)
        .bind(now_utc())
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("setting feedback {id} to {status}"))?;
        Ok(res.rows_affected())
    }

    /// How many rows are still open. Test-only since round 7 (2026-08-17): no
    /// production caller (no badge reads it) — kept as a test seam, not shipped.
    #[cfg(test)]
    pub async fn open_feedback_count(&self) -> Result<i64> {
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM agent_feedback WHERE status = 'open'")
                .fetch_one(&self.pool)
                .await
                .context("counting open feedback")?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Storage {
        Storage::memory().await.unwrap()
    }

    #[tokio::test]
    async fn filed_feedback_is_readable_across_sessions() {
        // The whole point: filed from a data-hub session, read with no session
        // scope at all by a later bot-hq session.
        let s = db().await;
        s.create_session("s-datahub", "t", None).await.unwrap();
        s.insert_feedback(
            "s-datahub",
            Some("acme-data-ingest"),
            "hands",
            "issue",
            "Gate command is unreadable",
            "The body-file content never renders.",
        )
        .await
        .unwrap();

        let all = s.list_feedback(None).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].project.as_deref(), Some("acme-data-ingest"));
        assert_eq!(all[0].status, "open");
        assert_eq!(all[0].agent, "hands");
    }

    #[tokio::test]
    async fn either_agent_may_file() {
        let s = db().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_feedback("s1", None, "hands", "issue", "a", "b")
            .await
            .unwrap();
        s.insert_feedback("s1", None, "eyes", "idea", "c", "d")
            .await
            .unwrap();
        let all = s.list_feedback(None).await.unwrap();
        let agents: Vec<_> = all.iter().map(|f| f.agent.as_str()).collect();
        assert!(agents.contains(&"hands") && agents.contains(&"eyes"));
    }

    #[tokio::test]
    async fn status_filter_and_transitions() {
        let s = db().await;
        s.create_session("s1", "t", None).await.unwrap();
        let id = s
            .insert_feedback("s1", None, "eyes", "idea", "batch approvals", "…")
            .await
            .unwrap();
        assert_eq!(s.open_feedback_count().await.unwrap(), 1);

        assert_eq!(s.set_feedback_status(id, "done").await.unwrap(), 1);
        assert_eq!(s.open_feedback_count().await.unwrap(), 0);
        assert!(s.list_feedback(Some("open")).await.unwrap().is_empty());
        assert_eq!(s.list_feedback(Some("done")).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unknown_id_reports_zero_rather_than_erroring() {
        let s = db().await;
        assert_eq!(s.set_feedback_status(9999, "done").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn newest_first() {
        let s = db().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_feedback("s1", None, "hands", "issue", "first", "…")
            .await
            .unwrap();
        s.insert_feedback("s1", None, "hands", "issue", "second", "…")
            .await
            .unwrap();
        let all = s.list_feedback(None).await.unwrap();
        assert_eq!(all[0].title, "second", "newest first");
    }
}
