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
            AppControlRequest::Choice { choice_id, .. } => storage
                .get_tray_entry(choice_id)
                .await?
                .map(|r| r.session_id),
            _ => target.map(str::to_owned),
        };
        let audit = storage
            .start_control_audit(&actor, target.as_deref(), operation)
            .await?;
        let result = self
            .control_read(&storage, request)
            .await
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
}
