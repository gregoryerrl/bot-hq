//! Session lifecycle commands.

use crate::core::session::{resolve_session_project, ProjectProvenance};
use crate::core::AppState as CoreAppState;
use crate::signaling::SignalingBridge;
use crate::storage::{Session, SessionWithPreview, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub working_repo_path: Option<String>,
    /// Set when the session runs in an isolated git worktree —
    /// `working_repo_path` is then the worktree and this is the repo it was
    /// carved from. None = direct mode.
    pub base_repo_path: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub closed_at: Option<String>,
    pub brian_model_at_spawn: Option<String>,
    pub rain_model_at_spawn: Option<String>,
    /// False = solo-Brian session (Rain disabled at create).
    pub rain_enabled: bool,
    /// First line preview of the latest text message + its author, for the
    /// dashboard Quickview. Both None on the closed-session and external
    /// JSON-RPC paths — only the dashboard `list_sessions` command populates
    /// them (via `list_active_sessions_with_preview`).
    pub last_message: Option<String>,
    pub last_author: Option<String>,
}

impl From<Session> for SessionInfo {
    fn from(s: Session) -> Self {
        Self {
            id: s.id,
            title: s.title,
            working_repo_path: s.working_repo_path,
            base_repo_path: s.base_repo_path,
            archived: s.archived != 0,
            created_at: s.created_at,
            closed_at: s.closed_at,
            brian_model_at_spawn: s.brian_model_at_spawn,
            rain_model_at_spawn: s.rain_model_at_spawn,
            rain_enabled: s.rain_enabled != 0,
            last_message: None,
            last_author: None,
        }
    }
}

