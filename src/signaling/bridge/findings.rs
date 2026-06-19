//! `findings` bridge methods — server-side logic for the EYES-sign-off gate.
//! EYES files via `eyes_flag`; HANDS resolves via `disposition_finding`; the
//! gate's prompted-primary read is `check_open_findings`. Thin bridge→storage,
//! mirroring the other tool surfaces. Per-agent access (eyes_flag = EYES-only,
//! disposition_finding = HANDS-only) is enforced in `jsonrpc.rs::call_tool`.

use super::*;
use crate::storage::{Finding, FindingSeverity, FindingStatus};
use uuid::Uuid;

impl SignalingBridge {
    /// Storage handle or a descriptive error (test bridges may have none).
    async fn findings_storage(&self) -> Result<Storage> {
        self.storage
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("findings: storage is not wired into this bridge"))
    }

    /// EYES files a review finding. Returns the generated `finding_uid` the
    /// HANDS agent passes to `disposition_finding`.
    pub async fn eyes_flag(
        &self,
        session_id: String,
        agent: String,
        severity: FindingSeverity,
        summary: String,
        code_ref: Option<String>,
    ) -> Result<String> {
        let storage = self.findings_storage().await?;

        // Re-raise dedup: if an OPEN finding with the same summary already exists,
        // don't insert a duplicate. Bump its raise_count — but ONLY if HANDS has
        // had a turn since it was last raised, so a double-flag before Brian's
        // turn (buffer / turn-boundary latency) can't false-escalate.
        if let Some(existing) = storage
            .latest_open_finding_by_summary(&session_id, &summary)
            .await?
        {
            let brian_acted = storage
                .has_message_from_author_since(&session_id, "brian", &existing.updated_at)
                .await
                .unwrap_or(false);
            if brian_acted {
                storage.increment_raise_count(&existing.finding_uid).await?;
                let _ = self
                    .event_tx
                    .send(SignalingEvent::FindingsChanged { session_id });
            }
            // Either way return the existing finding's id — no duplicate row.
            return Ok(existing.finding_uid);
        }

        let uid = Uuid::new_v4().to_string();
        storage
            .insert_finding(&session_id, &uid, &agent, severity, &summary, code_ref.as_deref())
            .await?;
        // Refresh the per-session findings banner (UI). Best-effort.
        let _ = self
            .event_tx
            .send(SignalingEvent::FindingsChanged { session_id });
        Ok(uid)
    }

    /// EYES (rain) confirms an escalated finding's resolution — clears the
    /// escalation "awaiting EYES confirm" signal (sets `eyes_approved`). NON-
    /// gating: the commit gate is already open once HANDS dispositioned, so this
    /// only closes the soft-escalation loop. Returns a human-readable result.
    pub async fn approve_finding(&self, finding_uid: String) -> Result<String> {
        let storage = self.findings_storage().await?;
        let affected = storage.approve_finding(&finding_uid).await?;
        if affected == 0 {
            return Ok(format!("no-op: finding '{finding_uid}' not found"));
        }
        if let Ok(Some(f)) = storage.get_finding(&finding_uid).await {
            let _ = self
                .event_tx
                .send(SignalingEvent::FindingsChanged {
                    session_id: f.session_id,
                });
        }
        Ok(format!(
            "finding '{finding_uid}' approved by EYES — escalation cleared"
        ))
    }

    /// HANDS dispositions a finding (`fixed` / `rebutted`). Returns a
    /// human-readable result. A `reason` is always supplied by the dispatch
    /// layer (required for both statuses).
    pub async fn disposition_finding(
        &self,
        finding_uid: String,
        status: FindingStatus,
        reason: String,
        disposed_by: String,
    ) -> Result<String> {
        let storage = self.findings_storage().await?;
        let affected = storage
            .disposition_finding(&finding_uid, status, Some(&reason), &disposed_by)
            .await?;
        if affected == 0 {
            return Ok(format!(
                "no-op: finding '{finding_uid}' is not open (unknown id, or already resolved)"
            ));
        }
        // Refresh the banner — the disposed finding stops gating, so the count
        // drops. Look up its session_id from the (still-present) row.
        if let Ok(Some(f)) = storage.get_finding(&finding_uid).await {
            let _ = self
                .event_tx
                .send(SignalingEvent::FindingsChanged { session_id: f.session_id });
        }
        Ok(format!("finding '{finding_uid}' marked {}", status.as_str()))
    }

    /// The gate's read: open blocking findings for the session. Returns `ok`
    /// when clear, else `blocked: <N> unresolved blocking finding(s)` + a list.
    /// Mirrors `check_commit_message`'s `ok` / `forbidden_word: …` contract.
    pub async fn check_open_findings(&self, session_id: &str) -> Result<String> {
        let storage = self.findings_storage().await?;
        let open = storage.open_blocking_findings_for_session(session_id).await?;
        Ok(render_open_findings(&open))
    }

    /// Open-blocking-findings count for the per-turn banner. FAIL-SAFE: returns
    /// 0 when storage isn't wired or the query errors — the banner is salience,
    /// not a gate, so it must never break the message pump.
    pub async fn open_blocking_count(&self, session_id: &str) -> usize {
        let Some(storage) = self.storage.lock().await.clone() else {
            return 0;
        };
        storage
            .count_open_blocking_findings(session_id)
            .await
            .unwrap_or(0) as usize
    }

    /// All findings for a session — backs the `list_session_findings` Tauri
    /// command (the UI banner + a future detail view).
    pub async fn list_findings_for_session(&self, session_id: &str) -> Result<Vec<Finding>> {
        let storage = self.findings_storage().await?;
        storage.findings_for_session(session_id).await
    }
}

