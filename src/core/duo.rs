//! Per-agent event pump. Persists agent events to storage, fans text chunks
//! out to the peer with the IPAV buffer rule.

use crate::agents::{AgentEvent, OutgoingUserMessage};
use crate::core::broadcast::peer_forward_message;
use crate::core::ipav::IpavState;
use crate::signaling::SignalingBridge;
use crate::storage::{Author, MessageKind, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Instant;
use tracing::{debug, warn};

/// Window during which I/P-phase prose chunks accumulate before forwarding.
pub const BUFFER_WINDOW: Duration = Duration::from_millis(1500);

#[derive(Clone)]
pub struct DuoConfig {
    pub session_id: String,
    pub author: Author,
    pub peer_author: Author,
    /// Override the buffer window — useful for tests. Defaults to BUFFER_WINDOW.
    pub buffer_window: Option<Duration>,
    /// Shared "user has been asked, halt the duo" flag. When set, flush_buffer
    /// drains the buffer to storage but does NOT forward to the peer — stops
    /// the Brian/Rain volley while we wait for the user. Cleared by
    /// core.broadcast when the user replies.
    pub awaiting: Option<Arc<AtomicBool>>,
    /// Optional bridge for firing MessagePersisted events after every
    /// successful storage.insert_message. None in tests that don't need
    /// event-driven readers.
    pub bridge: Option<Arc<SignalingBridge>>,
}

impl DuoConfig {
    pub fn new(session_id: impl Into<String>, author: Author, peer_author: Author) -> Self {
        Self {
            session_id: session_id.into(),
            author,
            peer_author,
            buffer_window: None,
            awaiting: None,
            bridge: None,
        }
    }

    fn window(&self) -> Duration {
        self.buffer_window.unwrap_or(BUFFER_WINDOW)
    }

    fn is_awaiting(&self) -> bool {
        self.awaiting
            .as_ref()
            .map(|f| f.load(Ordering::Acquire))
            .unwrap_or(false)
    }

    fn notify_persisted(&self, message_id: i64) {
        if let Some(bridge) = &self.bridge {
            bridge.notify_message_persisted(self.session_id.clone(), message_id);
        }
    }
}

/// Pump events from one agent. Each text chunk is persisted; the peer-forward
/// path depends on the current IPAV phase. `TurnComplete` flushes pending
/// buffered text immediately regardless of phase.
pub async fn pump_agent(
    cfg: DuoConfig,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    peer_input_tx: mpsc::Sender<OutgoingUserMessage>,
    storage: Storage,
    ipav_state: Arc<Mutex<IpavState>>,
) {
    let mut buffer = String::new();
    let mut flush_at: Option<Instant> = None;

    loop {
        let event = match flush_at {
            Some(deadline) => {
                let now = Instant::now();
                if deadline <= now {
                    flush_buffer(&cfg, &mut buffer, &peer_input_tx, &mut flush_at).await;
                    continue;
                }
                let remaining = deadline - now;
                tokio::select! {
                    biased;
                    ev = event_rx.recv() => ev,
                    _ = tokio::time::sleep(remaining) => {
                        flush_buffer(&cfg, &mut buffer, &peer_input_tx, &mut flush_at).await;
                        continue;
                    }
                }
            }
            None => event_rx.recv().await,
        };

        let Some(event) = event else { break };

        match event {
            AgentEvent::Text(text) => {
                match storage
                    .insert_message(&cfg.session_id, cfg.author, MessageKind::Text, &text)
                    .await
                {
                    Ok(id) => cfg.notify_persisted(id),
                    Err(e) => warn!(?e, "persisting text"),
                }

                let phase = ipav_state.lock().await.current_phase;
                buffer.push_str(&text);
                buffer.push('\n');

                if phase.uses_buffered_interleave() && flush_at.is_none() {
                    flush_at = Some(Instant::now() + cfg.window());
                }
            }
            AgentEvent::ToolUse { id, name, input } => {
                let payload = serde_json::json!({
                    "tool_use_id": id,
                    "name": name,
                    "input": input,
                });
                match storage
                    .insert_message(
                        &cfg.session_id,
                        cfg.author,
                        MessageKind::ToolUse,
                        &payload.to_string(),
                    )
                    .await
                {
                    Ok(id) => cfg.notify_persisted(id),
                    Err(e) => warn!(?e, "persisting tool_use"),
                }
            }
            AgentEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let payload = serde_json::json!({
                    "tool_use_id": tool_use_id,
                    "content": content,
                    "is_error": is_error,
                });
                match storage
                    .insert_message(
                        &cfg.session_id,
                        cfg.author,
                        MessageKind::ToolResult,
                        &payload.to_string(),
                    )
                    .await
                {
                    Ok(id) => cfg.notify_persisted(id),
                    Err(e) => warn!(?e, "persisting tool_result"),
                }
            }
            AgentEvent::TurnComplete { .. } => {
                // Always flush on turn-complete, both phases.
                flush_buffer(&cfg, &mut buffer, &peer_input_tx, &mut flush_at).await;
            }
            AgentEvent::Init { .. } => {
                debug!(agent = ?cfg.author, "init received");
            }
            AgentEvent::Exited(msg) => {
                warn!(agent = ?cfg.author, msg = %msg, "agent exited");
                flush_buffer(&cfg, &mut buffer, &peer_input_tx, &mut flush_at).await;
                break;
            }
            AgentEvent::Error(msg) => {
                warn!(agent = ?cfg.author, msg = %msg, "agent error");
            }
        }
    }
}

