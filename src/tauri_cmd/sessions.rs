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
    /// rc3: the participants the New Session dialog chose, in turn order.
    ///
    /// `None` is the pre-rc3 path and behaves EXACTLY as before — no roster is
    /// written at create and `ensure_session_roster` seeds the default pair at
    /// spawn. Every non-dialog caller (the external driver's `open_session`,
    /// the plugin proxy) is on that path and is untouched.
    pub participants: Option<Vec<ParticipantPick>>,
}

/// One row of the dialog's participant list: a role, optionally a model that
/// overrides the role's default (rc3 **D8**), and that row's own spawn knobs
/// (rc3 **D12**).
///
/// **There is no name field, and that is rc3 D10.** The slug and the display
/// name are derived from the role by `Storage::seed_session_roster`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantPick {
    pub role_id: i64,
    pub model_id: Option<String>,
    /// `None` = inherit. Per participant since rc3 D12; the dialog used to
    /// carry one effort select per AGENT, in two fixed blocks.
    pub effort: Option<String>,
    pub ultracode: Option<bool>,
    /// The palette entry the user picked for this row, by NAME ("Cyan"), or
    /// `None` to take the rotation (rc3 **D20**).
    pub color: Option<String>,
    /// The name the user typed for this row, or `None`/blank to take the
    /// ordinal (rc3 **D20**, migration 0053).
    pub label: Option<String>,
}

/// How many participants a session can be created with.
///
/// **Not the runtime limit any more.** This used to be 2 because
/// `spawn_session_handle` spawned two literally-named agents, so a third row
/// would be scheduled by the ring, never woken, and the consensus halt would
/// wait forever on a vote nobody could cast. rc3 D10 made spawn iterate the
/// roster, so the cap is now only a sanity bound on what one session can
/// usefully run — every participant is a claude-code subprocess with its own
/// context window and its own bill.
pub const MAX_SESSION_PARTICIPANTS: usize = 8;

/// What a resolved participant list means beyond the roster rows themselves.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedRoster {
    pub drafts: Vec<crate::storage::ParticipantDraft>,
    /// `sessions.rain_enabled`, which is now only a solo/duo BOOKKEEPING flag:
    /// spawn reads the roster, not this column. It is still written because
    /// `SessionInfo.rain_enabled` is part of the frozen frontend shape.
    pub rain_enabled: bool,
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
            "a session can run at most {MAX_SESSION_PARTICIPANTS} participants, not {}",
            picks.len()
        )));
    }
    let mut drafts = Vec::with_capacity(picks.len());
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
        // An `on_mention` role is a legal invite as of rc3 D17: it is spawned,
        // sits out the rotation, and takes a turn when the user names it. It
        // does not count towards `any_active` — a roster of nothing but
        // summonable participants has an empty ring, which is the check below.
        any_active |= role.participation_mode == "active";
        // rc3 D12: the knobs come off the participant's own row. The two
        // legacy per-agent fields on `SessionCreateOptions` still apply to
        // slots 0 and 1 when the pick leaves them unset, so a caller that has
        // not been updated to the per-row form keeps working unchanged.
        let legacy = match slot {
            0 => (options.brian_effort.clone(), options.brian_ultracode),
            1 => (options.rain_effort.clone(), options.rain_ultracode),
            _ => (None, None),
        };
        drafts.push(crate::storage::ParticipantDraft {
            role_id: role.id,
            // D8's fallback is applied again inside `seed_session_roster`;
            // resolving it here too keeps the value the dialog SAW and the value
            // stored identical when the role's default changes between the two.
            model_id: pick.model_id.clone().or_else(|| role.default_model_id.clone()),
            effort: pick.effort.clone().or(legacy.0),
            ultracode: pick.ultracode.or(legacy.1),
            color: pick.color.clone(),
            label: pick.label.clone(),
        });
    }
    if !any_active {
        // A roster with nobody in the rotation is a session that can never take
        // a turn: the ring is empty, so `all_active_voted_done` is vacuously
        // true and the session is "finished" before it starts.
        //
        // **Live again as of rc3 D17**, which lifted the blanket refusal of
        // `on_mention` picks. An all-summonable roster is the reachable way to
        // build one — every participant waiting to be named, nobody to name them
        // into a conversation that has not started.
        return Err(AppError::Validation(
            "at least one participant has to be in the turn rotation".into(),
        ));
    }
    Ok(ResolvedRoster {
        rain_enabled: picks.len() >= 2,
        drafts,
    })
}

