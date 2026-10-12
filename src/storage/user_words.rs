//! `session_user_words` table (migration 0092): the user's OWN words in a
//! session, recorded by the caller that knows whose words a user-origin row
//! carries, and mirrored to the per-session file the `compact-user-words`
//! hook prints after a compaction (`agents::handoff`).
//!
//! A user-origin row is not always the user speaking — the resume notice,
//! the close-out prompt, a plugin's send and an approved gate's output all
//! post as origin `user` (`post_to_channel_verbatim`'s doc) — so nothing here
//! re-derives it from the text: `record_user_words` is called by
//! `core::broadcast::broadcast_user_message` for a message the caller marked
//! as the user's (`Authored::ByUser`) and by the tray's `deliver_oob` with the
//! answer's summary.
//!
//! Known limit: the mirror reads the table and then writes the file, so two
//! records landing at the same instant (a typed Send and a tray click) can
//! leave the file one entry behind until the next record or spawn re-renders
//! it. The table is the fact; the file is a best-effort copy of it.

use super::*;
use crate::agents::handoff::{self, UserWordsEntry};

impl Storage {
    /// Record `text` as the user's own words, tied to the channel row
    /// `message_id`, and re-render the session's user-words file. The row
    /// insert is the fact; the file mirror is best-effort (logged, never
    /// returned), as for the handoff file.
    pub async fn record_user_words(&self, session_id: &str, message_id: i64, text: &str) -> Result<i64> {
        let now = now_utc();
        let row: (i64,) = sqlx::query_as(
            "INSERT INTO session_user_words (session_id, message_id, created_at, text) \
             VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(session_id)
        .bind(message_id)
        .bind(&now)
        .bind(text)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("insert session_user_words session={session_id} message={message_id}"))?;
        self.sync_user_words_file(session_id).await;
        Ok(row.0)
    }

    /// The user's words of a session, OLDEST first, as many of the newest as
    /// fit `budget` bytes of text — the file budget, so a long session's early
    /// words fall off the file first.
    pub async fn user_words(&self, session_id: &str, budget: usize) -> Result<Vec<UserWordsEntry>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT created_at, text FROM session_user_words WHERE session_id = ? ORDER BY id DESC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("read session_user_words session={session_id}"))?;
        let mut used = 0usize;
        let mut kept: Vec<UserWordsEntry> = Vec::new();
        for (at, text) in rows {
            let size = at.len() + text.len() + 4;
            if !kept.is_empty() && used + size > budget {
                break;
            }
            used += size;
            kept.push(UserWordsEntry { at, text });
        }
        kept.reverse();
        Ok(kept)
    }

    /// Re-render the session's user-words file (`agents::handoff::USER_WORDS_FILE`)
    /// from the table as it now is. Called by [`Self::record_user_words`] and
    /// at spawn beside `sync_handoff_file`, so the file exists before the
    /// first compaction and again after a relaunch. A no-op for a storage with
    /// no handoff directory; best-effort by design.
    pub async fn sync_user_words_file(&self, session_id: &str) {
        let Some(dir) = self.handoff_dir.as_deref() else { return };
        let Some(path) = handoff::user_words_path(dir, session_id) else { return };
        let entries = match self.user_words(session_id, handoff::USER_WORDS_FILE_BUDGET_BYTES).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(?e, session_id, "reading the user's words to mirror them");
                return;
            }
        };
        if let Err(e) = handoff::write_file(&path, &handoff::render_user_words_file(&entries)) {
            tracing::warn!(?e, path = %path.display(), "writing a user-words file");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_users_words_are_recorded_oldest_first_and_bounded_newest_kept() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "test", None).await.unwrap();
        s.create_session("s2", "other", None).await.unwrap();
        s.record_user_words("s1", 11, "first").await.unwrap();
        s.record_user_words("s1", 12, "second").await.unwrap();
        s.record_user_words("s2", 13, "elsewhere").await.unwrap();
        let words = s.user_words("s1", 64 * 1024).await.unwrap();
        assert_eq!(words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["first", "second"]);
        assert!(words[0].at <= words[1].at && words[0].at.ends_with('Z'), "{words:?}");
        // The budget keeps the NEWEST: a budget for one entry drops "first".
        let one = s.user_words("s1", 40).await.unwrap();
        assert_eq!(one.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["second"]);
        assert!(s.user_words("s3", 1_000).await.unwrap().is_empty());
    }

    /// The wire for the mirror: a record re-renders the file the hook prints,
    /// and a storage without a handoff dir writes nothing.
    #[tokio::test]
    async fn every_record_re_renders_the_sessions_user_words_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = handoff::dir_under(tmp.path());
        let s = Storage::memory().await.unwrap().with_handoff_dir(dir.clone());
        s.create_session("s1", "test", None).await.unwrap();
        let path = handoff::user_words_path(&dir, "s1").unwrap();
        assert!(!path.exists());
        s.record_user_words("s1", 1, "keep gating reads").await.unwrap();
        let entries = handoff::parse_user_words_file(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "keep gating reads");
        s.record_user_words("s1", 2, "and push").await.unwrap();
        let entries = handoff::parse_user_words_file(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["keep gating reads", "and push"]);
        // Removing the session's handoff files takes the words file with them.
        s.remove_handoff_files("s1");
        assert!(!path.exists());
        // A spawn-time sync recreates it from the table.
        s.sync_user_words_file("s1").await;
        assert_eq!(handoff::parse_user_words_file(&std::fs::read_to_string(&path).unwrap()).len(), 2);
        let bare = Storage::memory().await.unwrap();
        bare.create_session("s1", "test", None).await.unwrap();
        bare.record_user_words("s1", 1, "x").await.unwrap();
        bare.sync_user_words_file("s1").await;
    }
}
