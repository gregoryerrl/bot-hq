//! Context Library commands. Wrap the bridge's CL helpers so the frontend
//! Context-Library tab + plugin manager + audit views all hit one surface.

use crate::signaling::SignalingBridge;
use crate::storage::{ClFolder, ClIndexEntry, Project, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

/// Set the description (and optionally tags) on a CL index entry. Used by
/// the ContextLibrary UI's inline edit flow. Underlying call is the same
/// idempotent `upsert_cl_index` the backfill scan uses, so calling on an
/// entry that doesn't exist yet is fine — it creates the row.
#[tauri::command]
#[specta::specta]
pub async fn cl_set_description(
    storage: tauri::State<'_, Arc<Storage>>,
    project: String,
    file_path: String,
    description: String,
    tags: Option<String>,
) -> Result<(), AppError> {
    storage
        .upsert_cl_index(&project, &file_path, &description, tags.as_deref())
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}

/// Project as exposed to the frontend. Drives the project-filter dropdown
/// in ContextLibrary and (eventually) the New-Session repo picker.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ProjectView {
    pub name: String,
    pub display_name: String,
    pub working_repo_path: Option<String>,
    pub description: Option<String>,
}

impl From<Project> for ProjectView {
    fn from(p: Project) -> Self {
        Self {
            name: p.name,
            display_name: p.display_name,
            working_repo_path: p.working_repo_path,
            description: p.description,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_projects(
    storage: tauri::State<'_, Arc<Storage>>,
) -> Result<Vec<ProjectView>, AppError> {
    let rows = storage
        .list_projects()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(rows.into_iter().map(ProjectView::from).collect())
}

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

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ClFileContentView {
    pub project: String,
    pub file_path: String,
    pub content: String,
    /// Byte size of the file as it lives on disk. The `content` field is
    /// the full text — included as a sanity check for the frontend.
    pub size_bytes: u64,
    /// True when the file was truncated because it exceeded the read cap.
    /// Frontend can show a "showing first 1 MB" notice and offer to open
    /// in $EDITOR (deferred).
    pub truncated: bool,
}

/// Read a single CL file's contents, resolved as
/// `<data_dir>/projects/<project>/<file_path>`. Hard cap on read size so a
/// very large file can't pin the IPC. Path-traversal guarded by
/// canonicalizing both the project root and the resolved file and
/// rejecting any read that escapes.
#[tauri::command]
#[specta::specta]
pub async fn cl_read_file(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: String,
    file_path: String,
) -> Result<ClFileContentView, AppError> {
    const MAX_READ_BYTES: u64 = 1_048_576; // 1 MB

    // Resolve via the bridge helper so `_globals` maps to data_dir and
    // projects with a custom `cl_path` row are honored. Falls back to
    // `<data_dir>/projects/<name>` for the common case.
    let project_root = bridge
        .cl_project_root(&project)
        .await
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))?;

    // Canonicalize the project root first. If it doesn't exist (typo or
    // project removed), refuse rather than letting a relative file path
    // escape the data_dir.
    let project_root_real = project_root
        .canonicalize()
        .map_err(|e| AppError::NotFound(format!("project '{project}' not found: {e}")))?;

    let candidate = project_root_real.join(&file_path);
    let candidate_real = candidate
        .canonicalize()
        .map_err(|e| AppError::NotFound(format!("file '{file_path}' not found: {e}")))?;

    if !candidate_real.starts_with(&project_root_real) {
        return Err(AppError::Internal(
            "path traversal rejected — file resolves outside project root".into(),
        ));
    }

    let meta = std::fs::metadata(&candidate_real)
        .map_err(|e| AppError::Internal(format!("metadata: {e}")))?;
    if !meta.is_file() {
        return Err(AppError::Internal("not a regular file".into()));
    }
    let size_bytes = meta.len();

    let (content, truncated) = if size_bytes > MAX_READ_BYTES {
        use std::io::Read;
        let mut buf = vec![0u8; MAX_READ_BYTES as usize];
        let mut f = std::fs::File::open(&candidate_real)
            .map_err(|e| AppError::Internal(format!("open: {e}")))?;
        let n = f.read(&mut buf).map_err(|e| AppError::Internal(format!("read: {e}")))?;
        buf.truncate(n);
        (String::from_utf8_lossy(&buf).into_owned(), true)
    } else {
        let bytes = std::fs::read(&candidate_real)
            .map_err(|e| AppError::Internal(format!("read: {e}")))?;
        (String::from_utf8_lossy(&bytes).into_owned(), false)
    };

    Ok(ClFileContentView {
        project,
        file_path,
        content,
        size_bytes,
        truncated,
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
