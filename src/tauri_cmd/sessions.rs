//! Session lifecycle commands.

use crate::core::session::{resolve_session_project, ProjectProvenance};
use crate::core::AppState as CoreAppState;
use crate::signaling::SignalingBridge;
use crate::storage::{Author, Session, SessionWithPreview, Storage};
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
    /// rc3: the participants the New Session dialog chose, in turn order.
    ///
    /// `None` is the pre-rc3 path and behaves EXACTLY as before — no roster is
    /// written at create and `ensure_session_roster` seeds the default pair at
    /// spawn. Every non-dialog caller (the external driver's `open_session`,
    /// `dispatch_session`, the plugin proxy) is on that path and is untouched.
    pub participants: Option<Vec<ParticipantPick>>,
}

/// One row of the dialog's participant list: a role, and optionally a model
/// that overrides the role's default (rc3 **D8**).
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantPick {
    pub role_id: i64,
    pub model_id: Option<String>,
}

/// How many participants a session can be created with **today**.
///
/// Not a design limit — the runtime limit, and it is enforced here because the
/// alternative is a dialog that offers a third row and produces a participant
/// with no process behind it. `core::session::spawn_session_handle` spawns two
/// literally-named agents and finds their rows with
/// `roster_row(&roster, "brian")` / `"rain"`; a third row would be scheduled by
/// the ring, never woken, and the consensus halt would then wait forever on a
/// vote it can never cast. Raise this when spawning stops being name-keyed.
pub const MAX_SESSION_PARTICIPANTS: usize = 2;

/// The slug and display name a participant takes, by turn slot.
///
/// **What the user picks is the ROLE each slot plays; the two runtime
/// identities are not yet theirs to name.** Both halves are forced, and by
/// different things:
///
/// * the **slug** is the wire. `spawn_session_handle` finds a participant's row
///   with `roster_row(&roster, "brian")` / `"rain"`, and `insert_message`'s
///   dual-write resolves `participant_id` by matching it against the legacy
///   `author` string. A row slugged anything else has no process behind it.
/// * the **display name** is what layer 2 calls the participant
///   (`RosterFacts::display_name`, rendered as `**Brian** (HANDS)`), and layer 3
///   — the role prose migration 0046 seeded from `BRIAN_ROLE` / `RAIN_ROLE` —
///   opens with `You are **Brian**`. Naming the participant after its role
///   would put `**HANDS** (HANDS)` two paragraphs from `You are **Brian**` in
///   one prompt.
///
/// Both lift together when the name removal
/// (`docs/plans/2026-08-11-agent-name-removal.md`) takes the names out of the
/// prose and the spawn path.
const PARTICIPANT_SLOTS: [(&str, &str); MAX_SESSION_PARTICIPANTS] =
    [("brian", "Brian"), ("rain", "Rain")];

/// What a resolved participant list means for the columns spawn still reads.
///
/// The reframe in one struct: the dialog now picks participants, and the values
/// that used to come from the dialog's "Disable Rain" checkbox and its two
/// model selects are DERIVED from those picks instead. They land in the same
/// `sessions` columns as before, so spawn behaves identically — the source
/// moved, the behaviour did not.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedRoster {
    pub drafts: Vec<crate::storage::ParticipantDraft>,
    /// `sessions.rain_enabled`: two participants is a duo, one is solo — the
    /// same flag the checkbox set, read off the roster's length.
    pub rain_enabled: bool,
    /// `sessions.brian_model_id` / `rain_model_id`. **Load-bearing, not
    /// bookkeeping:** `spawn_session_handle` resolves each agent's model from
    /// these columns, not from the roster, so a per-participant model pick that
    /// only reached `session_participants.model_id` would be a picker that
    /// changes nothing.
    pub brian_model_id: Option<String>,
    pub rain_model_id: Option<String>,
}

