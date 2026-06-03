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
        let res = sqlx::query(
            "INSERT INTO messages (session_id, author, kind, content, created_at) \
             VALUES (?, ?, ?, ?, ?)",
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
