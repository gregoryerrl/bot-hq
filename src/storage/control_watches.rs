//! `control_watches` — a controller's standing subscriptions over the
//! control-event journal (Plan 5c-2). The rows, their one-shot wake claim,
//! the explicit acknowledgement, and the queries the wake engine and the boot
//! respawn need. Matching an event to a watch is Rust in
//! `signaling::bridge::control_events`; this module never decides a match.

use super::Storage;
use crate::storage::time::now_utc;
use anyhow::{Context, Result};
use serde::Serialize;

/// The kinds a watch wakes on when it names none: the states that need a
/// controller, not the running commentary. `health` is in the list for its
/// terminal state only — see [`default_kind_matches`].
pub const ATTENTION_KINDS: &[&str] = &["decision_pending", "halt_declared", "gate_urgent", "health", "session_closed"];

/// The health detail a DEFAULT watch wakes for: the runtime is gone and the
/// supervisor is not bringing it back. `running`/`stalled` flip on the stall
/// watchdog's 90-second rule (a reasoning model crosses it without a tool
/// event) and `retrying` is the supervisor's own recovery; none of them needs
/// a controller's turn (feedback #144).
pub const DEFAULT_HEALTH_WAKE_DETAIL: &str = "dead";

/// Does an event of `kind` with `detail` match a watch that names NO kinds?
/// The attention kinds, with `health` narrowed to [`DEFAULT_HEALTH_WAKE_DETAIL`].
/// A watch that names `health` explicitly is matched by kind alone (every
/// transition) — that is the caller's comparison, not this one.
pub fn default_kind_matches(kind: &str, detail: Option<&str>) -> bool {
    if !ATTENTION_KINDS.contains(&kind) {
        return false;
    }
    kind != "health" || detail == Some(DEFAULT_HEALTH_WAKE_DETAIL)
}
/// Every kind the journal writes that a watch may name.
pub const WATCHABLE_KINDS: &[&str] = &[
    "message", "decision_pending", "decision_resolved", "halt_declared", "halt_cleared", "session_closed", "health",
    "activity", "gate_urgent", "participant_changed", "doc_changed", "findings_changed", "phase_vote",
    "journal_restart", "journal_gap", "watch_dropped",
];
/// How many watches one controller may hold.
pub const MAX_WATCHES_PER_CONTROLLER: usize = 8;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ControlWatch {
    pub id: i64,
    pub session_id: String,
    pub participant_id: i64,
    pub slug: String,
    pub request_id: String,
    pub targets: Option<Vec<String>>,
    pub kinds: Option<Vec<String>>,
    pub wake: bool,
    pub cursor: i64,
    pub pending_since: Option<i64>,
    pub pending_kind: Option<String>,
    pub woke_for: Option<i64>,
    pub woke_at: Option<String>,
    pub wake_attempt_at: Option<String>,
    pub wake_delivered_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(sqlx::FromRow)]