async fn flush_buffer(
    cfg: &DuoConfig,
    buffer: &mut String,
    peer_input_tx: &mpsc::Sender<OutgoingUserMessage>,
    flush_at: &mut Option<Instant>,
) {
    if buffer.trim().is_empty() {
        *flush_at = None;
        buffer.clear();
        return;
    }
    let body = std::mem::take(buffer);
    // Halt: while the duo is awaiting the user, persist the agent's chunks
    // to storage (so the user sees what they were saying) but DO NOT forward
    // to the peer. Otherwise Rain sees Brian's "I'm waiting for the user"
    // monologue and starts replying, defeating the halt.
    if cfg.is_awaiting() {
        debug!(agent = ?cfg.author, "duo halted (awaiting user); skipping peer forward");
        *flush_at = None;
        return;
    }
    // Heartbeat suppression: short ack-style chunks ("Holding.",
    // "Standing by.", "[Silent — awaiting]", etc.) get persisted to storage
    // for UI visibility but NOT forwarded to the peer. Forwarding them
    // triggers the alternating-volley antipattern where Brian + Rain
    // bounce content-free acknowledgments back to each other while idling
    // between actions. Belt-and-suspenders with the prompt-side rule.
    if is_heartbeat_ack(&body) {
        debug!(agent = ?cfg.author, body = %body.trim(), "heartbeat ack; skipping peer forward");
        *flush_at = None;
        return;
    }
    if let Err(e) = peer_forward_message(cfg.author, body.trim_end(), peer_input_tx).await {
        warn!(?e, "peer forward failed");
    }
    *flush_at = None;
}

