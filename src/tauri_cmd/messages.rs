//! Message stream commands. Frontend SessionView reads `get_session_messages`
//! once on mount, then subscribes to `agent:messages:batch` for deltas (Tauri
//! event names take colons, never dots — `EVENT_NAME_BATCH`).
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
    let msgs = select_messages(&storage, &session_id, since_id, before_id, limit)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    Ok(msgs.into_iter().map(AgentMessage::from).collect())
}

/// The command's dispatch, as a plain function so a test can reach it: which
/// storage read each `(since_id, before_id, limit)` shape becomes. Extracted in
/// round 9 — the command had NO test caller (its two tests were named after it
/// and called storage directly), so `before_id` could be dropped on the floor
/// with a green suite: "load older" would re-serve the newest page forever.
pub(crate) async fn select_messages(
    storage: &Storage,
    session_id: &str,
    since_id: Option<i64>,
    before_id: Option<i64>,
    limit: Option<i64>,
) -> anyhow::Result<Vec<crate::storage::Message>> {
    match (since_id, limit) {
        (Some(_), _) => storage.messages_for_session(session_id, since_id).await,
        (None, Some(k)) => storage.messages_tail(session_id, before_id, k).await,
        (None, None) => storage.messages_for_session(session_id, None).await,
    }
}

/// Send a user message to a session: one row every participant reads off its
/// cursor (with phase envelope). Persists the raw text + notifies the bridge.
#[tauri::command]
#[specta::specta]
pub async fn broadcast_message(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    text: String,
) -> Result<(), AppError> {
    // The user's Send waits for the roster (EYES U2 / 2b1592e0): the view's
    // mount respawns a dormant session in the background, and a Send in the
    // seconds after a relaunch used to fail with "no live session" and need
    // a second click. `ensure_session_started` is idempotent, waits on the
    // spawn gate for a respawn in flight, and refuses a closed row.
    core.ensure_session_started(&session_id).await?;
    core.broadcast(&session_id, &text, crate::core::Authored::ByUser).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{MessageKind};

    /// **The user's two Send commands wait for the roster before sending**
    /// (EYES U2 / 2b1592e0): after a relaunch the view's mount respawns a
    /// dormant session in the background and a Send in those seconds failed
    /// with "no live session"; `send_user_response` records the picks before
    /// it posts the message, so the wait must come before any of it.
    /// Source-asserted: the commands take a live `CoreAppState`.
    #[test]
    fn the_users_send_commands_start_the_session_before_sending() {
        for (file, name, send) in [
            (include_str!("messages.rs"), "broadcast_message", "core.broadcast("),
            (include_str!("tray.rs"), "send_user_response", "core.send_user_response("),
        ] {
            let at = file.find(&format!("pub async fn {name}(")).unwrap_or_else(|| panic!("{name} exists"));
            let body = &file[at..];
            let end = body[1..].find("\n#[tauri::command]").map_or(body.len(), |n| n + 1);
            let body = &body[..end];
            let start = body.find("core.ensure_session_started(&session_id)").unwrap_or_else(|| panic!("{name} waits for the roster"));
            let sends = body.find(send).unwrap_or_else(|| panic!("{name} sends"));
            assert!(start < sends, "{name}: the roster is started before the send");
        }
    }

    #[tokio::test]
    async fn messages_for_session_returns_in_order_and_maps_authors() {
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
    async fn messages_for_session_respects_since_id() {
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

    /// The three shapes of the dispatch, through the function the command
    /// wraps. The last case is the kill test: drop `before_id` in
    /// `select_messages` and "load older" returns the newest page again.
    #[tokio::test]
    async fn select_messages_routes_each_shape_to_the_right_read() {
        let storage = Storage::memory().await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();
        let mut ids = Vec::new();
        for body in ["m1", "m2", "m3", "m4", "m5"] {
            ids.push(
                storage
                    .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), body, None)
                    .await
                    .unwrap()
                    .message_id(),
            );
        }
        let bodies = |rows: Vec<crate::storage::Message>| {
            rows.into_iter().map(|m| m.content).collect::<Vec<_>>()
        };
        // neither: the whole history, oldest first
        let all = select_messages(&storage, "s1", None, None, None).await.unwrap();
        assert_eq!(bodies(all), ["m1", "m2", "m3", "m4", "m5"]);
        // since_id: everything after it (limit ignored)
        let since = select_messages(&storage, "s1", Some(ids[2]), None, Some(1)).await.unwrap();
        assert_eq!(bodies(since), ["m4", "m5"]);
        // limit: the newest k, chronological
        let tail = select_messages(&storage, "s1", None, None, Some(2)).await.unwrap();
        assert_eq!(bodies(tail), ["m4", "m5"]);
        // limit + before_id: the page BEFORE that row — the "load older" read
        let older = select_messages(&storage, "s1", None, Some(ids[3]), Some(2)).await.unwrap();
        assert_eq!(bodies(older), ["m2", "m3"], "before_id must reach messages_tail");
    }
}
