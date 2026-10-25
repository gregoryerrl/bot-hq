//! The delegated application-control facade. Caller scope is authenticated;
//! target scope is explicit. Human-word records are never fabricated here.
use super::*;
use crate::storage::{ControlActor, HistoryRequest};
use anyhow::Context;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AppControlRequest {
    Send {
        session_id: String,
        request_id: String,
        text: String,
    },
    MessageStatus {
        session_id: String,
        message_id: i64,
    },
    Answer {
        choice_id: String,
        picked: String,
        expected_choice_sha256: String,
        #[serde(default)]
        confirm_stale: bool,
    },
    Sessions {
        before_id: Option<String>,
        #[serde(default)]
        include_closed: bool,
    },
    Session {
        session_id: String,
    },
    History {
        session_id: String,
        #[serde(default)]
        query: HistoryRequest,
    },
    Tray {
        session_id: String,
        before_id: Option<i64>,
    },
    Choice {
        choice_id: String,
        #[serde(default)]
        offset_bytes: usize,
    },
    Document {
        session_id: String,
        slug: String,
        #[serde(default)]
        offset_bytes: usize,
    },
    Audit {
        session_id: Option<String>,
        before_id: Option<i64>,
    },
}

impl AppControlRequest {
    fn identity(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Send { session_id, .. } => ("send", Some(session_id)),
            Self::MessageStatus { session_id, .. } => ("message_status", Some(session_id)),
            Self::Answer { .. } => ("answer", None),
            Self::Sessions { .. } => ("sessions", None),
            Self::Session { session_id } => ("session", Some(session_id)),
            Self::History { session_id, .. } => ("history", Some(session_id)),
            Self::Tray { session_id, .. } => ("tray", Some(session_id)),
            Self::Choice { .. } => ("choice", None),
            Self::Document { session_id, .. } => ("document", Some(session_id)),
            Self::Audit { session_id, .. } => ("audit", session_id.as_deref()),
        }
    }
}

impl SignalingBridge {
    pub(crate) async fn controller_authority(
        &self,
        session: &str,
        slug: &str,
    ) -> Result<ControlActor> {
        let storage = self
            .storage_handle()
            .await
            .context("storage is unavailable")?;
        let actor = storage.controller_actor(session, slug).await?;
        // A retired/paused controller must not issue new administrative work.
        // An ordinary awaiting-user recap is not a human Pause.
        if self
            .session_activity
            .lock()
            .await
            .get(session)
            .and_then(Weak::upgrade)
            .is_some_and(|a| a.holds_wakes())
        {
            anyhow::bail!("controller session is paused or cancelling");
        }
        Ok(actor)
    }

    pub(crate) async fn app_control(
        &self,
        source_session: &str,
        source_slug: &str,
        request: AppControlRequest,
        proof: Option<&McpTokenProof>,
    ) -> Result<Value> {
        let actor = self
            .controller_authority(source_session, source_slug)
            .await?;
        let storage = self
            .storage_handle()
            .await
            .context("storage is unavailable")?;
        let (operation, target) = request.identity();
        let target = match &request {
            AppControlRequest::Choice { choice_id, .. }
            | AppControlRequest::Answer { choice_id, .. } => storage
                .get_tray_entry(choice_id)
                .await?
                .map(|r| r.session_id),
            _ => target.map(str::to_owned),
        };
        // Capture the runtime/Pause ticket before even the audit INSERT can
        // queue behind another writer. A later Resume cannot renew this call.
        let claim = match &request {
            AppControlRequest::Send { session_id, .. } => Some(
                self.control_message_ticket(actor.clone(), proof, session_id.clone())
                    .await?,
            ),
            AppControlRequest::Answer {
                expected_choice_sha256,
                ..
            } => Some(
                self.control_claim(
                    actor.clone(),
                    proof,
                    target.clone().context("choice does not exist")?,
                    expected_choice_sha256.clone(),
                )
                .await?,
            ),
            _ => None,
        };
        let audit = storage
            .start_control_audit(&actor, target.as_deref(), operation)
            .await?;
        let result = match request {
            AppControlRequest::Send {
                session_id,
                request_id,
                text,
            } => {
                self.control_send(
                    &storage,
                    &session_id,
                    &request_id,
                    &text,
                    claim.as_ref().unwrap(),
                )
                .await
            }
            AppControlRequest::Answer {
                choice_id,
                picked,
                confirm_stale,
                ..
            } => {
                self.control_answer(&choice_id, picked, confirm_stale, claim.as_ref().unwrap())
                    .await
            }
            read => self.control_read(&storage, read).await,
        }
        .and_then(|value| {
            let reply = json!({"audit_id": audit, "actor": actor, "result": value});
            anyhow::ensure!(
                serde_json::to_vec(&reply)?.len() <= 49_152,
                "control reply exceeds 48 KiB; use narrower pages or document chunks"
            );
            Ok(reply)
        });
        storage
            .finish_control_audit(
                audit,
                result.as_ref().err().map(|e| e.to_string()).as_deref(),
            )
            .await?;
        result
    }

