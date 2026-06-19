//! Findings commands — read the per-session EYES-sign-off findings for the UI
//! banner. Thin wrapper over the bridge (durable `findings` table); no business
//! logic here (mirrors `tray.rs`).

use crate::signaling::SignalingBridge;
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

/// One `findings` row projected for the frontend. snake_case (no `rename_all`),
/// like `SessionTrayView` — the generated TS reads `finding_uid` / `code_ref` /
/// etc. directly.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct FindingView {
    pub id: i64,
    pub session_id: String,
    pub finding_uid: String,
    pub agent: String,
    pub severity: String,
    pub summary: String,
    pub code_ref: Option<String>,
    pub status: String,
    pub disposition_reason: Option<String>,
    pub disposed_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Times EYES raised this finding; `>= 2` = escalated (banner emphasis).
    pub raise_count: i64,
    /// True once EYES confirmed the resolution via `approve_finding`.
    pub eyes_approved: bool,
}

impl From<crate::storage::Finding> for FindingView {
    fn from(f: crate::storage::Finding) -> Self {
        Self {
            id: f.id,
            session_id: f.session_id,
            finding_uid: f.finding_uid,
            agent: f.agent,
            severity: f.severity,
            summary: f.summary,
            code_ref: f.code_ref,
            status: f.status,
            disposition_reason: f.disposition_reason,
            disposed_by: f.disposed_by,
            created_at: f.created_at,
            updated_at: f.updated_at,
            raise_count: f.raise_count,
            eyes_approved: f.eyes_approved != 0,
        }
    }
}

/// All findings for a session, oldest-first. The banner computes the open-
/// blocking count + escalation state; a future detail view can show the rest.
#[tauri::command]
#[specta::specta]
pub async fn list_session_findings(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
) -> Result<Vec<FindingView>, AppError> {
    let rows = bridge.list_findings_for_session(&session_id).await?;
    Ok(rows.into_iter().map(Into::into).collect())
}
