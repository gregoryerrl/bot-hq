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

/// Like [`with_phase_envelope`] but also prepends a persistent EYES-findings
/// banner when `open_blocking > 0`, so it rides every turn (it can't scroll
/// away) until the findings are dispositioned — the salience half of the
/// EYES-sign-off gate (post-mortem §5.2). `open_blocking == 0` delegates to the
/// plain envelope, so there's zero overhead in the common (nothing-open) case.
pub fn with_phase_and_findings_envelope(
    phase: IpavPhase,
    open_blocking: usize,
    body: &str,
) -> String {
    if open_blocking == 0 {
        return with_phase_envelope(phase, body);
    }
    format!(
        "[PHASE: {}]\n⚠ {open_blocking} unresolved EYES blocking finding(s) — run \
         check_open_findings and disposition each (fix/rebut) before you commit.\n{body}",
        phase.name()
    )
}

/// Persist a user-originated message and fan it out to both agents.
pub async fn broadcast_user_message(
    storage: &Storage,
    session_id: &str,
    text: &str,
    phase: IpavPhase,
    // Optional WIRE-ONLY system note prepended to the body — e.g. the
    // post-cancel reconciliation directive. NOT persisted: storage keeps the
    // raw user text, so chat history stays clean (like the findings banner).
    system_prefix: Option<&str>,
    brian_input: &mpsc::Sender<OutgoingUserMessage>,
    // None for a solo session (Rain disabled) — Brian still receives the message.
    rain_input: Option<&mpsc::Sender<OutgoingUserMessage>>,
) -> Result<i64> {
    let id = storage
        .insert_message(session_id, Author::User, MessageKind::Text, text)
        .await?;
    // Ride the open-blocking-findings banner on every user turn (fail-safe 0 on
    // any query error — the banner is salience, not a gate).
    let open_blocking = storage
        .count_open_blocking_findings(session_id)
        .await
        .unwrap_or(0) as usize;
    let wire_body = match system_prefix {
        Some(p) => format!("{p}\n{text}"),
        None => text.to_string(),
    };
    let wire = with_phase_and_findings_envelope(phase, open_blocking, &wire_body);
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
    open_blocking: usize,
    input_tx: &mpsc::Sender<OutgoingUserMessage>,
) {
    let prefix = match peer_author {
        Author::Brian => "[PEER MESSAGE — from Brian (HANDS), not the user]\n",
        Author::Rain => "[PEER MESSAGE — from Rain (EYES), not the user]\n",
        Author::User => "",
    };
    let inner = format!("{prefix}{text}");
    let wire = with_phase_and_findings_envelope(phase, open_blocking, &inner);
    // A send error means this agent's input pump has exited (stdin gone) and it
    // won't SEE the peer's message. Mirrors broadcast_user_message: log per agent
    // so a one-sided peer-forward loss is diagnosable instead of silent (the same
    // invisible-desync failure mode, on the peer path).
    if let Err(e) = input_tx.send(OutgoingUserMessage::text(wire)).await {
        warn!(agent = ?peer_author, error = %e, "peer forward not delivered (input pump closed)");
    }
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
        broadcast_user_message(&s, "s1", "hello", IpavPhase::Apply, None, &btx, Some(&rtx))
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
            None,
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
        broadcast_user_message(&s, "solo", "hi", IpavPhase::Apply, None, &btx, None)
            .await
            .unwrap();
        let bm = brx.recv().await.unwrap();
        assert_eq!(bm.message.content, "[PHASE: Apply]\nhi");
        assert_eq!(s.messages_for_session("solo", None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn peer_forward_envelopes_then_author_tags() {
        let (tx, mut rx) = mpsc::channel(8);
        peer_forward_message(Author::Rain, "concerns?", IpavPhase::Plan, 0, &tx).await;
        let m = rx.recv().await.unwrap();
        assert!(
            m.message
                .content
                .starts_with("[PHASE: Plan]\n[PEER MESSAGE — from Rain (EYES), not the user]\n"),
            "expected phase envelope wrapping peer provenance tag, got: {}",
            m.message.content
        );
        assert!(m.message.content.contains("concerns?"));
    }

    #[test]
    fn findings_envelope_plain_when_none_else_banner() {
        // 0 open → identical to the plain phase envelope (zero overhead).
        assert_eq!(
            with_phase_and_findings_envelope(IpavPhase::Apply, 0, "hi"),
            with_phase_envelope(IpavPhase::Apply, "hi")
        );
        // >0 → a ⚠ banner rides between the phase tag and the body.
        let w = with_phase_and_findings_envelope(IpavPhase::Apply, 2, "hi");
        assert!(
            w.starts_with("[PHASE: Apply]\n⚠ 2 unresolved EYES blocking finding(s)"),
            "got: {w}"
        );
        assert!(w.ends_with("\nhi"), "body still trails the envelope: {w}");
    }

    #[tokio::test]
    async fn broadcast_user_message_carries_findings_banner() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "test", None).await.unwrap();
        s.insert_finding(
            "s1",
            "f1",
            "rain",
            crate::storage::FindingSeverity::Blocking,
            "bug",
            None,
        )
        .await
        .unwrap();
        let (btx, mut brx) = mpsc::channel(8);
        broadcast_user_message(&s, "s1", "go", IpavPhase::Verify, None, &btx, None)
            .await
            .unwrap();
        let bm = brx.recv().await.unwrap();
        assert!(
            bm.message
                .content
                .contains("⚠ 1 unresolved EYES blocking finding"),
            "user-turn wire should carry the banner: {}",
            bm.message.content
        );
        assert!(bm.message.content.ends_with("\ngo"));
        // Storage still keeps the RAW text (no envelope), unchanged by the banner.
        let msgs = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs[0].content, "go");
    }

    #[tokio::test]
    async fn system_prefix_rides_the_wire_not_storage() {
        // The post-cancel reconciliation directive is wire-only: the agent sees
        // it prepended to the body, but storage keeps the raw user text so the
        // chat history stays clean.
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "test", None).await.unwrap();
        let (btx, mut brx) = mpsc::channel(8);
        broadcast_user_message(
            &s,
            "s1",
            "do the thing",
            IpavPhase::Apply,
            Some("[System: previous turn interrupted — verify workspace.]"),
            &btx,
            None,
        )
        .await
        .unwrap();
        let bm = brx.recv().await.unwrap();
        assert!(
            bm.message
                .content
                .contains("[System: previous turn interrupted"),
            "wire carries the system prefix: {}",
            bm.message.content
        );
        assert!(bm.message.content.ends_with("\ndo the thing"));
        // Storage keeps the RAW text — no prefix.
        let msgs = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs[0].content, "do the thing");
    }
}