struct WatchRow {
    id: i64,
    session_id: String,
    participant_id: i64,
    slug: String,
    request_id: String,
    targets: Option<String>,
    kinds: Option<String>,
    wake: i64,
    cursor: i64,
    pending_since: Option<i64>,
    pending_kind: Option<String>,
    woke_for: Option<i64>,
    woke_at: Option<String>,
    wake_attempt_at: Option<String>,
    wake_delivered_at: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<WatchRow> for ControlWatch {
    type Error = anyhow::Error;
    fn try_from(r: WatchRow) -> Result<Self> {
        let list = |text: Option<String>| -> Result<Option<Vec<String>>> {
            text.map(|t| serde_json::from_str::<Vec<String>>(&t).context("a watch's list is not JSON")).transpose()
        };
        Ok(ControlWatch {
            id: r.id,
            session_id: r.session_id,
            participant_id: r.participant_id,
            slug: r.slug,
            request_id: r.request_id,
            targets: list(r.targets)?,
            kinds: list(r.kinds)?,
            wake: r.wake != 0,
            cursor: r.cursor,
            pending_since: r.pending_since,
            pending_kind: r.pending_kind,
            woke_for: r.woke_for,
            woke_at: r.woke_at,
            wake_attempt_at: r.wake_attempt_at,
            wake_delivered_at: r.wake_delivered_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

const COLS: &str = "id, session_id, participant_id, slug, request_id, targets, kinds, wake, cursor, pending_since, pending_kind, woke_for, woke_at, wake_attempt_at, wake_delivered_at, created_at, updated_at";

/// How long one delivery attempt owns the wake before another may resume it.
pub const WAKE_ATTEMPT_LEASE_SECS: i64 = 60;

impl Storage {
    /// Create the watch, or update an existing one's filters and wake flag.
    /// An update never touches the cursor or the pending marker: an identical
    /// retry returns the watch as it stands (eyes-2, Plan 5c review). A new
    /// watch starts at `initial_cursor` — the journal's current end, so no
    /// history wakes it.
    pub async fn upsert_control_watch(
        &self,
        session_id: &str,
        participant_id: i64,
        slug: &str,
        request_id: &str,
        targets: Option<&[String]>,
        kinds: Option<&[String]>,
        wake: bool,
        initial_cursor: i64,
    ) -> Result<ControlWatch> {
        let targets_json = targets.map(serde_json::to_string).transpose()?;
        let kinds_json = kinds.map(serde_json::to_string).transpose()?;
        let now = now_utc();
        sqlx::query(
            "INSERT INTO control_watches (session_id, participant_id, slug, request_id, targets, kinds, wake, cursor, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(participant_id, request_id) DO UPDATE SET targets = excluded.targets, kinds = excluded.kinds, \
             wake = excluded.wake, updated_at = excluded.updated_at",
        )
        .bind(session_id)
        .bind(participant_id)
        .bind(slug)
        .bind(request_id)
        .bind(targets_json)
        .bind(kinds_json)
        .bind(wake as i64)
        .bind(initial_cursor)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .context("saving a control watch")?;
        self.control_watch(participant_id, request_id).await?.context("the watch just saved is missing")
    }

    pub async fn control_watch(&self, participant_id: i64, request_id: &str) -> Result<Option<ControlWatch>> {
        let row: Option<WatchRow> =
            sqlx::query_as(&format!("SELECT {COLS} FROM control_watches WHERE participant_id = ? AND request_id = ?"))
                .bind(participant_id)
                .bind(request_id)
                .fetch_optional(&self.pool)
                .await?;
        row.map(ControlWatch::try_from).transpose()
    }

    pub async fn control_watch_by_id(&self, id: i64) -> Result<Option<ControlWatch>> {
        let row: Option<WatchRow> = sqlx::query_as(&format!("SELECT {COLS} FROM control_watches WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(ControlWatch::try_from).transpose()
    }

    pub async fn control_watches_for(&self, participant_id: i64) -> Result<Vec<ControlWatch>> {
        let rows: Vec<WatchRow> =
            sqlx::query_as(&format!("SELECT {COLS} FROM control_watches WHERE participant_id = ? ORDER BY id"))
                .bind(participant_id)
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter().map(ControlWatch::try_from).collect()
    }

    /// Every watch that asked to wake its controller.
    pub async fn armed_control_watches(&self) -> Result<Vec<ControlWatch>> {
        let rows: Vec<WatchRow> = sqlx::query_as(&format!("SELECT {COLS} FROM control_watches WHERE wake <> 0 ORDER BY id"))
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(ControlWatch::try_from).collect()
    }

    /// The watches of one controller session that hold a pending event — what
    /// a change in that session (a halt declared, the ring coming up) retries.
    pub async fn pending_control_watches_for_session(&self, session_id: &str) -> Result<Vec<ControlWatch>> {
        let rows: Vec<WatchRow> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM control_watches WHERE session_id = ? AND wake <> 0 AND pending_since IS NOT NULL ORDER BY id"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(ControlWatch::try_from).collect()
    }

    pub async fn delete_control_watch(&self, participant_id: i64, request_id: &str) -> Result<bool> {
        Ok(sqlx::query("DELETE FROM control_watches WHERE participant_id = ? AND request_id = ?")
            .bind(participant_id)
            .bind(request_id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            > 0)
    }

    pub async fn delete_control_watch_by_id(&self, id: i64) -> Result<bool> {
        Ok(sqlx::query("DELETE FROM control_watches WHERE id = ?").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// Arm the watch with the first unacknowledged matching event: set once,
    /// never moved while pending (an older pending event stays the one the
    /// wake is for). The event's kind rides along for the boot query.
    pub async fn arm_control_watch(&self, id: i64, event_id: i64, kind: &str) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE control_watches SET pending_since = ?, pending_kind = ?, updated_at = ? \
             WHERE id = ? AND pending_since IS NULL AND cursor < ?",
        )
        .bind(event_id)
        .bind(kind)
        .bind(now_utc())
        .bind(id)
        .bind(event_id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// Drop a pending event the watch's CURRENT filter no longer matches —
    /// a `health: stalled` row a build that matched every health transition
    /// left armed (feedback #144) — so the re-arm can look again from the
    /// cursor. Only an UNCLAIMED pending (`woke_for` not this event): a claim
    /// is a wake in flight and settles through its own path. The cursor is
    /// untouched: it moves on the controller's explicit `ack` and nothing else.
    pub async fn disarm_control_watch(&self, id: i64, pending_since: i64) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE control_watches SET pending_since = NULL, pending_kind = NULL, wake_attempt_at = NULL, updated_at = ? \
             WHERE id = ? AND pending_since = ? AND (woke_for IS NULL OR woke_for <> ?)",
        )
        .bind(now_utc())
        .bind(id)
        .bind(pending_since)
        .bind(pending_since)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// Every wake-enabled watch, for the periodic retry and the start-up re-arm.
    pub async fn wake_control_watches(&self) -> Result<Vec<ControlWatch>> {
        self.armed_control_watches().await
    }

    /// The wake was decided but is being withheld (a newer halt, a Pause, a
    /// revocation seen at the release boundary): give the claim back so a
    /// later retry may fire for the same pending event. Fenced by the
    /// attempt's lease stamp when the caller has one, so a stale attempt's
    /// outcome cannot undo a successor's claim.
    pub async fn unclaim_control_watch_wake(&self, id: i64, pending_since: i64, attempt: Option<&str>) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE control_watches SET woke_for = NULL, woke_at = NULL, wake_attempt_at = NULL, wake_delivered_at = NULL, updated_at = ? \
             WHERE id = ? AND pending_since = ? AND woke_for = ? AND wake_delivered_at IS NULL \
             AND (? IS NULL OR wake_attempt_at = ?)",
        )
        .bind(now_utc())
        .bind(id)
        .bind(pending_since)
        .bind(pending_since)
        .bind(attempt)
        .bind(attempt)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// Whether the wake's notice was DELIVERED to the controller in a dealt
    /// turn — the ring's own durable receipt (`participant_deliveries`), which
    /// outlives a crash between the deal and the delivery marker and so
    /// dedupes a wake no marker recorded.
    pub async fn wake_notice_delivered(&self, participant_id: i64, notice_message_id: i64) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM participant_deliveries WHERE participant_id = ? AND message_id = ? \
             AND delivered_at IS NOT NULL AND withheld_reason IS NULL)",
        )
        .bind(participant_id)
        .bind(notice_message_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Clear a halt declared by a build before `halt_id` existed, only if the
    /// WHOLE observed slot still stands — a newer halt (always with an id now,
    /// and other fields) never matches (eyes-2 `8c22ab52`).
    pub async fn clear_session_halt_if_legacy(&self, session_id: &str, declared_by: &str, reason: &str, declared_at: &str) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE sessions SET halt_declared_by = NULL, halt_reason = NULL, halt_declared_at = NULL, \
             halt_wake_at = NULL, halt_id = NULL \
             WHERE id = ? AND halt_id IS NULL AND halt_declared_by = ? AND halt_reason = ? AND COALESCE(halt_declared_at, '') = ?",
        )
        .bind(session_id)
        .bind(declared_by)
        .bind(reason)
        .bind(declared_at)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// The notice is written and the ring released: the claim is delivered.
    pub async fn mark_control_watch_delivered(&self, id: i64, pending_since: i64) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE control_watches SET wake_delivered_at = ?, updated_at = ? WHERE id = ? AND woke_for = ? AND wake_delivered_at IS NULL",
        )
        .bind(now_utc())
        .bind(now_utc())
        .bind(id)
        .bind(pending_since)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// One journal row by id — `None` when retention swept it.
    pub async fn control_event_by_id(&self, id: i64) -> Result<Option<super::ControlEvent>> {
        Ok(sqlx::query_as(
            "SELECT id, at, session_id, kind, agent, ref_id, detail, source_session, source_slug FROM control_events WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The message a watch's wake notice was written as, if it was.
    pub async fn wake_notice_message_id(&self, watch: &ControlWatch, pending_since: i64) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT message_id FROM delegated_messages WHERE source_session = ? AND source_participant = ? AND request_id = ?",
        )
        .bind(&watch.session_id)
        .bind(watch.participant_id)
        .bind(format!("watch:{}:{pending_since}", watch.id))
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The explicit acknowledgement: the cursor moves forward only, never past
    /// the journal's end (`max_event_id`), and the pending marker clears only
    /// when the acknowledged range covers it — a newer pending event stays
    /// (eyes-2: ack must not lose an event that arrived after the page).
    /// Returns the watch as it stands.
    pub async fn ack_control_watch(&self, id: i64, up_to: i64, max_event_id: i64) -> Result<Option<ControlWatch>> {
        sqlx::query(
            "UPDATE control_watches SET cursor = MIN(MAX(cursor, ?), ?), \
             pending_kind = CASE WHEN pending_since IS NOT NULL AND pending_since <= MIN(MAX(cursor, ?), ?) THEN NULL ELSE pending_kind END, \
             pending_since = CASE WHEN pending_since IS NOT NULL AND pending_since <= MIN(MAX(cursor, ?), ?) THEN NULL ELSE pending_since END, \
             updated_at = ? WHERE id = ?",
        )
        .bind(up_to)
        .bind(max_event_id)
        .bind(up_to)
        .bind(max_event_id)
        .bind(up_to)
        .bind(max_event_id)
        .bind(now_utc())
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.control_watch_by_id(id).await
    }

    /// The one-shot wake claim: succeeds once per pending event, so two
    /// triggers racing to wake the controller fire one notice. A claim is
    /// INTENT: `mark_control_watch_delivered` says it happened, and an
    /// undelivered claim is resumed, not repeated.
    pub async fn claim_control_watch_wake(&self, id: i64, pending_since: i64) -> Result<bool> {
        let now = now_utc();
        Ok(sqlx::query(
            "UPDATE control_watches SET woke_for = ?, woke_at = ?, wake_attempt_at = ?, wake_delivered_at = NULL, updated_at = ? \
             WHERE id = ? AND pending_since = ? AND (woke_for IS NULL OR woke_for <> ?)",
        )
        .bind(pending_since)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .bind(pending_since)
        .bind(pending_since)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// Take the delivery-attempt lease of an undelivered claim: only when no
    /// attempt holds it or the holder's lease (`WAKE_ATTEMPT_LEASE_SECS`) has
    /// lapsed — so the journal task, the scheduler tick and an API re-arm
    /// racing to resume the same wake make one attempt (eyes-2 `9912471a`).
    pub async fn take_wake_attempt(&self, id: i64, pending_since: i64, lease_cutoff: &str) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE control_watches SET wake_attempt_at = ?, updated_at = ? \
             WHERE id = ? AND pending_since = ? AND woke_for = ? AND wake_delivered_at IS NULL \
             AND (wake_attempt_at IS NULL OR wake_attempt_at < ?)",
        )
        .bind(now_utc())
        .bind(now_utc())
        .bind(id)
        .bind(pending_since)
        .bind(pending_since)
        .bind(lease_cutoff)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// An attempt ended without delivering (the ring did not take the wake):
    /// drop its lease so the next retry need not wait it out; the claim stands.
    pub async fn release_wake_attempt(&self, id: i64, pending_since: i64) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE control_watches SET wake_attempt_at = NULL, updated_at = ? \
             WHERE id = ? AND woke_for = ? AND wake_delivered_at IS NULL",
        )
        .bind(now_utc())
        .bind(id)
        .bind(pending_since)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// The wake's notice and its link, in ONE transaction, so a losing
    /// attempt leaves no orphan notice: the link's primary key refuses a
    /// second notice for the same wake and the notice rolls back with it.
    /// `Ok(None)` is that refusal.
    pub async fn post_wake_notice(&self, session_id: &str, participant_id: i64, request_id: &str, text: &str) -> Result<Option<i64>> {
        use sha2::{Digest, Sha256};
        let text = crate::policy::secret_scan::redact(text);
        let mut tx = self.pool.begin().await?;
        let message_id = sqlx::query(
            "INSERT INTO messages (session_id, participant_id, origin, kind, content, envelope, author, created_at) \
             VALUES (?, NULL, 'system', 'system_notice', ?, NULL, 'user', ?)",
        )
        .bind(session_id)
        .bind(text.as_ref())
        .bind(now_utc())
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
        let linked = sqlx::query(
            "INSERT OR IGNORE INTO delegated_messages (source_session, source_participant, request_id, target_session, content_sha256, message_id) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(participant_id)
        .bind(request_id)
        .bind(session_id)
        .bind(format!("{:x}", Sha256::digest(text.as_bytes())))
        .bind(message_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if linked == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        tx.commit().await?;
        Ok(Some(message_id))
    }

    /// Journal rows after `cursor`, oldest first, for the re-arm scan.
    pub async fn control_events_after(&self, cursor: i64, limit: usize) -> Result<Vec<super::ControlEvent>> {
        Ok(sqlx::query_as(
            "SELECT id, at, session_id, kind, agent, ref_id, detail, source_session, source_slug \
             FROM control_events WHERE id > ? ORDER BY id ASC LIMIT ?",
        )
        .bind(cursor)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn max_control_event_id(&self) -> Result<i64> {
        Ok(sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM control_events")
            .fetch_one(&self.pool)
            .await?
            .unwrap_or(0))
    }

    /// The open sessions holding an enabled participant whose snapshot grants
    /// application control — excluded from "every other session".
    pub async fn controller_sessions_open(&self) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT p.session_id FROM session_participants p JOIN sessions s ON s.id = p.session_id \
             WHERE p.enabled <> 0 AND s.closed_at IS NULL AND s.archived = 0 \
             AND p.capabilities LIKE '%\"control_bot_hq\"%' ORDER BY p.session_id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Open, dormant-or-not sessions whose armed watch holds a pending event
    /// of an attention kind whose wake was not delivered — the controllers a
    /// boot respawns so their wake can fire, as it respawns temporary halts
    /// (EYES: a message-level watch must not start sessions at every
    /// relaunch). Reads the kind kept on the watch, so a pending row retention
    /// swept still counts.
    pub async fn sessions_with_pending_attention_watch(&self) -> Result<Vec<String>> {
        let placeholders = ATTENTION_KINDS.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT DISTINCT w.session_id FROM control_watches w \
             JOIN sessions s ON s.id = w.session_id \
             WHERE w.wake <> 0 AND w.pending_since IS NOT NULL \
             AND (w.woke_for IS NULL OR w.woke_for <> w.pending_since OR w.wake_delivered_at IS NULL) \
             AND s.closed_at IS NULL AND w.pending_kind IN ({placeholders}) ORDER BY w.session_id"
        );
        let mut query = sqlx::query_as::<_, (String,)>(&sql);
        for kind in ATTENTION_KINDS {
            query = query.bind(*kind);
        }
        Ok(query.fetch_all(&self.pool).await?.into_iter().map(|(id,)| id).collect())
    }

    /// Clear the session's halt only if it is still the one observed
    /// (`halt_id`): a wake decided against an older halt must not clear a
    /// newer one (eyes-2, Plan 5c review).
    pub async fn clear_session_halt_if_id(&self, session_id: &str, halt_id: &str) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE sessions SET halt_declared_by = NULL, halt_reason = NULL, halt_declared_at = NULL, \
             halt_wake_at = NULL, halt_id = NULL WHERE id = ? AND halt_id = ?",
        )
        .bind(session_id)
        .bind(halt_id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::NewControlEvent;

    async fn fixture() -> (Storage, i64) {
        let s = Storage::memory().await.unwrap();
        s.create_session("c", "controller", None).await.unwrap();
        s.ensure_session_roster("c", 2).await.unwrap();
        let pid: i64 = sqlx::query_scalar("SELECT id FROM session_participants WHERE session_id = 'c' AND slug = 'hands'")
            .fetch_one(s.pool())
            .await
            .unwrap();
        (s, pid)
    }

    async fn event(s: &Storage, session: &str, kind: &str) -> i64 {
        s.insert_control_event(&NewControlEvent { session_id: session.into(), kind: kind.into(), ..Default::default() }).await.unwrap()
    }

    /// Upsert keeps the cursor on a retry, the one-shot claim fires once per
    /// pending event, and the acknowledgement is monotonic, bounded by the
    /// journal's end and clears the pending marker only when it covers it.
    #[tokio::test]
    async fn a_watch_is_armed_once_claimed_once_and_acknowledged_forward_only() {
        let (s, pid) = fixture().await;
        let e1 = event(&s, "t", "decision_pending").await;
        let w = s.upsert_control_watch("c", pid, "hands", "w1", Some(&["t".to_string()]), None, true, e1).await.unwrap();
        assert_eq!((w.cursor, w.pending_since, w.wake, w.targets.as_deref()), (e1, None, true, Some(&["t".to_string()][..])));
        // A retry with other filters updates them and leaves the cursor.
        let w = s.upsert_control_watch("c", pid, "hands", "w1", None, Some(&["message".to_string()]), false, 999).await.unwrap();
        assert_eq!((w.id, w.cursor, w.targets, w.kinds.as_deref(), w.wake), (w.id, e1, None, Some(&["message".to_string()][..]), false));
        assert_eq!(s.control_watches_for(pid).await.unwrap().len(), 1);
        assert!(s.armed_control_watches().await.unwrap().is_empty(), "wake off");
        let w = s.upsert_control_watch("c", pid, "hands", "w1", None, None, true, 999).await.unwrap();
        assert_eq!(s.armed_control_watches().await.unwrap().len(), 1);
        // Arm: once, and never for an event at or before the cursor.
        assert!(!s.arm_control_watch(w.id, e1, "decision_pending").await.unwrap(), "at the cursor is already acknowledged");
        let e2 = event(&s, "t", "halt_declared").await;
        let e3 = event(&s, "t", "health").await;
        assert!(s.arm_control_watch(w.id, e2, "halt_declared").await.unwrap());
        assert!(!s.arm_control_watch(w.id, e3, "health").await.unwrap(), "an older pending event stays the one");
        assert_eq!(s.control_watch_by_id(w.id).await.unwrap().unwrap().pending_since, Some(e2));
        assert_eq!(s.pending_control_watches_for_session("c").await.unwrap().len(), 1);
        assert!(s.pending_control_watches_for_session("t").await.unwrap().is_empty());
        // Claim: once per pending event.
        assert!(s.claim_control_watch_wake(w.id, e2).await.unwrap());
        assert!(!s.claim_control_watch_wake(w.id, e2).await.unwrap(), "a second trigger does not fire again");
        assert!(!s.claim_control_watch_wake(w.id, e3).await.unwrap(), "not the pending event");
        let w = s.control_watch_by_id(w.id).await.unwrap().unwrap();
        assert_eq!(w.woke_for, Some(e2));
        assert!(w.woke_at.is_some());
        // Ack below the pending event: cursor moves, pending stays.
        let max = s.max_control_event_id().await.unwrap();
        let w = s.ack_control_watch(w.id, e1, max).await.unwrap().unwrap();
        assert_eq!((w.cursor, w.pending_since), (e1, Some(e2)));
        // Ack backwards: no-op on the cursor.
        let w = s.ack_control_watch(w.id, 0, max).await.unwrap().unwrap();
        assert_eq!(w.cursor, e1);
        // Ack covering the pending event clears it; beyond the journal's end is clamped.
        let w = s.ack_control_watch(w.id, 1_000_000, max).await.unwrap().unwrap();
        assert_eq!((w.cursor, w.pending_since), (e3, None));
        // Re-arm after the ack only for a newer event.
        assert!(!s.arm_control_watch(w.id, e3, "health").await.unwrap());
        let e4 = event(&s, "t", "gate_urgent").await;
        assert!(s.arm_control_watch(w.id, e4, "gate_urgent").await.unwrap());
        assert!(s.claim_control_watch_wake(w.id, e4).await.unwrap(), "a new pending event is a new wake");
        assert_eq!(s.control_events_after(e2, 10).await.unwrap().iter().map(|e| e.id).collect::<Vec<_>>(), [e3, e4]);
        assert!(s.delete_control_watch(pid, "w1").await.unwrap());
        assert!(!s.delete_control_watch(pid, "w1").await.unwrap());
        assert!(s.control_watch(pid, "w1").await.unwrap().is_none());
    }

    /// The boot query lists a controller session only for a pending event of
    /// an attention kind that has not fired yet; controller sessions are the
    /// ones whose roster holds the grant; the conditional halt clear keys on
    /// the halt id.
    #[tokio::test]
    async fn boot_and_exclusion_queries_read_the_durable_state() {
        let (s, pid) = fixture().await;
        assert!(s.controller_sessions_open().await.unwrap().is_empty());
        sqlx::query("UPDATE session_participants SET capabilities = '[\"control_bot_hq\"]' WHERE id = ?").bind(pid).execute(s.pool()).await.unwrap();
        assert_eq!(s.controller_sessions_open().await.unwrap(), ["c"]);
        let msg = event(&s, "t", "message").await;
        let w = s.upsert_control_watch("c", pid, "hands", "w1", None, Some(&["message".to_string()]), true, 0).await.unwrap();
        assert!(s.arm_control_watch(w.id, msg, "message").await.unwrap());
        assert!(s.sessions_with_pending_attention_watch().await.unwrap().is_empty(), "a message-level pending event does not start sessions at boot");
        let w2 = s.upsert_control_watch("c", pid, "hands", "w2", None, None, true, 0).await.unwrap();
        let decision = event(&s, "t", "decision_pending").await;
        assert!(s.arm_control_watch(w2.id, decision, "decision_pending").await.unwrap());
        assert_eq!(s.sessions_with_pending_attention_watch().await.unwrap(), ["c"]);
        assert!(s.claim_control_watch_wake(w2.id, decision).await.unwrap());
        assert_eq!(s.sessions_with_pending_attention_watch().await.unwrap(), ["c"], "a claim is intent, not delivery: a crash here still respawns");
        assert!(s.mark_control_watch_delivered(w2.id, decision).await.unwrap());
        assert!(s.sessions_with_pending_attention_watch().await.unwrap().is_empty(), "delivered");
        assert!(!s.mark_control_watch_delivered(w2.id, decision).await.unwrap(), "once");
        // A pending row retention swept still counts, by the kind kept on the watch.
        sqlx::query("DELETE FROM control_events WHERE id = ?").bind(decision).execute(s.pool()).await.unwrap();
        sqlx::query("UPDATE control_watches SET wake_delivered_at = NULL WHERE id = ?").bind(w2.id).execute(s.pool()).await.unwrap();
        assert_eq!(s.sessions_with_pending_attention_watch().await.unwrap(), ["c"]);
        assert!(s.control_event_by_id(decision).await.unwrap().is_none());
        // Handing a claim back: only an undelivered one, only for its event.
        assert!(s.unclaim_control_watch_wake(w2.id, decision, None).await.unwrap());
        assert_eq!(s.control_watch_by_id(w2.id).await.unwrap().unwrap().woke_for, None);
        assert!(!s.unclaim_control_watch_wake(w2.id, decision, None).await.unwrap());
        // The conditional halt clear.
        let first = s.declare_session_halt_stamped("c", "hands", "one").await.unwrap().unwrap();
        let second = s.declare_session_halt_stamped("c", "hands", "two").await.unwrap().unwrap();
        assert!(!s.clear_session_halt_if_id("c", &first).await.unwrap(), "an older halt's id clears nothing");
        assert!(s.session_halt("c").await.unwrap().is_some());
        assert!(s.clear_session_halt_if_id("c", &second).await.unwrap());
        assert!(s.session_halt("c").await.unwrap().is_none());
        assert!(ATTENTION_KINDS.iter().all(|k| WATCHABLE_KINDS.contains(k)));
    }
}
