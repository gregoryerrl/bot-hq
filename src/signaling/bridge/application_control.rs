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
    Admin { request: super::application_admin::AdminRequest },
    ChangeParticipant { session_id: String, request_id: String, revision: i64, request: crate::storage::ParticipantChangeRequest },
    CancelParticipantChange { session_id: String, request_id: String },
    Create {
        session_id: String,
        title: String,
        repo_path: Option<String>,
        project: Option<String>,
        participants: Vec<crate::tauri_cmd::sessions::ParticipantPick>,
        use_worktree: Option<bool>,
    },
    Start { session_id: String },
    Resume { session_id: String },
    Pause { session_id: String },
    Close { session_id: String, #[serde(default)] archive: bool },
    Reopen { session_id: String },
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
    /// Plan 5c-1: a forward page of the control-event journal. A pure read —
    /// no cursor moves (a watch acknowledges explicitly, in 5c-2), nothing is
    /// delivered.
    Events {
        #[serde(default)]
        since_id: i64,
        sessions: Option<Vec<String>>,
        kinds: Option<Vec<String>>,
        limit: Option<usize>,
    },
    /// Plan 5c-2: a standing subscription over the journal, optionally waking
    /// the controller (`bridge::control_watches`).
    Watch {
        request_id: String,
        sessions: Option<Vec<String>>,
        kinds: Option<Vec<String>>,
        #[serde(default)]
        wake: bool,
    },
    /// The explicit, per-watch, monotonic acknowledgement — the only thing
    /// that moves a watch's cursor and re-arms its wake.
    Ack {
        request_id: String,
        up_to: i64,
    },
    Unwatch {
        request_id: String,
    },
    Watches,
}

impl AppControlRequest {
    fn identity(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Admin { request } => { let (op, target, _) = request.identity(); (op, target) },
            Self::ChangeParticipant { session_id, .. } => ("change_participant", Some(session_id)),
            Self::CancelParticipantChange { session_id, .. } => ("cancel_participant_change", Some(session_id)),
            Self::Create { session_id, .. } => ("create", Some(session_id)),
            Self::Start { session_id } => ("start", Some(session_id)),
            Self::Resume { session_id } => ("resume", Some(session_id)),
            Self::Pause { session_id } => ("pause", Some(session_id)),
            Self::Close { session_id, .. } => ("close", Some(session_id)),
            Self::Reopen { session_id } => ("reopen", Some(session_id)),
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
            Self::Events { .. } => ("events", None),
            Self::Watch { .. } => ("watch", None),
            Self::Ack { .. } => ("ack", None),
            Self::Unwatch { .. } => ("unwatch", None),
            Self::Watches => ("watches", None),
        }
    }
}

impl SignalingBridge {
    pub(crate) async fn controller_authority(
        &self,
        session: &str,
        slug: &str,
    ) -> Result<ControlActor> {
        self.granted_authority(session, slug, crate::storage::CONTROL_GRANT).await
    }

