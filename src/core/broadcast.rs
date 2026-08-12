//! Duo broadcast helpers: persist + send to both agents.
//!
//! Lives separately so it can be mocked in tests.

use crate::agents::ParticipantInput;
use crate::core::ipav::IpavPhase;
use crate::storage::{Envelope, MessageKind, Storage};
use anyhow::Result;
use tracing::warn;


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
    // Every live participant, as `(slug, stdin)`. B4b: was a `brian_input` +
    // `Option<rain_input>` pair. The slug rides along so the per-agent delivery
    // warning below can still name WHICH agent missed the message.
    recipients: &[(&str, &ParticipantInput)],
) -> Result<i64> {
    // REORDERED (B5 Task 2): the banner count is read BEFORE the insert, because
    // it is part of what the agent will read and the row has to record that. It
    // used to be read after, and the wire was assembled from it afterwards —
    // which is exactly how storage ended up holding the raw user text while the
    // agent read something else.
    //
    // The move is inert: this is a read of the findings tables, and the insert
    // it now precedes writes only to `messages`.
    //
    // Ride the open-blocking-findings banner on every user turn (fail-safe 0 on
    // any query error — the banner is salience, not a gate).
    let open_blocking = storage
        .count_open_blocking_findings(session_id)
        .await
        .unwrap_or(0) as usize;
    let mut envelope = Envelope::phase(phase.name()).with_open_blocking(open_blocking);
    if let Some(prefix) = system_prefix {
        envelope = envelope.with_system_prefix(prefix);
    }
    // `origin = "user"` + no slug: what `insert_message(Author::User, ..)`
    // resolves to. Called directly only because the legacy wrapper has nowhere
    // to put an envelope.
    let persisted = storage
        .post_to_channel(
            session_id,
            "user",
            None,
            MessageKind::Text.as_str(),
            text,
            Some(envelope),
        )
        .await?;
    // Fan out to every agent. The message is persisted (above) regardless, but
    // a failed delivery means that agent's input pump has exited (stdin gone)
    // and the agent won't SEE this message. Previously swallowed with `let _`,
    // which is precisely how the #4 user→HANDS desync stayed invisible: a
    // failed send to Brian while Rain's succeeded looked like nothing wrong.
    // Log per agent so the asymmetry is diagnosable.
    for (slug, input) in recipients {
        if !input.deliver(&persisted).await {
            warn!(agent = %slug, "user broadcast not delivered (input pump closed)");
        }
    }
    Ok(persisted.message_id())
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::render_wire;

    /// `recv()` with a deadline.
    ///
    /// A bare `rx.recv().await` turns a regression into a HANG rather than a
    /// failure: the test waits forever for a wire the broken code never sends,
    /// and prints nothing. This batch produced two — one wedged a run for seven
    /// minutes, and one hung `cargo test` outright when a session-id mismatch
    /// made a scope check refuse every wire. Both would have been a clean
    /// failure in seconds through this.
    async fn next_wire<T>(rx: &mut tokio::sync::mpsc::Receiver<T>) -> T {
        tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("expected a wire within 2s; none arrived")
            .expect("the sender was dropped before a wire arrived")
    }

    use crate::agents::OutgoingUserMessage;
    use tokio::sync::mpsc;

    /// One participant's stdin, plus the receiver a test reads the wire from.
    ///
    /// `session_id` is a parameter and not a constant because
    /// [`ParticipantInput::deliver`] compares it against the receipt's: an input
    /// built for a session the test does not broadcast into refuses every wire
    /// and the receiver goes quiet. Hardcoding `"s1"` here did exactly that to
    /// `broadcast_solo_delivers_to_brian_only`, whose session is `solo` — and
    /// because the assertion was a bare `recv().await` with a live sender still
    /// in scope, it hung instead of failing.
    fn stub_input(session_id: &str) -> (ParticipantInput, mpsc::Receiver<OutgoingUserMessage>) {
        let (tx, rx) = mpsc::channel(8);
        (ParticipantInput::new(session_id, tx), rx)
    }

    #[tokio::test]
    async fn broadcast_persists_raw_and_envelopes_wire() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "test", None).await.unwrap();
        let (btx, mut brx) = stub_input("s1");
        let (rtx, mut rrx) = stub_input("s1");
        broadcast_user_message(
            &s,
            "s1",
            "hello",
            IpavPhase::Apply,
            None,
            &[("brian", &btx), ("rain", &rtx)],
        )
            .await
            .unwrap();
        let bm = next_wire(&mut brx).await;
        let rm = next_wire(&mut rrx).await;
        assert_eq!(bm.message.content, "[PHASE: Apply]\nhello");
        assert_eq!(rm.message.content, "[PHASE: Apply]\nhello");
        let msgs = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].content, "hello",
            "the BODY is still the raw user text"
        );
        assert_eq!(msgs[0].author, "user");
        // …and the decoration is now recorded beside it, so re-rendering the
        // stored row reproduces both agents' stdin byte for byte. Before B5
        // Task 2 the row said only "hello" and the `[PHASE: Apply]` the agents
        // read existed nowhere the user could see it.
        let row = &s.channel_after("s1", 0, 100).await.unwrap().rows[0];
        assert_eq!(
            render_wire(row.envelope.as_ref(), &row.content),
            bm.message.content
        );
    }

    #[tokio::test]
    async fn broadcast_does_not_leak_to_other_session() {
        // Regression: when the dashboard had tile-reordering bugs, the user
        // worried that broadcasting to session A might land in B. This locks
        // in the contract — broadcast is keyed strictly by session_id.
        let s = Storage::memory().await.unwrap();
        s.create_session("sess-a", "a", None).await.unwrap();
        s.create_session("sess-b", "b", None).await.unwrap();
        let (btx, _brx) = stub_input("sess-a");
        let (rtx, _rrx) = stub_input("sess-a");
        broadcast_user_message(
            &s,
            "sess-a",
            "msg-into-a",
            IpavPhase::Investigate,
            None,
            &[("brian", &btx), ("rain", &rtx)],
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
        let (btx, mut brx) = stub_input("solo");
        broadcast_user_message(&s, "solo", "hi", IpavPhase::Apply, None, &[("brian", &btx)])
            .await
            .unwrap();
        let bm = next_wire(&mut brx).await;
        assert_eq!(bm.message.content, "[PHASE: Apply]\nhi");
        assert_eq!(s.messages_for_session("solo", None).await.unwrap().len(), 1);
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
        let (btx, mut brx) = stub_input("s1");
        broadcast_user_message(&s, "s1", "go", IpavPhase::Verify, None, &[("brian", &btx)])
            .await
            .unwrap();
        let bm = next_wire(&mut brx).await;
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
        let (btx, mut brx) = stub_input("s1");
        broadcast_user_message(
            &s,
            "s1",
            "do the thing",
            IpavPhase::Apply,
            Some("[System: previous turn interrupted — verify workspace.]"),
            &[("brian", &btx)],
        )
        .await
        .unwrap();
        let bm = next_wire(&mut brx).await;
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