/// Identify a short ack-style chunk that should NOT be forwarded to the
/// peer (purely an idle "I'm here" volley). Stays conservative on purpose:
/// substantive acks ("Confirmed. The data at line 1580 is correct.") read
/// as long-enough or non-heartbeat-prefixed and slip through.
///
/// Patterns matched (case-insensitive, after trim, length ≤ 80 chars):
/// - Starts with `[Silent`, `[Awaiting`, `[Holding` (bracketed status
///   markers — always heartbeat by design).
/// - Starts with `holding`, `standing by`, `silent hold`,
///   `awaiting your`, `awaiting user`, `awaiting direction`.
fn is_heartbeat_ack(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() > 80 {
        return false;
    }
    // Bracketed status markers are always heartbeat — short or not.
    let lower_brackets = trimmed.to_lowercase();
    if lower_brackets.starts_with("[silent")
        || lower_brackets.starts_with("[awaiting")
        || lower_brackets.starts_with("[holding")
    {
        return true;
    }
    // Stem matches: the leading phrase tells us this is an ack-vibe message;
    // length cap above ensures it can't carry real content too.
    const HEARTBEAT_LEADS: &[&str] = &[
        "holding",
        "standing by",
        "silent hold",
        "silent —",
        "silent.",
        "awaiting your",
        "awaiting user",
        "awaiting direction",
    ];
    HEARTBEAT_LEADS
        .iter()
        .any(|lead| lower_brackets.starts_with(lead))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ipav::IpavPhase;

    async fn setup() -> (Storage, Arc<Mutex<IpavState>>) {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "test", None).await.unwrap();
        let st = Arc::new(Mutex::new(IpavState::default()));
        (s, st)
    }

    fn fast_cfg(author: Author, peer: Author) -> DuoConfig {
        DuoConfig {
            session_id: "s1".into(),
            author,
            peer_author: peer,
            buffer_window: Some(Duration::from_millis(50)),
            awaiting: None,
            bridge: None,
        }
    }

    #[test]
    fn heartbeat_ack_matches_observed_volleys() {
        // Exact strings emitted by Rain in round-2 testing.
        assert!(is_heartbeat_ack("Holding."));
        assert!(is_heartbeat_ack("Standing by."));
        assert!(is_heartbeat_ack("Holding. No browser actions from me this time."));
        assert!(is_heartbeat_ack("Standing by for Brian's snapshot result."));
        assert!(is_heartbeat_ack("[Silent — awaiting user]"));
        assert!(is_heartbeat_ack("[Silent hold — awaiting END-SSO]"));
        assert!(is_heartbeat_ack("[Awaiting END-SSO from the user driving Brave.]"));
        assert!(is_heartbeat_ack("Awaiting your direction."));
        assert!(is_heartbeat_ack("Holding silently."));
    }

    #[test]
    fn heartbeat_ack_passes_substantive_messages() {
        // These look ack-shaped but carry real content; must NOT be dropped.
        assert!(!is_heartbeat_ack(
            "Confirmed. The link at snapshot line 1580 points to mtu.edu/cs/undergraduate/software/what."
        ));
        assert!(!is_heartbeat_ack("Noted. The OOB message landed; testbuyer's row still needs restoring before we close."));
        assert!(!is_heartbeat_ack("I think we should retry the SSO with the new password and then verify the role assignment in Entra."));
        // Short but not in the heartbeat lead list:
        assert!(!is_heartbeat_ack("Done."));
        assert!(!is_heartbeat_ack("Confirmed."));
        // Empty / whitespace.
        assert!(!is_heartbeat_ack(""));
        assert!(!is_heartbeat_ack("   \n  "));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn heartbeat_chunks_are_not_forwarded_to_peer() {
        // End-to-end: Rain emits "Holding. ..." while Brian is mid-work.
        // Storage receives it (UI must show what Rain "said"), but peer-
        // forwarding does NOT fire so Brian's input_tx stays clean.
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (peer_tx, mut peer_rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_agent(
            fast_cfg(Author::Rain, Author::Brian),
            ev_rx,
            peer_tx,
            storage.clone(),
            state.clone(),
        ));

        ev_tx
            .send(AgentEvent::Text(
                "Holding. No browser actions from me this time.".into(),
            ))
            .await
            .unwrap();
        ev_tx.send(AgentEvent::TurnComplete { stop_reason: None, subtype: None }).await.unwrap();

        // Give the pump a moment, then assert nothing landed in peer_rx.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            peer_rx.try_recv().is_err(),
            "heartbeat ack should not have been forwarded to peer"
        );

        // Storage still has it (UI visibility preserved).
        drop(ev_tx);
        task.await.unwrap();
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].author, "rain");
        assert!(msgs[0].content.contains("Holding."));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn investigate_phase_buffers_text() {
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (peer_tx, mut peer_rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_agent(
            fast_cfg(Author::Brian, Author::Rain),
            ev_rx,
            peer_tx,
            storage.clone(),
            state.clone(),
        ));

        ev_tx.send(AgentEvent::Text("hello".into())).await.unwrap();

        let forwarded = peer_rx.recv().await.expect("buffer flushes after window");
        assert!(forwarded.message.content.contains("hello"));

        drop(ev_tx);
        task.await.unwrap();

        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].author, "brian");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_phase_doesnt_forward_until_turn_complete() {
        let (storage, state) = setup().await;
        state.lock().await.current_phase = IpavPhase::Apply;

        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (peer_tx, mut peer_rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_agent(
            fast_cfg(Author::Brian, Author::Rain),
            ev_rx,
            peer_tx,
            storage.clone(),
            state.clone(),
        ));

        ev_tx.send(AgentEvent::Text("step 1".into())).await.unwrap();
        // Brief real-time wait — A/V should NOT forward.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(peer_rx.try_recv().is_err());

        ev_tx.send(AgentEvent::Text("step 2".into())).await.unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: Some("end_turn".into()),
                subtype: None,
            })
            .await
            .unwrap();

        let forwarded = peer_rx.recv().await.expect("flushes on turn complete");
        assert!(forwarded.message.content.contains("step 1"));
        assert!(forwarded.message.content.contains("step 2"));

        drop(ev_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn turn_complete_flushes_in_both_phases() {
        let (storage, state) = setup().await;
        // Default = Investigate (buffered).
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (peer_tx, mut peer_rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_agent(
            fast_cfg(Author::Brian, Author::Rain),
            ev_rx,
            peer_tx,
            storage,
            state,
        ));

        ev_tx.send(AgentEvent::Text("quick".into())).await.unwrap();
        // Turn complete fires immediately — should flush without waiting 1.5s.
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: Some("end_turn".into()),
                subtype: None,
            })
            .await
            .unwrap();

        let forwarded = peer_rx.recv().await.expect("flushed");
        assert!(forwarded.message.content.contains("quick"));

        drop(ev_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn awaiting_flag_halts_peer_forward() {
        // When the awaiting flag is set, buffered text is dropped instead of
        // forwarded to the peer (storage still receives it, of course). When
        // the flag clears, the next chunk volleys normally again.
        let (storage, state) = setup().await;
        let awaiting = Arc::new(AtomicBool::new(true));
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (peer_tx, mut peer_rx) = mpsc::channel(8);
        let cfg = DuoConfig {
            awaiting: Some(Arc::clone(&awaiting)),
            ..fast_cfg(Author::Brian, Author::Rain)
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, peer_tx, storage.clone(), state));

        // While awaiting, this text is persisted but NOT forwarded.
        ev_tx.send(AgentEvent::Text("halted line".into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(peer_rx.try_recv().is_err(), "halted chunk leaked to peer");

        // Clearing the flag and sending more text causes the next flush to
        // forward — including any newly arrived text.
        awaiting.store(false, Ordering::Release);
        ev_tx.send(AgentEvent::Text("resumed line".into())).await.unwrap();
        let forwarded = peer_rx.recv().await.expect("peer should receive after resume");
        assert!(forwarded.message.content.contains("resumed line"));

        drop(ev_tx);
        task.await.unwrap();

        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 2, "both chunks persisted regardless of halt");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_use_persists_but_doesnt_forward() {
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (peer_tx, mut peer_rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_agent(
            fast_cfg(Author::Brian, Author::Rain),
            ev_rx,
            peer_tx,
            storage.clone(),
            state,
        ));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu1".into(),
                name: "ask_user_choice".into(),
                input: serde_json::json!({"question":"?","options":["a","b"]}),
            })
            .await
            .unwrap();

        // Give the task a chance to process.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(peer_rx.try_recv().is_err());

        drop(ev_tx);
        task.await.unwrap();

        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].kind, "tool_use");
    }
}