    /// The caller's standing to act under `grant` right now: snapshot and live
    /// role carry it, and its session is not paused or cancelling.
    pub(crate) async fn granted_authority(
        &self,
        session: &str,
        slug: &str,
        grant: &str,
    ) -> Result<ControlActor> {
        let storage = self
            .storage_handle()
            .await
            .context("storage is unavailable")?;
        let actor = storage.granted_actor(session, slug, grant).await?;
        // A retired/paused caller must not issue new work.
        // An ordinary awaiting-user recap is not a human Pause.
        if self
            .session_activity
            .lock()
            .await
            .get(session)
            .and_then(Weak::upgrade)
            .is_some_and(|a| a.holds_wakes())
        {
            anyhow::bail!("caller session is paused or cancelling");
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
            AppControlRequest::ChangeParticipant { session_id, .. } | AppControlRequest::CancelParticipantChange { session_id, .. } => Some(self.control_message_ticket(actor.clone(), proof, session_id.clone()).await?),
            AppControlRequest::Admin { request } if request.identity().2 => Some(self.control_message_ticket(actor.clone(), proof, request.identity().1.unwrap_or(source_session).to_string()).await?),
            AppControlRequest::Pause { session_id } | AppControlRequest::Close { session_id, .. } | AppControlRequest::Reopen { session_id } => Some(self.control_message_ticket(actor.clone(), proof, session_id.clone()).await?),
            AppControlRequest::Create { session_id, .. } => Some(self.control_message_ticket(actor.clone(), proof, session_id.clone()).await?),
            AppControlRequest::Start { session_id } | AppControlRequest::Resume { session_id } => Some(self.control_message_ticket(actor.clone(), proof, session_id.clone()).await?),
            AppControlRequest::Send { session_id, .. } => Some(
                self.control_message_ticket(actor.clone(), proof, session_id.clone())
                    .await?,
            ),
            // A watch is the controller's own state: the ticket names its own session.
            AppControlRequest::Watch { .. } | AppControlRequest::Ack { .. } | AppControlRequest::Unwatch { .. } => Some(
                self.control_message_ticket(actor.clone(), proof, source_session.to_string()).await?,
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
            op @ (AppControlRequest::ChangeParticipant { .. } | AppControlRequest::CancelParticipantChange { .. }) => self.control_participants(op, claim.as_ref().unwrap()).await,
            AppControlRequest::Admin { request } => self.control_admin(&storage, request, claim.as_ref(), audit).await,
            op @ (AppControlRequest::Pause { .. } | AppControlRequest::Close { .. } | AppControlRequest::Reopen { .. }) => self.control_lifecycle(&storage, op, claim.as_ref().unwrap(), audit).await,
            AppControlRequest::Create { session_id, title, repo_path, project, participants, use_worktree } => {
                self.control_create(crate::tauri_cmd::sessions::SessionCreateInput {
                    id: session_id, title, repo_path, project, multi_participant: None,
                    slot0_model_id: None, slot1_model_id: None,
                    options: crate::tauri_cmd::sessions::SessionCreateOptions {
                        participants: Some(participants), use_worktree, ..Default::default()
                    },
                }, claim.as_ref().unwrap()).await
            }
            AppControlRequest::Start { session_id } => {
                self.control_start(&session_id, claim.as_ref().unwrap()).await
            }
            AppControlRequest::Resume { session_id } => {
                let core = self.app_state.get().and_then(Weak::upgrade);
                match core {
                    Some(core) => core.resume_session_controlled(&session_id, claim.as_ref().unwrap()).await.map(|_|json!({"status":"resume_requested", "session_id":session_id})),
                    None => Err(anyhow::anyhow!("application control is not connected to the runtime")),
                }
            }
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
            // No `?` in these arms: a refusal must still reach the audit finish below.
            AppControlRequest::Watch { request_id, sessions, kinds, wake } => match claim.as_ref().unwrap().check(self) {
                Err(e) => Err(e),
                Ok(()) => self.control_watch_upsert(&storage, &actor, &request_id, sessions, kinds, wake).await.map(|watch| {
                    json!({"status":"watching", "watch": watch,
                        "note":"cursor is the last acknowledged journal id; pending_since the first unacknowledged matching event, recorded with or without wake (the wake, if asked, fires once for it when you are idle, unpaused, ungated and still granted); a retry or a wake change keeps it and its receipts, a filter change re-derives it; ack {request_id, up_to} moves the cursor forward and re-arms. sessions omitted = every other session without a controller; kinds omitted = decision_pending, halt_declared, gate_urgent, session_closed, and health only when a runtime is dead (name health in kinds to receive every transition: running, stalled, retrying, dead)."})
                }),
            },
            AppControlRequest::Ack { request_id, up_to } => match claim.as_ref().unwrap().check(self) {
                Err(e) => Err(e),
                Ok(()) => self
                    .control_watch_ack(&storage, &actor, &request_id, up_to)
                    .await
                    .map(|watch| json!({"status":"acknowledged", "watch": watch})),
            },
            AppControlRequest::Unwatch { request_id } => match claim.as_ref().unwrap().check(self) {
                Err(e) => Err(e),
                Ok(()) => self
                    .control_watch_remove(&storage, &actor, &request_id)
                    .await
                    .map(|removed| json!({"status": if removed {"removed"} else {"absent"}, "request_id": request_id})),
            },
            AppControlRequest::Watches => storage
                .control_watches_for(actor.participant_id)
                .await
                .map(|watches| json!({"watches": watches})),
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

    async fn control_participants(&self, request: AppControlRequest, claim: &crate::signaling::ControlClaim) -> Result<Value> {
        let core = self.app_state.get().and_then(Weak::upgrade).context("application control is not connected to the runtime")?;
        match request {
            AppControlRequest::ChangeParticipant { session_id, request_id, revision, request } => {
                let row = core.change_participant_controlled(&session_id, &request_id, revision, request, claim.clone()).await?;
                serde_json::to_value(crate::tauri_cmd::participant_changes::ParticipantChangeView::try_from(row)?).map_err(Into::into)
            }
            AppControlRequest::CancelParticipantChange { session_id, request_id } => Ok(json!({"cancelled":core.cancel_participant_change_controlled(&session_id, &request_id, claim).await?})),
            _ => anyhow::bail!("not a participant operation"),
        }
    }

    async fn control_lifecycle(&self, storage: &Storage, request: AppControlRequest, claim: &crate::signaling::ControlClaim, audit: i64) -> Result<Value> {
        let core = self.app_state.get().and_then(Weak::upgrade)
            .context("application control is not connected to the runtime")?;
        match request {
            AppControlRequest::Pause { session_id } => {
                anyhow::ensure!(storage.get_session(&session_id).await?.is_some(), "target session does not exist");
                let paused = core.pause_session_controlled(&session_id, claim, audit).await?;
                Ok(json!({"session_id":session_id, "status":if paused {"pause_requested"} else {"not_running"}}))
            }
            AppControlRequest::Close { session_id, archive } => {
                core.close_session_controlled(&session_id, archive, claim, audit).await?;
                let row = storage.get_session(&session_id).await?.context("target session no longer exists")?;
                Ok(json!({"session_id":session_id,"status":if row.closed_at.is_some() {"closed"} else {"close_requested"},"closed_at":row.closed_at,"archived":row.archived != 0}))
            }
            AppControlRequest::Reopen { session_id } => {
                let (moved, startup_error) = core.reopen_session_controlled(&session_id, claim).await?;
                Ok(json!({"session_id":session_id,"status":if moved {"reopened"} else {"already_open"},
                    "startup": {"status":if !moved {"unchanged"} else if startup_error.is_some() {"failed"} else {"started"},
                    "error":startup_error.map(crate::policy::secret_scan::redact_string)}}))
            }
            _ => anyhow::bail!("not a lifecycle operation"),
        }
    }

    async fn control_create(
        &self,
        input: crate::tauri_cmd::sessions::SessionCreateInput,
        claim: &crate::signaling::ControlClaim,
    ) -> Result<Value> {
        let core = self.app_state.get().and_then(Weak::upgrade)
            .context("application control is not connected to the runtime")?;
        let session = crate::tauri_cmd::sessions::create_session_record(&core, input, Some(claim)).await?;
        let startup = match self.control_start(&session.id, claim).await {
            Ok(result) => result,
            Err(error) => json!({"status":"failed", "error":crate::policy::secret_scan::redact_string(error.to_string())}),
        };
        // Creation succeeded even if startup failed. Never invite a duplicate
        // create retry when the right action is `start` on the existing row.
        core.notify_session_created(&session.id);
        Ok(json!({"status":"created", "session":session, "startup":startup}))
    }

    async fn control_read(&self, storage: &Storage, request: AppControlRequest) -> Result<Value> {
        use AppControlRequest::*;
        let value = match request {
            ChangeParticipant { .. } | CancelParticipantChange { .. } => anyhow::bail!("participant changes are write operations"),
            Admin { .. } => anyhow::bail!("admin requests use the configuration facade"),
            Create { .. } => anyhow::bail!("creation is a write operation"),
            Start { .. } | Resume { .. } | Pause { .. } | Close { .. } | Reopen { .. } => anyhow::bail!("lifecycle is a write operation"),
            Send { .. } => anyhow::bail!("send is a write operation"),
            MessageStatus {
                session_id,
                message_id,
            } => self.message_receipts(storage, &session_id, message_id).await?,
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
                let runtime = self.session_activity.lock().await.get(&session_id).and_then(Weak::upgrade)
                    .map(|a|json!({"activity":a.current().as_str(),"paused":a.is_paused()}));
                let core = self.app_state.get().and_then(Weak::upgrade);
                let controls = crate::tauri_cmd::participant_changes::controls_with_runtime(storage, &session_id, core.as_deref()).await?;
                json!({"session": crate::tauri_cmd::sessions::SessionInfo::from(session), "participants": participants, "runtime":runtime,
                    "halt": storage.session_halt(&session_id).await?, "controls": controls,
                    "accepts_relay": storage.session_accepts_relay(&session_id).await?})
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
                // Beside the row, never inside it: `sha256` stays the hash of
                // the row alone, which is what the answer claim recomputes.
                let explained = self.explain_tray_row(&row).await;
                json!({"choice_id":row.choice_id,"session_id":row.session_id,"sha256":format!("{:x}",Sha256::digest(body.as_bytes())),
                    "total_bytes":body.len(),"offset_bytes":offset_bytes,"body":&body[offset_bytes..end],"next_offset_bytes":(end<body.len()).then_some(end),
                    "explained": explained})
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
            Watch { .. } | Ack { .. } | Unwatch { .. } | Watches => anyhow::bail!("watch operations are routed by the facade"),
            Events { since_id, sessions, kinds, limit } => {
                anyhow::ensure!(since_id >= 0, "since_id must be 0 or a journal id");
                let page = storage
                    .control_events_page(since_id, sessions.as_deref(), kinds.as_deref(), limit.unwrap_or(50))
                    .await?;
                json!({"events": page.events, "scanned": page.scanned, "next_since_id": page.next_since_id, "truncated": page.truncated, "markers": page.markers,
                    "note": "A read: no cursor moved, nothing delivered. next_since_id is the last journal id scanned, so a filtered page still progresses; truncated means rows between since_id and the oldest retained row were swept; markers lists the journal_restart / journal_gap rows among the scanned ones whatever the filters — events before a restart marker, or counted by a gap row, may be missing, and the pending decisions / standing halts re-listed after a restart repeat earlier rows: (kind, session_id, ref_id) identifies a decision or a halt."})
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
        let wake_requested = self.summon_for_message(storage, session, id, text).await?;
        Ok(
            json!({"status":"queued", "message_id":id, "wake_requested":wake_requested,
            "note":"Delivery waits behind active turns, gates, Pause and declared halts; dormant sessions must be started. Read message_status for actual receipts."}),
        )
    }

    /// Announce a persisted host row and summon its readers: the `@mentioned`
    /// participants, else the first active one. A Summon queues behind the
    /// live turn, gates, Pause and declared halts; it never replaces the
    /// human's staged text or sends an interrupt. Returns whether any summon
    /// was requested (a dormant session has no ring to ask).
    pub(super) async fn summon_for_message(
        &self,
        storage: &Storage,
        session: &str,
        id: i64,
        text: &str,
    ) -> Result<bool> {
        self.notify_message_persisted(Arc::from(session), id);
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
        Ok(wake_requested)
    }

    async fn control_start(&self, session_id: &str, claim: &super::control_claim::ControlClaim) -> Result<Value> {
        let core = self.app_state.get().and_then(Weak::upgrade).context("application control is not connected to the runtime")?;
        core.ensure_session_started_controlled(session_id, claim).await?;
        core.notify_session_created(session_id);
        Ok(json!({"status":"started", "session_id":session_id,
            "note":"Agents are available or orienting. Send after start (retry the same message request_id if it was queued while dormant). Resume explicitly to release Pause or a declared halt."}))
    }

    /// The tray row in plain words, for a controller reading a card (feedback
    /// #143): the reviewer's read as recorded (or why there is none), whether
    /// the body files the review covered still match, and the decision. A
    /// read, never a check that authorizes anything — the approval re-hashes
    /// the files itself and refuses a changed or missing one.
    pub(crate) async fn explain_tray_row(&self, row: &crate::storage::SessionTrayEntry) -> Value {
        use crate::storage::{review_state, ReviewState};
        let command = row.command_text.as_deref();
        let (status, note) = match review_state(row) {
            ReviewState::Reviewed if row.reviewed_by.is_none() => ("reviewed", "the queued body was read and nothing blocking was filed, at `at`; who read it was not recorded (a read from before readers were recorded) — the session's current reviewer is not assumed"),
            ReviewState::Reviewed => ("reviewed", "the reviewer read the queued body and filed nothing blocking; the card was parked for the user after that read"),
            ReviewState::Skipped => ("skipped", "the card reached the user WITHOUT a reviewer's read; the prompt says why"),
            ReviewState::QueuedForReview => ("queued_for_review", "waiting for the reviewer's read; it parks for the user (or runs under auto_allow) after their turn"),
            ReviewState::NotApplicable if row.command_text.is_some() => ("not_applicable", "a push gate has no reviewer step: the pre-push hook parks it for the user directly; whether the pushed commits were reviewed is a matter of the session's review record, not of this card"),
            ReviewState::NotApplicable => ("not_applicable", "a question or request is answered, not reviewed"),
            ReviewState::Unrecorded => ("unrecorded", "no structured review provenance on this row (a card older than the column, or one that never went through the review queue); this is not evidence that it was unreviewed — read the prompt"),
        };
        let review = json!({"status": status, "by": row.reviewed_by, "at": row.reviewed_at, "note": note});
        let body_files = match command {
            None => json!({"pinned": false, "paths": [], "current": "none", "note": "no command"}),
            Some(command) => match self.body_files_snapshot(&row.session_id, command).await {
                None => json!({"pinned": false, "paths": [], "current": "none", "note": "the command reads no body file"}),
                Some(Err(path)) => json!({"pinned": row.body_sha256.is_some(), "paths": [path.clone()], "current": format!("missing {path}"),
                    "note": "a body file the command reads does not exist now; an approval would refuse the run"}),
                Some(Ok((now, files))) => {
                    let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
                    let current = match row.body_sha256.as_deref() {
                        None => "not_pinned",
                        Some(then) if then == now => "unchanged",
                        Some(_) => "changed",
                    };
                    json!({"pinned": row.body_sha256.is_some(), "paths": paths, "current": current,
                        "digest": "body_sha256 is one sha256 over every body file's path and bytes in command order — not a plain file hash, so it will not match shasum; compare `current` instead",
                        "note": match current {
                            "unchanged" => "the body files are byte-identical to what was pinned when this parked (what the reviewer read, if there was a read)",
                            "changed" => "a body file changed after it was pinned: an approval re-hashes and REFUSES the run (nothing is published); the agent must re-issue the command, which is re-read by the reviewer",
                            "not_pinned" => "no body hash was recorded when this parked (it predates the check); the files are not re-checked at approval",
                            _ => "",
                        }})
                }
            },
        };
        let live_body = json!({"pinned": row.live_body_sha256.is_some(),
            "note": if row.live_body_sha256.is_some() {
                "live_body_sha256 is the remote body this edit replaces, as it was when the reviewer saw the diff; it is re-read at approval and the edit is refused if it changed"
            } else { "not an edit of a live body, or none was recorded" }});
        // The decision's actor rides a column the row projection leaves out
        // (`decision_actor_json`, written by the delegated-answer claim). The
        // read's failure is kept: an unread actor is not a NULL one.
        let actor = match self.storage_handle().await {
            Some(storage) => sqlx::query_scalar::<_, Option<String>>("SELECT decision_actor_json FROM session_tray WHERE choice_id = ?")
                .bind(&row.choice_id)
                .fetch_optional(storage.pool())
                .await
                .map_err(|e| e.to_string()),
            None => Err("storage is not connected".to_string()),
        };
        let decision = decision_explained(&row.status, row.picked_option.as_deref(), row.answered_at.as_deref(), actor);
        json!({"review": review, "body_files": body_files, "live_body": live_body, "decision": decision})
    }

}

/// A tray row's decision as the record shows it (feedback #143, EYES
/// `36985429`). Only an ANSWERED row has a decider. The human UI's resolve
/// is the one answer path that writes no actor, so a successful read of a
/// NULL `decision_actor_json` is the user; a parsed actor is the delegated
/// controller it names. An actor that could not be read (`actor` is `Err`),
/// a row that vanished (`Ok(None)`) or a record without its slug and session
/// leaves the decider UNKNOWN, with the reason — never "the user". A
/// withdrawal (a reviewer's blocking finding, the session's close, a body or
/// recap that could not be posted) answered nothing: its `picked_option` is
/// the host's reason and is reported as that.
fn decision_explained(status: &str, picked: Option<&str>, answered_at: Option<&str>, actor: Result<Option<Option<String>>, String>) -> Value {
    let note = "only the exact text `Approve` approves an Approve/Reject gate; `Reject — <reason>` carries the reason to the executor";
    match status {
        "answered" => {
            let decider: Result<String, String> = match actor {
                Ok(Some(None)) => Ok("the user".to_string()),
                Ok(Some(Some(record))) => serde_json::from_str::<Value>(&record)
                    .ok()
                    .and_then(|a| Some(format!("{} of {} (delegated controller)", a.get("slug")?.as_str()?, a.get("session_id")?.as_str()?)))
                    .ok_or_else(|| "the answer's actor record could not be read (malformed); who answered is unknown".to_string()),
                Ok(None) => Err("the row could not be read again; who answered is unknown".to_string()),
                Err(e) => Err(format!("the answer's actor record could not be read ({e}); who answered is unknown")),
            };
            match decider {
                Ok(by) => json!({"status": status, "picked": picked, "decided_by": by, "answered_at": answered_at, "note": note}),
                Err(why) => json!({"status": status, "picked": picked, "decided_by": null, "decided_by_unknown": why, "answered_at": answered_at, "note": note}),
            }
        }
        "withdrawn" => json!({"status": status, "picked": null, "decided_by": null, "withdrawn_reason": picked,
            "note": "withdrawn by bot-hq or the asker, not answered: `withdrawn_reason` is the host's reason when one was recorded"}),
        _ => json!({"status": status, "picked": null, "decided_by": null, "answered_at": answered_at, "note": note}),
    }
}

impl SignalingBridge {
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

#[cfg(test)]
mod tests {
    use super::decision_explained;

    /// EYES `36985429`: every way the actor record can be missing or bad
    /// keeps the decider unknown; only a NULL actor read back from an
    /// answered row is the user; a withdrawal and an open row have no decider.
    #[test]
    fn a_decider_is_named_only_from_a_read_record_of_an_answer() {
        let answered = |actor| decision_explained("answered", Some("Reject — wait"), Some("2026-10-28T22:00:00.000Z"), actor);
        let d = answered(Ok(Some(None)));
        assert_eq!(d["decided_by"], "the user", "{d}");
        let d = answered(Ok(Some(Some(r#"{"slug":"driver","session_id":"s-91409754","participant_id":3}"#.into()))));
        assert_eq!(d["decided_by"], "driver of s-91409754 (delegated controller)", "{d}");
        for (actor, why) in [
            (Err("database is locked".to_string()), "database is locked"),
            (Ok(None), "could not be read again"),
            (Ok(Some(Some("not json".into()))), "malformed"),
            (Ok(Some(Some(r#"{"session_id":"s1"}"#.into()))), "malformed"),
        ] {
            let d = answered(actor);
            assert!(d["decided_by"].is_null(), "{d}");
            assert!(d["decided_by_unknown"].as_str().is_some_and(|w| w.contains(why)), "{why}: {d}");
            assert_eq!(d["picked"], "Reject — wait");
        }
        let d = decision_explained("withdrawn", Some("withdrawn: the reviewer filed a blocking finding"), None, Ok(Some(None)));
        assert!(d["decided_by"].is_null() && d["picked"].is_null(), "{d}");
        assert_eq!(d["withdrawn_reason"], "withdrawn: the reviewer filed a blocking finding");
        let d = decision_explained("withdrawn", None, None, Ok(Some(None)));
        assert!(d["decided_by"].is_null() && d["withdrawn_reason"].is_null(), "an agent's own withdrawal records no reason: {d}");
        for status in ["pending", "queued", "superseded", "auto_running", "auto_resolved"] {
            let d = decision_explained(status, None, None, Ok(Some(None)));
            assert!(d["decided_by"].is_null(), "{status}: no answer, no decider: {d}");
        }
    }
}
