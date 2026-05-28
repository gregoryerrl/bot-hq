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
) -> Result<SessionInfo, AppError> {
    let session = storage
        .create_session(&id, &title, repo_path.as_deref())
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    bridge.register_session(id.clone(), project).await;
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
    core.ensure_session_started(&session_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
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
    core.advance_phase(&session_id, phase)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// Close a session from the UI. Delegates to `core.close_session`, which is
/// the single source of truth for closing: it removes the live handle, KILLS
/// the brian/rain subprocesses, marks the row closed/archived in storage, and
/// wipes session permission grants. The previous version called
/// `storage.close_session` directly, so it set `closed_at` but left the
/// subprocesses running — a session that "closed" in the DB yet kept taking
/// turns. Routing through core fixes that.
#[tauri::command]
#[specta::specta]
pub async fn close_session(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    archive: bool,
) -> Result<(), AppError> {
    core.close_session(&session_id, archive)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
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