/// One participant of a session, as the UI names it (rc3 **D10**).
///
/// The two display halves are returned SEPARATELY, and the join is the display
/// rule — `role_display_name · model_display_name`, falling back to the model
/// alone and then to the slug. Returning them apart lets the UI style them
/// differently; `storage::participant_display_name` is the backend's
/// implementation of the same rule for the prompt.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct ParticipantView {
    pub id: i64,
    /// The internal key — the `@mention` handle and the `messages.author`
    /// string. **Never displayed.** It is the tiebreaker between two
    /// participants of one role on one model, whose display names are identical
    /// by construction.
    pub slug: String,
    /// `null` when the role row is gone.
    pub role_display_name: Option<String>,
    /// `null` when the participant has no model, or the model row is gone.
    pub model_display_name: Option<String>,
    pub turn_position: i64,
    /// `active` | `on_mention` (which create refuses today — see rc3 D17).
    pub participation_mode: String,
    pub enabled: bool,
    /// The user's colour pick, by palette NAME ("Cyan"), or `null` to take the
    /// rotation the UI assigns by roster position (rc3 **D20**).
    pub color: Option<String>,
    /// The user's name for this participant, or `null` to take the ordinal
    /// (rc3 **D20**, migration 0053). `participant_display_name` is what joins
    /// this with the role and the model; the frontend must not re-derive it.
    pub label: Option<String>,
    /// This participant's effort override (rc3 D12), or `null` to inherit.
    ///
    /// The New Session dialog writes both this and `ultracode` per row and
    /// nothing could read them back, so the session view had no way to show
    /// what a running participant was actually spawned with. Read off the
    /// participant row, where spawn reads them from.
    pub effort: Option<String>,
    /// This participant's ultracode override (rc3 D12), or `null` to inherit.
    pub ultracode: Option<bool>,
}

/// A session's roster in turn order — the read side of rc3 D10.
///
/// This is what replaces every `brian_*` / `rain_*` pair the session view read:
/// there is no fixed number of participants any more, so the UI has to ask.
#[tauri::command]
#[specta::specta]
pub async fn list_session_participants(
    storage: tauri::State<'_, Arc<Storage>>,
    session_id: String,
) -> Result<Vec<ParticipantView>, AppError> {
    participant_views(&storage, &session_id).await
}

/// One participant's composed system prompt — the ~48 KB of standing
/// instruction bot-hq assembled for it at spawn (rc3 **P1**).
///
/// Exactly one of `content` / `unavailable` is set. `unavailable` exists so the
/// view never renders an empty pane and calls it a prompt: a session that has
/// ended, a participant that was never spawned, and a file that would not read
/// are three different facts, and each says which one it is.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct ParticipantSystemPrompt {
    /// The participant this prompt was composed for. The caller supplies it and
    /// it is echoed back, so a late response cannot be attributed to whichever
    /// chip the user clicked most recently.
    pub slug: String,
    /// The bytes `--append-system-prompt-file` handed the CLI, or `null`.
    ///
    /// **bot-hq's portion only.** The flag APPENDS: claude-code's own system
    /// prompt is still in front of this and is not ours to show.
    pub content: Option<String>,
    /// Length of `content` in bytes; 0 when there is none.
    pub bytes: u32,
    /// Why there is nothing to show, in the user's terms. `null` when `content`
    /// is set.
    pub unavailable: Option<String>,
}