/// Turn the dialog's picks into a roster, refusing the ones that cannot run.
///
/// Split out of the command body because a `#[tauri::command]` takes
/// `tauri::State`, which cannot be constructed in a unit test — the convention
/// `roles.rs::load_roles` already follows.
pub(crate) async fn resolve_participant_picks(
    storage: &Storage,
    picks: &[ParticipantPick],
    options: &SessionCreateOptions,
) -> Result<ResolvedRoster, AppError> {
    if picks.is_empty() {
        return Err(AppError::Validation(
            "a session needs at least one participant".into(),
        ));
    }
    if picks.len() > MAX_SESSION_PARTICIPANTS {
        return Err(AppError::Validation(format!(
            "a session can run at most {MAX_SESSION_PARTICIPANTS} participants today, \
             not {}",
            picks.len()
        )));
    }
    // The effort/ultracode picks are still per-SLOT in the dialog, because they
    // are still per-slot in the columns spawn reads.
    let per_slot_effort = [
        (options.brian_effort.clone(), options.brian_ultracode),
        (options.rain_effort.clone(), options.rain_ultracode),
    ];
    let mut drafts = Vec::with_capacity(picks.len());
    let mut models: Vec<Option<String>> = Vec::with_capacity(picks.len());
    let mut any_active = false;
    for (slot, pick) in picks.iter().enumerate() {
        let role = storage
            .role_by_id(pick.role_id)
            .await
            .map_err(|e| AppError::DbError(e.to_string()))?
            .ok_or_else(|| AppError::Validation(format!("role {} does not exist", pick.role_id)))?;
        if role.archived {
            // The picker lists live roles only, so this is a stale dialog —
            // the role was archived between the list read and Create.
            return Err(AppError::Validation(format!(
                "role {} is archived",
                role.display_name
            )));
        }
        if role.participation_mode == "on_demand" {
            // rc3 D1: an `on_demand` participant wakes on a user `@mention`,
            // and that is not built. Inviting one produces a participant the
            // ring skips and nothing ever wakes.
            return Err(AppError::Validation(format!(
                "role {} is on-demand, and waking one is not built yet",
                role.display_name
            )));
        }
        any_active |= role.participation_mode == "active";
        let (effort, ultracode) = per_slot_effort[slot].clone();
        // D8's fallback, resolved ONCE: the same value goes into the
        // participant row and into the `sessions` column spawn reads, so the
        // two cannot disagree about which model this participant runs.
        let model_id = pick.model_id.clone().or_else(|| role.default_model_id.clone());
        let (slug, display_name) = PARTICIPANT_SLOTS[slot];
        drafts.push(crate::storage::ParticipantDraft {
            slug: slug.to_string(),
            display_name: display_name.to_string(),
            role_id: role.id,
            model_id: model_id.clone(),
            effort,
            ultracode,
        });
        models.push(model_id);
    }
    if !any_active {
        // Every participant an observer is a session that can never take a
        // turn: the ring is empty, so `all_active_voted_done` is vacuously
        // true and the session is "finished" before it starts.
        return Err(AppError::Validation(
            "at least one participant has to be in the turn rotation".into(),
        ));
    }
    Ok(ResolvedRoster {
        rain_enabled: picks.len() >= 2,
        brian_model_id: models.first().cloned().flatten(),
        rain_model_id: models.get(1).cloned().flatten(),
        drafts,
    })
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
    // rc3: resolve the picked roster BEFORE the session row exists, so a
    // refused pick (an archived role, an on-demand one, too many rows) leaves
    // nothing behind to clean up.
    let roster = match options.participants.as_deref() {
        Some(picks) => Some(resolve_participant_picks(storage, picks, &options).await?),
        None => None,
    };
    // With a roster, the solo/duo flag and both model columns are DERIVED from
    // it — same columns, same spawn, a different source. Without one this is
    // the pre-rc3 path, argument for argument.
    let (rain_enabled, brian_model_id, rain_model_id) = match &roster {
        Some(r) => (
            r.rain_enabled,
            r.brian_model_id.clone(),
            r.rain_model_id.clone(),
        ),
        None => (rain_enabled.unwrap_or(true), brian_model_id, rain_model_id),
    };
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
            rain_enabled,
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
    // The picked roster, written before the background spawn below reaches
    // `ensure_session_roster` — which seeds the default pair only into a
    // session that has none, so the two never both fire.
    if let Some(roster) = &roster {
        storage
            .seed_session_roster(&id, &roster.drafts)
            .await
            .map_err(|e| AppError::DbError(format!("{e:#}")))?;
    }
    core.bridge.register_session(id.clone(), project).await;
    // Re-fetch so the returned SessionInfo reflects the persisted config.
    let session = storage
        .get_session(&id)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?
        .ok_or_else(|| AppError::DbError("session vanished after create".into()))?;
    // Spawn the duo in the background so the session primes (CL-opener nudge)
    // without the user having to open it. Not awaited: worktree
    // materialization can take seconds and the create dialog shouldn't block
    // on it. `ensure_session_started` is idempotent + spawn-gate-serialized,
    // so the SessionView mount's `respawn_session` stays a harmless no-op and
    // doubles as the retry path if this background spawn fails.
    let core_bg = Arc::clone(core.inner());
    let spawn_id = id.clone();
    tokio::spawn(async move {
        if let Err(e) = core_bg.ensure_session_started(&spawn_id).await {
            tracing::warn!(session_id = %spawn_id, error = ?e, "post-create background spawn failed");
        }
    });
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
    dispatch_session_inner(&core, &storage, &bridge, id, title, project, repo_path, prompt, None)
        .await
}

