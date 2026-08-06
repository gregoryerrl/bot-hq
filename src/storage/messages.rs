//! `messages` table: append-only per-session turn log.

use super::*;

/// Column list for a `Message` row — shared across the `messages_for_session`
/// query branches so the projection can't drift between them.
const MESSAGE_COLUMNS: &str = "id, session_id, author, kind, content, created_at";

impl Storage {
    /// The legacy write path — the `Author` + `MessageKind` shape ~30 call
    /// sites still speak — expressed as a thin wrapper over
    /// [`Storage::post_to_channel`]. It owns no SQL of its own, so there is
    /// exactly ONE insert into `messages` and exactly one place a
    /// [`PersistedMessage`] can be minted.
    ///
    /// **Why the two paths converged (B5 Task 1b).** They were coherent while
    /// this recorded an agent's *output* and delivery was a separate act: an
    /// output row needed no receipt. B5 collapses that — an agent's output row
    /// IS the row its peer reads through its cursor, one row, and delivery is
    /// receipt-gated. A second receipt-less insert would have forced the send
    /// path to either write a *second* row for one logical message (two rows in
    /// the channel for one utterance) or re-read the row to synthesise a
    /// receipt — a SELECT per chunk, and forgery-by-reconstruction back on the
    /// table.
    ///
    /// The `Author` → `(origin, participant_slug)` map is total and lossless:
    /// `Author` has no `system` variant, and 0044 seeded participants on
    /// exactly the identity `slug == author`, so the slug IS the legacy author
    /// string rather than a translation of it. `author`, `origin`,
    /// `participant_id` and the RFC3339-Z timestamp are all written exactly as
    /// this method wrote them before — `every_legacy_message_query_still_works_after_0044`
    /// and `the_legacy_write_path_now_populates_the_new_columns` are the pins.
    pub async fn insert_message(
        &self,
        session_id: &str,
        author: Author,
        kind: MessageKind,
        content: &str,
    ) -> Result<PersistedMessage> {
        let (origin, slug) = match author {
            Author::User => ("user", None),
            participant => ("participant", Some(participant.as_str())),
        };
        self.post_to_channel(session_id, origin, slug, kind.as_str(), content, None)
            .await
    }

    /// [`Storage::insert_message`] for the call sites that genuinely only want
    /// the row id: the `notify_message_persisted` emitters (phase advance, the
    /// idle-watchdog notice, the tray's out-of-band answer and phase request)
    /// and the user broadcast, which returns an id to its own callers. They
    /// persist a row and wire their text by a separate route, so a receipt
    /// there would be carried past its purpose. Everything on the path B5 makes
    /// receipt-gated — the duo pump's per-chunk writes — calls
    /// `insert_message` and keeps the receipt.
    pub async fn insert_message_id(
        &self,
        session_id: &str,
        author: Author,
        kind: MessageKind,
        content: &str,
    ) -> Result<i64> {
        Ok(self
            .insert_message(session_id, author, kind, content)
            .await?
            .message_id())
    }

