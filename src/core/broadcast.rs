//! The user's message, persisted as the one row every participant reads.
//!
//! Named `broadcast` from when it wrote every agent's stdin; since rc3 D19 it
//! writes none of them. The row IS the broadcast — the ring hands it to each
//! participant off its own cursor, one turn at a time.
//!
//! Lives separately so it can be mocked in tests.

use crate::core::ipav::IpavPhase;
use crate::storage::{Envelope, MessageKind, Storage};
use anyhow::Result;

/// Persist a user-originated message. The ring delivers it; see the module doc.
pub async fn broadcast_user_message(
    storage: &Storage,
    session_id: &str,
    text: &str,
    phase: IpavPhase,
    // Optional WIRE-ONLY system note prepended to the body — e.g. the
    // post-cancel reconciliation directive. NOT persisted: storage keeps the
    // raw user text, so chat history stays clean (like the findings banner).
    system_prefix: Option<&str>,
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
    // **The row IS the delivery. This no longer writes anyone's stdin** (rc3
    // D19, 2026-08-13).
    //
    // It used to fan the text into EVERY participant's stdin directly, which
    // made the ring decorative: every participant woke on every user message
    // regardless of whose turn it was, and a participant woken outside its turn
    // snapshots its epoch before the ring has published one. It then carried
    // `epoch = 0` forever, so every completion it sent was discarded and the
    // cycle could not step past the first slot. Measured live in `s-cc30fc19`:
    // slot 0 carried epoch 3, slots 1 and 2 carried 0, and the ring advanced
    // exactly one place per user message.
    //
    // This is the same argument the router deletion made — two paths delivering
    // into one stdin, only one of which the ring can reason about. The caller
    // now tells the ring instead (`SignalingBridge::notify_ring_user_message`),
    // which resets the cycle and hands the turn to the front; every participant
    // reads the row off its own cursor when its turn comes, which is what "a
    // turn is a PULL" has always meant.
    Ok(persisted.message_id())
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::render_wire;

    /// **These tests moved with the mechanism (rc3 D19).** They used to assert
    /// the WIRE each participant received, because `broadcast_user_message` wrote
    /// every stdin directly. It no longer does: the row is the delivery, and the
    /// ring hands it out off each participant's cursor. So what is checked here
    /// is what is PERSISTED and how it RENDERS — the same content the ring will
    /// deliver, one layer down.
    ///
    /// The old direct-write is what made the ring decorative; see the comment on
    /// the function for the epoch failure it caused.
    async fn session(id: &str) -> Storage {
        let s = Storage::memory().await.unwrap();
        s.create_session(id, "test", None).await.unwrap();
        s
    }

    /// The one row a broadcast writes, as `(raw stored content, rendered wire)`
    /// — the two things that must differ, and the pair every test here is about.
    async fn only_row(s: &Storage, id: &str) -> (String, String) {
        let page = s.channel_after(id, 0, 100).await.unwrap();
        assert_eq!(page.rows.len(), 1, "a broadcast writes exactly one row");
        let row = page.rows.into_iter().next().unwrap();
        let wire = render_wire(row.envelope.as_ref(), &row.content);
        (row.content, wire)
    }

    #[tokio::test]
    async fn broadcast_persists_raw_and_envelopes_the_render() {
        let s = session("s1").await;
        broadcast_user_message(&s, "s1", "hello", IpavPhase::Apply, None)
            .await
            .unwrap();
        let (content, wire) = only_row(&s, "s1").await;
        // Storage keeps the RAW user text; the phase lives in the envelope and
        // appears only when the row is rendered for a participant.
        assert_eq!(content, "hello");
        assert_eq!(wire, "[PHASE: Apply]\nhello");
    }

    #[tokio::test]
    async fn a_broadcast_writes_only_into_its_own_session() {
        let s = session("s1").await;
        s.create_session("s2", "other", None).await.unwrap();
        broadcast_user_message(&s, "s1", "for s1", IpavPhase::Investigate, None)
            .await
            .unwrap();
        assert_eq!(only_row(&s, "s1").await.0, "for s1");
        assert!(
            s.channel_after("s2", 0, 100).await.unwrap().rows.is_empty(),
            "a broadcast must not appear in another session's channel"
        );
    }

    #[tokio::test]
    async fn broadcast_user_message_carries_findings_banner() {
        let s = session("s1").await;
        s.insert_finding(
            "s1",
            "f-1",
            "eyes",
            crate::storage::FindingSeverity::Blocking,
            "a real problem",
            None,
        )
        .await
        .unwrap();
        broadcast_user_message(&s, "s1", "carry on", IpavPhase::Apply, None)
            .await
            .unwrap();
        let (content, wire) = only_row(&s, "s1").await;
        assert!(
            wire.contains("blocking"),
            "an open blocking finding must ride the render the participant reads: {wire}"
        );
        // Storage still holds the user's own words, unprefixed.
        assert_eq!(content, "carry on");
    }

    #[tokio::test]
    async fn system_prefix_rides_the_render_not_storage() {
        let s = session("s1").await;
        broadcast_user_message(
            &s,
            "s1",
            "go on",
            IpavPhase::Verify,
            Some("[System: reconcile first]"),
        )
        .await
        .unwrap();
        let (content, wire) = only_row(&s, "s1").await;
        assert_eq!(
            content, "go on",
            "the prefix is wire-only — chat history keeps the raw user text"
        );
        assert!(wire.contains("reconcile first"));
    }
}
