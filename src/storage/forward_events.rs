//! `forward_events` queries — one row per DISCARDED peer forward.
//!
//! See the migration for why only drops are recorded. A row here always means a
//! message was lost and names the reason; there is no "delivered" row to filter
//! out.

use super::{time::now_utc, Storage};
use crate::storage::row_types::ForwardEvent;
use anyhow::{Context, Result};

/// Longest preview stored. Enough to recognise which turn was lost without
/// duplicating whole agent turns into a diagnostics table.
const PREVIEW_MAX: usize = 240;

/// Truncate on a CHARACTER boundary — agent turns routinely contain multi-byte
/// characters (arrows, em dashes, emoji), and slicing bytes would panic.
fn preview(body: &str) -> String {
    let one_line = body.replace('\n', " ");
    if one_line.chars().count() <= PREVIEW_MAX {
        one_line
    } else {
        one_line.chars().take(PREVIEW_MAX).collect::<String>() + "…"
    }
}

impl Storage {
    /// Record a dropped forward. Best-effort by contract: the caller logs and
    /// continues, because losing telemetry must never break message routing.
    pub async fn insert_forward_drop(
        &self,
        session_id: &str,
        from_agent: &str,
        to_agent: &str,
        reason: &str,
        body: &str,
    ) -> Result<i64> {
        let res = sqlx::query(
            "INSERT INTO forward_events \
             (session_id, occurred_at, from_agent, to_agent, reason, body_len, body_preview) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(now_utc())
        .bind(from_agent)
        .bind(to_agent)
        .bind(reason)
        .bind(body.len() as i64)
        .bind(preview(body))
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording dropped forward for {session_id}"))?;
        Ok(res.last_insert_rowid())
    }

    /// Dropped forwards newest-first. `session_id = None` spans every session —
    /// the cross-session view is what shows a systemic breaker misfiring.
    pub async fn list_forward_drops(&self, session_id: Option<&str>) -> Result<Vec<ForwardEvent>> {
        const COLS: &str = "id, session_id, occurred_at, from_agent, to_agent, reason, \
                            body_len, body_preview";
        let rows = match session_id {
            Some(sid) => {
                sqlx::query_as::<_, ForwardEvent>(&format!(
                    "SELECT {COLS} FROM forward_events WHERE session_id = ? ORDER BY id DESC"
                ))
                .bind(sid)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as::<_, ForwardEvent>(&format!(
                    "SELECT {COLS} FROM forward_events ORDER BY id DESC"
                ))
                .fetch_all(&self.pool)
                .await
            }
        }
        .context("listing dropped forwards")?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_drop_row_names_the_message_and_the_reason() {
        let s = Storage::memory().await.unwrap();
        s.insert_forward_drop(
            "s1",
            "brian",
            "rain",
            "convergence",
            "here is the plan you should review",
        )
        .await
        .unwrap();

        let rows = s.list_forward_drops(Some("s1")).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].reason, "convergence");
        assert_eq!(rows[0].from_agent, "brian");
        assert_eq!(rows[0].to_agent, "rain");
        assert_eq!(rows[0].body_len, 34);
        assert!(rows[0].body_preview.contains("plan you should review"));
    }

    #[tokio::test]
    async fn preview_truncates_on_a_character_boundary() {
        // Agent turns are full of multi-byte characters; a byte slice would
        // panic. Build a body that is long in chars AND multi-byte throughout.
        let body = "→ em—dash and é ".repeat(60);
        assert!(body.chars().count() > PREVIEW_MAX);
        let p = preview(&body);
        assert_eq!(p.chars().count(), PREVIEW_MAX + 1, "truncated plus the ellipsis");

        let s = Storage::memory().await.unwrap();
        s.insert_forward_drop("s1", "rain", "brian", "hard_cap", &body)
            .await
            .unwrap();
        let rows = s.list_forward_drops(None).await.unwrap();
        // body_len is the FULL length, so a short preview is never mistaken for
        // a short message.
        assert_eq!(rows[0].body_len, body.len() as i64);
        assert!(rows[0].body_preview.ends_with('…'));
    }

    #[tokio::test]
    async fn newlines_are_flattened_so_a_preview_stays_one_line() {
        let s = Storage::memory().await.unwrap();
        s.insert_forward_drop("s1", "brian", "rain", "no_peer", "line one\nline two")
            .await
            .unwrap();
        let rows = s.list_forward_drops(None).await.unwrap();
        assert_eq!(rows[0].body_preview, "line one line two");
    }

    #[tokio::test]
    async fn listing_scopes_by_session_and_is_newest_first() {
        let s = Storage::memory().await.unwrap();
        s.insert_forward_drop("s1", "brian", "rain", "hard_cap", "first")
            .await
            .unwrap();
        s.insert_forward_drop("s2", "rain", "brian", "convergence", "other session")
            .await
            .unwrap();
        s.insert_forward_drop("s1", "rain", "brian", "convergence", "second")
            .await
            .unwrap();

        assert_eq!(s.list_forward_drops(None).await.unwrap().len(), 3);
        let s1 = s.list_forward_drops(Some("s1")).await.unwrap();
        assert_eq!(s1.len(), 2);
        assert_eq!(s1[0].body_preview, "second", "newest first");
    }

    #[tokio::test]
    async fn no_drops_is_empty_not_an_error() {
        let s = Storage::memory().await.unwrap();
        assert!(s.list_forward_drops(None).await.unwrap().is_empty());
    }
}
