//! Agent feedback about bot-hq itself — the reader side.
//!
//! Agents file via the `file_feedback` MCP tool from whatever project they are
//! working on; these commands are how the app shows and triages that queue.

use crate::core::AppState as CoreAppState;
use crate::storage::{AgentFeedback, FEEDBACK_STATUSES};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

/// One feedback row, projected for the UI.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct AgentFeedbackView {
    pub id: i64,
    pub session_id: String,
    /// The project the FILING session was on — provenance, not subject.
    pub project: Option<String>,
    pub agent: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<AgentFeedback> for AgentFeedbackView {
    fn from(f: AgentFeedback) -> Self {
        Self {
            id: f.id,
            session_id: f.session_id,
            project: f.project,
            agent: f.agent,
            kind: f.kind,
            title: f.title,
            body: f.body,
            status: f.status,
            created_at: f.created_at,
            updated_at: f.updated_at,
        }
    }
}

/// Feedback rows newest-first. `status = None` returns every row.
#[tauri::command]
#[specta::specta]
pub async fn list_agent_feedback(
    core: tauri::State<'_, Arc<CoreAppState>>,
    status: Option<String>,
) -> Result<Vec<AgentFeedbackView>, AppError> {
    let rows = core
        .storage
        .list_feedback(status.as_deref())
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    Ok(rows.into_iter().map(AgentFeedbackView::from).collect())
}

/// Move one row through its lifecycle (`open` / `done` / `dismissed`).
#[tauri::command]
#[specta::specta]
pub async fn set_agent_feedback_status(
    core: tauri::State<'_, Arc<CoreAppState>>,
    id: i64,
    status: String,
) -> Result<bool, AppError> {
    if !FEEDBACK_STATUSES.contains(&status.as_str()) {
        return Err(AppError::Validation(format!(
            "unknown status '{status}' — expected one of {FEEDBACK_STATUSES:?}"
        )));
    }
    let n = core
        .storage
        .set_feedback_status(id, &status)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    #[tokio::test]
    async fn view_carries_provenance_without_claiming_subject() {
        // project is where the friction was HIT; the subject is always bot-hq.
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_feedback("s1", Some("bcc-data-hub-ingest"), "eyes", "idea", "t", "b")
            .await
            .unwrap();
        let v: Vec<AgentFeedbackView> = s
            .list_feedback(None)
            .await
            .unwrap()
            .into_iter()
            .map(AgentFeedbackView::from)
            .collect();
        assert_eq!(v[0].project.as_deref(), Some("bcc-data-hub-ingest"));
        assert_eq!(v[0].agent, "eyes");
        assert_eq!(v[0].status, "open");
    }

    #[test]
    fn only_known_statuses_are_accepted() {
        assert!(FEEDBACK_STATUSES.contains(&"done"));
        assert!(!FEEDBACK_STATUSES.contains(&"wontfix"));
    }
}