impl From<SessionWithPreview> for SessionInfo {
    fn from(s: SessionWithPreview) -> Self {
        let mut info = SessionInfo::from(s.session);
        info.last_message = s.last_message;
        info.last_author = s.last_author;
        info
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SessionProjectInfo {
    /// Resolved project name, or None for a repo-less session.
    pub project: Option<String>,
    /// How `project` was derived — drives the gear-tab policy-origin badge.
    pub provenance: ProjectProvenance,
}

/// Resolve a session's project + how it was derived, so the gear tab can show
/// WHY the session inherited its policy (registered repo vs path basename vs
/// no project → general). Deterministic from the session's repo paths — no
/// persisted column needed.
#[tauri::command]
#[specta::specta]
pub async fn get_session_project_info(
    storage: tauri::State<'_, Arc<Storage>>,
    session_id: String,
) -> Result<SessionProjectInfo, AppError> {
    let session = storage
        .get_session(&session_id)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?;
    let (project, provenance) = resolve_session_project(
        &storage,
        session.base_repo_path.as_deref(),
        session.working_repo_path.as_deref().map(Path::new),
    )
    .await;
    Ok(SessionProjectInfo { project, provenance })
}

/// Per-session create-dialog picks beyond the positional args. Bundled into
/// one struct because `create_session` sits at tauri-specta's 10-arg command
/// limit; every field is `None` = inherit the configured default.
/// (Renamed from `SessionEffortChoices` when `use_worktree` joined.)
#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionCreateOptions {
    pub brian_effort: Option<String>,
    pub rain_effort: Option<String>,
    pub brian_ultracode: Option<bool>,
    pub rain_ultracode: Option<bool>,
    /// Run the session in an isolated git worktree (None → the
    /// `worktree_default` app setting, which defaults ON for repo-backed
    /// sessions).
    pub use_worktree: Option<bool>,
}

/// Where a new session runs: `(working_repo_path, base_repo_path)`.
///
/// Worktree mode (the default for repo-backed sessions) places the session at
/// `<data_dir>/.local/worktrees/<sid>/<repo-basename>` and remembers the base
/// repo; the worktree itself is materialized lazily at spawn
/// (`spawn_session_handle`), so this only decides paths. Direct mode — and
/// every repo-less or non-git path — runs in the repo itself. Blank paths
/// normalize to None (matching `Storage::create_session`).
async fn resolve_session_placement(
    storage: &Storage,
    data_dir: &std::path::Path,
    session_id: &str,
    repo_path: Option<String>,
    use_worktree: Option<bool>,
) -> (Option<String>, Option<String>) {
    let repo = match repo_path.filter(|p| !p.trim().is_empty()) {
        Some(r) => r,
        None => return (None, None),
    };
    let enabled = match use_worktree {
        Some(b) => b,
        None => storage.default_worktree_enabled().await,
    };
    // A path with no `.git` can't host a worktree (hooks skip it too) —
    // direct mode rather than a guaranteed spawn-time fallback.
    if !enabled || !std::path::Path::new(&repo).join(".git").exists() {
        return (Some(repo), None);
    }
    match crate::core::worktree::session_worktree_path(
        data_dir,
        session_id,
        std::path::Path::new(&repo),
    ) {
        Some(wt) => (Some(wt.to_string_lossy().into_owned()), Some(repo)),
        None => (Some(repo), None),
    }
}

#[tauri::command]
#[specta::specta]
// Param count is inflated by Tauri-injected `State` handles, not real fan-out.
#[allow(clippy::too_many_arguments)]
pub async fn create_session(
    core: tauri::State<'_, Arc<CoreAppState>>,
    id: String,
    title: String,
    repo_path: Option<String>,
    project: Option<String>,
    // Create-dialog choices. Defaults preserve the historical duo behavior so
    // older callers that omit them keep spawning Rain with agent-config models.
    rain_enabled: Option<bool>,
    brian_model_id: Option<String>,
    rain_model_id: Option<String>,
    // Effort/ultracode/worktree picks (bundled — see SessionCreateOptions).
    options: SessionCreateOptions,
) -> Result<SessionInfo, AppError> {
    let storage = &core.storage;
    let (working, base) = resolve_session_placement(
        storage,
        &core.paths.data_dir,
        &id,
        repo_path,
        options.use_worktree,
    )
    .await;
    storage
        .create_session(&id, &title, working.as_deref())
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    if base.is_some() {
        storage
            .set_session_base_repo(&id, base.as_deref())
            .await
            .map_err(|e| AppError::DbError(e.to_string()))?;
    }
    // Persist the Rain toggle + per-agent model picks on the row BEFORE the
    // session is spawned (respawn_session reads them off the row).
    storage
        .set_session_spawn_config(
            &id,
            rain_enabled.unwrap_or(true),
            brian_model_id.as_deref(),
            rain_model_id.as_deref(),
        )
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    // Per-session effort/ultracode overrides (separate setter to avoid an
    // 8-param method; also persisted pre-spawn).
    storage
        .set_session_effort_config(
            &id,
            options.brian_effort.as_deref(),
            options.rain_effort.as_deref(),
            options.brian_ultracode,
            options.rain_ultracode,
        )
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    core.bridge.register_session(id.clone(), project).await;
    // Re-fetch so the returned SessionInfo reflects the persisted config.
    let session = storage
        .get_session(&id)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?
        .ok_or_else(|| AppError::DbError("session vanished after create".into()))?;
    Ok(session.into())
}

/// Dispatch a session pre-loaded with a first prompt: create the row, register
/// the project, spawn the duo, and broadcast `prompt` to their stdin — all in
/// one call so delivery is deterministic. A fresh session spawns blank
/// (`resume_session_id = None`) and bot-hq does NOT replay storage to stdin, so
/// the prompt has to be broadcast to a LIVE session — which means spawning
/// first. `ensure_session_started` inserts the handle before returning, so the
/// subsequent `broadcast` always finds it; it's idempotent, so the SessionView
/// mount's `respawn_session` is a harmless no-op.
///
/// Generic on purpose — the caller supplies the prompt. The Context Library
/// "Maintain CL" button calls this with a hardcoded CL-maintenance prompt.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_session(
    core: tauri::State<'_, Arc<CoreAppState>>,
    storage: tauri::State<'_, Arc<Storage>>,
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    id: String,
    title: String,
    project: Option<String>,
    repo_path: Option<String>,
    prompt: String,
) -> Result<SessionInfo, AppError> {
    // No create dialog on this path → both placement and solo/duo come from
    // the configured defaults (worktree_default / rain_disabled_default).
    let (working, base) = resolve_session_placement(
        &storage,
        &core.paths.data_dir,
        &id,
        repo_path,
        None,
    )
    .await;
    let mut session = storage
        .create_session(&id, &title, working.as_deref())
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    if base.is_some() {
        storage
            .set_session_base_repo(&id, base.as_deref())
            .await
            .map_err(|e| AppError::DbError(e.to_string()))?;
        session.base_repo_path = base;
    }
    // Honor the user's solo/duo default. Without this the DB default
    // (`rain_enabled=1`) always spawned the duo regardless of
    // `rain_disabled_default`. Models stay NULL = per-agent defaults, same as
    // the dialog's "(agent default)" pick.
    let rain_enabled = storage.default_rain_enabled().await;
    storage
        .set_session_spawn_config(&id, rain_enabled, None, None)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    session.rain_enabled = if rain_enabled { 1 } else { 0 };
    // Register the project mapping BEFORE spawn so the agents' system prompt
    // picks up project-scoped CL conventions.
    bridge.register_session(id.clone(), project).await;
    core.ensure_session_started(&id).await?;
    core.broadcast(&id, &prompt).await?;
    Ok(session.into())
}