/// Where [`prompt_view`] should read a participant's prompt from — or why it
/// cannot.
///
/// An enum rather than an `Option<&Path>` because the two empty cases are not
/// the same news: a closed session's prompt file is gone by design (it lives in
/// the session's `TempDir`), while a participant with no live agent was never
/// spawned at all — a disabled or `on_mention` row, which is a roster fact the
/// user may not expect.
pub(crate) enum PromptSource<'a> {
    /// No live handle for this session — closed, or not started since the app
    /// last launched.
    SessionNotLive,
    /// The session is live, but nothing in it spawned under this slug.
    NotSpawned,
    /// The file the spawn wrote and the CLI read.
    File(&'a Path),
}

/// Read one participant's composed prompt, or explain the absence.
///
/// Split from the command for the usual reason — a `#[tauri::command]` takes
/// `tauri::State`, which no unit test can build — and because the three empty
/// cases are the part worth pinning.
pub(crate) fn prompt_view(source: PromptSource<'_>, slug: &str) -> ParticipantSystemPrompt {
    let unavailable = |reason: String| ParticipantSystemPrompt {
        slug: slug.to_string(),
        content: None,
        bytes: 0,
        unavailable: Some(reason),
    };
    match source {
        PromptSource::SessionNotLive => unavailable(
            "This session has no live agents. The composed prompt is written to a temp file \
             at spawn and removed when the session ends, so there is nothing left to show."
                .to_string(),
        ),
        PromptSource::NotSpawned => unavailable(format!(
            "No agent is running as `{slug}` in this session — a participant that is \
             disabled, or waiting to be called on, never had a prompt composed for it."
        )),
        PromptSource::File(path) => match std::fs::read_to_string(path) {
            Ok(content) => ParticipantSystemPrompt {
                slug: slug.to_string(),
                bytes: content.len() as u32,
                content: Some(content),
                unavailable: None,
            },
            Err(e) => unavailable(format!(
                "Couldn't read the prompt file this participant spawned with ({}): {e}",
                path.display()
            )),
        },
    }
}

/// The prompt one participant is actually running under (rc3 **P1**).
///
/// The defect this closes: ~48 KB of standing instruction, assembled from six
/// layers and APPENDED to claude-code's own system prompt, that nobody — user
/// or agent — could see. Every "the prompt asserts an enforcement that is not
/// wired" defect was invisible by construction, and the Roles tab let the user
/// edit role prose with no way to view the result in context.
///
/// It reads the file the spawn WROTE, off the live agent
/// ([`SessionAgent::system_prompt_path`](crate::core::session::SessionAgent)),
/// rather than recomposing the prompt or rebuilding the filename here. A
/// recomposition would show what a spawn TODAY would produce — which is a
/// different claim, and would go quietly wrong the moment a role row or a CL
/// file changed after the agent started.
#[tauri::command]
#[specta::specta]
pub async fn get_participant_system_prompt(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    slug: String,
) -> Result<ParticipantSystemPrompt, AppError> {
    let sessions = core.sessions.lock().await;
    let view = match sessions.get(&session_id) {
        None => prompt_view(PromptSource::SessionNotLive, &slug),
        Some(handle) => match handle.by_slug(&slug) {
            None => prompt_view(PromptSource::NotSpawned, &slug),
            Some(agent) => prompt_view(PromptSource::File(&agent.system_prompt_path), &slug),
        },
    };
    Ok(view)
}

/// How many readings the history view asks for. The tail is what a
/// post-mortem wants — "what was it doing before it died" — and a long session
/// can hold thousands.
const CONTEXT_HISTORY_LIMIT: i64 = 200;

