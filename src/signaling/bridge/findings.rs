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
        // A new blocking finding changes the open-blocking count → recompute the
        // router's lock-free cache (cold path), then refresh the UI banner.
        self.refresh_open_blocking(&session_id).await;
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
            // A disposed finding stops gating → the open-blocking count drops;
            // recompute the router's cache (cold path) before the banner event.
            self.refresh_open_blocking(&f.session_id).await;
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
        let blocking = render_open_findings(&open);
        if blocking != "ok" {
            return Ok(blocking); // open blocking findings gate first
        }
        // Batch 7 fail-closed: in a DUO session, a Stalled/Dead reviewer (Rain)
        // can't have reviewed this change — block commit unless HANDS overrode it.
        // No-op in solo (no reviewer) or when Rain is healthy.
        let duo = storage
            .get_session(session_id)
            .await?
            .map(|s| s.rain_enabled != 0)
            .unwrap_or(false);
        let verdict = reviewer_block_decision(
            duo,
            self.current_agent_health(session_id, "rain").as_deref(),
            self.agent_rpc_recent(session_id, "rain", REVIEWER_LIVENESS_WINDOW),
            self.reviewer_override_reason(session_id).as_deref(),
        );
        Ok(verdict.unwrap_or_else(|| "ok".to_string()))
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

/// How recent an RPC call must be to overrule an event-derived Stalled/Dead
/// verdict in the reviewer gate. Generous on purpose: a reviewer mid-review
/// makes a call at least this often, while a genuinely dead one never does.
const REVIEWER_LIVENESS_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Pure: the reviewer-down gate verdict (Batch 7). `Some(gate_string)` when a duo
/// reviewer is down — either `blocked: …` or, with a HANDS override,
/// `ok (reviewer-down overridden: …)`. `None` when the gate should fall through to
/// plain "ok" (solo session, or the reviewer is healthy).
///
/// `recently_active` is the wire-level liveness signal (`agent_rpc_recent`): the
/// health map is event-derived and once reported the reviewer Stalled 4ms after
/// her own tool call (archive study, s-32196a61), burning a tray question and
/// nearly prompting a needless override. An agent talking to the bridge is
/// alive, whatever the last health event said.
fn reviewer_block_decision(
    duo: bool,
    rain_health: Option<&str>,
    recently_active: bool,
    override_reason: Option<&str>,
) -> Option<String> {
    if !duo || !matches!(rain_health, Some("stalled") | Some("dead")) {
        return None;
    }
    if recently_active {
        tracing::debug!(
            health = rain_health,
            "reviewer gate: health says down but the reviewer made an RPC call \
             within the liveness window — treating as alive"
        );
        return None;
    }
    Some(match override_reason {
        Some(r) => format!("ok (reviewer-down overridden: {r})"),
        None => format!(
            "blocked: reviewer down — review cannot be confirmed (Rain is {} and has \
             made no tool call in the last {}s). This means the REVIEWER IS GONE, not \
             that the change is unreviewed — restore the reviewer, or override with \
             override_reviewer_block(reason) if you've confirmed the change is safe to \
             ship unreviewed.",
            rain_health.unwrap_or("down"),
            REVIEWER_LIVENESS_WINDOW.as_secs()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    #[test]
    fn reviewer_block_decision_cases() {
        // Solo (no reviewer) → never blocks.
        assert_eq!(reviewer_block_decision(false, Some("stalled"), false, None), None);
        // Duo + reviewer healthy (running / no transition yet) → no block.
        assert_eq!(reviewer_block_decision(true, Some("running"), false, None), None);
        assert_eq!(reviewer_block_decision(true, None, false, None), None);
        // Duo + reviewer down + no override → blocked.
        assert!(reviewer_block_decision(true, Some("stalled"), false, None)
            .unwrap()
            .starts_with("blocked: reviewer down"));
        assert!(reviewer_block_decision(true, Some("dead"), false, None)
            .unwrap()
            .starts_with("blocked:"));
        // Duo + reviewer down + override → ok-with-reason (not blocked).
        let ov = reviewer_block_decision(
            true,
            Some("stalled"),
            false,
            Some("verified safe; reviewer crashed"),
        )
        .unwrap();
        assert!(ov.starts_with("ok (reviewer-down overridden:"));
        assert!(ov.contains("verified safe"));
    }

    #[test]
    fn recent_rpc_activity_overrules_a_stale_stalled_verdict() {
        // The s-32196a61 false positive: health said "stalled" 4ms after Rain's
        // own tool call. Wire activity wins over the event-derived health map.
        assert_eq!(reviewer_block_decision(true, Some("stalled"), true, None), None);
        assert_eq!(reviewer_block_decision(true, Some("dead"), true, None), None);
        // Activity doesn't matter when health is fine anyway.
        assert_eq!(reviewer_block_decision(true, Some("running"), true, None), None);
    }

    #[tokio::test]
    async fn gate_treats_calling_reviewer_as_alive() {
        // End-to-end through the bridge: mark Rain stalled, then stamp an RPC
        // call — the gate must return plain ok, not "reviewer down".
        let bridge = bridge_with_session("s-live").await;
        {
            let storage = bridge.storage.lock().await.clone().unwrap();
            sqlx::query("UPDATE sessions SET rain_enabled = 1 WHERE id = 's-live'")
                .execute(storage.pool())
                .await
                .unwrap();
        }
        bridge.notify_agent_health("s-live".to_string(), "rain", "stalled");
        assert!(
            bridge
                .check_open_findings("s-live")
                .await
                .unwrap()
                .starts_with("blocked: reviewer down"),
            "without RPC activity the stalled verdict blocks"
        );
        bridge.note_agent_rpc("s-live", "rain");
        assert_eq!(
            bridge.check_open_findings("s-live").await.unwrap(),
            "ok",
            "an actively-calling reviewer is alive regardless of health events"
        );
    }

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