#[tauri::command]
#[specta::specta]
pub async fn get_session(
    storage: tauri::State<'_, Arc<Storage>>,
    session_id: String,
) -> Result<Option<SessionInfo>, AppError> {
    storage
        .get_session(&session_id)
        .await
        .map(|opt| opt.map(Into::into))
        .map_err(|e| AppError::DbError(e.to_string()))
}

/// Uncommitted-work probe for the close-confirm dialog: how many entries
/// `git status --porcelain` reports in the session's working tree. `has_repo`
/// is false for a repo-less session (nothing to warn about). Best-effort —
/// never an error path that could block closing. Return struct uses snake_case
/// (mirrors `SessionInfo`); the React side reads `has_repo` / `dirty_count`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SessionDirty {
    pub has_repo: bool,
    pub dirty_count: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn check_session_dirty(
    storage: tauri::State<'_, Arc<Storage>>,
    session_id: String,
) -> Result<SessionDirty, AppError> {
    let repo = storage
        .get_session(&session_id)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?
        .and_then(|s| s.working_repo_path)
        .filter(|p| !p.is_empty());
    Ok(match repo {
        Some(path) => SessionDirty {
            has_repo: true,
            // Sync git call off the async executor (worktree.rs convention).
            dirty_count: tokio::task::spawn_blocking(move || {
                crate::core::worktree::working_tree_dirty_count(std::path::Path::new(&path))
            })
            .await
            .unwrap_or(0),
        },
        None => SessionDirty {
            has_repo: false,
            dirty_count: 0,
        },
    })
}

/// C1: the kept-worktree path for a closed worktree-session, if its isolated
/// worktree still exists on disk. `close_session` keeps (never force-removes) a
/// dirty worktree, so its presence after close ⇒ uncommitted work was left
/// there. `None` for a direct-mode session or a clean worktree that was removed.
/// Lets the Archive surface "work was kept here" for recovery.
#[tauri::command]
#[specta::specta]
pub async fn session_worktree_kept(
    core: tauri::State<'_, Arc<CoreAppState>>,
    storage: tauri::State<'_, Arc<Storage>>,
    session_id: String,
) -> Result<Option<String>, AppError> {
    let Some(session) = storage
        .get_session(&session_id)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?
    else {
        return Ok(None);
    };
    let Some(base_repo) = session.base_repo_path else {
        return Ok(None); // direct-mode session — no worktree
    };
    let kept = crate::core::worktree::session_worktree_path(
        &core.paths.data_dir,
        &session_id,
        std::path::Path::new(&base_repo),
    )
    .filter(|p| p.exists())
    .map(|p| p.to_string_lossy().into_owned());
    Ok(kept)
}

#[tauri::command]
#[specta::specta]
pub async fn list_sessions(
    storage: tauri::State<'_, Arc<Storage>>,
) -> Result<Vec<SessionInfo>, AppError> {
    storage
        .list_active_sessions_with_preview()
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::DbError(e.to_string()))
}

