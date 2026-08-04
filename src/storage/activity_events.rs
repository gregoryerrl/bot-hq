//! `activity_events` queries — one durable row per duo-activity transition.
//!
//! Exists because `SessionActivity` was broadcast-only: the UI consumed it and
//! it was gone. `messages` records what the agents said and did; this records
//! what state the session was in while they did it, so the two can be joined.
//! See the migration for why the per-agent flags are not redundant with `state`.

use super::Storage;
use crate::storage::row_types::ActivityEvent;
use crate::storage::time::now_utc;
use anyhow::{Context, Result};

impl Storage {
    /// Record one activity transition. Best-effort by contract: the caller logs
    /// and continues on error, because losing telemetry must never interfere
    /// with the signal that gates the chat input.
    pub async fn insert_activity_event(
        &self,
        session_id: &str,
        state: &str,
        brian_busy: bool,
        rain_busy: bool,
    ) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO activity_events \
             (session_id, recorded_at, state, brian_busy, rain_busy) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(now_utc())
        .bind(state)
        .bind(brian_busy as i64)
        .bind(rain_busy as i64)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording activity event for {session_id}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Activity transitions newest-first. `session_id = None` returns every
    /// session's — the cross-session view is the point when hunting a pattern
    /// rather than diagnosing one session.
    pub async fn list_activity_events(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<ActivityEvent>> {
        const COLS: &str = "id, session_id, recorded_at, state, brian_busy, rain_busy";
        let rows = match session_id {
            Some(sid) => {
                sqlx::query_as::<_, ActivityEvent>(&format!(
                    "SELECT {COLS} FROM activity_events WHERE session_id = ? ORDER BY id DESC"
                ))
                .bind(sid)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as::<_, ActivityEvent>(&format!(
                    "SELECT {COLS} FROM activity_events ORDER BY id DESC"
                ))
                .fetch_all(&self.pool)
                .await
            }
        }
        .context("listing activity events")?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn activity_events_round_trip_newest_first_and_scope_by_session() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "One", None).await.unwrap();
        s.create_session("s2", "Two", None).await.unwrap();

        // The sequence the reported bug produces: Brian busy, then a parked
        // question flips the derived state to awaiting_user WITHOUT Brian
        // stopping, then Brian finally goes idle while the state stays put.
        s.insert_activity_event("s1", "busy", true, false)
            .await
            .unwrap();
        s.insert_activity_event("s1", "awaiting_user", true, false)
            .await
            .unwrap();
        s.insert_activity_event("s1", "awaiting_user", false, false)
            .await
            .unwrap();
        s.insert_activity_event("s2", "idle", false, false)
            .await
            .unwrap();

        let rows = s.list_activity_events(Some("s1")).await.unwrap();
        assert_eq!(rows.len(), 3, "other sessions must not leak in");
        // Newest-first.
        assert_eq!(rows[0].state, "awaiting_user");
        assert_eq!(rows[0].brian_busy, 0);
        // The middle row is the one a state-change-only trigger would have
        // dropped — and without it the latest awaiting_user row would still
        // claim brian_busy = 1 after he stopped.
        assert_eq!(rows[1].state, "awaiting_user");
        assert_eq!(rows[1].brian_busy, 1);
        assert_eq!(rows[2].state, "busy");

        let all = s.list_activity_events(None).await.unwrap();
        assert_eq!(all.len(), 4, "None must span every session");
    }
}