/// Format the open-blocking-findings list into the gate's response string.
/// Pure → unit-testable without a bridge.
fn render_open_findings(open: &[Finding]) -> String {
    if open.is_empty() {
        return "ok".to_string();
    }
    let list = open
        .iter()
        .map(|f| {
            let r = f
                .code_ref
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            format!("- [{}] {}{}", f.finding_uid, f.summary, r)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "blocked: {} unresolved blocking finding(s) — resolve each via \
         disposition_finding(finding_id, status, reason):\n{list}",
        open.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    async fn bridge_with_session(sid: &str) -> Arc<SignalingBridge> {
        let bridge = SignalingBridge::new();
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session(sid, "t", None).await.unwrap();
        bridge
    }

    #[tokio::test]
    async fn flag_blocks_then_disposition_clears() {
        let bridge = bridge_with_session("s1").await;
        assert_eq!(bridge.check_open_findings("s1").await.unwrap(), "ok");

        let uid = bridge
            .eyes_flag(
                "s1".into(),
                "rain".into(),
                FindingSeverity::Blocking,
                "NPE: job reads adAccount->id but command aliased it away".into(),
                Some("ReconcileMetaData.php:42".into()),
            )
            .await
            .unwrap();

        let blocked = bridge.check_open_findings("s1").await.unwrap();
        assert!(blocked.starts_with("blocked: 1"), "got: {blocked}");
        assert!(blocked.contains(&uid), "block message lists the uid: {blocked}");

        let res = bridge
            .disposition_finding(uid, FindingStatus::Fixed, "fixed in abc123".into(), "brian".into())
            .await
            .unwrap();
        assert!(res.contains("fixed"), "got: {res}");
        assert_eq!(bridge.check_open_findings("s1").await.unwrap(), "ok");
    }

    #[tokio::test]
    async fn advisory_does_not_block() {
        let bridge = bridge_with_session("s1").await;
        bridge
            .eyes_flag(
                "s1".into(),
                "rain".into(),
                FindingSeverity::Advisory,
                "nit: rename a variable".into(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            bridge.check_open_findings("s1").await.unwrap(),
            "ok",
            "advisory findings never gate"
        );
    }

    #[tokio::test]
    async fn disposition_unknown_uid_is_noop() {
        let bridge = bridge_with_session("s1").await;
        let res = bridge
            .disposition_finding("nope".into(), FindingStatus::Fixed, "x".into(), "brian".into())
            .await
            .unwrap();
        assert!(res.contains("no-op"), "got: {res}");
    }

    #[test]
    fn render_open_findings_empty_is_ok() {
        assert_eq!(render_open_findings(&[]), "ok");
    }

    #[tokio::test]
    async fn reraise_dedups_and_escalates_only_after_brian_turn() {
        let bridge = SignalingBridge::new();
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();

        let uid = bridge
            .eyes_flag("s1".into(), "rain".into(), FindingSeverity::Blocking, "same bug".into(), None)
            .await
            .unwrap();
        // Re-flag with NO Brian turn since → dedups to the same finding, NO bump.
        let uid2 = bridge
            .eyes_flag("s1".into(), "rain".into(), FindingSeverity::Blocking, "same bug".into(), None)
            .await
            .unwrap();
        assert_eq!(uid, uid2, "re-flag dedups to the same finding id");
        assert_eq!(
            storage.get_finding(&uid).await.unwrap().unwrap().raise_count,
            1,
            "no Brian turn → no false escalation"
        );

        // Backdate the finding so the next Brian message is unambiguously "after"
        // (deterministic; no reliance on wall-clock advancing mid-test).
        sqlx::query("UPDATE findings SET updated_at = '2000-01-01T00:00:00.000Z' WHERE finding_uid = ?")
            .bind(&uid)
            .execute(storage.pool())
            .await
            .unwrap();
        storage
            .insert_message(
                "s1",
                crate::storage::Author::Brian,
                crate::storage::MessageKind::Text,
                "looking",
            )
            .await
            .unwrap();
        // Re-flag now that Brian has had a turn → escalates.
        let uid3 = bridge
            .eyes_flag("s1".into(), "rain".into(), FindingSeverity::Blocking, "same bug".into(), None)
            .await
            .unwrap();
        assert_eq!(uid, uid3);
        assert_eq!(
            storage.get_finding(&uid).await.unwrap().unwrap().raise_count,
            2,
            "Brian had a turn → re-raise escalates"
        );
    }

    #[tokio::test]
    async fn approve_finding_sets_eyes_approved() {
        let bridge = SignalingBridge::new();
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        let uid = bridge
            .eyes_flag("s1".into(), "rain".into(), FindingSeverity::Blocking, "bug".into(), None)
            .await
            .unwrap();
        // HANDS fixes (gate clears); escalation still awaits EYES confirm.
        bridge
            .disposition_finding(uid.clone(), FindingStatus::Fixed, "fixed".into(), "brian".into())
            .await
            .unwrap();
        assert_eq!(storage.get_finding(&uid).await.unwrap().unwrap().eyes_approved, 0);
        // EYES approves → escalation cleared.
        let res = bridge.approve_finding(uid.clone()).await.unwrap();
        assert!(res.contains("approved"), "got: {res}");
        assert_eq!(storage.get_finding(&uid).await.unwrap().unwrap().eyes_approved, 1);
    }
}
