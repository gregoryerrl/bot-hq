//! Context Library commands. Wrap the bridge's CL helpers so the frontend
//! Context-Library tab + plugin manager + audit views all hit one surface.

use crate::signaling::SignalingBridge;
use crate::storage::{ClFolder, ClIndexEntry, Project, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::Emitter;

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
    /// Custom CL root. NULL/None = default convention
    /// `<data_dir>/projects/<name>/`. Lets the folder-view show whether a
    /// project was registered at an arbitrary on-disk location.
    pub cl_path: Option<String>,
}

impl From<Project> for ProjectView {
    fn from(p: Project) -> Self {
        Self {
            name: p.name,
            display_name: p.display_name,
            working_repo_path: p.working_repo_path,
            description: p.description,
            cl_path: p.cl_path,
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

/// Resolve an existing path (file OR directory) inside the canonicalized root.
/// Used by rename-source + delete, which operate on both files and folders.
fn resolve_within_root(
    project_root_real: &std::path::Path,
    rel_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let real = project_root_real
        .join(rel_path)
        .canonicalize()
        .map_err(|e| AppError::NotFound(format!("path '{rel_path}' not found: {e}")))?;
    if !real.starts_with(project_root_real) {
        return Err(AppError::Internal(
            "path traversal rejected — resolves outside project root".into(),
        ));
    }
    Ok(real)
}

/// Resolve a NOT-yet-existing path for create / mkdir / rename-destination: the
/// PARENT directory must already exist inside the root and the leaf must not
/// exist yet. Traversal is guarded via the canonicalized parent (the leaf can't
/// be canonicalized since it doesn't exist).
fn resolve_new_cl_path(
    project_root_real: &std::path::Path,
    rel_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let joined = project_root_real.join(rel_path);
    let parent = joined
        .parent()
        .ok_or_else(|| AppError::Internal("invalid path: no parent".into()))?;
    let file_name = joined
        .file_name()
        .ok_or_else(|| AppError::Internal("invalid path: no final segment".into()))?;
    let parent_real = parent
        .canonicalize()
        .map_err(|e| AppError::NotFound(format!("parent directory not found: {e}")))?;
    if !parent_real.starts_with(project_root_real) {
        return Err(AppError::Internal(
            "path traversal rejected — resolves outside project root".into(),
        ));
    }
    let target = parent_real.join(file_name);
    if target.exists() {
        return Err(AppError::Internal(format!("'{rel_path}' already exists")));
    }
    Ok(target)
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

    // Path resolution (canonicalize + traversal guard) and the read are blocking
    // syscalls — run them on a blocking thread so they don't stall an async
    // worker. `project`/`file_path` are cloned for the closure since the result
    // view needs them afterward.
    let project_label = project.clone();
    let fp = file_path.clone();
    let (content, size_bytes, truncated, binary) =
        tokio::task::spawn_blocking(move || -> Result<(String, u64, bool, bool), AppError> {
            // Canonicalize the project root first. If it doesn't exist (typo or
            // project removed), refuse rather than letting a relative file path
            // escape the data_dir.
            let project_root_real = project_root.canonicalize().map_err(|e| {
                AppError::NotFound(format!("project '{project_label}' not found: {e}"))
            })?;
            let candidate_real = resolve_existing_cl_file(&project_root_real, &fp)?;
            let size_bytes = std::fs::metadata(&candidate_real)
                .map_err(|e| AppError::Internal(format!("metadata: {e}")))?
                .len();
            let (content, truncated, binary) = if size_bytes > MAX_READ_BYTES {
                use std::io::Read;
                let mut buf = vec![0u8; MAX_READ_BYTES as usize];
                let mut f = std::fs::File::open(&candidate_real)
                    .map_err(|e| AppError::Internal(format!("open: {e}")))?;
                let n = f
                    .read(&mut buf)
                    .map_err(|e| AppError::Internal(format!("read: {e}")))?;
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
            Ok((content, size_bytes, truncated, binary))
        })
        .await
        .map_err(|e| AppError::Internal(format!("cl_read_file task panicked: {e}")))??;

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

    tokio::task::spawn_blocking(move || {
        let project_root_real = project_root
            .canonicalize()
            .map_err(|e| AppError::NotFound(format!("project '{project}' not found: {e}")))?;
        let candidate_real = resolve_existing_cl_file(&project_root_real, &file_path)?;
        // Atomic write: a sibling temp in the SAME directory (intra-filesystem,
        // no cross-mount EXDEV) then rename into place — so a crash mid-write
        // can't leave the file truncated or half-written.
        let mut tmp = candidate_real.clone().into_os_string();
        tmp.push(".bot-hq-tmp");
        let tmp = std::path::PathBuf::from(tmp);
        std::fs::write(&tmp, content.as_bytes())
            .map_err(|e| AppError::Internal(format!("write temp: {e}")))?;
        std::fs::rename(&tmp, &candidate_real)
            .map_err(|e| AppError::Internal(format!("rename temp into place: {e}")))?;
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("cl_write_file task panicked: {e}")))?
}

/// Upsert a folder's description + tags (`cl_folders`). Used by the Context
/// Library folder-view editor. Routes through the bridge helper so the project
/// row is ensured to exist first (same path as the agent-facing
/// `cl_register_folder_description` MCP tool). `folder_path = ""` is the
/// project-root folder's description.
#[tauri::command]
#[specta::specta]
pub async fn cl_set_folder_description(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: String,
    folder_path: String,
    description: String,
    tags: Option<String>,
) -> Result<(), AppError> {
    bridge
        .cl_register_folder_description(&project, &folder_path, &description, tags.as_deref())
        .await?;
    Ok(())
}

/// Delete a folder's description row. The folder itself stays in the tree
/// (it's still on disk); only the CL annotation is removed.
#[tauri::command]
#[specta::specta]
pub async fn cl_delete_folder_description(
    storage: tauri::State<'_, Arc<Storage>>,
    project: String,
    folder_path: String,
) -> Result<(), AppError> {
    storage
        .delete_folder_description(&project, &folder_path)
        .await?;
    Ok(())
}

/// Register a project, or update an existing one. Used by the Context Library
/// to promote an arbitrary on-disk folder to a project (`cl_path` = that
/// folder) and to edit an existing project's working-repo from the folder-view.
/// `upsert_project` COALESCEs `None` fields, so passing only the values you
/// want to change preserves the rest. When `cl_path` is supplied it must point
/// at a real directory (guards against registering a typo'd path).
#[tauri::command]
#[specta::specta]
pub async fn cl_register_project(
    storage: tauri::State<'_, Arc<Storage>>,
    app: tauri::AppHandle,
    name: String,
    display_name: Option<String>,
    working_repo_path: Option<String>,
    cl_path: Option<String>,
    description: Option<String>,
) -> Result<(), AppError> {
    // Same name guard create/rename use — this is also a frontend entry point
    // (Promote-to-project, Advanced index-a-folder), so the backend must reject
    // reserved / malformed names rather than trusting the UI. Safe on the
    // working-repo-edit path too: an already-registered name is already valid.
    validate_project_name(&name)?;
    if let Some(p) = cl_path.as_deref() {
        if !p.is_empty() {
            let real = std::path::Path::new(p)
                .canonicalize()
                .map_err(|e| AppError::NotFound(format!("cl_path '{p}' not found: {e}")))?;
            if !real.is_dir() {
                return Err(AppError::Internal(format!(
                    "cl_path '{p}' is not a directory"
                )));
            }
        }
    }
    let display = display_name.unwrap_or_else(|| name.clone());
    storage
        .upsert_project(
            &name,
            &display,
            working_repo_path.as_deref(),
            description.as_deref(),
            cl_path.as_deref(),
        )
        .await?;
    emit_project_and_cl_changed(&app, &name);
    Ok(())
}

/// Soft-unregister a project: clears `cl_path` + `working_repo_path` but KEEPS
/// the row and all child CL rows (index, folders, reads). The project stops
/// being a usable session target; its descriptions survive for re-registration.
#[tauri::command]
#[specta::specta]
pub async fn cl_unregister_project(
    storage: tauri::State<'_, Arc<Storage>>,
    app: tauri::AppHandle,
    name: String,
) -> Result<(), AppError> {
    storage.unregister_project(&name).await?;
    emit_project_and_cl_changed(&app, &name);
    Ok(())
}

/// Emit both the project-registry nudge (`list_projects`) and the CL-tree nudge
/// (`cl_index_search`/`cl_folder_search`). A DB-only project mutation fires no
/// filesystem-watcher event, so create/delete/rename emit `cl:changed`
/// themselves or the tree would stay stale until the next disk touch.
fn emit_project_and_cl_changed(app: &tauri::AppHandle, project: &str) {
    use crate::tauri_events::types::{ClChangedEvent, PROJECT_CHANGED};
    let _ = app.emit(PROJECT_CHANGED, ());
    let _ = app.emit(
        ClChangedEvent::EVENT_NAME,
        ClChangedEvent {
            project: Some(project.to_string()),
        },
    );
}

/// Validate a user-supplied project name. Mirrors the guards in the frontend
/// "Register as project" path (ContextLibrary.tsx) so both entry points reject
/// the same reserved / malformed names. `_globals` + `projects` are reserved
/// (the CL root layout owns them); names can't contain a path separator or
/// start with `.` (would collide with hidden-file handling).
fn validate_project_name(name: &str) -> Result<(), AppError> {
    let n = name.trim();
    if n.is_empty() {
        return Err(AppError::Validation("project name is required".into()));
    }
    if n == Project::GLOBALS || n == "projects" {
        return Err(AppError::Validation(format!("'{n}' is a reserved name")));
    }
    if n.contains('/') || n.contains('\\') || n.starts_with('.') {
        return Err(AppError::Validation(format!("'{n}' is not a valid project name")));
    }
    Ok(())
}

/// Create a NEW Context Library project at the default managed location
/// (`<data_dir>/library/projects/<name>/`). Unlike [`cl_register_project`] this
/// NEVER sets `cl_path` and NEVER indexes an external folder — it makes the
/// convention dir, seeds starter `conventions.md` + `notes.md` (if absent), and
/// rescans just that dir. `working_repo_path` only binds the repo sessions run
/// in; it is NOT scanned. This is the common "add a project" flow.
#[tauri::command]
#[specta::specta]
pub async fn cl_create_project(
    storage: tauri::State<'_, Arc<Storage>>,
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    app: tauri::AppHandle,
    name: String,
    working_repo_path: Option<String>,
    description: Option<String>,
) -> Result<(), AppError> {
    let name = name.trim().to_string();
    validate_project_name(&name)?;
    if storage.get_project(&name).await?.is_some() {
        return Err(AppError::Validation(format!(
            "a project named '{name}' already exists"
        )));
    }

    // Row first so the cl_index FK (project_id -> projects.name) holds when the
    // rescan inserts the seeded files. cl_path stays NULL => convention root.
    let working = working_repo_path.as_deref().filter(|s| !s.trim().is_empty());
    storage
        .upsert_project(&name, &name, working, description.as_deref(), None)
        .await?;

    let root = bridge
        .cl_project_root(&name)
        .await
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))?;
    // Create the managed dir + seed starters on a blocking thread — fs syscalls
    // belong off the async runtime, like every other cl.rs file op. Seeds are
    // best-effort: a failed write is logged, not fatal (the project row + dir
    // already exist), so the user gets a signal instead of a silent empty project.
    let seed_name = name.clone();
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        std::fs::create_dir_all(&root)
            .map_err(|e| AppError::Internal(format!("create project dir: {e}")))?;
        let conventions = root.join("conventions.md");
        if !conventions.exists() {
            if let Err(e) = std::fs::write(
                &conventions,
                format!("# {seed_name} — conventions\n\n_(Repo, stack, build/test commands, gates, house rules. Edit me.)_\n"),
            ) {
                tracing::warn!(?e, project = %seed_name, "failed to seed conventions.md");
            }
        }
        let notes = root.join("notes.md");
        if !notes.exists() {
            if let Err(e) = std::fs::write(
                &notes,
                format!("# {seed_name} — notes\n\n_(Durable, non-obvious learnings — gotchas, where-things-live. Edit me.)_\n"),
            ) {
                tracing::warn!(?e, project = %seed_name, "failed to seed notes.md");
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("seed project task panicked: {e}")))??;

    bridge.cl_rescan(&name).await?;
    emit_project_and_cl_changed(&app, &name);
    Ok(())
}

/// Hard-delete a project: purges the `projects` row + all child CL rows
/// (`cl_index`/`cl_folders`/`cl_reads` cascade). When `delete_cl_dir` is set AND
/// the project uses the default managed location (no custom `cl_path`), its
/// on-disk dir under `library/projects/` is removed too. A custom `cl_path`
/// (an external folder / repo) is NEVER touched — the flag is ignored for it.
#[tauri::command]
#[specta::specta]
pub async fn cl_delete_project(
    storage: tauri::State<'_, Arc<Storage>>,
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    app: tauri::AppHandle,
    name: String,
    delete_cl_dir: bool,
) -> Result<(), AppError> {
    if name == Project::GLOBALS || name == "projects" {
        return Err(AppError::Validation(format!("'{name}' cannot be deleted")));
    }
    let Some(proj) = storage.get_project(&name).await? else {
        return Ok(()); // already gone — idempotent
    };

    // Only remove disk for a managed (default-convention) project. A custom
    // cl_path points at the user's own folder/repo; never rm that.
    let managed = proj.cl_path.as_deref().unwrap_or("").trim().is_empty();
    if delete_cl_dir && managed {
        if let Some(root) = bridge.cl_project_root(&name).await {
            // remove_dir_all on a populated managed dir is the heaviest fs op
            // here — keep it off the async runtime. Best-effort (the DB purge is
            // the source of truth).
            tokio::task::spawn_blocking(move || {
                if root.is_dir() {
                    let _ = std::fs::remove_dir_all(&root);
                }
            })
            .await
            .map_err(|e| AppError::Internal(format!("delete project dir task panicked: {e}")))?;
        }
    }

    storage.delete_project(&name).await?;
    emit_project_and_cl_changed(&app, &name);
    Ok(())
}

/// Rename a project: repoint the row + all child CL rows from `name` to
/// `new_name`, and (for a managed default-convention project) rename its on-disk
/// dir `library/projects/<name>/` -> `<new_name>/`. A custom `cl_path` keeps
/// pointing at the same external folder (only the project identifier changes).
#[tauri::command]
#[specta::specta]
pub async fn cl_rename_project(
    storage: tauri::State<'_, Arc<Storage>>,
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    app: tauri::AppHandle,
    name: String,
    new_name: String,
) -> Result<(), AppError> {
    let new_name = new_name.trim().to_string();
    if name == Project::GLOBALS || name == "projects" {
        return Err(AppError::Validation(format!("'{name}' cannot be renamed")));
    }
    validate_project_name(&new_name)?;
    if new_name == name {
        return Ok(());
    }
    let Some(proj) = storage.get_project(&name).await? else {
        return Err(AppError::NotFound(format!("project '{name}' not found")));
    };
    if storage.get_project(&new_name).await?.is_some() {
        return Err(AppError::Validation(format!(
            "a project named '{new_name}' already exists"
        )));
    }

    // Managed dir move (default convention only). Done BEFORE the DB repoint so
    // a disk failure aborts without leaving the row pointing at a missing dir;
    // on a later DB failure we roll the dir back.
    let managed = proj.cl_path.as_deref().unwrap_or("").trim().is_empty();
    let mut moved: Option<(std::path::PathBuf, std::path::PathBuf)> = None;
    if managed {
        let old_root = bridge.cl_project_root(&name).await;
        let new_root = bridge.cl_project_root(&new_name).await;
        if let (Some(old_root), Some(new_root)) = (old_root, new_root) {
            // Move the managed dir on the blocking pool, BEFORE the DB repoint so
            // a disk failure aborts without leaving the row pointing at a missing
            // dir; on a later DB failure we roll the dir back (also off-runtime).
            let (or, nr) = (old_root.clone(), new_root.clone());
            let did_move = tokio::task::spawn_blocking(move || -> Result<bool, AppError> {
                if or.is_dir() {
                    std::fs::rename(&or, &nr)
                        .map_err(|e| AppError::Internal(format!("rename project dir: {e}")))?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            })
            .await
            .map_err(|e| AppError::Internal(format!("rename project dir task panicked: {e}")))??;
            if did_move {
                moved = Some((old_root, new_root));
            }
        }
    }

    if let Err(e) = storage.rename_project(&name, &new_name, &new_name).await {
        if let Some((old_root, new_root)) = moved {
            let _ = tokio::task::spawn_blocking(move || {
                let _ = std::fs::rename(&new_root, &old_root); // best-effort rollback
            })
            .await;
        }
        return Err(e.into());
    }

    emit_project_and_cl_changed(&app, &new_name);
    Ok(())
}

/// Resolve + canonicalize a project's CL root for the disk-op commands below.
async fn canonical_cl_root(
    bridge: &SignalingBridge,
    project: &str,
) -> Result<std::path::PathBuf, AppError> {
    let root = bridge
        .cl_project_root(project)
        .await
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))?;
    root.canonicalize()
        .map_err(|e| AppError::NotFound(format!("project '{project}' not found: {e}")))
}

/// `_globals` paths that bot-hq itself owns: the agents/ subtree (custom
/// instructions) and custom-general-rules.md. Session spawn resolves these
/// exact paths, so rename/delete would silently break agent startup — they
/// are read+update only. Mirrors `isInternalGlobalsPath` in
/// frontend contextLibraryShared.tsx (keep in sync); the UI hides the menu
/// items and this is the enforcement behind them. Compares CANONICALIZED
/// paths so case-insensitive filesystems can't sidestep the check.
fn assert_not_protected_globals_path(
    project: &str,
    root_real: &std::path::Path,
    candidate_real: &std::path::Path,
) -> Result<(), AppError> {
    if project != crate::storage::Project::GLOBALS {
        return Ok(());
    }
    let agents = root_real.join("agents");
    let rules = root_real.join("custom-general-rules.md");
    if candidate_real == rules || candidate_real == agents || candidate_real.starts_with(&agents) {
        return Err(AppError::Validation(
            "protected bot-hq path — agent custom-instructions and \
             custom-general-rules.md can be edited but not renamed, moved, or deleted"
                .into(),
        ));
    }
    Ok(())
}

/// Create a new empty file. Parent dir must exist; the file must not. The
/// frontend follows with `cl_rescan` to index it.
#[tauri::command]
#[specta::specta]
pub async fn cl_create_file(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: String,
    file_path: String,
) -> Result<(), AppError> {
    let root = canonical_cl_root(&bridge, &project).await?;
    tokio::task::spawn_blocking(move || {
        let target = resolve_new_cl_path(&root, &file_path)?;
        std::fs::write(&target, b"").map_err(|e| AppError::Internal(format!("create file: {e}")))?;
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("cl_create_file task panicked: {e}")))?
}

/// Create a new directory. Parent dir must exist; the directory must not.
#[tauri::command]
#[specta::specta]
pub async fn cl_mkdir(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: String,
    folder_path: String,
) -> Result<(), AppError> {
    let root = canonical_cl_root(&bridge, &project).await?;
    tokio::task::spawn_blocking(move || {
        let target = resolve_new_cl_path(&root, &folder_path)?;
        std::fs::create_dir(&target).map_err(|e| AppError::Internal(format!("mkdir: {e}")))?;
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("cl_mkdir task panicked: {e}")))?
}

/// Rename / move a file or folder within the project's CL root. Source must
/// exist; destination's parent must exist and the destination must not.
#[tauri::command]
#[specta::specta]
pub async fn cl_rename(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: String,
    from_path: String,
    to_path: String,
) -> Result<(), AppError> {
    let root = canonical_cl_root(&bridge, &project).await?;
    tokio::task::spawn_blocking(move || {
        let from_real = resolve_within_root(&root, &from_path)?;
        assert_not_protected_globals_path(&project, &root, &from_real)?;
        let to_target = resolve_new_cl_path(&root, &to_path)?;
        // Also guard the destination — renaming INTO agents/ would shadow a
        // bot-hq-owned path.
        assert_not_protected_globals_path(&project, &root, &to_target)?;
        std::fs::rename(&from_real, &to_target)
            .map_err(|e| AppError::Internal(format!("rename: {e}")))?;
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("cl_rename task panicked: {e}")))?
}

/// Delete a file, or a folder and everything under it. Must exist + resolve
/// inside the project root. Destructive — the frontend gates this behind a
/// confirmation dialog. Does NOT reconcile the index itself: the caller must
/// follow with `cl_rescan` to drop the now-orphaned `cl_index`/`cl_folders`
/// rows (the frontend does at ContextLibrary.tsx). A future agent/driver path
/// that deletes without rescanning would leave orphan rows until the next one.
#[tauri::command]
#[specta::specta]
pub async fn cl_delete_path(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    project: String,
    path: String,
) -> Result<(), AppError> {
    let root = canonical_cl_root(&bridge, &project).await?;
    tokio::task::spawn_blocking(move || {
        let target = resolve_within_root(&root, &path)?;
        assert_not_protected_globals_path(&project, &root, &target)?;
        if target.is_dir() {
            std::fs::remove_dir_all(&target)
                .map_err(|e| AppError::Internal(format!("delete folder: {e}")))?;
        } else {
            std::fs::remove_file(&target)
                .map_err(|e| AppError::Internal(format!("delete file: {e}")))?;
        }
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("cl_delete_path task panicked: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_project_name_rejects_reserved_and_malformed() {
        // Valid names pass (and leading/trailing space is trimmed, not rejected).
        assert!(validate_project_name("my-proj").is_ok());
        assert!(validate_project_name("  spaced  ").is_ok());
        // Empty / whitespace-only.
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("   ").is_err());
        // Reserved (the CL root layout owns these).
        assert!(validate_project_name("_globals").is_err());
        assert!(validate_project_name("projects").is_err());
        // Path separators + hidden-file prefix.
        assert!(validate_project_name("a/b").is_err());
        assert!(validate_project_name("a\\b").is_err());
        assert!(validate_project_name(".hidden").is_err());
    }

    #[tokio::test]
    async fn cl_index_search_empty_when_bridge_has_no_storage() {
        let bridge = SignalingBridge::new();
        let res = bridge.cl_index_search(None, None).await.unwrap();
        assert!(res.is_empty());
    }

    #[test]
    fn protected_globals_paths_block_rename_and_delete() {
        use std::fs;
        let base = std::env::temp_dir().join(format!("bot-hq-clprot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("agents/brian")).unwrap();
        fs::write(base.join("agents/brian/custom-instruction.md"), b"x").unwrap();
        fs::write(base.join("custom-general-rules.md"), b"x").unwrap();
        fs::write(base.join("eod.md"), b"x").unwrap();
        fs::create_dir_all(base.join("projects")).unwrap();
        fs::create_dir_all(base.join("notes")).unwrap();
        let root = base.canonicalize().unwrap();

        let check = |rel: &str| {
            let real = resolve_within_root(&root, rel).unwrap();
            assert_not_protected_globals_path("_globals", &root, &real)
        };
        // bot-hq-owned paths are blocked…
        assert!(check("agents").is_err());
        assert!(check("agents/brian").is_err());
        assert!(check("agents/brian/custom-instruction.md").is_err());
        assert!(check("custom-general-rules.md").is_err());
        // …loose cross-project content is not.
        assert!(check("eod.md").is_ok());
        assert!(check("notes").is_ok());

        // The register-from-Global move target (projects/<name>) is allowed…
        let to = resolve_new_cl_path(&root, "projects/notes").unwrap();
        assert!(assert_not_protected_globals_path("_globals", &root, &to).is_ok());
        // …but renaming INTO agents/ is blocked.
        let into = resolve_new_cl_path(&root, "agents/sneaky.md").unwrap();
        assert!(assert_not_protected_globals_path("_globals", &root, &into).is_err());

        // Non-_globals projects never match, even with identical layouts.
        let real = resolve_within_root(&root, "agents").unwrap();
        assert!(assert_not_protected_globals_path("some-project", &root, &real).is_ok());

        let _ = fs::remove_dir_all(&base);
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

    #[test]
    fn resolve_within_root_allows_files_and_dirs_blocks_escape() {
        use std::fs;
        let base =
            std::env::temp_dir().join(format!("bot-hq-b3within-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("sub")).unwrap();
        fs::write(base.join("f.md"), b"x").unwrap();
        let root = base.canonicalize().unwrap();

        assert!(resolve_within_root(&root, "f.md").is_ok());
        assert!(resolve_within_root(&root, "sub").is_ok()); // dirs allowed
        assert!(resolve_within_root(&root, "missing").is_err());
        assert!(resolve_within_root(&root, "..").is_err());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_new_cl_path_requires_existing_parent_and_absent_leaf() {
        use std::fs;
        let base =
            std::env::temp_dir().join(format!("bot-hq-b3new-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("sub")).unwrap();
        fs::write(base.join("exists.md"), b"x").unwrap();
        let root = base.canonicalize().unwrap();

        assert!(resolve_new_cl_path(&root, "sub/new.md").is_ok());
        assert!(resolve_new_cl_path(&root, "new-at-root.md").is_ok());
        assert!(resolve_new_cl_path(&root, "exists.md").is_err()); // leaf exists
        assert!(resolve_new_cl_path(&root, "nope/child.md").is_err()); // parent missing
        assert!(resolve_new_cl_path(&root, "../escape.md").is_err()); // traversal

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn create_rename_delete_through_guards() {
        use std::fs;
        let base =
            std::env::temp_dir().join(format!("bot-hq-b3ops-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let root = base.canonicalize().unwrap();

        let created = resolve_new_cl_path(&root, "note.md").unwrap();
        fs::write(&created, b"").unwrap();
        assert!(root.join("note.md").is_file());

        let from = resolve_within_root(&root, "note.md").unwrap();
        let to = resolve_new_cl_path(&root, "renamed.md").unwrap();
        fs::rename(&from, &to).unwrap();
        assert!(!root.join("note.md").exists());
        assert!(root.join("renamed.md").is_file());

        let target = resolve_within_root(&root, "renamed.md").unwrap();
        fs::remove_file(&target).unwrap();
        assert!(!root.join("renamed.md").exists());

        let _ = fs::remove_dir_all(&base);
    }
}
