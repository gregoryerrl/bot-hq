//! `messages` table: append-only per-session turn log.

use super::*;

/// Column list for a `Message` row — shared across the `messages_for_session`
/// query branches so the projection can't drift between them.
const MESSAGE_COLUMNS: &str = "id, session_id, author, kind, content, created_at";

impl Storage {
    pub async fn insert_message(
        &self,
        session_id: &str,
        author: Author,
        kind: MessageKind,
        content: &str,
    ) -> Result<i64> {
        // Writes the legacy `author` AND the session-focused `participant_id` /
        // `origin` (migration 0044). Dual-write on purpose: `author` keeps every
        // unmigrated reader working, while the new columns are correct from the
        // moment 0044 applies — so the channel needs no backfill later.
        //
        // The participant is resolved INLINE by subquery rather than with a
        // prior SELECT: this runs on every text/tool_use/tool_result chunk, and
        // an extra round trip per chunk is a cost worth not paying. It resolves
        // to NULL when the session has no matching participant, which is
        // correct rather than an error — `author` still carries the
        // attribution. `ensure_session_roster` seeds the roster pre-spawn and
        // repairs anything written before it, so a NULL here means a row from
        // the window between 0044 and that fix, not a steady state.
        let res = sqlx::query(
            "INSERT INTO messages \
             (session_id, author, kind, content, created_at, participant_id, origin) \
             VALUES (?1, ?2, ?3, ?4, ?5, \
                     (SELECT id FROM session_participants \
                      WHERE session_id = ?1 AND slug = ?2), \
                     CASE WHEN ?2 = 'user' THEN 'user' ELSE 'participant' END)",
        )
        .bind(session_id)
        .bind(author.as_str())
        .bind(kind.as_str())
        .bind(content)
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .with_context(|| format!("inserting message into session {session_id}"))?;
        Ok(res.last_insert_rowid())
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
}