/// One participant's recorded context readings, oldest first (rc3 **P7**).
///
/// Reads the `context_readings` rows, so it answers for a CLOSED session too —
/// which is the whole point. The live meter is forwarded to a UI that may not
/// be open, is overwritten by the next turn, and dies with the session; that is
/// why the 2026-08-12 `Prompt is too long` death left nothing to diagnose.
///
/// Unusable readings are returned alongside usable ones, unaltered. A row whose
/// `reported_window` is null means the provider sent no window and the meter
/// could not have warned anyone — the distinction the caller most needs, and
/// the reason nothing here substitutes the model's configured
/// `context_window`.
#[tauri::command]
#[specta::specta]
pub async fn list_participant_context_readings(
    storage: tauri::State<'_, Arc<Storage>>,
    session_id: String,
    slug: String,
) -> Result<Vec<ContextReadingView>, AppError> {
    let rows = storage
        .context_readings_for_participant(&session_id, &slug, CONTEXT_HISTORY_LIMIT)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    Ok(rows.into_iter().map(ContextReadingView::from).collect())
}

/// One recorded reading, as the UI reads it. A view type rather than the
/// storage row because no storage struct carries UI traits here.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct ContextReadingView {
    /// `modelUsage` key the operands came from; `null` when none was usable.
    pub model: Option<String>,
    /// Point-in-time prompt size. `null` when the turn reported no usage.
    pub used_tokens: Option<i64>,
    /// The window EXACTLY as the provider reported it — `null` when it
    /// reported none, which is the case in which no meter was ever possible.
    pub reported_window: Option<i64>,
    /// `usable` | `no_window` | `no_usage` | `implausible_window`.
    pub verdict: String,
    pub created_at: String,
}

impl From<crate::storage::ContextReading> for ContextReadingView {
    fn from(r: crate::storage::ContextReading) -> Self {
        Self {
            model: r.model,
            used_tokens: r.used_tokens,
            reported_window: r.reported_window,
            verdict: r.verdict,
            created_at: r.created_at,
        }
    }
}

