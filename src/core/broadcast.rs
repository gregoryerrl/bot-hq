//! Duo broadcast helpers: persist + send to both agents.
//!
//! Lives separately so it can be mocked in tests.

use crate::agents::OutgoingUserMessage;
use crate::storage::{Author, MessageKind, Storage};
use anyhow::Result;
use tokio::sync::mpsc;

/// Persist a user-originated message and fan it out to both agents.
pub async fn broadcast_user_message(
    storage: &Storage,
    session_id: &str,
    text: &str,
    brian_input: &mpsc::Sender<OutgoingUserMessage>,
    rain_input: &mpsc::Sender<OutgoingUserMessage>,
) -> Result<i64> {
    let id = storage
        .insert_message(session_id, Author::User, MessageKind::Text, text)
        .await?;
    let msg = OutgoingUserMessage::text(text);
    // Best-effort: if one channel is closed, persist still happens.
    let _ = brian_input.send(msg.clone()).await;
    let _ = rain_input.send(msg).await;
    Ok(id)
}

/// Forward a peer's prose chunk into an agent's stdin. Called by the duo
/// coordinator after the buffer-rule flush. The message is rendered as if
/// from the user but tagged so the agent knows who said it.
pub async fn peer_forward_message(
    peer_author: Author,
    text: &str,
    input_tx: &mpsc::Sender<OutgoingUserMessage>,
) -> Result<()> {
    let prefix = match peer_author {
        Author::Brian => "[Brian]\n",
        Author::Rain => "[Rain]\n",
        Author::Emma => "[Emma]\n",
        Author::User => "",
    };
    let body = format!("{prefix}{text}");
    let _ = input_tx.send(OutgoingUserMessage::text(body)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn broadcast_persists_and_fans_out() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "test", None).await.unwrap();
        let (btx, mut brx) = mpsc::channel(8);
        let (rtx, mut rrx) = mpsc::channel(8);
        broadcast_user_message(&s, "s1", "hello", &btx, &rtx)
            .await
            .unwrap();
        let bm = brx.recv().await.unwrap();
        let rm = rrx.recv().await.unwrap();
        assert_eq!(bm.message.content, "hello");
        assert_eq!(rm.message.content, "hello");
        let msgs = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[0].author, "user");
    }

    #[tokio::test]
    async fn broadcast_does_not_leak_to_other_session() {
        // Regression: when the dashboard had tile-reordering bugs, the user
        // worried that broadcasting to session A might land in B. This locks
        // in the contract — broadcast is keyed strictly by session_id.
        let s = Storage::memory().await.unwrap();
        s.create_session("sess-a", "a", None).await.unwrap();
        s.create_session("sess-b", "b", None).await.unwrap();
        let (btx, _brx) = mpsc::channel(8);
        let (rtx, _rrx) = mpsc::channel(8);
        broadcast_user_message(&s, "sess-a", "msg-into-a", &btx, &rtx)
            .await
            .unwrap();

        let a_msgs = s.messages_for_session("sess-a", None).await.unwrap();
        let b_msgs = s.messages_for_session("sess-b", None).await.unwrap();
        assert_eq!(a_msgs.len(), 1);
        assert_eq!(a_msgs[0].content, "msg-into-a");
        assert!(b_msgs.is_empty(), "broadcast leaked into other session: {:?}", b_msgs);
    }

    #[tokio::test]
    async fn peer_forward_prefixes_author() {
        let (tx, mut rx) = mpsc::channel(8);
        peer_forward_message(Author::Rain, "concerns?", &tx)
            .await
            .unwrap();
        let m = rx.recv().await.unwrap();
        assert!(m.message.content.starts_with("[Rain]"));
        assert!(m.message.content.contains("concerns?"));
    }
}
