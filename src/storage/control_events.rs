//! `control_events` — the durable journal a controller reads with
//! `app_control events` (Plan 5c). One row per journaled bridge event, `id`
//! the cursor. The bridge's journal task is the only writer
//! (`signaling::bridge::control_events`); this module is the rows, the
//! forward page, the retention sweep, and the reconciliation queries the task
//! runs at start.

use super::Storage;
use crate::storage::time::now_utc;
use anyhow::{Context, Result};
use serde::Serialize;

/// Rows older than this are swept on insert.
pub const CONTROL_EVENT_RETENTION_DAYS: i64 = 7;
/// And rows past this count, oldest first.
pub const CONTROL_EVENT_MAX_ROWS: i64 = 50_000;
/// The most a page returns or scans.
pub const CONTROL_EVENT_PAGE_MAX: usize = 100;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, PartialEq)]
pub struct ControlEvent {
    pub id: i64,
    pub at: String,
    pub session_id: String,
    pub kind: String,
    pub agent: Option<String>,
    pub ref_id: Option<String>,
    pub detail: Option<String>,
    pub source_session: Option<String>,
    pub source_slug: Option<String>,
}

/// What the journal writes; `at` is stamped here.
#[derive(Debug, Clone, Default)]
pub struct NewControlEvent {
    pub session_id: String,
    pub kind: String,
    pub agent: Option<String>,
    pub ref_id: Option<String>,
    pub detail: Option<String>,
    pub source_session: Option<String>,
    pub source_slug: Option<String>,
}

/// One forward page: the rows that passed the filters among the `scanned`
/// rows after `since_id`; `next_since_id` is the LAST ID SCANNED (so a page
/// whose rows were all filtered out still progresses), or `since_id` when
/// nothing was scanned; `truncated` says rows between `since_id` and the
/// oldest retained row were swept; `markers` are the journal's own
/// `journal_restart` / `journal_gap` rows among the scanned ones, listed
/// WHATEVER the filters (eyes-2 `5c-1`: a filtered page must not consume the
/// only evidence that events were lost) — a reader must not take the page
/// for the whole story.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ControlEventPage {
    pub events: Vec<ControlEvent>,
    pub scanned: usize,
    pub next_since_id: i64,
    pub truncated: bool,
    pub markers: Vec<ControlEvent>,
}

/// The journal's own rows use this in place of a session.
pub const JOURNAL_SESSION: &str = "*";

/// A pending decision or a declared halt the journal may have missed — what
/// the task reconciles at start.
#[derive(Debug, Clone, PartialEq)]
pub struct AttentionRow {
    pub session_id: String,
    pub agent: String,
    pub ref_id: Option<String>,
    pub detail: String,
    pub at: String,
}

