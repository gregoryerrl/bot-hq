//! Duo broadcast helpers: persist + send to both agents.
//!
//! Lives separately so it can be mocked in tests.

use crate::agents::OutgoingUserMessage;
use crate::core::ipav::IpavPhase;
use crate::storage::{Author, MessageKind, Storage};
use anyhow::Result;
use tokio::sync::mpsc;
use tracing::warn;

/// Prefix every outgoing stdin payload with the active IPAV phase so the
/// agent re-encounters it on each turn. Storage keeps the raw text — the
/// envelope is wire-only.
pub fn with_phase_envelope(phase: IpavPhase, body: &str) -> String {
    format!("[PHASE: {}]\n{body}", phase.name())
}

/// Persist a user-originated message and fan it out to both agents.
pub async fn broadcast_user_message(
    storage: &Storage,
    session_id: &str,
    text: &str,
    phase: IpavPhase,
    brian_input: &mpsc::Sender<OutgoingUserMessage>,
    // None for a solo session (Rain disabled) — Brian still receives the message.
    rain_input: Option<&mpsc::Sender<OutgoingUserMessage>>,
) -> Result<i64> {
    let id = storage
        .insert_message(session_id, Author::User, MessageKind::Text, text)
        .await?;
    let wire = with_phase_envelope(phase, text);
    let msg = OutgoingUserMessage::text(wire);
    // Fan out to both agents. The message is persisted (above) regardless, but
    // a send error means that agent's input pump has exited (stdin gone) and
    // the agent won't SEE this message. Previously swallowed with `let _`,
    // which is precisely how the #4 user→HANDS desync stayed invisible: a
    // failed send to Brian while Rain's succeeded looked like nothing wrong.
    // Log per agent so the asymmetry is diagnosable.
    if let Err(e) = brian_input.send(msg.clone()).await {
        warn!(agent = "brian", error = %e, "user broadcast not delivered (input pump closed)");
    }
    if let Some(rain_input) = rain_input {
        if let Err(e) = rain_input.send(msg).await {
            warn!(agent = "rain", error = %e, "user broadcast not delivered (input pump closed)");
        }
    }
    Ok(id)
}

/// Forward a peer's prose chunk into an agent's stdin. Called by the duo
/// coordinator after the buffer-rule flush. The message is rendered as if
/// from the user but tagged so the agent knows who said it.
pub async fn peer_forward_message(
    peer_author: Author,
    text: &str,
    phase: IpavPhase,
    input_tx: &mpsc::Sender<OutgoingUserMessage>,
) -> Result<()> {
    let prefix = match peer_author {
        Author::Brian => "[Brian]\n",
        Author::Rain => "[Rain]\n",
        Author::User => "",
    };
    let inner = format!("{prefix}{text}");
    let wire = with_phase_envelope(phase, &inner);
    let _ = input_tx.send(OutgoingUserMessage::text(wire)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn broadcast_persists_raw_and_envelopes_wire() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "test", None).await.unwrap();
        let (btx, mut brx) = mpsc::channel(8);
        let (rtx, mut rrx) = mpsc::channel(8);
        broadcast_user_message(&s, "s1", "hello", IpavPhase::Apply, &btx, Some(&rtx))
            .await
            .unwrap();
        let bm = brx.recv().await.unwrap();
        let rm = rrx.recv().await.unwrap();
        assert_eq!(bm.message.content, "[PHASE: Apply]\nhello");
        assert_eq!(rm.message.content, "[PHASE: Apply]\nhello");
        let msgs = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].content, "hello",
            "storage keeps raw text, no envelope"
        );
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
        broadcast_user_message(
            &s,
            "sess-a",
            "msg-into-a",
            IpavPhase::Investigate,
            &btx,
            Some(&rtx),
        )
        .await
        .unwrap();

        let a_msgs = s.messages_for_session("sess-a", None).await.unwrap();
        let b_msgs = s.messages_for_session("sess-b", None).await.unwrap();
        assert_eq!(a_msgs.len(), 1);
        assert_eq!(a_msgs[0].content, "msg-into-a");
        assert!(
            b_msgs.is_empty(),
            "broadcast leaked into other session: {:?}",
            b_msgs
        );
    }

    #[tokio::test]
    async fn broadcast_solo_delivers_to_brian_only() {
        // Rain disabled: rain_input is None. Brian still receives the message
        // and it's persisted exactly once — no panic on the absent peer.
        let s = Storage::memory().await.unwrap();
        s.create_session("solo", "test", None).await.unwrap();
        let (btx, mut brx) = mpsc::channel(8);
        broadcast_user_message(&s, "solo", "hi", IpavPhase::Apply, &btx, None)
            .await
            .unwrap();
        let bm = brx.recv().await.unwrap();
        assert_eq!(bm.message.content, "[PHASE: Apply]\nhi");
        assert_eq!(s.messages_for_session("solo", None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn peer_forward_envelopes_then_author_tags() {
        let (tx, mut rx) = mpsc::channel(8);
        peer_forward_message(Author::Rain, "concerns?", IpavPhase::Plan, &tx)
            .await
            .unwrap();
        let m = rx.recv().await.unwrap();
        assert!(
            m.message.content.starts_with("[PHASE: Plan]\n[Rain]\n"),
            "expected phase envelope wrapping author tag, got: {}",
            m.message.content
        );
        assert!(m.message.content.contains("concerns?"));
    }
}