/// Testable/plugin-reachable body of [`dispatch_session`] (the command is a
/// thin `State`-unwrapping shim, matching the plugins-command pattern). Also
/// the target of the plugin proxy's `spawn_session` arm — which is why it
/// takes plain refs, not `tauri::State`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_session_inner(
    core: &CoreAppState,
    storage: &Storage,
    bridge: &SignalingBridge,
    id: String,
    title: String,
    project: Option<String>,
    repo_path: Option<String>,
    prompt: String,
    rain_override: Option<bool>,
) -> Result<SessionInfo, AppError> {
    // No create dialog on this path → placement comes from the configured
    // default (worktree_default). solo/duo is `rain_override` when the caller
    // pins it (the `plugin_sessions` create arm forces solo unless the plugin
    // asks for a duo), else the configured default (rain_disabled_default).
    let (working, base) = resolve_session_placement(
        storage,
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
    // Honor `rain_override` when the caller pins solo/duo, else the user's
    // configured default. Without this the DB default (`rain_enabled=1`)
    // always spawned the duo regardless of `rain_disabled_default`. Models
    // stay NULL = per-agent defaults, same as the dialog's "(agent default)".
    let rain_enabled = match rain_override {
        Some(v) => v,
        None => storage.default_rain_enabled().await,
    };
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

/// Current runtime state (derived activity + per-agent health) for every LIVE
/// session — a snapshot the frontend BACKFILLS its event-driven activity/health
/// stores from on mount. Those stores are seeded only by `session:activity` /
/// `session:agent_health` events, which fire on transitions and can be missed
/// during the respawn window before the React listeners mount (Bug C: footer /
/// tiles / input-indicator left stale until the next transition). snake_case
/// return (mirrors `SessionInfo`); React reads `session_id`/`activity`/
/// `brian_health`/`rain_health`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SessionRuntime {
    pub session_id: String,
    pub activity: String,
    /// Per-agent busy flags (the derived `activity` collapses them) so the chat
    /// input can label which agent is working after a backfill, not just guess.
    pub brian_busy: bool,
    pub rain_busy: bool,
    pub brian_health: Option<String>,
    pub rain_health: Option<String>,
    /// Idle-unflagged attention state ("idle_unflagged" or None = clear).
    /// Seeds the "needs direction" chip on mount; live updates arrive via
    /// `session:attention`.
    pub attention: Option<String>,
    /// declare_working reason while HANDS has background work declared
    /// (None = clear). Seeds the WORKING badge; live via `session:working`.
    pub working: Option<String>,
    /// Peer-forward router liveness (duo only). `None` = solo, or never reported
    /// (assume alive — the event fires only on change). Seeds the UI router dot.
    pub router_alive: Option<bool>,
}

#[tauri::command]
#[specta::specta]
pub async fn get_session_runtime(
    core: tauri::State<'_, Arc<CoreAppState>>,
) -> Result<Vec<SessionRuntime>, AppError> {
    let sessions = core.sessions.lock().await;
    let mut out = Vec::with_capacity(sessions.len());
    for (id, handle) in sessions.iter() {
        out.push(SessionRuntime {
            session_id: id.clone(),
            activity: handle.activity.current().as_str().to_string(),
            brian_busy: handle.activity.is_busy(Author::Brian),
            rain_busy: handle.activity.is_busy(Author::Rain),
            brian_health: core.bridge.current_agent_health(id, "brian"),
            rain_health: core.bridge.current_agent_health(id, "rain"),
            attention: core.bridge.current_session_attention(id),
            working: core.bridge.current_session_working(id),
            router_alive: core.bridge.current_router_health(id),
        });
    }
    Ok(out)
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

/// Cancel a session's in-flight turn (the Stop button — interrupt redesign,
/// Batch 3 + 3.1, now interrupt-first). Sends a `control_request` interrupt to
/// abort the turn while KEEPING the process alive (warm cache, no `--resume`
/// respawn); if an agent doesn't honor it within ~2s it's SIGKILLed as a
/// fallback. The session returns to `Idle` (the chat input unlocks). If HANDS is
/// mid an atomic op (`git commit`/`git push`/migration), the interrupt is
/// DEFERRED until the op completes (≤ ~8s cap) so the working tree isn't left
/// half-written. The command returns immediately and a detached task drives the
/// escalation. No-op if the session isn't live.
#[tauri::command]
#[specta::specta]
pub async fn cancel_session_turn(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
) -> Result<(), AppError> {
    use crate::core::state::CancelOutcome;
    // Stamped at the top so `cancel_events.pressed_at` is when the USER acted,
    // not when the escalation finished. The gap between the two is precisely
    // what a user experiences as "Stop didn't do anything".
    let pressed_at = crate::storage::now_utc();
    match core.cancel_session_turn(&session_id).await? {
        CancelOutcome::Done => {}
        CancelOutcome::Interrupting => {
            // The common path: interrupt both agents and drive the ~2s SIGKILL
            // escalation off-thread. Detached so the command returns immediately
            // and the UI shows "Cancelling…" for the window. We own an
            // `Arc<CoreAppState>` (not the `&self` core method) so the task can
            // re-acquire `sessions` without holding it across the wait.
            let core = core.inner().clone();
            tokio::spawn(async move {
                core.interrupt_then_escalate(&session_id, &pressed_at, 0, false)
                    .await;
            });
        }
        CancelOutcome::Deferred(flag) => {
            // HANDS is mid an atomic op. Poll the flag lock-free until it clears,
            // THEN interrupt+escalate — with a hard ~8s cap so a hung op still
            // gets cancelled (the SIGKILL fallback reaps it). Detached so the
            // command returns immediately and the UI keeps showing "Cancelling…".
            let core = core.inner().clone();
            tokio::spawn(async move {
                let started = tokio::time::Instant::now();
                let deadline = started + std::time::Duration::from_secs(8);
                let mut capped = false;
                while flag.load(std::sync::atomic::Ordering::Acquire) {
                    if tokio::time::Instant::now() >= deadline {
                        tracing::warn!(
                            %session_id,
                            "cancel: atomic-op deferral hit ~8s cap — interrupting now"
                        );
                        capped = true;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                // Recorded because this window is the leading candidate for
                // "Stop kept working": it is HANDS-only (the flag is set for
                // git commit/push/migrate) and delays the interrupt by up to 8s
                // before anything is even sent.
                let deferred_ms = started.elapsed().as_millis() as u64;
                core.interrupt_then_escalate(&session_id, &pressed_at, deferred_ms, capped)
                    .await;
            });
        }
    }
    Ok(())
}

/// Resume a paused session (the Paused bar's Resume button). Releases the pause
/// latch by broadcasting a host-authored resume notice; held peer-forwards and
/// OOB answer wakes flush in behind it, and the post-Stop reconcile directive
/// rides the same message. No-op if the session isn't live or isn't paused.
#[tauri::command]
#[specta::specta]
pub async fn resume_session(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
) -> Result<(), AppError> {
    core.resume_session(&session_id).await?;
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

    // ---- rc3: the New Session dialog picks participants ------------------

    fn pick(role_id: i64, model_id: Option<&str>) -> ParticipantPick {
        ParticipantPick {
            role_id,
            model_id: model_id.map(str::to_string),
        }
    }

    async fn role_with_mode(storage: &Storage, name: &str, mode: &str) -> i64 {
        storage
            .create_role(&crate::storage::RoleDraft {
                display_name: name.into(),
                capabilities: "[\"read_channel\"]".into(),
                participation_mode: mode.into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn picks_derive_the_solo_flag_and_the_columns_spawn_reads() {
        // The reframe, at the layer that does it: the dialog's "Disable Rain"
        // checkbox and its two model selects are GONE, and the values they used
        // to write are read off the participant list instead. Spawn still reads
        // `sessions.rain_enabled` / `brian_model_id` / `rain_model_id`, so a
        // pick that did not reach those columns would be a picker that changes
        // nothing.
        let (storage, _b) = setup().await;
        let hands = storage.role_by_slug("hands").await.unwrap().unwrap();
        let eyes = storage.role_by_slug("eyes").await.unwrap().unwrap();
        let options = SessionCreateOptions {
            brian_effort: Some("max".into()),
            rain_effort: Some("low".into()),
            brian_ultracode: Some(true),
            ..Default::default()
        };

        let solo = resolve_participant_picks(&storage, &[pick(hands.id, Some("opus"))], &options)
            .await
            .unwrap();
        assert!(!solo.rain_enabled, "one participant is a solo session");
        assert_eq!(solo.brian_model_id.as_deref(), Some("opus"));
        assert_eq!(solo.rain_model_id, None);
        assert_eq!(solo.drafts.len(), 1);
        assert_eq!(solo.drafts[0].slug, "brian", "slot 0 is the handle spawn looks up");
        assert_eq!(solo.drafts[0].effort.as_deref(), Some("max"));
        assert_eq!(solo.drafts[0].ultracode, Some(true));

        let duo = resolve_participant_picks(
            &storage,
            &[pick(hands.id, Some("opus")), pick(eyes.id, Some("sonnet"))],
            &options,
        )
        .await
        .unwrap();
        assert!(duo.rain_enabled, "two participants is the duo");
        assert_eq!(duo.rain_model_id.as_deref(), Some("sonnet"));
        let slugs: Vec<&str> = duo.drafts.iter().map(|d| d.slug.as_str()).collect();
        assert_eq!(slugs, ["brian", "rain"], "in the order they were picked");
        assert_eq!(duo.drafts[1].effort.as_deref(), Some("low"), "slot 1 takes Rain's knobs");
    }

    #[tokio::test]
    async fn a_pick_without_a_model_takes_the_roles_default() {
        // D8's fallback has to be applied HERE and not only in storage: the
        // session column spawn reads is written from this value, so leaving it
        // NULL would silently spawn the agent-config model instead of the one
        // the Roles tab names.
        let (storage, _b) = setup().await;
        let role = storage
            .create_role(&crate::storage::RoleDraft {
                display_name: "Reviewer".into(),
                capabilities: "[\"read_channel\"]".into(),
                participation_mode: "active".into(),
                default_model_id: Some("role-default".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        let inherited =
            resolve_participant_picks(&storage, &[pick(role.id, None)], &Default::default())
                .await
                .unwrap();
        assert_eq!(inherited.brian_model_id.as_deref(), Some("role-default"));
        assert_eq!(inherited.drafts[0].model_id.as_deref(), Some("role-default"));

        let overridden =
            resolve_participant_picks(&storage, &[pick(role.id, Some("chosen"))], &Default::default())
                .await
                .unwrap();
        assert_eq!(overridden.brian_model_id.as_deref(), Some("chosen"));
        assert_eq!(overridden.drafts[0].model_id.as_deref(), Some("chosen"));
    }

    #[tokio::test]
    async fn a_roster_the_runtime_cannot_run_is_refused() {
        let (storage, _b) = setup().await;
        let hands = storage.role_by_slug("hands").await.unwrap().unwrap();
        let opts = SessionCreateOptions::default();

        assert!(
            resolve_participant_picks(&storage, &[], &opts).await.is_err(),
            "a session with nobody in it"
        );
        // One more than the two agents `spawn_session_handle` spawns. Offering
        // a third row would produce a participant the ring schedules and
        // nothing ever wakes — and the consensus halt would then wait forever
        // on its vote.
        let three = vec![pick(hands.id, None), pick(hands.id, None), pick(hands.id, None)];
        assert!(
            resolve_participant_picks(&storage, &three, &opts).await.is_err(),
            "more participants than the runtime can spawn"
        );
        assert!(
            resolve_participant_picks(&storage, &[pick(9999, None)], &opts).await.is_err(),
            "a role id that names nothing"
        );

        // rc3 D1: an on-demand participant wakes on a user @mention, and that
        // is not built. Inviting one is inviting a participant nothing wakes.
        // Paired with an ACTIVE participant on purpose — a lone on-demand
        // roster is also caught by the empty-rotation check below, so it would
        // not tell us whether this rule exists at all.
        let on_demand = role_with_mode(&storage, "Specialist", "on_demand").await;
        assert!(
            resolve_participant_picks(&storage, &[pick(hands.id, None), pick(on_demand, None)], &opts)
                .await
                .is_err(),
            "on-demand is not offered anywhere yet"
        );

        // An all-observer roster leaves the rotation empty, which
        // `all_active_voted_done` reports as vacuously DONE — a session that is
        // finished before it starts.
        let observer = role_with_mode(&storage, "Watcher", "observer").await;
        assert!(
            resolve_participant_picks(&storage, &[pick(observer, None)], &opts).await.is_err(),
            "nobody in the turn rotation"
        );
        assert!(
            resolve_participant_picks(&storage, &[pick(hands.id, None), pick(observer, None)], &opts)
                .await
                .is_ok(),
            "an observer alongside an active participant is a legal roster"
        );

        // The picker lists live roles only, so an archived pick means the
        // dialog was open while the Roles tab archived it.
        let archived = role_with_mode(&storage, "Retired", "active").await;
        storage.set_role_archived(archived, true).await.unwrap();
        assert!(
            resolve_participant_picks(&storage, &[pick(archived, None)], &opts).await.is_err(),
            "a role the user removed"
        );
    }
}