/// Testable body of [`list_session_participants`] — the command is a thin
/// `State`-unwrapping shim, matching `dispatch_session_inner`.
pub(crate) async fn participant_views(
    storage: &Storage,
    session_id: &str,
) -> Result<Vec<ParticipantView>, AppError> {
    let roster = storage
        .participants_for_session(session_id)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    let mut out = Vec::with_capacity(roster.len());
    for p in roster {
        // Both halves are read LIVE rather than off the row's frozen
        // `display_name`, so renaming a role or swapping a model shows up
        // without waiting for a respawn. Each failure is `None` on its own — a
        // deleted model must not also hide the role.
        let role_display_name = match p.role_id {
            Some(id) => storage
                .role_by_id(id)
                .await
                .map_err(|e| AppError::DbError(e.to_string()))?
                .map(|r| r.display_name),
            None => None,
        };
        let model_display_name = match p.model_id.as_deref().filter(|m| !m.is_empty()) {
            Some(id) => storage
                .get_model(id)
                .await
                .map_err(|e| AppError::DbError(e.to_string()))?
                .map(|m| m.display_name),
            None => None,
        };
        out.push(ParticipantView {
            id: p.id,
            slug: p.slug,
            role_display_name,
            model_display_name,
            turn_position: p.turn_position,
            participation_mode: p.participation_mode,
            enabled: p.enabled,
            color: p.color,
            label: p.label,
            effort: p.effort,
            ultracode: p.ultracode,
        });
    }
    Ok(out)
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
    // With a roster, the solo/duo flag is DERIVED from it. The two model
    // arguments now reach the participant ROWS (spawn reads those), so they are
    // only written to `sessions` on the legacy path, where they are the caller's
    // only way to say which model a slot runs — and `ensure_session_roster`'s
    // seed is what carries them there.
    let (rain_enabled, brian_model_id, rain_model_id) = match &roster {
        Some(r) => (r.rain_enabled, None, None),
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
    // `ensure_session_roster` — which seeds the default only into a session
    // that has none, so the two never both fire.
    match &roster {
        Some(roster) => storage
            .seed_session_roster(&id, &roster.drafts)
            .await
            .map(|_| ())
            .map_err(|e| AppError::DbError(format!("{e:#}")))?,
        // The pre-rc3 path: no participant list, so the default roster takes the
        // caller's per-slot model + effort picks. Spawn reads them off the
        // participant rows now, so writing them only to `sessions` above would
        // be a picker that changes nothing.
        None => {
            crate::core::session::seed_default_roster(
                storage,
                &id,
                !rain_enabled,
                &[brian_model_id.clone(), rain_model_id.clone()],
                &[
                    (options.brian_effort.clone(), options.brian_ultracode),
                    (options.rain_effort.clone(), options.rain_ultracode),
                ],
            )
            .await
        }
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
/// the project, spawn the roster, and broadcast `prompt` to their stdin — all
/// in one call so delivery is deterministic. A fresh session spawns blank
/// (`resume_session_id = None`) and bot-hq does NOT replay storage to stdin, so
/// the prompt has to be broadcast to a LIVE session — which means spawning
/// first. `ensure_session_started` inserts the handle before returning, so the
/// subsequent `broadcast` always finds it; it's idempotent, so the SessionView
/// mount's `respawn_session` is a harmless no-op.
///
/// Generic on purpose — the caller supplies the prompt. **There is no Tauri
/// command over this any more (rc3 D15).** The Context Library's "Maintain CL"
/// button was its only UI caller, and D15 deleted it: library-wide maintenance
/// is a session the user starts and instructs through the New Session dialog,
/// not a bespoke dialog-less create path with a hardcoded prompt. The plugin
/// proxy (`plugin_api.rs`, the `spawn_session` and `plugin_session_create`
/// arms) is what keeps this reachable — which is why it takes plain refs, not
/// `tauri::State`.
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
    // default (worktree_default), and the roster from `rain_override` when the
    // caller pins it (the `plugin_sessions` create arm) or from the product
    // default below.
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
    // **rc3 D13: no setting behind this any more — the product default is ONE
    // participant.** The `rain_disabled_default` toggle this used to read is
    // deleted ("there is no 'disable the reviewer by default'; just don't add
    // the role to your session creation"), and design §1 puts the default at one
    // agent. A caller that wants more pins it through `rain_override`; the seed
    // itself is `Storage::ensure_session_roster`, which documents the same rule.
    // Models stay NULL = role/agent defaults, as the dialog's "(agent default)".
    let rain_enabled = rain_override.unwrap_or(false);
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
    /// Per-slot busy flags (the derived `activity` collapses them) so the chat
    /// input can label who is working after a backfill, not just guess.
    ///
    /// **The field NAMES are frozen wire, and they name TURN SLOTS, not agents**
    /// (rc3 D10). `brian_*` is the participant at turn position 0 and `rain_*`
    /// the one at position 1; a session with one participant leaves the second
    /// pair at its empty value and a session with three does not report the
    /// rest here — `list_session_participants` is the roster-shaped read. They
    /// keep these names only until the session view that consumes them is
    /// rewritten.
    pub brian_busy: bool,
    pub rain_busy: bool,
    pub brian_health: Option<String>,
    pub rain_health: Option<String>,
    /// Idle-unflagged attention state ("idle_unflagged" or None = clear).
    /// Seeds the "needs direction" chip on mount; live updates arrive via
    /// `session:attention`.
    pub attention: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn get_session_runtime(
    core: tauri::State<'_, Arc<CoreAppState>>,
) -> Result<Vec<SessionRuntime>, AppError> {
    let sessions = core.sessions.lock().await;
    let mut out = Vec::with_capacity(sessions.len());
    for (id, handle) in sessions.iter() {
        // Slot lookup off the LIVE handle, whose `participants` are already in
        // turn order — the slugs are role-derived now, so `"brian"` / `"rain"`
        // would match nothing and every backfill would report both agents idle
        // and healthless.
        let slot = |i: usize| handle.participants.get(i).map(|a| a.slug.as_str());
        out.push(SessionRuntime {
            session_id: id.clone(),
            activity: handle.activity.current().as_str().to_string(),
            brian_busy: slot(0).is_some_and(|s| handle.activity.is_busy_slug(s)),
            rain_busy: slot(1).is_some_and(|s| handle.activity.is_busy_slug(s)),
            brian_health: slot(0).and_then(|s| core.bridge.current_agent_health(id, s)),
            rain_health: slot(1).and_then(|s| core.bridge.current_agent_health(id, s)),
            attention: core.bridge.current_session_attention(id),
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
    core.close_session(
        &session_id,
        archive,
        crate::core::close_learnings::ClosePath::User,
    )
    .await?;
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

    /// rc3 **P1**: the three ways a prompt can be absent are three different
    /// sentences, and none of them is an empty pane.
    ///
    /// The failure this guards is the one the task doc names: a viewer that
    /// renders blank when the file is gone teaches the user that the prompt is
    /// empty, which is the opposite of what P1 exists to show. Asserted through
    /// `prompt_view` for the usual reason — the command takes `tauri::State`.
    #[test]
    fn an_absent_prompt_says_which_absence_it_is() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("hands-system-prompt.txt");
        std::fs::write(&path, "ROLE PROSE\n\n## Context Library\n").unwrap();

        let present = prompt_view(PromptSource::File(&path), "hands");
        assert_eq!(present.slug, "hands");
        assert_eq!(
            present.content.as_deref(),
            Some("ROLE PROSE\n\n## Context Library\n")
        );
        assert_eq!(present.bytes, 31);
        assert!(present.unavailable.is_none());

        // Each empty case names its own cause, and none of them carries content.
        let dead = prompt_view(PromptSource::SessionNotLive, "hands");
        let unspawned = prompt_view(PromptSource::NotSpawned, "eyes");
        let unreadable = prompt_view(PromptSource::File(&dir.path().join("gone.txt")), "hands");
        for view in [&dead, &unspawned, &unreadable] {
            assert!(view.content.is_none(), "an unavailable prompt carried content");
            assert_eq!(view.bytes, 0);
            assert!(
                view.unavailable.as_deref().is_some_and(|r| !r.trim().is_empty()),
                "an unavailable prompt gave no reason"
            );
        }
        assert_ne!(
            dead.unavailable, unspawned.unavailable,
            "a closed session and a participant that never spawned read the same"
        );
        assert!(
            unspawned.unavailable.as_deref().unwrap().contains("eyes"),
            "the reason must name the participant it is about"
        );
        assert!(
            unreadable.unavailable.as_deref().unwrap().contains("gone.txt"),
            "a read failure must name the file it could not read"
        );
    }

    /// The read side of the rc3 contract, on real rows.
    ///
    /// Asserted through the same body the command runs (`participant_views`),
    /// because a `#[tauri::command]` takes `tauri::State`, which cannot be built
    /// in a unit test — the convention `resolve_participant_picks` already
    /// follows.
    #[tokio::test]
    async fn participant_views_are_turn_ordered_and_name_no_agent() {
        let (storage, _b) = setup().await;
        storage.create_session("s1", "t", None).await.unwrap();
        sqlx::query(
            "INSERT INTO models (id, display_name, provider, model_name) \
             VALUES ('m-opus', 'Claude Opus 5', 'anthropic', 'claude-opus-5')",
        )
        .execute(storage.pool())
        .await
        .unwrap();
        let hands = storage.role_by_slug("hands").await.unwrap().unwrap();
        let eyes = storage.role_by_slug("eyes").await.unwrap().unwrap();
        storage
            .seed_session_roster(
                "s1",
                &[
                    crate::storage::ParticipantDraft {
                        role_id: hands.id,
                        model_id: Some("m-opus".into()),
                        ..Default::default()
                    },
                    crate::storage::ParticipantDraft { role_id: eyes.id, ..Default::default() },
                ],
            )
            .await
            .unwrap();

        let views = participant_views(&storage, "s1").await.unwrap();
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].turn_position, 0);
        assert_eq!(views[0].slug, "hands");
        assert_eq!(views[0].role_display_name.as_deref(), Some("HANDS"));
        assert_eq!(views[0].model_display_name.as_deref(), Some("Claude Opus 5"));
        assert_eq!(views[0].participation_mode, "active");
        assert!(views[0].enabled);
        // The second row picked no model, and its role has no default either, so
        // the model half is genuinely absent — `null`, not an invented string.
        assert_eq!(views[1].slug, "eyes");
        assert_eq!(views[1].role_display_name.as_deref(), Some("EYES"));
        assert_eq!(views[1].model_display_name, None);

        // The display rule, over exactly the halves this command returns. The
        // frontend joins them the same way; this is the shared implementation.
        assert_eq!(
            crate::storage::participant_display_name(
                views[0].role_display_name.as_deref(),
                views[0].model_display_name.as_deref(),
                &views[0].slug,
                views[0].label.as_deref(),
            ),
            "HANDS · Claude Opus 5"
        );
        assert_eq!(
            crate::storage::participant_display_name(
                views[1].role_display_name.as_deref(),
                views[1].model_display_name.as_deref(),
                &views[1].slug,
                views[1].label.as_deref(),
            ),
            "EYES",
            "no model means the role alone, never a placeholder"
        );

        // A participant with no role is `null` rather than an error — the row is
        // still real and still has a handle to render. `role_id` is nullable and
        // FK-constrained, so this (not a dangling id) is how "no role" occurs.
        sqlx::query("UPDATE session_participants SET role_id = NULL WHERE slug = 'eyes'")
            .execute(storage.pool())
            .await
            .unwrap();
        let views = participant_views(&storage, "s1").await.unwrap();
        assert_eq!(views[1].role_display_name, None);
        assert_eq!(
            crate::storage::participant_display_name(None, None, &views[1].slug, None),
            "eyes",
            "with both halves gone the slug is the last resort"
        );

        // rc3 D10's line, on the payload the UI actually renders.
        for v in &views {
            for banned in ["Brian", "Rain"] {
                assert!(!v.slug.contains(banned));
                assert!(!v.role_display_name.as_deref().unwrap_or_default().contains(banned));
                assert!(!v.model_display_name.as_deref().unwrap_or_default().contains(banned));
            }
        }
    }

    /// **The dialog's per-participant effort + ultracode become readable.**
    ///
    /// The New Session dialog writes both onto the participant row (rc3 D12)
    /// and spawn reads them from there, but nothing could read them BACK — so
    /// the session view could not show what a running participant was actually
    /// spawned with. Two nullable fields on the row the view already returns;
    /// the columns predate this (0044).
    #[tokio::test]
    async fn participant_views_carry_the_rows_effort_and_ultracode() {
        let (storage, _b) = setup().await;
        storage.create_session("s1", "t", None).await.unwrap();
        storage.ensure_session_roster("s1", false).await.unwrap();
        let roster = storage.participants_for_session("s1").await.unwrap();

        // Only the first row is given knobs, so the assertion is a DIFFERENCE
        // between rows rather than a constant the view could invent.
        storage
            .set_participant_spawn_knobs(roster[0].id, Some("xhigh"), Some(true))
            .await
            .unwrap();

        let views = participant_views(&storage, "s1").await.unwrap();
        assert_eq!(views[0].effort.as_deref(), Some("xhigh"));
        assert_eq!(views[0].ultracode, Some(true));
        assert_eq!(
            views[1].effort, None,
            "a row that chose nothing inherits — null, not a guessed default"
        );
        assert_eq!(views[1].ultracode, None);
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
            effort: None,
            ultracode: None,
            color: None,
            label: None,
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
    async fn picks_carry_a_model_and_per_participant_spawn_knobs() {
        // rc3 D12: the dialog's effort section was two fixed blocks keyed on an
        // agent name; it is one select per participant ROW now, and the value
        // has to reach that row or the select changes nothing.
        let (storage, _b) = setup().await;
        let hands = storage.role_by_slug("hands").await.unwrap().unwrap();
        let eyes = storage.role_by_slug("eyes").await.unwrap().unwrap();

        let mut picks = vec![pick(hands.id, Some("opus")), pick(eyes.id, Some("sonnet"))];
        picks[0].effort = Some("max".into());
        picks[0].ultracode = Some(true);
        picks[1].effort = Some("low".into());
        let duo = resolve_participant_picks(&storage, &picks, &Default::default())
            .await
            .unwrap();
        assert!(duo.rain_enabled, "two participants is not a solo session");
        assert_eq!(duo.drafts.len(), 2);
        assert_eq!(duo.drafts[0].model_id.as_deref(), Some("opus"));
        assert_eq!(duo.drafts[0].effort.as_deref(), Some("max"));
        assert_eq!(duo.drafts[0].ultracode, Some(true));
        assert_eq!(duo.drafts[1].model_id.as_deref(), Some("sonnet"));
        assert_eq!(duo.drafts[1].effort.as_deref(), Some("low"));
        assert_eq!(duo.drafts[1].ultracode, None, "an unset knob inherits");

        // The pre-D12 per-agent fields still land on slots 0 and 1 when the row
        // says nothing, so a caller that has not been updated keeps working.
        let legacy = SessionCreateOptions {
            brian_effort: Some("high".into()),
            rain_effort: Some("none".into()),
            brian_ultracode: Some(false),
            ..Default::default()
        };
        let inherited = resolve_participant_picks(
            &storage,
            &[pick(hands.id, None), pick(eyes.id, None)],
            &legacy,
        )
        .await
        .unwrap();
        assert_eq!(inherited.drafts[0].effort.as_deref(), Some("high"));
        assert_eq!(inherited.drafts[0].ultracode, Some(false));
        assert_eq!(inherited.drafts[1].effort.as_deref(), Some("none"));

        let solo = resolve_participant_picks(&storage, &[pick(hands.id, None)], &Default::default())
            .await
            .unwrap();
        assert!(!solo.rain_enabled, "one participant is a solo session");
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
        assert_eq!(inherited.drafts[0].model_id.as_deref(), Some("role-default"));

        let overridden =
            resolve_participant_picks(&storage, &[pick(role.id, Some("chosen"))], &Default::default())
                .await
                .unwrap();
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
        // THREE is now fine — rc3 D10 made spawn iterate the roster instead of
        // naming two rows — so the refusal has to be tested at the actual cap.
        let three = vec![pick(hands.id, None), pick(hands.id, None), pick(hands.id, None)];
        assert!(
            resolve_participant_picks(&storage, &three, &opts).await.is_ok(),
            "three participants is a session the runtime can now spawn"
        );
        let too_many: Vec<ParticipantPick> = (0..MAX_SESSION_PARTICIPANTS + 1)
            .map(|_| pick(hands.id, None))
            .collect();
        assert!(
            resolve_participant_picks(&storage, &too_many, &opts).await.is_err(),
            "more participants than the cap allows"
        );
        assert!(
            resolve_participant_picks(&storage, &[pick(9999, None)], &opts).await.is_err(),
            "a role id that names nothing"
        );

        // rc3 D17: an `on_mention` participant is a legal invite — spawned,
        // skipped by the ring, handed a turn when the user names it.
        let summonable = role_with_mode(&storage, "Specialist", "on_mention").await;
        assert!(
            resolve_participant_picks(
                &storage,
                &[pick(hands.id, None), pick(summonable, None)],
                &opts
            )
            .await
            .is_ok(),
            "a summonable participant alongside an active one is a legal roster"
        );
        // …but it is not IN the rotation, so it cannot be the only member. A
        // roster of nothing but summonable participants has an empty ring, which
        // `all_active_voted_done` reports as vacuously done: a session finished
        // before it starts, with nobody to name anyone into it.
        let refused = resolve_participant_picks(&storage, &[pick(summonable, None)], &opts)
            .await
            .expect_err("an all-summonable roster has nobody in the turn rotation");
        assert!(
            refused.to_string().contains("turn rotation"),
            "refused for the wrong reason: {refused}"
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
