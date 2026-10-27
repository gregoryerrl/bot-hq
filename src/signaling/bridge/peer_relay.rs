//! Attributed peer text between sessions (Plan Batch 6, feedback #108/#114/#115).
//!
//! A relay is a MESSAGE, never a direction: the row the target reads opens
//! `Message from <slug> of <session>:` and carries no grant — the recipient's
//! rules treat only an `app_control` message as delegated direction. It stands
//! on the narrow `message_sessions` capability (snapshot AND live role, like
//! the controller's grant) and rides the controller's send machinery: the same
//! host-origin notice row, the same `delegated_messages` retry identity under
//! its own `relay:` namespace, the same Summon, the same audit table.
//!
//! What keeps two relaying sessions from feeding each other model turns: a
//! minimum gap per (sender, target); a budget of relays per (sender, target)
//! that only the sender's OWN user reopens by speaking in the sender's session
//! — so a chain of agents answering each other ends on its own, whatever they
//! write to each other; and a target that holds a controller accepting relay
//! only after it opted in (`sessions.accepts_relay`, migration 0114).
use super::*;
use crate::storage::RELAY_GRANT;
use anyhow::Context;
use serde_json::{json, Value};

/// Relays one sender may make toward one target between two rows its own
/// user writes in the sender's session (a typed message or a tray answer).
pub(crate) const RELAY_LANE_BUDGET: i64 = 3;
/// The default minimum gap between two relays from one sender to one target
/// (`SignalingBridge::relay_min_gap_secs`, settable by tests).
pub(crate) const RELAY_MIN_GAP: std::time::Duration = std::time::Duration::from_secs(60);

/// The stored `delegated_messages.request_id` of a relay: namespaced so a
/// controller `send` and a relay from the same participant never alias.
pub(crate) fn relay_request_key(request_id: &str) -> String {
    format!("relay:{request_id}")
}