/// All closed sessions (just-closed + archived), most-recently-closed first.
/// Backs the Settings → Archive tab.
#[tauri::command]
#[specta::specta]
pub async fn list_closed_sessions(
    storage: tauri::State<'_, Arc<Storage>>,
) -> Result<Vec<SessionInfo>, AppError> {
    storage
        .list_closed_sessions()
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::DbError(e.to_string()))
}

/// Spawn (or re-spawn) the agent subprocesses for an existing session row.
/// Idempotent — `core::AppState::ensure_session_started` is a no-op if the
/// session is already live. Mirrors the click-to-respawn flow:
/// frontend SessionView calls this on mount so a reopened bot-hq window
/// brings Brian + Rain back via `claude --resume <uuid>`.
#[tauri::command]
#[specta::specta]
pub async fn respawn_session(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
) -> Result<(), AppError> {
    core.ensure_session_started(&session_id).await?;
    Ok(())
}

/// Hard-cancel a session's in-flight turn (the Stop button — interrupt
/// redesign, Batch 3 + 3.1 Part 1). Kills both agents' current turn; the
/// session returns to `Idle` (the chat input unlocks) and the next message
/// respawns each agent with `--resume`, restoring its prior context. If HANDS
/// is mid an atomic op (`git commit`/`git push`/migration), the kill is
/// DEFERRED until the op completes (≤ ~8s, then force-killed) so the working
/// tree isn't left half-written — the command returns immediately and a
/// detached task does the kill once the op clears. No-op if the session isn't
/// live.
#[tauri::command]
#[specta::specta]
pub async fn cancel_session_turn(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
) -> Result<(), AppError> {
    use crate::core::state::CancelOutcome;
    match core.cancel_session_turn(&session_id).await? {
        CancelOutcome::Done => {}
        CancelOutcome::Deferred(flag) => {
            // HANDS is mid an atomic op. Poll the flag lock-free until it clears,
            // then kill — with a hard ~8s cap so a hung op still gets
            // force-interrupted. Detached so the command returns immediately and
            // the UI keeps showing "Cancelling…" for the whole window. We own an
            // `Arc<CoreAppState>` (not the `&self` core method) so the task can
            // re-acquire `sessions` to kill without holding it during the poll.
            let core = core.inner().clone();
            tokio::spawn(async move {
                let deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_secs(8);
                while flag.load(std::sync::atomic::Ordering::Acquire) {
                    if tokio::time::Instant::now() >= deadline {
                        tracing::warn!(
                            %session_id,
                            "cancel: atomic-op deferral hit ~8s cap — force-killing"
                        );
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                core.cancel_kill_now(&session_id).await;
            });
        }
    }
    Ok(())
}

/// Force-restart a live session's agents so they pick up a Claude-config change
/// (overrides + inherited settings are read at spawn). Unlike `respawn_session`
/// this is NOT a no-op on a healthy session — it evicts and re-spawns. Agents
/// resume their prior conversation via `--resume`.
#[tauri::command]
#[specta::specta]
pub async fn restart_session(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
) -> Result<(), AppError> {
    core.restart_session(&session_id).await?;
    Ok(())
}

/// Rename a session (inline edit in the SessionView header). Blank titles are
/// rejected — an empty header is indistinguishable from a render bug.
#[tauri::command]
#[specta::specta]
pub async fn rename_session(
    storage: tauri::State<'_, Arc<Storage>>,
    session_id: String,
    title: String,
) -> Result<(), AppError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(AppError::Validation("title cannot be empty".into()));
    }
    storage
        .rename_session(&session_id, title)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))
}

/// Read the current IPAV phase for a session. Returns one of "investigate" /
/// "plan" / "apply" / "verify", or `None` if the session isn't live (IPAV
/// state is in-memory only — restart loses it). Frontend SessionView header
/// uses this for the initial phase chip; subsequent updates come from the
/// `session:phase_changed` Tauri event.
#[tauri::command]
#[specta::specta]
pub async fn get_session_phase(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
) -> Result<Option<String>, AppError> {
    Ok(core
        .current_phase(&session_id)
        .await
        .map(|p| p.name().to_ascii_lowercase()))
}

