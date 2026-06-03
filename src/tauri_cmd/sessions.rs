//! Session lifecycle commands.

use crate::core::ipav::IpavPhase;
use crate::core::AppState as CoreAppState;
use crate::signaling::SignalingBridge;
use crate::storage::{Session, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub working_repo_path: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub closed_at: Option<String>,
    pub brian_model_at_spawn: Option<String>,
    pub rain_model_at_spawn: Option<String>,
}

impl From<Session> for SessionInfo {
    fn from(s: Session) -> Self {
        Self {
            id: s.id,
            title: s.title,
            working_repo_path: s.working_repo_path,
            archived: s.archived != 0,
            created_at: s.created_at,
            closed_at: s.closed_at,
            brian_model_at_spawn: s.brian_model_at_spawn,
            rain_model_at_spawn: s.rain_model_at_spawn,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn create_session(
    storage: tauri::State<'_, Arc<Storage>>,
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    id: String,
    title: String,
    repo_path: Option<String>,
    project: Option<String>,
    // Create-dialog choices. Defaults preserve the historical duo behavior so
    // older callers that omit them keep spawning Rain with agent-config models.
    rain_enabled: Option<bool>,
    brian_model_id: Option<String>,
    rain_model_id: Option<String>,
) -> Result<SessionInfo, AppError> {
    storage
        .create_session(&id, &title, repo_path.as_deref())
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
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
    bridge.register_session(id.clone(), project).await;
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
    let session = storage
        .create_session(&id, &title, repo_path.as_deref())
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
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

#[tauri::command]
#[specta::specta]
pub async fn list_sessions(
    storage: tauri::State<'_, Arc<Storage>>,
) -> Result<Vec<SessionInfo>, AppError> {
    storage
        .list_active_sessions()
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::DbError(e.to_string()))
}

/// All closed sessions (just-closed + archived), most-recently-closed first.
/// Backs the Settings → Archive tab. Excludes the emma singleton.
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

/// Advance the IPAV phase from the UI. Target accepts single-letter chips
/// (`I`/`P`/`A`/`V`) or full names (`Investigate`/`Plan`/`Apply`/`Verify`).
/// Synthesizes a phase-change message in storage + feeds the transition
/// notice to both agents' stdin so they pick up the new phase as a prompt.
#[tauri::command]
#[specta::specta]
pub async fn advance_session_phase(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    target: String,
) -> Result<(), AppError> {
    let phase = IpavPhase::parse(&target).ok_or_else(|| {
        AppError::Validation(format!(
            "invalid phase {target:?} \u{2014} expected {}",
            IpavPhase::error_hint()
        ))
    })?;
    core.advance_phase(&session_id, phase).await?;
    Ok(())
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
