//! Context Library commands. Wrap the bridge's CL helpers so the frontend
//! Context-Library tab + plugin manager + audit views all hit one surface.

use crate::signaling::SignalingBridge;
use crate::storage::{ClFolder, ClIndexEntry};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ClIndexEntryView {
    pub id: i64,
    pub project_id: String,
    pub file_path: String,
    pub description: String,
    pub tags: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ClIndexEntry> for ClIndexEntryView {
    fn from(e: ClIndexEntry) -> Self {
        Self {
            id: e.id,
            project_id: e.project_id,
            file_path: e.file_path,
            description: e.description,
            tags: e.tags,
            created_at: e.created_at,
            updated_at: e.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ClFolderView {
    pub id: i64,
    pub project_id: String,
    pub folder_path: String,
    pub description: String,
    pub tags: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ClFolder> for ClFolderView {
    fn from(f: ClFolder) -> Self {
        Self {
            id: f.id,
            project_id: f.project_id,
            folder_path: f.folder_path,
            description: f.description,
            tags: f.tags,
            created_at: f.created_at,
            updated_at: f.updated_at,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn cl_index_search(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: Option<String>,
    query: Option<String>,
) -> Result<Vec<ClIndexEntryView>, AppError> {
    bridge
        .cl_index_search(project.as_deref(), query.as_deref())
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn cl_folder_search(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: Option<String>,
    query: Option<String>,
) -> Result<Vec<ClFolderView>, AppError> {
    bridge
        .cl_folder_search(project.as_deref(), query.as_deref())
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn cl_register_read(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    agent: String,
    session_id: Option<String>,
    project: String,
    file_path: String,
) -> Result<(), AppError> {
    bridge
        .cl_register_read(&agent, session_id.as_deref(), &project, &file_path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ClRescanReportView {
    pub added: Vec<String>,
    pub touched: Vec<String>,
    pub orphaned: Vec<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn cl_rescan(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: String,
) -> Result<ClRescanReportView, AppError> {
    let report = bridge
        .cl_rescan(&project)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(ClRescanReportView {
        added: report.added,
        touched: report.touched,
        orphaned: report.orphaned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cl_index_search_empty_when_bridge_has_no_storage() {
        let bridge = SignalingBridge::new();
        let res = bridge.cl_index_search(None, None).await.unwrap();
        assert!(res.is_empty());
    }
}
