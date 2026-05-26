//! Session document (IPAV tabs) commands.

use crate::signaling::SignalingBridge;
use crate::storage::SessionDocument;
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SessionDocumentView {
    pub id: i64,
    pub session_id: String,
    pub slug: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub phase: Option<String>,
}

impl From<SessionDocument> for SessionDocumentView {
    fn from(d: SessionDocument) -> Self {
        Self {
            id: d.id,
            session_id: d.session_id,
            slug: d.slug,
            body: d.body,
            created_at: d.created_at,
            updated_at: d.updated_at,
            phase: d.phase,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn session_doc_search(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
    query: Option<String>,
    phase: Option<String>,
) -> Result<Vec<SessionDocumentView>, AppError> {
    bridge
        .session_doc_search(&session_id, query.as_deref(), phase.as_deref())
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn session_doc_read(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
    slug: String,
) -> Result<Option<SessionDocumentView>, AppError> {
    bridge
        .session_doc_read(&session_id, &slug)
        .await
        .map(|opt| opt.map(Into::into))
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_doc_search_empty_when_storage_absent() {
        let bridge = SignalingBridge::new();
        let res = bridge.session_doc_search("sx", None, None).await.unwrap();
        assert!(res.is_empty());
    }
}
