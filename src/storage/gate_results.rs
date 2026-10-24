//! Gate answers/results not yet delivered through the turn ring (#112).
//! Handoff timestamps do not establish receipt of an out-of-band result.

use super::*;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UnreadGateResult {
    pub participant_id: i64,
    pub choice_id: String,
    pub message_id: i64,
    pub at: String,
    pub excerpt: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::handoff;

    async fn fixture(root: &Path) -> (Storage, i64, i64) {
        let s = Storage::memory().await.unwrap().with_handoff_dir(handoff::dir_under(&root.join(".local")));
        s.create_session("s1", "gates", None).await.unwrap();
        s.ensure_session_roster("s1", 2).await.unwrap();
        let hands = s.participant_by_slug("s1", "hands").await.unwrap().unwrap().id;
        let eyes = s.participant_by_slug("s1", "eyes").await.unwrap().unwrap().id;
        (s, hands, eyes)
    }

    async fn outcome(s: &Storage, gate: &str, text: &str) -> i64 {
        s.insert_tray_entry("s1", gate, "hands", QuestionKind::Approval, "Run fixture?",
            Some(&["Approve".into(), "Reject".into()]), None, Some("printf fixture")).await.unwrap();
        s.answer_tray_entry(gate, "Approve").await.unwrap();
        let receipt = s.post_to_channel("s1", "system", None, "system_notice", text, None).await.unwrap();
        s.set_tray_run_result(gate, Some(receipt.message_id()), Some(0), 1).await.unwrap();
        receipt.message_id()
    }

    #[tokio::test]
    async fn a_newer_handoff_cannot_hide_an_undelivered_gate_result() {
        let root = tempfile::tempdir().unwrap();
        let (s, hands, eyes) = fixture(root.path()).await;
        let row = outcome(&s, "g1", "REFRESH_FAILED_DESPITE_EXIT_ZERO").await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        s.upsert_session_document("s1", "handoff-hands", "g1 is still pending (stale)", None).await.unwrap();
        let restored = crate::policy::hooks::compact_gate_results_text(root.path(), Some("s1"), "hands").unwrap();
        assert!(restored.contains("REFRESH_FAILED_DESPITE_EXIT_ZERO"));
        assert!(restored.contains("gate_status(\"g1\")"));
        let (a, b) = tokio::join!(s.take_gate_result_notices("s1", "hands"), s.take_gate_result_notices("s1", "hands"));
        assert_eq!(a.unwrap().len() + b.unwrap().len(), 1, "one notice, even with concurrent tools");
        assert!(s.take_gate_result_notices("s1", "eyes").await.unwrap().is_empty(), "mid-turn warnings belong to the issuer");
        assert_eq!(s.cursor_for(hands).await.unwrap(), 0, "a notice is not delivery");
        s.upsert_session_document("s1", "handoff-hands", "handoff rewritten again", None).await.unwrap();
        assert!(crate::policy::hooks::compact_gate_results_text(root.path(), Some("s1"), "hands").is_some());
        s.commit_delivery(hands, &[(row, None)]).await.unwrap();
        assert!(crate::policy::hooks::compact_gate_results_text(root.path(), Some("s1"), "hands").is_none());
        sqlx::query("DELETE FROM participant_deliveries WHERE participant_id = ?").bind(hands).execute(s.pool()).await.unwrap();
        s.sync_gate_results_file("s1", "hands").await;
        assert!(crate::policy::hooks::compact_gate_results_text(root.path(), Some("s1"), "hands").is_none(), "telemetry retention must not resurrect a received gate result");
        assert!(crate::policy::hooks::compact_gate_results_text(root.path(), Some("s1"), "eyes").is_some(), "the peer has its own receipt");
        s.commit_delivery(eyes, &[(row, Some("withheld fixture"))]).await.unwrap();
        assert_eq!(s.cursor_for(eyes).await.unwrap(), row);
        assert!(crate::policy::hooks::compact_gate_results_text(root.path(), Some("s1"), "eyes").is_some(), "a cursor passing a withheld row is not receipt");
        assert!(s.take_gate_result_notices("other-session", "hands").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_late_link_and_a_later_run_receive_independent_notices() {
        let root = tempfile::tempdir().unwrap();
        let (s, hands, _) = fixture(root.path()).await;
        s.insert_tray_entry("s1", "late", "hands", QuestionKind::Approval, "late", None, None, None).await.unwrap();
        let older = s.post_to_channel("s1", "system", None, "system_notice", "late linked", None).await.unwrap();
        outcome(&s, "newer", "newer linked first").await;
        assert_eq!(s.take_gate_result_notices("s1", "hands").await.unwrap().len(), 1);
        s.link_gate_resolution("late", older.message_id()).await.unwrap();
        assert_eq!(s.take_gate_result_notices("s1", "hands").await.unwrap()[0].choice_id, "late");
        let finished = s.post_to_channel("s1", "system", None, "system_notice", "scheduled run completed", None).await.unwrap();
        s.set_tray_run_result("late", Some(finished.message_id()), Some(0), 12).await.unwrap();
        assert_eq!(s.take_gate_result_notices("s1", "hands").await.unwrap()[0].message_id, finished.message_id());
        let restored = crate::policy::hooks::compact_gate_results_text(root.path(), Some("s1"), "hands").unwrap();
        assert!(!restored.contains("late linked"));
        assert!(restored.contains("scheduled run completed"));
        s.insert_tray_entry("s1", "already-read", "hands", QuestionKind::Approval, "late link", None, None, None).await.unwrap();
        let read = s.post_to_channel("s1", "system", None, "system_notice", "ALREADY_RECEIVED_BEFORE_LINK", None).await.unwrap();
        s.commit_delivery(hands, &[(read.message_id(), None)]).await.unwrap();
        s.link_gate_resolution("already-read", read.message_id()).await.unwrap();
        sqlx::query("DELETE FROM participant_deliveries WHERE participant_id = ?").bind(hands).execute(s.pool()).await.unwrap();
        s.sync_gate_results_file("s1", "hands").await;
        let restored = crate::policy::hooks::compact_gate_results_text(root.path(), Some("s1"), "hands").unwrap();
        assert!(!restored.contains("ALREADY_RECEIVED_BEFORE_LINK"));
    }

    #[tokio::test]
    async fn gate_result_replay_and_notices_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let (s, _, _) = fixture(root.path()).await;
        for n in 0..12 { outcome(&s, &format!("g{n}"), &"🌎\n\"\\".repeat(2000)).await; }
        let text = crate::policy::hooks::compact_gate_results_text(root.path(), Some("s1"), "hands").unwrap();
        assert!(text.len() <= handoff::GATE_RESULTS_BUDGET_BYTES);
        assert!(text.contains("omitted"));
        assert_eq!(s.take_gate_result_notices("s1", "hands").await.unwrap().len(), 8);
        assert_eq!(s.take_gate_result_notices("s1", "hands").await.unwrap().len(), 4);
        assert!(s.take_gate_result_notices("s1", "hands").await.unwrap().is_empty());
    }
}

impl Storage {
    async fn unread_gate_rows(&self, session_id: &str, agent: &str, notices_only: bool) -> Result<Vec<UnreadGateResult>> {
        // The cursor may pass a withheld row. Only an actual delivery receipt
        // (`delivered_at`), not a high watermark, establishes channel receipt.
        sqlx::query_as::<_, UnreadGateResult>(
            "SELECT p.id AS participant_id, t.choice_id, m.id AS message_id, \
                    m.created_at AS at, substr(m.content, -1200) AS excerpt \
             FROM session_participants p JOIN sessions s ON s.id = p.session_id \
             JOIN session_tray t ON t.session_id = p.session_id \
             JOIN messages m ON m.id = t.resolution_row_id AND m.session_id = t.session_id \
             WHERE p.session_id = ?1 AND p.slug = ?2 AND p.enabled = 1 AND s.closed_at IS NULL \
               AND NOT EXISTS (SELECT 1 FROM participant_deliveries d \
                   WHERE d.participant_id = p.id AND d.message_id = m.id AND d.delivered_at IS NOT NULL) \
               AND NOT EXISTS (SELECT 1 FROM gate_result_notices n \
                   WHERE n.participant_id = p.id AND n.message_id = m.id AND n.delivered_at IS NOT NULL) \
               AND (?3 = 0 OR (t.agent = p.slug AND NOT EXISTS (SELECT 1 FROM gate_result_notices n \
                   WHERE n.participant_id = p.id AND n.message_id = m.id))) \
             ORDER BY m.id DESC LIMIT 9"
        ).bind(session_id).bind(agent).bind(notices_only).fetch_all(self.pool()).await
            .context("reading undelivered gate results")
    }

    /// Claim at most eight new issuer notices. Concurrent tool replies cannot
    /// both claim one result; this never changes the channel delivery ledger.
    pub async fn take_gate_result_notices(&self, session_id: &str, agent: &str) -> Result<Vec<UnreadGateResult>> {
        let rows = self.unread_gate_rows(session_id, agent, true).await?;
        let mut claimed = Vec::new();
        for row in rows.into_iter().take(8) {
            let inserted = sqlx::query("INSERT OR IGNORE INTO gate_result_notices (participant_id, message_id, noticed_at) VALUES (?, ?, ?)")
                .bind(row.participant_id).bind(row.message_id).bind(now_utc())
                .execute(self.pool()).await;
            match inserted {
                Ok(result) if result.rows_affected() == 1 => claimed.push(row),
                Ok(_) => {},
                Err(error) => tracing::warn!(%error, "could not record a gate-result tool notice"),
            }
        }
        Ok(claimed)
    }

    /// Link a gate's first answer. A later actual run replaces this receipt
    /// through set_tray_run_result (e.g. approval of a scheduled command).
    pub async fn link_gate_resolution(&self, choice_id: &str, message_id: i64) -> Result<()> {
        let changed = sqlx::query("UPDATE session_tray SET resolution_row_id = ? WHERE choice_id = ? AND resolution_row_id IS NULL")
            .bind(message_id).bind(choice_id).execute(self.pool()).await
            .context("linking a gate's resolution receipt")?;
        if changed.rows_affected() > 0 { self.sync_gate_result_readers(choice_id).await; }
        Ok(())
    }

    pub(super) async fn sync_gate_result_readers(&self, choice_id: &str) {
        // A result may be linked after its message was already delivered.
        // Capture that proof before the ordinary delivery telemetry expires.
        if let Err(error) = sqlx::query(
            "INSERT INTO gate_result_notices (participant_id, message_id, delivered_at) \
             SELECT d.participant_id, d.message_id, d.delivered_at FROM session_tray t \
             JOIN participant_deliveries d ON d.message_id = t.resolution_row_id \
             JOIN session_participants p ON p.id = d.participant_id AND p.session_id = t.session_id \
             JOIN messages m ON m.id = d.message_id AND m.session_id = t.session_id \
             WHERE t.choice_id = ? AND d.delivered_at IS NOT NULL \
             ON CONFLICT(participant_id, message_id) DO UPDATE SET \
                 delivered_at = COALESCE(gate_result_notices.delivered_at, excluded.delivered_at)"
        ).bind(choice_id).execute(self.pool()).await {
            tracing::warn!(%error, "could not retain gate delivery proof");
        }
        if self.handoff_dir.is_none() { return; }
        let session: Option<String> = sqlx::query_scalar("SELECT session_id FROM session_tray WHERE choice_id = ?")
            .bind(choice_id).fetch_optional(self.pool()).await.ok().flatten();
        let Some(session) = session else { return };
        let Ok(roster) = self.participants_for_session(&session).await else { return };
        for reader in roster.into_iter().filter(|p| p.enabled) {
            self.sync_gate_results_file(&session, &reader.slug).await;
        }
    }

    pub async fn sync_gate_results_file(&self, session_id: &str, agent: &str) {
        let Some(dir) = self.handoff_dir.as_deref() else { return };
        let Some(path) = crate::agents::handoff::gate_results_path(dir, session_id, agent) else { return };
        // Serialize read + write so a slower earlier refresh cannot overwrite
        // a newer delivery/result. Clones of this Storage share the lock.
        let _guard = self.gate_result_sync.lock().await;
        let rows = match self.unread_gate_rows(session_id, agent, false).await {
            Ok(rows) => rows,
            Err(error) => { tracing::warn!(%error, session_id, agent, "could not mirror unread gate results"); return; }
        };
        if rows.is_empty() {
            match std::fs::remove_file(&path) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => tracing::warn!(%error, "could not clear the gate-results mirror"),
            }
            return;
        }
        let text = crate::agents::handoff::render_gate_results(&rows);
        if let Err(error) = crate::agents::handoff::write_file(&path, &text) {
            tracing::warn!(%error, "could not write the gate-results mirror");
        }
    }

    pub(super) async fn sync_gate_results_after_delivery(&self, participant_id: i64) {
        if self.handoff_dir.is_none() { return; }
        let reader: Option<(String, String)> = sqlx::query_as("SELECT session_id, slug FROM session_participants WHERE id = ?")
            .bind(participant_id).fetch_optional(self.pool()).await.ok().flatten();
        if let Some((session, agent)) = reader { self.sync_gate_results_file(&session, &agent).await; }
    }
}
