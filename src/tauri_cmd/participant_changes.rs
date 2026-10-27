//! Session Settings: thin host entry points to the live roster worker.
use super::AppError;
use crate::core::AppState;
use crate::storage::{
    ParticipantChange, ParticipantChangeRequest, Storage, MAX_SESSION_PARTICIPANTS,
};
use serde::Serialize;
use specta::Type;
use std::sync::Arc;

#[derive(Debug, Serialize, Type)]
pub struct ParticipantChangeView {
    pub id: String,
    pub request: ParticipantChangeRequest,
    pub status: String,
    pub error: Option<String>,
}

impl TryFrom<ParticipantChange> for ParticipantChangeView {
    type Error = AppError;
    fn try_from(row: ParticipantChange) -> Result<Self, Self::Error> {
        Ok(Self {
            request: row.request()?,
            id: row.id,
            status: row.status,
            error: row.error,
        })
    }
}

#[derive(Debug, Serialize, Type)]
pub struct ParticipantSelectionView {
    pub participant_id: i64,
    pub model_id: Option<String>,
    pub effort: Option<crate::agents::effort::EffortStatus>,
}

#[derive(Debug, Serialize, Type)]
pub struct SessionParticipantControls {
    pub revision: i64,
    pub closed: bool,
    pub max_participants: usize,
    pub selections: Vec<ParticipantSelectionView>,
    pub latest_change: Option<ParticipantChangeView>,
}

pub(crate) async fn controls(
    storage: &Storage,
    session: &str,
) -> Result<SessionParticipantControls, AppError> {
    // Read the revision FIRST: if a publication races these display reads,
    // submitting this snapshot is refused rather than overwriting newer work.
    let revision = storage.roster_revision(session).await?;
    let row = storage
        .get_session(session)
        .await?
        .ok_or_else(|| AppError::NotFound(session.into()))?;
    let selections = storage
        .participants_for_session(session)
        .await?
        .into_iter()
        .map(|p| ParticipantSelectionView {
            participant_id: p.id,
            model_id: p.model_id,
            effort: None,
        })
        .collect();
    Ok(SessionParticipantControls {
        revision,
        closed: row.closed_at.is_some() || row.archived != 0,
        max_participants: MAX_SESSION_PARTICIPANTS,
        selections,
        latest_change: storage
            .latest_participant_change(session)
            .await?
            .map(TryInto::try_into)
            .transpose()?,
    })
}

pub(crate) async fn controls_with_runtime(storage: &Storage, session: &str, core: Option<&AppState>) -> Result<SessionParticipantControls, AppError> {
    let mut view = controls(storage, session).await?;
    if let Some(core) = core {
        for (id, status) in core.participant_effort_status(session).await {
            if let Some(p) = view.selections.iter_mut().find(|p| p.participant_id == id) { p.effort = Some(status); }
        }
    }
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn get_session_participant_controls(
    core: tauri::State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<SessionParticipantControls, AppError> {
    controls_with_runtime(&core.storage, &session_id, Some(&core)).await
}

#[tauri::command]
#[specta::specta]
pub async fn change_session_participant(
    core: tauri::State<'_, Arc<AppState>>,
    session_id: String,
    request_id: String,
    revision: i64,
    request: ParticipantChangeRequest,
) -> Result<ParticipantChangeView, AppError> {
    core.change_participant(&session_id, &request_id, revision, request)
        .await?
        .try_into()
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_session_participant_change(
    core: tauri::State<'_, Arc<AppState>>,
    session_id: String,
    request_id: String,
) -> Result<bool, AppError> {
    Ok(core
        .cancel_participant_change(&session_id, &request_id)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn controls_recover_saved_requests_and_do_not_expose_checkpoints() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "one", None).await.unwrap();
        s.create_session("s2", "two", None).await.unwrap();
        s.ensure_session_roster("s1", 2).await.unwrap();
        let rows = s.participants_for_session("s1").await.unwrap();
        let before = controls(&s, "s1").await.unwrap();
        let request = ParticipantChangeRequest::Remove {
            participant_id: rows[1].id,
        };
        s.request_participant_change(
            "s1",
            "id1",
            before.revision,
            request.clone(),
            crate::storage::ParticipantChangeOrigin::User,
        )
        .await
        .unwrap();
        let view = controls(&s, "s1").await.unwrap();
        assert_eq!(view.latest_change.as_ref().unwrap().request, request);
        assert_eq!(view.latest_change.as_ref().unwrap().status, "pending");
        let json = serde_json::to_value(view).unwrap();
        assert!(json["latest_change"].get("recovery_json").is_none());
        assert!(json["latest_change"].get("selection_fingerprint").is_none());
        assert!(controls(&s, "s2").await.unwrap().latest_change.is_none());
        assert!(s.cancel_participant_change("s1", "id1").await.unwrap());
        assert_eq!(
            controls(&s, "s1")
                .await
                .unwrap()
                .latest_change
                .unwrap()
                .status,
            "cancelled"
        );
        s.close_session("s1", false).await.unwrap();
        assert!(controls(&s, "s1").await.unwrap().closed);
        assert!(controls(&s, "missing").await.is_err());
    }
}
