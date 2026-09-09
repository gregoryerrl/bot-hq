//! `cancel_events` queries — one durable row per Stop.
//!
//! Exists because the cancel path used to record nothing at all, so a Stop that
//! "didn't hold" could only be argued about, never queried. See the migration
//! for what each field discriminates.

use super::Storage;
#[cfg(test)]
use crate::storage::row_types::CancelEvent;
use anyhow::{Context, Result};

/// Everything one escalation learned. Grouped into a struct because the insert
/// otherwise takes ten positional arguments, most of them bools — exactly the
/// shape that silently transposes.
#[derive(Debug, Clone)]
pub struct CancelEventRecord {
    pub session_id: String,
    pub pressed_at: String,
    pub settled_at: String,
    pub deferred_ms: i64,
    pub deferral_capped: bool,
    /// `None` when the session has no such slot (a solo roster).
    pub slot0_interrupt_queued: Option<bool>,
    pub slot1_interrupt_queued: Option<bool>,
    pub both_idle: bool,
    pub cancel_superseded: bool,
    pub idled_since_cancel: bool,
    pub outcome: String,
}

impl Storage {
    /// Record one Stop. Best-effort by contract: the caller logs and continues
    /// on error, because losing telemetry must never block a cancel.
    pub async fn insert_cancel_event(&self, r: &CancelEventRecord) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO cancel_events \
             (session_id, pressed_at, settled_at, deferred_ms, deferral_capped, \
              slot0_interrupt_queued, slot1_interrupt_queued, both_idle, \
              cancel_superseded, idled_since_cancel, outcome) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.session_id)
        .bind(&r.pressed_at)
        .bind(&r.settled_at)
        .bind(r.deferred_ms)
        .bind(r.deferral_capped as i64)
        .bind(r.slot0_interrupt_queued.map(|b| b as i64))
        .bind(r.slot1_interrupt_queued.map(|b| b as i64))
        .bind(r.both_idle as i64)
        .bind(r.cancel_superseded as i64)
        .bind(r.idled_since_cancel as i64)
        .bind(&r.outcome)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording cancel event for {}", r.session_id))?;
        Ok(res.last_insert_rowid())
    }

    /// Cancel events newest-first. `session_id = None` returns every session's —
    /// the cross-session view is the point when hunting a recurring failure.
    /// Test-only since round 7 (2026-08-17): no production caller — kept as a test seam, not shipped.
    #[cfg(test)]
    pub async fn list_cancel_events(&self, session_id: Option<&str>) -> Result<Vec<CancelEvent>> {
        const COLS: &str = "id, session_id, pressed_at, settled_at, deferred_ms, \
                            deferral_capped, slot0_interrupt_queued, slot1_interrupt_queued, \
                            both_idle, cancel_superseded, idled_since_cancel, outcome";
        let rows = match session_id {
            Some(sid) => {
                sqlx::query_as::<_, CancelEvent>(&format!(
                    "SELECT {COLS} FROM cancel_events WHERE session_id = ? ORDER BY id DESC"
                ))
                .bind(sid)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as::<_, CancelEvent>(&format!(
                    "SELECT {COLS} FROM cancel_events ORDER BY id DESC"
                ))
                .fetch_all(&self.pool)
                .await
            }
        }
        .context("listing cancel events")?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(session: &str, outcome: &str) -> CancelEventRecord {
        CancelEventRecord {
            session_id: session.into(),
            pressed_at: "2026-07-29T01:00:00Z".into(),
            settled_at: "2026-07-29T01:00:02Z".into(),
            deferred_ms: 0,
            deferral_capped: false,
            slot0_interrupt_queued: Some(true),
            slot1_interrupt_queued: Some(true),
            both_idle: outcome == "honored",
            cancel_superseded: false,
            idled_since_cancel: outcome != "sigkill",
            outcome: outcome.into(),
        }
    }

    #[tokio::test]
    async fn round_trip_preserves_every_discriminator() {
        let s = Storage::memory().await.unwrap();
        let mut r = rec("s1", "sigkill");
        r.deferred_ms = 8000;
        r.deferral_capped = true;
        r.slot0_interrupt_queued = Some(false); // dropped
        r.slot1_interrupt_queued = None; // slot 1 unused (a one-participant roster)
        r.cancel_superseded = true;
        s.insert_cancel_event(&r).await.unwrap();

        let got = s.list_cancel_events(Some("s1")).await.unwrap();
        assert_eq!(got.len(), 1);
        let e = &got[0];
        assert_eq!(e.outcome, "sigkill");
        assert_eq!(e.deferred_ms, 8000);
        assert_eq!(e.deferral_capped, 1);
        // The distinction the old code threw away: dropped vs ignored vs absent.
        assert_eq!(e.slot0_interrupt_queued, Some(0), "dropped interrupt");
        assert_eq!(e.slot1_interrupt_queued, None, "no second slot in this session");
        assert_eq!(e.cancel_superseded, 1);
        assert_eq!(e.both_idle, 0);
    }

    #[tokio::test]
    async fn listing_is_newest_first_and_scopes_by_session() {
        let s = Storage::memory().await.unwrap();
        s.insert_cancel_event(&rec("s1", "honored")).await.unwrap();
        s.insert_cancel_event(&rec("s2", "sigkill")).await.unwrap();
        s.insert_cancel_event(&rec("s1", "superseded")).await.unwrap();

        let all = s.list_cancel_events(None).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].outcome, "superseded", "newest first");

        let s1 = s.list_cancel_events(Some("s1")).await.unwrap();
        assert_eq!(s1.len(), 2);
        assert!(s1.iter().all(|e| e.session_id == "s1"));
    }

    #[tokio::test]
    async fn no_events_is_empty_not_an_error() {
        let s = Storage::memory().await.unwrap();
        assert!(s.list_cancel_events(None).await.unwrap().is_empty());
        assert!(s.list_cancel_events(Some("nope")).await.unwrap().is_empty());
    }
}
