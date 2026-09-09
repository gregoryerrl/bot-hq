//! Boot-time GC for the append-only telemetry tables (round 10).
//!
//! `session_tray` and `activity_events` had a retention sweep; five other
//! tables that grow on every turn or tool call had none: `participant_deliveries`
//! (one row per message PER participant — N× `messages`), `context_readings`
//! (one per completed turn), `retrieval_events` (one per `cl_retrieve`),
//! `cancel_events` (one per Stop) and `cl_reads` (one per `cl_register_read`).
//! Each purge below deletes rows older than the caller's horizon — the same
//! `GC_RETENTION_DAYS` main.rs hands the other two — and nothing else: no
//! table is read back after that horizon by anything in the tree
//! (`scripts/turn-latency.py` reads deliveries for recent sessions; the
//! context meter reads the latest reading per participant).
//!
//! `messages` itself is deliberately NOT here — its retention is a parked
//! user decision (rounds 8–10), and deleting message rows would also drop the
//! deliveries that point at them.

use anyhow::{Context, Result};

use super::Storage;

impl Storage {
    /// Delivery receipts older than `retention_days`. A withheld row carries no
    /// `delivered_at` (its `withheld_reason` is the fact), so it ages by the
    /// message it withheld.
    pub async fn purge_participant_deliveries(&self, retention_days: i64) -> Result<u64> {
        let cutoff = crate::storage::cutoff_days_ago(retention_days);
        let res = sqlx::query(
            "DELETE FROM participant_deliveries \
             WHERE delivered_at < ?1 \
                OR (delivered_at IS NULL \
                    AND message_id IN (SELECT id FROM messages WHERE created_at < ?1))",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await
        .context("purging old participant deliveries")?;
        Ok(res.rows_affected())
    }

    /// Context-meter readings older than `retention_days`.
    pub async fn purge_context_readings(&self, retention_days: i64) -> Result<u64> {
        let cutoff = crate::storage::cutoff_days_ago(retention_days);
        let res = sqlx::query("DELETE FROM context_readings WHERE created_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .context("purging old context readings")?;
        Ok(res.rows_affected())
    }

    /// `cl_retrieve` telemetry older than `retention_days`.
    pub async fn purge_retrieval_events(&self, retention_days: i64) -> Result<u64> {
        let cutoff = crate::storage::cutoff_days_ago(retention_days);
        let res = sqlx::query("DELETE FROM retrieval_events WHERE created_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .context("purging old retrieval events")?;
        Ok(res.rows_affected())
    }

    /// Stop-button escalation records older than `retention_days` (by the
    /// instant they settled).
    pub async fn purge_cancel_events(&self, retention_days: i64) -> Result<u64> {
        let cutoff = crate::storage::cutoff_days_ago(retention_days);
        let res = sqlx::query("DELETE FROM cancel_events WHERE settled_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .context("purging old cancel events")?;
        Ok(res.rows_affected())
    }

    /// CL read receipts (`cl_register_read`) older than `retention_days`.
    pub async fn purge_cl_reads(&self, retention_days: i64) -> Result<u64> {
        let cutoff = crate::storage::cutoff_days_ago(retention_days);
        let res = sqlx::query("DELETE FROM cl_reads WHERE read_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .context("purging old CL reads")?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "2020-01-01T00:00:00.000Z";

    /// A session with one participant, one message dated `created_at`, and the
    /// ids a delivery row needs.
    async fn seeded(s: &Storage, msg_at: &str) -> (i64, i64) {
        s.create_session("s1", "t", None).await.unwrap();
        let pid = s
            .insert_participant("s1", "hands", "HANDS", None, None, "[]", "active", 0)
            .await
            .unwrap();
        let mid: i64 = sqlx::query_scalar(
            "INSERT INTO messages (session_id, participant_id, origin, kind, content, created_at, author) \
             VALUES ('s1', ?, 'participant', 'text', 'x', ?, 'hands') RETURNING id",
        )
        .bind(pid)
        .bind(msg_at)
        .fetch_one(s.pool())
        .await
        .unwrap();
        (pid, mid)
    }

    async fn count(s: &Storage, table: &str) -> i64 {
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(s.pool())
            .await
            .unwrap()
    }

    /// Every purge deletes past the horizon and keeps inside it — one row of
    /// each age per table, so a purge that deleted everything (or nothing)
    /// fails here.
    #[tokio::test]
    async fn each_purge_drops_only_rows_past_the_horizon() {
        let s = Storage::memory().await.unwrap();
        let now = crate::storage::now_utc();
        let (pid, old_mid) = seeded(&s, OLD).await;
        let new_mid: i64 = sqlx::query_scalar(
            "INSERT INTO messages (session_id, participant_id, origin, kind, content, created_at, author) \
             VALUES ('s1', ?, 'participant', 'text', 'y', ?, 'hands') RETURNING id",
        )
        .bind(pid)
        .bind(&now)
        .fetch_one(s.pool())
        .await
        .unwrap();

        // participant_deliveries: an old delivered row, a recent delivered
        // row, an old WITHHELD row (no delivered_at — ages by its message),
        // and a recent withheld row. (participant, message) is UNIQUE, so the
        // withheld pair goes to a second participant.
        let peer = s
            .insert_participant("s1", "eyes", "EYES", None, None, "[]", "active", 1)
            .await
            .unwrap();
        for (who, mid, delivered_at, withheld) in [
            (pid, old_mid, Some(OLD), None),
            (pid, new_mid, Some(now.as_str()), None),
            (peer, old_mid, None, Some("spin")),
            (peer, new_mid, None, Some("spin")),
        ] {
            sqlx::query(
                "INSERT INTO participant_deliveries \
                 (participant_id, message_id, delivered_at, withheld_reason) VALUES (?, ?, ?, ?)",
            )
            .bind(who)
            .bind(mid)
            .bind(delivered_at)
            .bind(withheld)
            .execute(s.pool())
            .await
            .unwrap();
        }
        // The other four: one old, one recent, minimal columns.
        for at in [OLD, now.as_str()] {
            sqlx::query(
                "INSERT INTO context_readings \
                 (session_id, participant_slug, model, used_tokens, reported_window, verdict, created_at) \
                 VALUES ('s1', 'hands', 'm', 1, 100, 'usable', ?)",
            )
            .bind(at)
            .execute(s.pool())
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO retrieval_events \
                 (session_id, agent, project_id, query, atom_count, tokens_returned, budget_tokens, created_at) \
                 VALUES ('s1', 'hands', 'p', 'q', 0, 0, 100, ?)",
            )
            .bind(at)
            .execute(s.pool())
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO cancel_events \
                 (session_id, pressed_at, settled_at, both_idle, cancel_superseded, idled_since_cancel, outcome) \
                 VALUES ('s1', ?, ?, 1, 0, 1, 'honored')",
            )
            .bind(at)
            .bind(at)
            .execute(s.pool())
            .await
            .unwrap();
        }
        // cl_reads needs an index row to point at (and that row a project).
        s.upsert_project("p", "P", None, None, None).await.unwrap();
        s.upsert_cl_index("p", "notes.md", "desc", None).await.unwrap();
        let cl_id: i64 = sqlx::query_scalar("SELECT id FROM cl_index WHERE file_path = 'notes.md'")
            .fetch_one(s.pool())
            .await
            .unwrap();
        for at in [OLD, now.as_str()] {
            sqlx::query(
                "INSERT INTO cl_reads (cl_index_id, session_id, agent, read_at) VALUES (?, 's1', 'hands', ?)",
            )
            .bind(cl_id)
            .bind(at)
            .execute(s.pool())
            .await
            .unwrap();
        }

        assert_eq!(s.purge_participant_deliveries(90).await.unwrap(), 2);
        assert_eq!(count(&s, "participant_deliveries").await, 2, "the two recent rows stay");
        assert_eq!(s.purge_context_readings(90).await.unwrap(), 1);
        assert_eq!(count(&s, "context_readings").await, 1);
        assert_eq!(s.purge_retrieval_events(90).await.unwrap(), 1);
        assert_eq!(count(&s, "retrieval_events").await, 1);
        assert_eq!(s.purge_cancel_events(90).await.unwrap(), 1);
        assert_eq!(count(&s, "cancel_events").await, 1);
        assert_eq!(s.purge_cl_reads(90).await.unwrap(), 1);
        assert_eq!(count(&s, "cl_reads").await, 1);
        // Idempotent: a second sweep finds nothing past the horizon.
        assert_eq!(s.purge_participant_deliveries(90).await.unwrap(), 0);
        assert_eq!(s.purge_context_readings(90).await.unwrap(), 0);
    }
}
