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
        .await?;
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
    let rows = storage.list_projects().await?;
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
    let rows = bridge
        .cl_index_search(project.as_deref(), query.as_deref())
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[tauri::command]
#[specta::specta]
pub async fn cl_folder_search(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: Option<String>,
    query: Option<String>,
) -> Result<Vec<ClFolderView>, AppError> {
    let rows = bridge
        .cl_folder_search(project.as_deref(), query.as_deref())
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
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
        .await?;
    Ok(())
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
    let report = bridge.cl_rescan(&project).await?;
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
    /// True when the on-disk bytes were NOT valid UTF-8, so `content` is a
    /// lossy decode (`from_utf8_lossy` had to allocate replacement chars).
    /// The editor must refuse to save such a file — writing the lossy
    /// content back would corrupt the original bytes.
    pub binary: bool,
}

/// Resolve `file_path` inside an already-canonicalized project root. Rejects
/// path traversal (the resolved file must stay within the root) and
/// non-regular files. Shared by [`cl_read_file`] + [`cl_write_file`] so both
/// honor the exact same guard. Returns the canonicalized absolute path.
fn resolve_existing_cl_file(
    project_root_real: &std::path::Path,
    file_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let candidate_real = project_root_real
        .join(file_path)
        .canonicalize()
        .map_err(|e| AppError::NotFound(format!("file '{file_path}' not found: {e}")))?;
    if !candidate_real.starts_with(project_root_real) {
        return Err(AppError::Internal(
            "path traversal rejected — file resolves outside project root".into(),
        ));
    }
    let meta = std::fs::metadata(&candidate_real)
        .map_err(|e| AppError::Internal(format!("metadata: {e}")))?;
    if !meta.is_file() {
        return Err(AppError::Internal("not a regular file".into()));
    }
    Ok(candidate_real)
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

    let candidate_real = resolve_existing_cl_file(&project_root_real, &file_path)?;

    let size_bytes = std::fs::metadata(&candidate_real)
        .map_err(|e| AppError::Internal(format!("metadata: {e}")))?
        .len();

    let (content, truncated, binary) = if size_bytes > MAX_READ_BYTES {
        use std::io::Read;
        let mut buf = vec![0u8; MAX_READ_BYTES as usize];
        let mut f = std::fs::File::open(&candidate_real)
            .map_err(|e| AppError::Internal(format!("open: {e}")))?;
        let n = f.read(&mut buf).map_err(|e| AppError::Internal(format!("read: {e}")))?;
        buf.truncate(n);
        let cow = String::from_utf8_lossy(&buf);
        let binary = matches!(cow, std::borrow::Cow::Owned(_));
        (cow.into_owned(), true, binary)
    } else {
        let bytes = std::fs::read(&candidate_real)
            .map_err(|e| AppError::Internal(format!("read: {e}")))?;
        let cow = String::from_utf8_lossy(&bytes);
        let binary = matches!(cow, std::borrow::Cow::Owned(_));
        (cow.into_owned(), false, binary)
    };

    Ok(ClFileContentView {
        project,
        file_path,
        content,
        size_bytes,
        truncated,
        binary,
    })
}

/// Overwrite an existing CL file's contents, resolved exactly like
/// [`cl_read_file`] (same `cl_project_root` + path-traversal guard via
/// [`resolve_existing_cl_file`]). Edits existing regular files only —
/// creating new files / directories are separate commands. `content` is
/// written as UTF-8 bytes; the editor is responsible for not saving a file
/// it flagged `binary` or `truncated` (either would lose data).
#[tauri::command]
#[specta::specta]
pub async fn cl_write_file(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: String,
    file_path: String,
    content: String,
) -> Result<(), AppError> {
    let project_root = bridge
        .cl_project_root(&project)
        .await
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))?;

    let project_root_real = project_root
        .canonicalize()
        .map_err(|e| AppError::NotFound(format!("project '{project}' not found: {e}")))?;

    let candidate_real = resolve_existing_cl_file(&project_root_real, &file_path)?;

    std::fs::write(&candidate_real, content.as_bytes())
        .map_err(|e| AppError::Internal(format!("write: {e}")))?;
    Ok(())
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

    #[test]
    fn resolve_existing_cl_file_allows_files_and_blocks_escapes() {
        use std::fs;
        let base = std::env::temp_dir().join(format!("bot-hq-clguard-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("sub")).unwrap();
        fs::write(base.join("a.md"), b"hello").unwrap();
        fs::write(base.join("sub/b.md"), b"world").unwrap();
        // macOS temp_dir is a /var -> /private/var symlink; the guard expects a
        // canonicalized root, so canonicalize here too before comparing.
        let root = base.canonicalize().unwrap();

        assert!(resolve_existing_cl_file(&root, "a.md").is_ok());
        assert!(resolve_existing_cl_file(&root, "sub/b.md").is_ok());
        // a directory is not a regular file
        assert!(resolve_existing_cl_file(&root, "sub").is_err());
        // missing file
        assert!(resolve_existing_cl_file(&root, "nope.md").is_err());
        // `..` escapes the root → traversal rejected
        assert!(resolve_existing_cl_file(&root, "..").is_err());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn write_through_guard_roundtrips_and_blocks_traversal() {
        use std::fs;
        let base = std::env::temp_dir().join(format!("bot-hq-clwrite-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("note.md"), b"old").unwrap();
        let root = base.canonicalize().unwrap();

        // Resolve-then-write is exactly what cl_write_file does after the
        // (untestable here) bridge root lookup.
        let path = resolve_existing_cl_file(&root, "note.md").unwrap();
        fs::write(&path, b"new content").unwrap();
        assert_eq!(fs::read_to_string(root.join("note.md")).unwrap(), "new content");

        // A traversal target never resolves to a writable path.
        assert!(resolve_existing_cl_file(&root, "../escape.md").is_err());

        let _ = fs::remove_dir_all(&base);
    }
}