    async fn control_read(&self, storage: &Storage, request: AppControlRequest) -> Result<Value> {
        use AppControlRequest::*;
        let value = match request {
            Send { .. } => anyhow::bail!("send is a write operation"),
            MessageStatus {
                session_id,
                message_id,
            } => {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ? AND session_id = ?)",
                )
                .bind(message_id)
                .bind(&session_id)
                .fetch_one(storage.pool())
                .await?;
                anyhow::ensure!(exists, "message does not belong to this session");
                let rows: Vec<(i64, String, bool, Option<String>, Option<String>)> = sqlx::query_as("SELECT p.id, p.slug, p.enabled <> 0, d.delivered_at, d.withheld_reason FROM session_participants p LEFT JOIN participant_deliveries d ON d.participant_id = p.id AND d.message_id = ? WHERE p.session_id = ? AND (p.enabled <> 0 OR d.delivered_at IS NOT NULL) ORDER BY p.turn_position, p.id")
                    .bind(message_id).bind(&session_id).fetch_all(storage.pool()).await?;
                json!({"message_id":message_id,"session_id":session_id,"participants":rows.into_iter().map(|(id,slug,enabled,delivered_at,withheld_reason)|json!({"participant_id":id,"slug":slug,"enabled":enabled,"delivered_at":delivered_at,"withheld_reason":withheld_reason})).collect::<Vec<_>>()})
            }
            Answer { .. } => anyhow::bail!("answer is a write operation"),
            Sessions {
                before_id,
                include_closed,
            } => {
                let rows: Vec<(String, String, Option<String>, bool)> = sqlx::query_as(
                    "SELECT id, title, closed_at, archived <> 0 FROM sessions \
                     WHERE (? OR (closed_at IS NULL AND archived = 0)) AND (? IS NULL OR id < ?) \
                     ORDER BY id DESC LIMIT 20",
                )
                .bind(include_closed)
                .bind(&before_id)
                .bind(&before_id)
                .fetch_all(storage.pool())
                .await?;
                let next = (rows.len() == 20).then(|| rows.last().unwrap().0.clone());
                json!({"sessions": rows.into_iter().map(|(id,title,closed_at,archived)| json!({"id":id,"title":title,"closed_at":closed_at,"archived":archived})).collect::<Vec<_>>(), "next_before_id": next})
            }
            Session { session_id } => {
                let session = storage
                    .get_session(&session_id)
                    .await?
                    .context("session does not exist")?;
                let participants =
                    crate::tauri_cmd::sessions::participant_views(storage, &session_id).await?;
                json!({"session": crate::tauri_cmd::sessions::SessionInfo::from(session), "participants": participants,
                    "halt": storage.session_halt(&session_id).await?, "controls": crate::tauri_cmd::participant_changes::controls(storage, &session_id).await?})
            }
            History { session_id, query } => {
                serde_json::to_value(storage.read_session_history(&session_id, &query).await?)?
            }
            Tray {
                session_id,
                before_id,
            } => {
                let rows: Vec<crate::storage::SessionTrayEntry> = sqlx::query_as(
                    "SELECT * FROM session_tray WHERE session_id = ? AND (? IS NULL OR id < ?) ORDER BY id DESC LIMIT 10"
                ).bind(&session_id).bind(before_id).bind(before_id).fetch_all(storage.pool()).await?;
                let next = (rows.len() == 10).then(|| rows.last().unwrap().id);
                json!({"rows": rows.into_iter().map(|r| json!({"id":r.id,"choice_id":r.choice_id,"session_id":r.session_id,"agent":r.agent,"kind":r.kind,"status":r.status,"asked_at":r.asked_at,
                    "prompt_preview": &r.prompt[..crate::text::floor_char_boundary(&r.prompt, 400)], "prompt_bytes":r.prompt.len()})).collect::<Vec<_>>(), "next_before_id": next})
            }
            Choice {
                choice_id,
                offset_bytes,
            } => {
                let row = storage
                    .get_tray_entry(&choice_id)
                    .await?
                    .context("choice does not exist")?;
                let body = serde_json::to_string(&row)?;
                anyhow::ensure!(
                    offset_bytes <= body.len() && body.is_char_boundary(offset_bytes),
                    "offset_bytes must be a UTF-8 boundary in this choice"
                );
                let end = crate::text::floor_char_boundary(
                    &body,
                    offset_bytes.saturating_add(5000).min(body.len()),
                );
                use sha2::{Digest, Sha256};
                json!({"choice_id":row.choice_id,"session_id":row.session_id,"sha256":format!("{:x}",Sha256::digest(body.as_bytes())),
                    "total_bytes":body.len(),"offset_bytes":offset_bytes,"body":&body[offset_bytes..end],"next_offset_bytes":(end<body.len()).then_some(end)})
            }
            Document {
                session_id,
                slug,
                offset_bytes,
            } => {
                let doc = storage
                    .session_document_by_slug(&session_id, &slug)
                    .await?
                    .context("document does not exist")?;
                anyhow::ensure!(
                    offset_bytes <= doc.body.len() && doc.body.is_char_boundary(offset_bytes),
                    "offset_bytes must be a UTF-8 boundary in this document"
                );
                let end = crate::text::floor_char_boundary(
                    &doc.body,
                    offset_bytes.saturating_add(5000).min(doc.body.len()),
                );
                json!({"slug":doc.slug,"updated_at":doc.updated_at,"total_bytes":doc.body.len(),"offset_bytes":offset_bytes,"body":&doc.body[offset_bytes..end],"next_offset_bytes": (end < doc.body.len()).then_some(end)})
            }
            Audit {
                session_id,
                before_id,
            } => {
                let rows = storage
                    .control_audit_page(session_id.as_deref(), before_id)
                    .await?;
                let next = (rows.len() == 20).then(|| rows.last().unwrap().id);
                json!({"rows":rows,"next_before_id":next})
            }
        };
        anyhow::ensure!(
            serde_json::to_vec(&value)?.len() <= 48_000,
            "control reply exceeds 48 KB; read individual history records or document chunks"
        );
        Ok(value)
    }

    async fn control_send(
        &self,
        storage: &Storage,
        session: &str,
        request_id: &str,
        text: &str,
        claim: &super::control_claim::ControlClaim,
    ) -> Result<Value> {
        let id = self
            .persist_controlled_message(storage, claim, request_id, text)
            .await?;
        self.notify_message_persisted(Arc::from(session), id);
        // A Summon queues behind the live turn, gates, Pause and declared
        // halts. It never replaces the human's staged text or sends an interrupt.
        let mut recipients = Vec::new();
        for slug in crate::core::mentions::parse_mention_slugs(text) {
            if let Some(p) = storage
                .participant_by_mention(session, &slug)
                .await?
                .filter(|p| p.enabled)
            {
                if !recipients.contains(&p.id) {
                    recipients.push(p.id);
                }
            }
        }
        if recipients.is_empty() {
            if let Some(p) = storage
                .participants_for_session(session)
                .await?
                .into_iter()
                .find(|p| p.enabled && p.participation_mode == "active")
            {
                recipients.push(p.id);
            }
        }
        let mut wake_requested = false;
        if let Some(ring) = self.session_ring(session).await {
            for participant_id in recipients {
                let unread: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM participant_deliveries WHERE participant_id = ? AND message_id = ? AND delivered_at IS NOT NULL AND withheld_reason IS NULL)")
                    .bind(participant_id).bind(id).fetch_one(storage.pool()).await?;
                if unread {
                    wake_requested |= ring
                        .try_send(crate::core::sequencer::SequencerCommand::Summon {
                            participant_id,
                        })
                        .is_ok();
                }
            }
        }
        Ok(
            json!({"status":"queued", "message_id":id, "wake_requested":wake_requested,
            "note":"Delivery waits behind active turns, gates, Pause and declared halts; dormant sessions must be started. Read message_status for actual receipts."}),
        )
    }

    async fn control_answer(
        &self,
        choice_id: &str,
        picked: String,
        confirm_stale: bool,
        claim: &super::control_claim::ControlClaim,
    ) -> Result<Value> {
        anyhow::ensure!(
            !picked.trim().is_empty() && picked.len() <= 64_000,
            "answer must be non-empty and at most 64 KB"
        );
        let core = self
            .app_state
            .get()
            .and_then(Weak::upgrade)
            .context("application control is not connected to the runtime")?;
        let outcome = core
            .resolve_controlled_choice(choice_id, picked, confirm_stale, claim)
            .await?;
        Ok(match outcome {
            ResolveOutcome::Delivered => json!({"status":"delivered", "choice_id":choice_id}),
            ResolveOutcome::DeliveredOutOfBand {
                session_id,
                receipt,
                ..
            } => {
                json!({"status":"recorded", "choice_id":choice_id, "session_id":session_id, "message_id":receipt.map(|r|r.message_id())})
            }
            ResolveOutcome::StaleGateNeedsConfirm { command, asked_at } => {
                json!({"status":"stale_gate_needs_confirmation", "choice_id":choice_id, "command":command, "asked_at":asked_at})
            }
        })
    }
}