    /// True if `author` posted any message in `session_id` with `created_at`
    /// strictly after `since` (an RFC3339-Z timestamp). Powers the findings
    /// re-raise turn-evidence guard: EYES only escalates a re-raise once HANDS
    /// has had a turn since the finding's last raise, so buffer/turn latency
    /// can't false-escalate.
    pub async fn has_message_from_author_since(
        &self,
        session_id: &str,
        author: &str,
        since: &str,
    ) -> Result<bool> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM messages \
             WHERE session_id = ? AND author = ? AND created_at > ?)",
        )
        .bind(session_id)
        .bind(author)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("checking {author} messages since {since}"))?;
        Ok(exists != 0)
    }

    /// Count of user-authored TEXT rows for a session. Seeds the in-memory
    /// `SessionHandle.user_broadcasts` counter at spawn, so an app restart
    /// mid-task doesn't disarm the idle-unflagged watchdog until the next
    /// typed message (found by the d61d277 live smoke). Text-only on purpose:
    /// `phase_change` / `system_notice` rows are persisted as synthetic
    /// `author=user` and are host artifacts, not user engagement.
    pub async fn count_user_messages(&self, session_id: &str) -> Result<u64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM messages \
             WHERE session_id = ? AND author = 'user' AND kind = 'text'",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("counting user messages for {session_id}"))?;
        Ok(n as u64)
    }

    /// All messages for the session, oldest first.
    /// If `since_id` is provided, returns only messages with id > since_id.
    pub async fn messages_for_session(
        &self,
        session_id: &str,
        since_id: Option<i64>,
    ) -> Result<Vec<Message>> {
        let rows = match since_id {
            Some(sid) => {
                sqlx::query_as::<_, Message>(&format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages \
                     WHERE session_id = ? AND id > ? ORDER BY id ASC"
                ))
                .bind(session_id)
                .bind(sid)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, Message>(&format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages \
                     WHERE session_id = ? ORDER BY id ASC"
                ))
                .bind(session_id)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Author, MessageKind, Storage};

    #[tokio::test]
    async fn count_user_messages_counts_text_only_and_per_session() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s-1", "S", None).await.unwrap();
        s.create_session("s-2", "Other", None).await.unwrap();

        // Two real user prompts (incl. an OOB answer, stored the same way)…
        s.insert_message("s-1", Author::User, MessageKind::Text, "task")
            .await
            .unwrap();
        s.insert_message("s-1", Author::User, MessageKind::Text, "oob answer")
            .await
            .unwrap();
        // …plus synthetic author=user host rows that must NOT count…
        s.insert_message("s-1", Author::User, MessageKind::PhaseChange, "Plan")
            .await
            .unwrap();
        s.insert_message("s-1", Author::User, MessageKind::SystemNotice, "nudged")
            .await
            .unwrap();
        // …plus agent text and another session's user text (both excluded).
        s.insert_message("s-1", Author::Brian, MessageKind::Text, "ack")
            .await
            .unwrap();
        s.insert_message("s-2", Author::User, MessageKind::Text, "elsewhere")
            .await
            .unwrap();

        assert_eq!(s.count_user_messages("s-1").await.unwrap(), 2);
        assert_eq!(s.count_user_messages("s-2").await.unwrap(), 1);
        assert_eq!(s.count_user_messages("s-none").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn insert_message_still_writes_an_rfc3339_z_timestamp() {
        // Delegating to `post_to_channel` moved where `created_at` comes from,
        // and that method wrote SQLite's `datetime('now')` — a ZONE-LESS
        // `2026-08-06 11:22:33`. The regression would have been silent in both
        // directions: the frontend parses a zone-less string as LOCAL time (the
        // staleness hallucination `storage::time` exists to prevent), and
        // `has_message_from_author_since` is a STRING compare against an
        // RFC3339-Z bound, so it would have gone permanently false for a
        // same-day bound and quietly disarmed the findings re-raise
        // turn-evidence guard.
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "S", None).await.unwrap();
        s.insert_message("s1", Author::Brian, MessageKind::Text, "work")
            .await
            .unwrap();

        let rows = s.messages_for_session("s1", None).await.unwrap();
        let ts = rows[0].created_at.clone();
        assert!(ts.contains('T') && ts.ends_with('Z'), "expected RFC3339-Z, got {ts}");
        chrono::DateTime::parse_from_rfc3339(&ts).expect("created_at must parse as RFC3339");

        // The comparison that actually breaks, derived from the row itself so
        // it needs no wall clock: midnight of the row's own day. ' ' (0x20)
        // sorts BEFORE 'T' (0x54), so a zone-less `created_at` reads as EARLIER
        // than midnight of the same day and this goes false.
        let midnight = format!("{}T00:00:00.000Z", &ts[..10]);
        assert!(
            s.has_message_from_author_since("s1", "brian", &midnight).await.unwrap(),
            "a row written today must read as after today's midnight ({ts} vs {midnight})"
        );
    }

    #[tokio::test]
    async fn insert_message_returns_a_receipt_for_the_row_it_wrote() {
        // The point of the convergence: the duo pump's per-chunk write is now
        // receipt-bearing, so B5's receipt-gated send needs neither a second
        // row nor a re-read of the row to obtain one. Asserted against the
        // PERSISTED row rather than the arguments — a receipt that can diverge
        // from its row is exactly the lie the type exists to rule out.
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "S", None).await.unwrap();
        let pm = s
            .insert_message("s1", Author::Brian, MessageKind::Text, "work")
            .await
            .unwrap();

        let rows = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(pm.message_id(), rows[0].id);
        assert_eq!(pm.session_id(), rows[0].session_id);
        assert_eq!(pm.body(), rows[0].content);
        assert_eq!(pm.envelope(), None, "the legacy shape carries no envelope");

        // And the id-only shim is the same write, just discarding the receipt.
        let id = s
            .insert_message_id("s1", Author::User, MessageKind::Text, "reply")
            .await
            .unwrap();
        let rows = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(rows[1].id, id);
        assert_eq!(rows[1].author, "user");
    }
}
