//! Message stream commands. Frontend SessionView polls `get_session_messages`
//! once on mount, then subscribes to `agent.messages.batch` for deltas.
//! `broadcast_message` is the inverse — the user→agent send path.

use crate::core::AppState as CoreAppState;
use crate::storage::Storage;
use crate::tauri_cmd::error::AppError;
use crate::tauri_events::types::AgentMessage;
use std::sync::Arc;

/// The chat's history read. Three shapes (round 8, N2):
/// - `since_id: Some(n)` — everything after `n` (a resync / catch-up read);
/// - `limit: Some(k)` — the newest `k` rows, chronological, optionally those
///   before `before_id` (the mount read and its "load older" page — the chat
///   only renders the tail, and the largest session held 3,412 rows / 6.5 MB,
///   all pulled across IPC on every mount before this);
/// - neither — the whole history (kept for the two callers that mean it).
#[tauri::command]
#[specta::specta]
pub async fn get_session_messages(
    storage: tauri::State<'_, Arc<Storage>>,
    session_id: String,
    since_id: Option<i64>,
    before_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<AgentMessage>, AppError> {
    let msgs = match (since_id, limit) {
        (Some(_), _) => storage.messages_for_session(&session_id, since_id).await,
        (None, Some(k)) => storage.messages_tail(&session_id, before_id, k).await,
        (None, None) => storage.messages_for_session(&session_id, None).await,
    }
    .map_err(|e| AppError::DbError(e.to_string()))?;
    Ok(msgs.into_iter().map(AgentMessage::from).collect())
}

/// Send a user message to a session. For the duo session this fans out to
/// both Brian and Rain (with phase envelope). Persists the raw text + notifies the bridge.
#[tauri::command]
#[specta::specta]
pub async fn broadcast_message(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    text: String,
) -> Result<(), AppError> {
    core.broadcast(&session_id, &text).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{MessageKind};

    #[tokio::test]
    async fn get_session_messages_returns_in_order() {
        let storage = Arc::new(Storage::memory().await.unwrap());
        storage.create_session("s1", "t", None).await.unwrap();
        storage
            .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "a", None)
            .await
            .unwrap();
        storage
            .post_to_channel("s1", "participant", Some("eyes"), MessageKind::Text.as_str(), "b", None)
            .await
            .unwrap();

        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        let agent_msgs: Vec<AgentMessage> = msgs.into_iter().map(AgentMessage::from).collect();
        assert_eq!(agent_msgs.len(), 2);
        assert_eq!(agent_msgs[0].content, "a");
        assert_eq!(agent_msgs[0].author, "hands");
        assert_eq!(agent_msgs[1].author, "eyes");
    }

    #[tokio::test]
    async fn get_session_messages_respects_since_id() {
        let storage = Arc::new(Storage::memory().await.unwrap());
        storage.create_session("s1", "t", None).await.unwrap();
        let id1 = storage
            .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "first", None)
            .await
            .unwrap()
            .message_id();
        storage
            .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "second", None)
            .await
            .unwrap();

        let after = storage.messages_for_session("s1", Some(id1)).await.unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].content, "second");
    }
}