/// Close a session from the UI. Delegates to `core.close_session`, which is
/// the single source of truth for closing: it removes the live handle, KILLS
/// the brian/rain subprocesses, and marks the row closed/archived in storage.
/// The previous version called `storage.close_session` directly, so it set
/// `closed_at` but left the subprocesses running — a session that "closed" in
/// the DB yet kept taking turns. Routing through core fixes that.
#[tauri::command]
#[specta::specta]
pub async fn close_session(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    archive: bool,
) -> Result<(), AppError> {
    core.close_session(&session_id, archive).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (Arc<Storage>, Arc<SignalingBridge>) {
        let s = Arc::new(Storage::memory().await.unwrap());
        let b = SignalingBridge::new();
        (s, b)
    }

    #[tokio::test]
    async fn placement_repo_less_and_blank_are_direct_none() {
        let (storage, _b) = setup().await;
        let dd = std::path::Path::new("/dd");
        assert_eq!(
            resolve_session_placement(&storage, dd, "s-1", None, None).await,
            (None, None)
        );
        assert_eq!(
            resolve_session_placement(&storage, dd, "s-1", Some("  ".into()), None).await,
            (None, None)
        );
    }

    #[tokio::test]
    async fn placement_non_git_dir_is_direct() {
        let (storage, _b) = setup().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("plain");
        std::fs::create_dir(&repo).unwrap();
        let got = resolve_session_placement(
            &storage,
            std::path::Path::new("/dd"),
            "s-1",
            Some(repo.to_string_lossy().into_owned()),
            None,
        )
        .await;
        assert_eq!(got.0.as_deref(), repo.to_str());
        assert_eq!(got.1, None);
    }

    #[tokio::test]
    async fn placement_git_repo_defaults_to_worktree_and_honors_optout() {
        let (storage, _b) = setup().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("myproj");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let repo_s = repo.to_string_lossy().into_owned();
        let dd = std::path::Path::new("/dd");

        // Default (setting unset) → worktree placement, basename preserved.
        let (working, base) =
            resolve_session_placement(&storage, dd, "s-wt", Some(repo_s.clone()), None).await;
        assert_eq!(base.as_deref(), Some(repo_s.as_str()));
        let w = working.unwrap();
        assert!(w.contains(".local/worktrees/s-wt"), "got {w}");
        assert!(w.ends_with("myproj"), "got {w}");

        // Explicit per-session opt-out wins.
        let got =
            resolve_session_placement(&storage, dd, "s-d", Some(repo_s.clone()), Some(false))
                .await;
        assert_eq!(got, (Some(repo_s.clone()), None));

        // worktree_default = "0" flips the unset default to direct.
        storage
            .set_setting(crate::storage::WORKTREE_DEFAULT_KEY, "0")
            .await
            .unwrap();
        let got = resolve_session_placement(&storage, dd, "s-e", Some(repo_s.clone()), None).await;
        assert_eq!(got, (Some(repo_s.clone()), None));
        // …and an explicit opt-IN overrides the "0" setting.
        let (_, base) =
            resolve_session_placement(&storage, dd, "s-f", Some(repo_s.clone()), Some(true)).await;
        assert_eq!(base.as_deref(), Some(repo_s.as_str()));
    }

    #[tokio::test]
    async fn create_and_get_session_roundtrip() {
        let (storage, bridge) = setup().await;
        storage
            .create_session("s1", "Hello", Some("/tmp/repo"))
            .await
            .unwrap();
        bridge
            .register_session("s1".to_string(), Some("bot-hq".to_string()))
            .await;
        let fetched = storage.get_session("s1").await.unwrap().unwrap();
        let info: SessionInfo = fetched.into();
        assert_eq!(info.id, "s1");
        assert_eq!(info.title, "Hello");
        assert_eq!(info.working_repo_path.as_deref(), Some("/tmp/repo"));
        assert!(!info.archived);
    }

    #[tokio::test]
    async fn list_sessions_returns_active_only() {
        let (storage, _bridge) = setup().await;
        storage.create_session("s1", "A", None).await.unwrap();
        storage.create_session("s2", "B", None).await.unwrap();
        storage.close_session("s2", true).await.unwrap();

        let list = storage.list_active_sessions().await.unwrap();
        let ids: Vec<String> = list.into_iter().map(|s| s.id).collect();
        assert!(ids.contains(&"s1".to_string()));
    }
}