impl Storage {
    pub async fn insert_control_event(&self, event: &NewControlEvent) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO control_events (at, session_id, kind, agent, ref_id, detail, source_session, source_slug) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(now_utc())
        .bind(&event.session_id)
        .bind(&event.kind)
        .bind(&event.agent)
        .bind(&event.ref_id)
        .bind(&event.detail)
        .bind(&event.source_session)
        .bind(&event.source_slug)
        .execute(&self.pool)
        .await
        .context("journaling a control event")?;
        Ok(res.last_insert_rowid())
    }

    /// The forward page described on [`ControlEventPage`]. `sessions` / `kinds`
    /// filter the returned rows, never what is scanned; `limit` is clamped to
    /// 1..=[`CONTROL_EVENT_PAGE_MAX`].
    pub async fn control_events_page(
        &self,
        since_id: i64,
        sessions: Option<&[String]>,
        kinds: Option<&[String]>,
        limit: usize,
    ) -> Result<ControlEventPage> {
        let limit = limit.clamp(1, CONTROL_EVENT_PAGE_MAX);
        let scanned_rows: Vec<ControlEvent> = sqlx::query_as(
            "SELECT id, at, session_id, kind, agent, ref_id, detail, source_session, source_slug \
             FROM control_events WHERE id > ? ORDER BY id ASC LIMIT ?",
        )
        .bind(since_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        let oldest: Option<i64> = sqlx::query_scalar("SELECT MIN(id) FROM control_events")
            .fetch_one(&self.pool)
            .await?;
        let truncated = oldest.is_some_and(|oldest| since_id.saturating_add(1) < oldest);
        let scanned = scanned_rows.len();
        let next_since_id = scanned_rows.last().map_or(since_id, |r| r.id);
        let markers = scanned_rows.iter().filter(|r| r.session_id == JOURNAL_SESSION).cloned().collect();
        let events = scanned_rows
            .into_iter()
            .filter(|r| sessions.is_none_or(|s| s.iter().any(|id| *id == r.session_id)))
            .filter(|r| kinds.is_none_or(|k| k.iter().any(|kind| *kind == r.kind)))
            .collect();
        Ok(ControlEventPage { events, scanned, next_since_id, truncated, markers })
    }

    /// The newest journal row's id and stamp (`None` on an empty journal).
    pub async fn last_control_event(&self) -> Result<Option<(i64, String)>> {
        Ok(sqlx::query_as("SELECT id, at FROM control_events ORDER BY id DESC LIMIT 1")
            .fetch_optional(&self.pool)
            .await?)
    }

    /// The last `activity` state the journal holds for a session — so a
    /// repeated state is journaled once across restarts too.
    pub async fn last_journaled_activity(&self, session_id: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT detail FROM control_events WHERE session_id = ? AND kind = 'activity' ORDER BY id DESC LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    /// The retention sweep: rows older than `cutoff` (RFC3339-Z) and rows past
    /// the cap, oldest first. Returns how many went.
    pub async fn purge_control_events(&self, cutoff: &str, max_rows: i64) -> Result<u64> {
        let aged = sqlx::query("DELETE FROM control_events WHERE at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?
            .rows_affected();
        let over = sqlx::query(
            "DELETE FROM control_events WHERE id <= COALESCE(\
                 (SELECT id FROM control_events ORDER BY id DESC LIMIT 1 OFFSET ?), 0)",
        )
        .bind(max_rows)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(aged + over)
    }

    /// Whether the journal already holds a row of `kind` for this identity —
    /// a decision's choice id, a halt's declared-at stamp — so reconciliation
    /// and a buffered broadcast never write the same attention state twice.
    pub async fn control_event_exists(&self, kind: &str, session_id: &str, ref_id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM control_events WHERE kind = ? AND session_id = ? AND ref_id = ?)",
        )
        .bind(kind)
        .bind(session_id)
        .bind(ref_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Every pending decision of every open session — what the journal
    /// reconciles at start (deduped by choice id), since a `PendingChoice`
    /// broadcast the previous process never journaled is otherwise lost. No
    /// time cutoff: the newest journal row proves nothing about coverage
    /// (eyes-2 `5c-1`).
    pub async fn pending_decisions_open(&self) -> Result<Vec<AttentionRow>> {
        let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT t.session_id, t.agent, t.choice_id, t.prompt, t.asked_at FROM session_tray t \
             JOIN sessions s ON s.id = t.session_id \
             WHERE t.status = 'pending' AND s.closed_at IS NULL \
             ORDER BY t.id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(session_id, agent, choice_id, prompt, at)| AttentionRow {
                session_id,
                agent,
                ref_id: Some(choice_id),
                detail: prompt,
                at,
            })
            .collect())
    }

    /// Every standing halt of every open session — the other durable
    /// attention state a restart may have lost; `ref_id` is the slot's
    /// `halt_id`, the identity the declaration minted (`None` for a halt
    /// declared by a build before the column existed).
    pub async fn standing_halts_open(&self) -> Result<Vec<AttentionRow>> {
        let rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, halt_declared_by, halt_reason, COALESCE(halt_declared_at, ''), halt_id FROM sessions \
             WHERE halt_declared_by IS NOT NULL AND closed_at IS NULL \
             ORDER BY halt_declared_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(session_id, agent, reason, at, halt_id)| AttentionRow { session_id, agent, ref_id: halt_id, detail: reason, at })
            .collect())
    }

    /// The slot's current `halt_id`, for tests and callers correlating a halt.
    pub async fn session_halt_id(&self, session_id: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT halt_id FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?
            .flatten())
    }

    /// Whether any enabled participant of an open session holds the
    /// application-control grant in its snapshot — the journal's reason to
    /// exist; without one it journals only its own markers.
    pub async fn any_controller_present(&self) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM session_participants p JOIN sessions s ON s.id = p.session_id \
             WHERE p.enabled <> 0 AND s.closed_at IS NULL AND s.archived = 0 \
             AND p.capabilities LIKE '%\"control_bot_hq\"%')",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    /// The controller that caused a delegated message row, and its request
    /// id, if `message_id` is one — a `send`, or a watch wake's notice
    /// (request id `watch:<watch id>:<event id>`).
    pub async fn delegated_message_source(&self, message_id: i64) -> Result<Option<(String, String, String)>> {
        Ok(sqlx::query_as(
            "SELECT d.source_session, p.slug, d.request_id FROM delegated_messages d \
             JOIN session_participants p ON p.id = d.source_participant WHERE d.message_id = ?",
        )
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Origin, kind and participant slug of one message row, for the journal.
    pub async fn message_journal_facts(&self, message_id: i64) -> Result<Option<(String, String, Option<String>)>> {
        Ok(sqlx::query_as(
            "SELECT m.origin, m.kind, p.slug FROM messages m \
             LEFT JOIN session_participants p ON p.id = m.participant_id WHERE m.id = ?",
        )
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(session: &str, kind: &str) -> NewControlEvent {
        NewControlEvent { session_id: session.into(), kind: kind.into(), ..Default::default() }
    }

    /// The page contract: forward by id, filters apply to what is returned
    /// and never to what is scanned, the cursor is the last id scanned, a
    /// cursor older than retention reads as truncated.
    #[tokio::test]
    async fn the_page_scans_forward_and_names_its_cursor_and_gaps() {
        let s = Storage::memory().await.unwrap();
        let mut ids = Vec::new();
        for (session, kind) in [("a", "message"), ("b", "decision_pending"), ("a", "halt_declared"), ("b", "message"), ("a", "message")] {
            ids.push(s.insert_control_event(&ev(session, kind)).await.unwrap());
        }
        assert!(ids.windows(2).all(|w| w[1] > w[0]), "monotonic ids: {ids:?}");
        let page = s.control_events_page(0, None, None, 3).await.unwrap();
        assert_eq!(page.events.iter().map(|e| e.id).collect::<Vec<_>>(), ids[..3]);
        assert_eq!((page.scanned, page.next_since_id, page.truncated), (3, ids[2], false));
        let page = s.control_events_page(page.next_since_id, None, None, 10).await.unwrap();
        assert_eq!(page.events.iter().map(|e| e.id).collect::<Vec<_>>(), ids[3..]);
        assert_eq!(page.next_since_id, ids[4]);
        // Filters: returned rows shrink, the scan does not.
        let page = s.control_events_page(0, Some(&["b".to_string()]), None, 10).await.unwrap();
        assert_eq!(page.events.iter().map(|e| e.session_id.as_str()).collect::<Vec<_>>(), ["b", "b"]);
        assert_eq!((page.scanned, page.next_since_id), (5, ids[4]));
        let page = s.control_events_page(0, None, Some(&["decision_pending".to_string()]), 1).await.unwrap();
        assert!(page.events.is_empty(), "the one scanned row is a message");
        assert_eq!(page.next_since_id, ids[0], "an empty page still progresses");
        // Past the end: nothing scanned, the cursor stays put.
        let page = s.control_events_page(ids[4], None, None, 10).await.unwrap();
        assert_eq!((page.scanned, page.next_since_id), (0, ids[4]));
        // The largest cursor neither panics nor reads as a gap.
        let page = s.control_events_page(i64::MAX, None, None, 10).await.unwrap();
        assert_eq!((page.scanned, page.next_since_id, page.truncated), (0, i64::MAX, false));
        // Limit is clamped, never zero.
        assert_eq!(s.control_events_page(0, None, None, 0).await.unwrap().scanned, 1);
        assert_eq!(s.control_events_page(0, None, None, 10_000).await.unwrap().scanned, 5);
        // Retention gap: once the first two rows are gone, a cursor before
        // them is told so; a cursor at the oldest row is not.
        s.purge_control_events("2000-01-01T00:00:00.000Z", 3).await.unwrap();
        let page = s.control_events_page(0, None, None, 10).await.unwrap();
        assert!(page.truncated);
        assert_eq!(page.events.iter().map(|e| e.id).collect::<Vec<_>>(), ids[2..]);
        assert!(!s.control_events_page(ids[2] - 1, None, None, 10).await.unwrap().truncated);
        assert!(!Storage::memory().await.unwrap().control_events_page(0, None, None, 10).await.unwrap().truncated, "an empty journal has no gap");
        // The journal's own markers ride every page whatever the filters.
        let marker = s.insert_control_event(&ev(JOURNAL_SESSION, "journal_gap")).await.unwrap();
        s.insert_control_event(&ev("b", "message")).await.unwrap();
        let page = s.control_events_page(ids[4], Some(&["a".to_string()]), Some(&["decision_pending".to_string()]), 10).await.unwrap();
        assert!(page.events.is_empty());
        assert_eq!(page.markers.iter().map(|m| m.id).collect::<Vec<_>>(), [marker], "{page:?}");
        assert_eq!(s.control_events_page(0, None, None, 100).await.unwrap().markers.len(), 1);
        assert!(s.control_events_page(marker, None, None, 100).await.unwrap().markers.is_empty(), "a marker before the cursor is not re-listed");
    }

    /// The sweep by age and by count, the newest-row read, and the last
    /// journaled activity per session.
    #[tokio::test]
    async fn retention_sweeps_by_age_and_count() {
        let s = Storage::memory().await.unwrap();
        for i in 0..6 {
            s.insert_control_event(&NewControlEvent { detail: Some(format!("st{i}")), ..ev("a", "activity") }).await.unwrap();
        }
        assert_eq!(s.last_journaled_activity("a").await.unwrap().as_deref(), Some("st5"));
        assert_eq!(s.last_journaled_activity("b").await.unwrap(), None);
        let (last, at) = s.last_control_event().await.unwrap().unwrap();
        assert!(at.ends_with('Z'));
        assert_eq!(s.purge_control_events("2000-01-01T00:00:00.000Z", 4).await.unwrap(), 2, "two over the cap");
        let page = s.control_events_page(0, None, None, 10).await.unwrap();
        assert_eq!(page.events.len(), 4);
        assert_eq!(page.events.last().unwrap().id, last);
        let future = "2999-01-01T00:00:00.000Z";
        assert_eq!(s.purge_control_events(future, 100).await.unwrap(), 4, "all aged out");
        assert_eq!(s.last_control_event().await.unwrap(), None);
        assert_eq!(Storage::memory().await.unwrap().purge_control_events(future, 10).await.unwrap(), 0);
    }

    /// The reconciliation queries: pending decisions and standing halts
    /// declared after a stamp, open sessions only; the controller-presence
    /// and delegated-source lookups.
    #[tokio::test]
    async fn reconciliation_and_presence_queries_read_the_durable_state() {
        let s = Storage::memory().await.unwrap();
        s.create_session("open", "t", None).await.unwrap();
        s.ensure_session_roster("open", 2).await.unwrap();
        s.create_session("closed", "t", None).await.unwrap();
        s.ensure_session_roster("closed", 2).await.unwrap();
        assert!(!s.any_controller_present().await.unwrap());
        let stamp = now_utc();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let yes_no = ["yes".to_string(), "no".to_string()];
        s.insert_tray_entry("open", "q1", "hands", crate::storage::QuestionKind::Choice, "Deploy?", Some(&yes_no), None, None).await.unwrap();
        s.insert_tray_entry("closed", "q2", "hands", crate::storage::QuestionKind::Choice, "Other?", Some(&yes_no), None, None).await.unwrap();
        s.declare_session_halt("open", "eyes", "waiting on CI").await.unwrap();
        s.declare_session_halt("closed", "hands", "gone").await.unwrap();
        sqlx::query("UPDATE sessions SET closed_at = '2026-01-01T00:00:00.000Z' WHERE id = 'closed'").execute(s.pool()).await.unwrap();
        let decisions = s.pending_decisions_open().await.unwrap();
        assert_eq!(decisions.len(), 1, "{decisions:?}");
        assert_eq!((decisions[0].session_id.as_str(), decisions[0].agent.as_str(), decisions[0].ref_id.as_deref(), decisions[0].detail.as_str()), ("open", "hands", Some("q1"), "Deploy?"));
        assert!(decisions[0].at > stamp);
        let halts = s.standing_halts_open().await.unwrap();
        assert_eq!(halts.len(), 1);
        assert_eq!((halts[0].session_id.as_str(), halts[0].agent.as_str(), halts[0].detail.as_str()), ("open", "eyes", "waiting on CI"));
        let halt_id = s.session_halt_id("open").await.unwrap().expect("the declare minted an id");
        assert_eq!(halts[0].ref_id.as_deref(), Some(halt_id.as_str()), "a halt's identity is the id its declaration minted");
        assert!(halts[0].at.ends_with('Z'));
        // Two declarations in the same instant are two identities.
        let a = s.declare_session_halt_stamped("open", "eyes", "one").await.unwrap().unwrap();
        let b = s.declare_session_halt_stamped("open", "eyes", "two").await.unwrap().unwrap();
        assert_ne!(a, b);
        assert_eq!(s.session_halt_id("open").await.unwrap().as_deref(), Some(b.as_str()));
        assert!(s.clear_session_halt("open").await.unwrap());
        assert_eq!(s.session_halt_id("open").await.unwrap(), None, "cleared with the slot");
        // Identity check for the dedupe.
        assert!(!s.control_event_exists("decision_pending", "open", "q1").await.unwrap());
        s.insert_control_event(&NewControlEvent { ref_id: Some("q1".into()), ..ev("open", "decision_pending") }).await.unwrap();
        assert!(s.control_event_exists("decision_pending", "open", "q1").await.unwrap());
        assert!(!s.control_event_exists("decision_pending", "other", "q1").await.unwrap());
        assert!(!s.control_event_exists("halt_declared", "open", "q1").await.unwrap());
        sqlx::query("UPDATE session_participants SET capabilities = '[\"control_bot_hq\"]' WHERE session_id = 'open' AND slug = 'hands'").execute(s.pool()).await.unwrap();
        assert!(s.any_controller_present().await.unwrap());
        sqlx::query("UPDATE session_participants SET enabled = 0 WHERE session_id = 'open' AND slug = 'hands'").execute(s.pool()).await.unwrap();
        assert!(!s.any_controller_present().await.unwrap(), "a removed controller does not count");
        let m = s.post_to_channel("open", "participant", Some("eyes"), "text", "hello", None).await.unwrap().message_id();
        assert_eq!(s.message_journal_facts(m).await.unwrap(), Some(("participant".into(), "text".into(), Some("eyes".into()))));
        assert_eq!(s.delegated_message_source(m).await.unwrap(), None);
        assert_eq!(s.message_journal_facts(m + 1000).await.unwrap(), None);
    }
}