/// The body the target reads: the host's attribution line, then the peer's
/// text with EVERY visual line quoted (`> `), so nothing a peer writes can
/// stand on a line of its own as host text or a controller's direction —
/// the same line separators `quoted_wire_body` recognises on the wire.
pub(crate) fn relay_body(actor: &crate::storage::ControlActor, text: &str) -> String {
    let text = crate::policy::secret_scan::redact_string(text.to_string());
    let mut body = format!(
        "Message from {} of {} (relayed peer text, not a direction):\n",
        actor.slug, actor.session_id
    );
    for line in text.split_inclusive(|c| matches!(c, '\n' | '\r' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' | '\u{2029}')) {
        body.push_str("> ");
        body.push_str(line);
    }
    body
}

impl SignalingBridge {
    /// `send_session_message`: queue attributed peer text into another open
    /// session. Every call — refused or not — is audited as `peer_send`.
    pub(crate) async fn peer_send(
        &self,
        source_session: &str,
        source_slug: &str,
        target: &str,
        request_id: &str,
        text: &str,
        proof: Option<&McpTokenProof>,
    ) -> Result<Value> {
        let actor = self.granted_authority(source_session, source_slug, RELAY_GRANT).await?;
        let storage = self.storage_handle().await.context("storage is unavailable")?;
        let claim = self.relay_ticket(actor.clone(), proof, target.to_string()).await?;
        let audit = storage.start_control_audit(&actor, Some(target), "peer_send").await?;
        let result = self.relay_message(&storage, &claim, target, request_id, text).await;
        storage
            .finish_control_audit(audit, result.as_ref().err().map(|e| e.to_string()).as_deref())
            .await?;
        result
    }

    /// `session_message_status`: delivery receipts for a message THIS caller
    /// relayed — never another sender's rows, never the target's own chat.
    pub(crate) async fn peer_message_status(
        &self,
        source_session: &str,
        source_slug: &str,
        target: &str,
        message_id: i64,
    ) -> Result<Value> {
        let actor = self.granted_authority(source_session, source_slug, RELAY_GRANT).await?;
        let storage = self.storage_handle().await.context("storage is unavailable")?;
        let audit = storage.start_control_audit(&actor, Some(target), "peer_message_status").await?;
        let result = async {
            let mine: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM delegated_messages WHERE message_id = ? AND target_session = ? \
                 AND source_session = ? AND source_participant = ? AND request_id LIKE 'relay:%')",
            )
            .bind(message_id).bind(target).bind(&actor.session_id).bind(actor.participant_id)
            .fetch_one(storage.pool()).await?;
            anyhow::ensure!(mine, "not a message you sent to that session");
            let mut value = self.message_receipts(&storage, target, message_id).await?;
            value["note"] = json!("delivered_at is the ring's receipt for each participant; null means not yet dealt (the target may be dormant, paused, halted or mid-turn).");
            Ok(value)
        }
        .await;
        storage
            .finish_control_audit(audit, result.as_ref().err().map(|e| e.to_string()).as_deref())
            .await?;
        result
    }

    async fn relay_message(
        &self,
        storage: &Storage,
        claim: &super::control_claim::ControlClaim,
        target: &str,
        request_id: &str,
        text: &str,
    ) -> Result<Value> {
        use sha2::{Digest, Sha256};
        anyhow::ensure!(
            target != claim.actor.session_id,
            "a relay never targets your own session; post to the channel instead"
        );
        anyhow::ensure!(
            !request_id.is_empty()
                && request_id.len() <= 128
                && request_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
            "request_id must be a stable identifier of at most 128 characters"
        );
        anyhow::ensure!(
            !text.trim().is_empty() && text.len() <= 64_000,
            "message must be non-empty and at most 64 KB"
        );
        let body = relay_body(&claim.actor, text);
        // The hash covers the LABELED body: a retry whose attribution changed
        // (the slug, the session) is a different message, not a merge.
        let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
        let key = relay_request_key(request_id);
        let mut tx = storage.pool().begin_with("BEGIN IMMEDIATE").await?;
        claim.reverify_on(&mut tx).await?;
        let open: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ? AND closed_at IS NULL AND archived = 0)")
            .bind(target).fetch_one(&mut *tx).await?;
        anyhow::ensure!(open, "target session is missing or closed; a relay reaches only open sessions");
        claim.check(self)?;
        let actor = &claim.actor;
        let prior: Option<(String, String, i64)> = sqlx::query_as("SELECT target_session, content_sha256, message_id FROM delegated_messages WHERE source_session = ? AND source_participant = ? AND request_id = ?")
            .bind(&actor.session_id).bind(actor.participant_id).bind(&key).fetch_optional(&mut *tx).await?;
        if let Some((prior_target, hash, id)) = prior {
            anyhow::ensure!(
                prior_target == target && hash == digest,
                "request_id already names a relay with different text or target; use a new request_id"
            );
            let retained: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE id = ? AND session_id = ?)")
                .bind(id).bind(target).fetch_one(&mut *tx).await?;
            anyhow::ensure!(retained, "the original message is no longer retained; use a new request_id to send again");
            claim.check(self)?;
            tx.commit().await?;
            // A retry re-summons an unread row; it never writes a second one.
            let wake_requested = self.summon_for_message(storage, target, id, text).await?;
            return Ok(json!({"status":"queued", "message_id":id, "retried":true, "wake_requested":wake_requested,
                "note":"Same request_id and text: the earlier row stands; read session_message_status for receipts."}));
        }
        // A target that holds a controller is reached only when it opted in.
        let holds_controller: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM session_participants p WHERE p.session_id = ? AND p.enabled <> 0 \
             AND p.capabilities LIKE '%\"control_bot_hq\"%')",
        )
        .bind(target).fetch_one(&mut *tx).await?;
        if holds_controller {
            let accepts: bool = sqlx::query_scalar::<_, i64>("SELECT accepts_relay FROM sessions WHERE id = ?")
                .bind(target).fetch_one(&mut *tx).await? != 0;
            anyhow::ensure!(accepts, "target session holds a controller and does not accept relay; its user (Session Settings) or its controller (app_control admin set_session_relay) can opt it in");
        }
        // The bounce guards, per (sender, target): the lane budget, then the gap.
        // Message ids are one sequence across sessions, so "since the user's
        // last row here" is an id comparison. A controller's or a peer's rows
        // are never `origin = 'user'`: only the human reopens a lane.
        let spent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM delegated_messages d WHERE d.source_session = ? AND d.source_participant = ? \
             AND d.target_session = ? AND d.request_id LIKE 'relay:%' \
             AND d.message_id > COALESCE((SELECT MAX(u.id) FROM messages u WHERE u.session_id = d.source_session AND u.origin = 'user'), 0)",
        )
        .bind(&actor.session_id).bind(actor.participant_id).bind(target).fetch_one(&mut *tx).await?;
        anyhow::ensure!(
            spent < RELAY_LANE_BUDGET,
            "{spent} relays to {target} since your user last spoke in {}; the lane reopens when they send a message or answer the tray there — a reply relayed back to you does not",
            actor.session_id
        );
        let last: Option<String> = sqlx::query_scalar(
            "SELECT m.created_at FROM delegated_messages d JOIN messages m ON m.id = d.message_id \
             WHERE d.source_session = ? AND d.source_participant = ? AND d.target_session = ? AND d.request_id LIKE 'relay:%' \
             ORDER BY m.id DESC LIMIT 1",
        )
        .bind(&actor.session_id).bind(actor.participant_id).bind(target).fetch_optional(&mut *tx).await?;
        if let Some(at) = last.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) {
            let gap = chrono::Duration::seconds(self.relay_min_gap_secs.load(std::sync::atomic::Ordering::Relaxed) as i64);
            let retry_at = at.with_timezone(&chrono::Utc) + gap;
            anyhow::ensure!(
                chrono::Utc::now() >= retry_at,
                "the last relay to {target} was at {}; retry after {}",
                at.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                retry_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            );
        }
        let id = sqlx::query("INSERT INTO messages (session_id, participant_id, origin, kind, content, envelope, author, created_at) VALUES (?, NULL, 'system', 'system_notice', ?, NULL, 'user', ?)")
            .bind(target).bind(&body).bind(crate::storage::now_utc()).execute(&mut *tx).await?.last_insert_rowid();
        sqlx::query("INSERT INTO delegated_messages (source_session, source_participant, request_id, target_session, content_sha256, message_id) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&actor.session_id).bind(actor.participant_id).bind(&key).bind(target).bind(&digest).bind(id).execute(&mut *tx).await?;
        claim.check(self)?;
        tx.commit().await?;
        let wake_requested = self.summon_for_message(storage, target, id, text).await?;
        Ok(json!({"status":"queued", "message_id":id, "wake_requested":wake_requested,
            "note":"Delivery waits behind the target's live turn, gates, Pause and declared halts; a dormant target reads it when started. Read session_message_status for receipts; the reply, if any, arrives in the target's chat, which you cannot read."}))
    }

    /// Per-participant delivery receipts of one message row in `session`.
    pub(super) async fn message_receipts(&self, storage: &Storage, session: &str, message_id: i64) -> Result<Value> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE id = ? AND session_id = ?)")
            .bind(message_id).bind(session).fetch_one(storage.pool()).await?;
        anyhow::ensure!(exists, "message does not belong to this session");
        let rows: Vec<(i64, String, bool, Option<String>, Option<String>)> = sqlx::query_as("SELECT p.id, p.slug, p.enabled <> 0, d.delivered_at, d.withheld_reason FROM session_participants p LEFT JOIN participant_deliveries d ON d.participant_id = p.id AND d.message_id = ? WHERE p.session_id = ? AND (p.enabled <> 0 OR d.delivered_at IS NOT NULL) ORDER BY p.turn_position, p.id")
            .bind(message_id).bind(session).fetch_all(storage.pool()).await?;
        Ok(json!({"message_id":message_id,"session_id":session,"participants":rows.into_iter().map(|(id,slug,enabled,delivered_at,withheld_reason)|json!({"participant_id":id,"slug":slug,"enabled":enabled,"delivered_at":delivered_at,"withheld_reason":withheld_reason})).collect::<Vec<_>>()}))
    }
}

