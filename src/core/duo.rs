//! Per-agent event pump. Persists agent events to storage, fans text chunks
//! out to the peer with the IPAV buffer rule.

use crate::agents::{AgentEvent, AgentHealth, OutgoingUserMessage};
use crate::core::activity::ActivityTracker;
use crate::core::broadcast::peer_forward_message;
use crate::core::ipav::{IpavPhase, IpavState};
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
    /// `SessionState::broadcast` (core::state) on the user's next message.
    pub awaiting: Option<Arc<AtomicBool>>,
    /// Optional bridge for firing MessagePersisted events after every
    /// successful storage.insert_message. None in tests that don't need
    /// event-driven readers.
    pub bridge: Option<Arc<SignalingBridge>>,
    /// The agent's OWN stdin sender (distinct from the peer's `peer_input_tx`),
    /// for A3a self-nudges — e.g. nudging Brian when he mutates during
    /// Investigate/Plan. `None` disables self-nudging (Rain; tests that don't
    /// need it). Set only for Brian's pump at spawn.
    pub self_input_tx: Option<mpsc::Sender<OutgoingUserMessage>>,
    /// Per-session activity tracker (interrupt redesign, Batch 2). The pump
    /// clears this agent's `busy` on `TurnComplete`/`Exited`, and sets the
    /// PEER's `busy` when it forwards a chunk. `None` in tests / solo configs
    /// that don't drive the input lock.
    pub activity: Option<Arc<ActivityTracker>>,
    /// Shared "this agent is mid-atomic-tool" flag (interrupt redesign, Batch
    /// 3.1 Part 1). The pump sets it on an atomic `ToolUse` (git commit/push/
    /// migration) and clears it on the matching `ToolResult`/`TurnComplete`, so
    /// `cancel_session_turn` can DEFER a kill until the op completes (no
    /// half-written worktree). Shared session-level; only HANDS trips it. `None`
    /// in tests / solo configs that don't drive cancel deferral.
    pub in_atomic_tool: Option<Arc<AtomicBool>>,
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
            self_input_tx: None,
            activity: None,
            in_atomic_tool: None,
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

/// True for a tool call that performs an atomic, hard-to-resume mutation — a
/// `git commit`/`git push` or a DB migration. A cancel arriving mid-flight
/// should DEFER the agent kill until such an op finishes, so the working tree /
/// repo isn't left half-written (interrupt redesign, Batch 3.1 Part 1). Matches
/// HANDS's two atomic-op surfaces: a direct `Bash` command, or an `action_gate`
/// (a gated command — surfaced MCP-prefixed as
/// `mcp__bot-hq-signaling__action_gate`, so match by suffix). Rain is read-only
/// and never trips this. The `migrate` match is deliberately broad (sqlx /
/// artisan / rails / npm): a false positive only defers a kill briefly (8s-
/// capped, self-clears on the ToolResult); a false negative is the exact bug
/// this prevents.
fn is_atomic_command(name: &str, input: &serde_json::Value) -> bool {
    let is_command_surface = name == "Bash" || name.ends_with("action_gate");
    if !is_command_surface {
        return false;
    }
    let cmd = input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    cmd.contains("git commit") || cmd.contains("git push") || cmd.contains("migrate")
}

/// Pump events from one agent. Each text chunk is persisted; the peer-forward
/// path depends on the current IPAV phase. `TurnComplete` flushes pending
/// buffered text immediately regardless of phase.
pub async fn pump_agent(
    cfg: DuoConfig,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    // None = solo session (Rain disabled): events still persist to storage, but
    // there's no peer to forward prose to.
    peer_input_tx: Option<mpsc::Sender<OutgoingUserMessage>>,
    storage: Storage,
    ipav_state: Arc<Mutex<IpavState>>,
) {
    let mut buffer = String::new();
    let mut flush_at: Option<Instant> = None;
    // Circuit breaker for the idle-volley antipattern: counts consecutive
    // short (ack-shaped) peer-forwards. A substantive forward or any tool use
    // resets it. Once it trips, further short forwards are suppressed so a
    // novel ack phrasing the keyword list doesn't catch can't volley forever.
    let mut consecutive_short: u32 = 0;
    // A3a: one-shot guard so Brian gets at most one "you're mutating before
    // Apply" nudge per session (delivered to his own stdin via self_input_tx).
    let mut mutate_nudged = false;
    // Batch 3.1 Part 1: the tool_use_id of an in-flight atomic op (git commit/
    // push/migration), so a cancel can defer the kill until it completes. We
    // match the clearing ToolResult by id — claude-code can emit parallel tool
    // calls, so clearing on ANY result would race a still-running commit.
    let mut atomic_tool_id: Option<String> = None;

    loop {
        let event = match flush_at {
            Some(deadline) => {
                let now = Instant::now();
                if deadline <= now {
                    flush_buffer(&cfg, &mut buffer, peer_input_tx.as_ref(), &mut flush_at, &ipav_state, &mut consecutive_short).await;
                    continue;
                }
                let remaining = deadline - now;
                tokio::select! {
                    biased;
                    ev = event_rx.recv() => ev,
                    _ = tokio::time::sleep(remaining) => {
                        flush_buffer(&cfg, &mut buffer, peer_input_tx.as_ref(), &mut flush_at, &ipav_state, &mut consecutive_short).await;
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
                // A tool call is substantive activity — reset the idle breaker
                // so post-work ack chunks aren't pre-emptively suppressed.
                consecutive_short = 0;
                // Batch 3.1 Part 1: flag an atomic op (git commit/push/
                // migration) so a cancel defers the kill until it completes.
                // Shared session flag; only HANDS trips it (Rain is read-only).
                if let Some(flag) = cfg.in_atomic_tool.as_ref() {
                    if is_atomic_command(&name, &input) {
                        flag.store(true, Ordering::Release);
                        atomic_tool_id = Some(id.clone());
                    }
                }
                // A3a (adherence): catch Brian mutating before the Apply phase —
                // a one-time self-nudge to advance first. Brian-only (Rain can't
                // mutate), gated by adherence_nudges, fired at most once.
                if !mutate_nudged
                    && matches!(cfg.author, Author::Brian)
                    && matches!(name.as_str(), "Edit" | "Write" | "NotebookEdit")
                {
                    if let Some(tx) = cfg.self_input_tx.as_ref() {
                        let phase = ipav_state.lock().await.current_phase;
                        if matches!(phase, IpavPhase::Investigate | IpavPhase::Plan)
                            && storage.adherence_nudges_enabled().await
                        {
                            let _ = tx
                                .send(OutgoingUserMessage::text(
                                    "🔔 You're editing files before the Apply phase. Per IPAV, \
                                     mutations belong in Apply — call advance_phase(\"Apply\") \
                                     first, or note why this edit is intentional. (One-time \
                                     reminder.)",
                                ))
                                .await;
                            mutate_nudged = true;
                        }
                    }
                }
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
                // Batch 3.1 Part 1: clear the atomic-op flag once THIS op's
                // result returns (id-matched → parallel-call safe).
                if atomic_tool_id.as_deref() == Some(tool_use_id.as_str()) {
                    if let Some(flag) = cfg.in_atomic_tool.as_ref() {
                        flag.store(false, Ordering::Release);
                    }
                    atomic_tool_id = None;
                }
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
            AgentEvent::TurnComplete { is_error, .. } => {
                if is_error {
                    // Failed turn (API/permission error). The error text is
                    // already persisted per-chunk above for UI visibility, but
                    // must NOT be peer-forwarded: forwarding it bounces the
                    // error to the peer, the peer replies, and that re-triggers
                    // this failing agent — an unbounded error-spam loop (Rain
                    // on the DeepSeek gateway, 2026-05-29). Drain silently.
                    debug!(agent = ?cfg.author, "errored turn; draining buffer without peer-forward");
                    buffer.clear();
                    flush_at = None;
                } else {
                    // Always flush on a successful turn-complete, both phases.
                    flush_buffer(&cfg, &mut buffer, peer_input_tx.as_ref(), &mut flush_at, &ipav_state, &mut consecutive_short).await;
                }
                // Turn ended → this agent is idle. Runs AFTER flush_buffer so a
                // peer hand-off (which set the peer busy) keeps the session
                // `Busy` with no momentary `Idle` flicker between the two.
                if let Some(activity) = &cfg.activity {
                    activity.set_busy(cfg.author, false);
                }
                // Batch 3.1 Part 1: safety-clear a stranded atomic-op flag at
                // turn end (an atomic ToolUse with no matching ToolResult
                // shouldn't happen, but never strand the flag → never wedge a
                // future cancel). Guarded by our own id so this pump can't clear
                // a flag it didn't set (the flag is HANDS-only; Rain's pump
                // never holds an id).
                if atomic_tool_id.is_some() {
                    if let Some(flag) = cfg.in_atomic_tool.as_ref() {
                        flag.store(false, Ordering::Release);
                    }
                    atomic_tool_id = None;
                }
            }
            AgentEvent::Init { session_id, .. } => {
                debug!(agent = ?cfg.author, ?session_id, "init received");
                // Persist the claude-code session UUID so the next reopen of
                // this bot-hq session can resume each agent's prior context
                // via `--resume <uuid>`. Idempotent UPDATE — on a resume spawn
                // the same UUID comes back and we just overwrite with itself.
                if let Some(claude_id) = session_id {
                    if let Err(e) = storage
                        .set_session_claude_id(&cfg.session_id, cfg.author.as_str(), &claude_id)
                        .await
                    {
                        warn!(?e, agent = ?cfg.author, "persisting claude session id");
                    }
                }
            }
            AgentEvent::Exited(msg) => {
                warn!(agent = ?cfg.author, msg = %msg, "agent exited");
                flush_buffer(&cfg, &mut buffer, peer_input_tx.as_ref(), &mut flush_at, &ipav_state, &mut consecutive_short).await;
                if let Some(activity) = &cfg.activity {
                    activity.set_busy(cfg.author, false);
                }
                break;
            }
            AgentEvent::Error(msg) => {
                warn!(agent = ?cfg.author, msg = %msg, "agent error");
            }
            AgentEvent::Health(state) => {
                // B2: relay the retry-supervisor's liveness transition to the
                // UI as a health dot. Not persisted — purely a status signal.
                if let Some(bridge) = &cfg.bridge {
                    bridge.notify_agent_health(
                        cfg.session_id.clone(),
                        cfg.author.as_str(),
                        state.as_str(),
                    );
                }
            }
        }
    }

    // Pump terminated (channel closed — the supervisor suppresses per-incarnation
    // Exited events, so a closed channel is the reliable "agent stopped" signal).
    // Clear its busy unconditionally so a crashed/stopped agent can't strand the
    // session `Busy` with the chat input locked.
    if let Some(activity) = &cfg.activity {
        activity.set_busy(cfg.author, false);
    }
    // Batch 3.1 Part 1: crashed/stopped mid-atomic-tool → clear the flag so a
    // pending deferred cancel can proceed (the agent's already dead) and a
    // respawn isn't blocked. Guarded by our own id (Rain's pump never sets it).
    if atomic_tool_id.is_some() {
        if let Some(flag) = cfg.in_atomic_tool.as_ref() {
            flag.store(false, Ordering::Release);
        }
    }
    // B2: the event loop ended → the agent's supervisor returned (exhausted
    // retries / permanent error / process exit / intentional close). Flag it
    // dead so the UI dot goes red. On an intentional close the session is being
    // removed anyway, so a late "dead" is harmless.
    if let Some(bridge) = &cfg.bridge {
        bridge.notify_agent_health(
            cfg.session_id.clone(),
            cfg.author.as_str(),
            AgentHealth::Dead.as_str(),
        );
    }
}

async fn flush_buffer(
    cfg: &DuoConfig,
    buffer: &mut String,
    peer_input_tx: Option<&mpsc::Sender<OutgoingUserMessage>>,
    flush_at: &mut Option<Instant>,
    ipav_state: &Arc<Mutex<IpavState>>,
    consecutive_short: &mut u32,
) {
    if buffer.trim().is_empty() {
        *flush_at = None;
        buffer.clear();
        return;
    }
    // Solo session (Rain disabled): no peer. Chunks were already persisted to
    // storage in the Text arm, so just drop the forward buffer.
    let Some(peer_input_tx) = peer_input_tx else {
        buffer.clear();
        *flush_at = None;
        return;
    };
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
    // Heartbeat suppression: short ack-style chunks ("Holding.", "Idle.",
    // "No response needed.", "(Silent.)", "[Silent — awaiting]", etc.) get
    // persisted to storage for UI visibility but NOT forwarded to the peer.
    // Forwarding them triggers the alternating-volley antipattern where Brian
    // + Rain bounce content-free acknowledgments back to each other while
    // idling. Belt-and-suspenders with the prompt-side rule.
    if is_heartbeat_ack(&body) {
        debug!(agent = ?cfg.author, body = %body.trim(), "heartbeat ack; skipping peer forward");
        *flush_at = None;
        return;
    }
    // Circuit breaker: even if the keyword list misses a novel ack phrasing,
    // a run of short forwards is the volley's signature. Forward the first
    // few (some are legit: "Done.", "Pushed."), but once too many short
    // chunks volley in a row with no substantive message resetting the count,
    // suppress further short forwards until a long one breaks the streak.
    let is_short = body.trim().len() <= HEARTBEAT_MAX_LEN;
    if is_short {
        *consecutive_short += 1;
    } else {
        *consecutive_short = 0;
    }
    if is_short && *consecutive_short > MAX_CONSECUTIVE_SHORT_FORWARDS {
        debug!(
            agent = ?cfg.author,
            streak = *consecutive_short,
            body = %body.trim(),
            "idle-volley breaker tripped; skipping peer forward"
        );
        *flush_at = None;
        return;
    }
    let phase = ipav_state.lock().await.current_phase;
    // Ride the open-blocking-findings banner on every peer forward, so it reaches
    // HANDS each turn. Fail-safe 0 when there's no bridge (solo / test config).
    let open_blocking = match &cfg.bridge {
        Some(bridge) => bridge.open_blocking_count(&cfg.session_id).await,
        None => 0,
    };
    peer_forward_message(cfg.author, body.trim_end(), phase, open_blocking, peer_input_tx).await;
    // The peer just received input → it's now busy (this is the duo's
    // turn-start signal). Set AFTER the forward so a failed send (dead peer)
    // doesn't wrongly mark it busy.
    if let Some(activity) = &cfg.activity {
        activity.set_busy(cfg.peer_author, true);
    }
    *flush_at = None;
}

/// Max chars for a chunk to count as "short/ack-shaped" — shared by the
/// keyword matcher and the circuit breaker.
const HEARTBEAT_MAX_LEN: usize = 80;

/// How many consecutive short forwards to allow before the idle-volley
/// breaker suppresses further short chunks. Generous enough for legit rapid
/// exchanges ("Done." / "Pushed." / "Confirmed."), tight enough to kill a
/// runaway volley within a few turns.
const MAX_CONSECUTIVE_SHORT_FORWARDS: u32 = 4;

/// Identify a short ack-style chunk that should NOT be forwarded to the
/// peer (purely an idle "I'm here" volley). Stays conservative on purpose:
/// substantive acks ("Confirmed. The data at line 1580 is correct.") read
/// as long-enough or non-heartbeat-prefixed and slip through.
///
/// KNOWN false-positive (accepted, not a bug): a SHORT (≤ HEARTBEAT_MAX_LEN)
/// message that OPENS with a lead but carries real content ("Holding. Check
/// line 42.") IS suppressed — the prefix match can't tell an idle continuation
/// ("Awaiting your direction.") from a substantive one. Bounded: the chunk is
/// still persisted (the UI shows it), the length cap keeps the blast radius
/// small, and the idle-volley breaker is the backstop. Pinned by
/// `heartbeat_ack_lead_plus_short_content_is_a_known_false_positive`.
///
/// Matching (case-insensitive, after trim, length ≤ HEARTBEAT_MAX_LEN):
/// the chunk's leading bracket/paren/emphasis chars are stripped, then the
/// remainder is matched against a list of idle lead phrases. This catches
/// both `[Silent — awaiting]` and `(Silent.)` and bare `Silent.` with one
/// list. Observed live volley phrasings ("No response needed.", "(Silent.)",
/// "Idle.", "(No further messages.)") are all covered.
fn is_heartbeat_ack(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() > HEARTBEAT_MAX_LEN {
        return false;
    }
    // Strip leading wrapper chars so "(Silent.)", "[Silent …]", "*Idle*"
    // all normalize to the same stem as the bare word.
    let norm = trimmed
        .trim_start_matches(['(', '[', ')', ']', '*', '_', '-', '—', ' '])
        .to_lowercase();
    const HEARTBEAT_LEADS: &[&str] = &[
        "holding",
        "standing by",
        "on standby",
        "ready when you are",
        "silent",
        "awaiting",
        "idle",
        "no response needed",
        "no response required",
        "no further message",
        "no further input",
        "no further action",
        "nothing to add",
        "nothing further",
        "no action needed",
        "no action required",
    ];
    HEARTBEAT_LEADS.iter().any(|lead| norm.starts_with(lead))
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
            self_input_tx: None,
            activity: None,
            in_atomic_tool: None,
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
        // 2026-06-17 cross-model survey: pure acks the prefix list previously
        // missed (slipped through to the circuit breaker only on a runaway streak).
        assert!(is_heartbeat_ack("Ready when you are."));
        assert!(is_heartbeat_ack("On standby."));
        assert!(is_heartbeat_ack("On standby for the next step."));
        // Phrases from the 2026-05-28 volley that the original
        // list missed (the bug): bare + parenthesized forms.
        assert!(is_heartbeat_ack("No response needed."));
        assert!(is_heartbeat_ack("(Silent.)"));
        assert!(is_heartbeat_ack("Idle."));
        assert!(is_heartbeat_ack("(Idle.)"));
        assert!(is_heartbeat_ack("(No further messages.)"));
        assert!(is_heartbeat_ack("(No further messages from me.)"));
        assert!(is_heartbeat_ack("Nothing to add. Standing by."));
        assert!(is_heartbeat_ack("(Silent — standing by.)"));
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

    #[test]
    fn heartbeat_ack_lead_plus_short_content_is_a_known_false_positive() {
        // KNOWN, ACCEPTED LIMITATION (not a bug): is_heartbeat_ack is a prefix
        // match, so a SHORT (<= HEARTBEAT_MAX_LEN) message that opens with a lead
        // but carries real content IS suppressed from peer-forward. The matcher
        // deliberately allows idle continuations ("Awaiting your direction.",
        // "Holding silently."), so there's no syntactic way to separate those
        // from substantive tails. Harm is bounded: the chunk is still PERSISTED
        // (the user sees it in the UI) and the idle-volley breaker backstops it.
        // These assertions pin the boundary — a future matcher change (e.g. to
        // whole-message matching) would flip them and force an explicit decision.
        assert!(is_heartbeat_ack("Holding. Check the error in line 42."));
        assert!(is_heartbeat_ack("Nothing further to add. The build failed."));
        assert!(is_heartbeat_ack("Ready when you are. The logs show error #1452."));

        // The HEARTBEAT_MAX_LEN cap is what bounds the blast radius: the SAME
        // leads with enough trailing content cross the cap and forward normally.
        assert!(!is_heartbeat_ack(
            "Holding. Check the error in line 42 of spawn.rs — the deny-list builder \
             dropped a verb, so the read forms are blocked again."
        ));
        assert!(!is_heartbeat_ack(
            "Nothing further to add. The build failed on the release gate; see the cargo \
             output above for the failing test name and the assertion message."
        ));
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
            Some(peer_tx),
            storage.clone(),
            state.clone(),
        ));

        ev_tx
            .send(AgentEvent::Text(
                "Holding. No browser actions from me this time.".into(),
            ))
            .await
            .unwrap();
        ev_tx.send(AgentEvent::TurnComplete { stop_reason: None, subtype: None, is_error: false, api_error_status: None }).await.unwrap();

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
    async fn errored_turn_is_not_forwarded_to_peer() {
        // Regression (Rain on the DeepSeek gateway, 2026-05-29): a turn that
        // ends in an API error must NOT be peer-forwarded. Forwarding the
        // error text bounces it to the peer, the peer replies, and that
        // re-triggers the failing agent — an unbounded error-spam loop. The
        // error text is still persisted (UI visibility) but never volleyed.
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (peer_tx, mut peer_rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_agent(
            fast_cfg(Author::Rain, Author::Brian),
            ev_rx,
            Some(peer_tx),
            storage.clone(),
            state.clone(),
        ));

        let err = "API Error: 400 Failed to deserialize the JSON body into the \
                   target type: messages[17].role: unknown variant `system`, \
                   expected `user` or `assistant` at line 1 column 49275";
        ev_tx.send(AgentEvent::Text(err.into())).await.unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: Some("error_during_execution".into()),
                is_error: true,
                api_error_status: None,
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            peer_rx.try_recv().is_err(),
            "errored turn must not be forwarded to peer (would re-trigger the loop)"
        );

        drop(ev_tx);
        task.await.unwrap();
        // Persisted for UI visibility even though not forwarded to the peer.
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].content.contains("API Error"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn idle_volley_breaker_suppresses_runaway_short_forwards() {
        // Novel ack phrasings the keyword list doesn't catch ("Ok.", "Sure.")
        // still volley as short chunks. The breaker forwards the first
        // MAX_CONSECUTIVE_SHORT_FORWARDS, then suppresses the rest.
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (peer_tx, mut peer_rx) = mpsc::channel(64);
        let task = tokio::spawn(pump_agent(
            fast_cfg(Author::Rain, Author::Brian),
            ev_rx,
            Some(peer_tx),
            storage.clone(),
            state.clone(),
        ));

        // Six short, non-heartbeat chunks, each its own turn so each flushes.
        for word in ["Ok.", "Sure.", "Yep.", "Right.", "Fine.", "Cool."] {
            ev_tx.send(AgentEvent::Text(word.into())).await.unwrap();
            ev_tx
                .send(AgentEvent::TurnComplete { stop_reason: None, subtype: None, is_error: false, api_error_status: None })
                .await
                .unwrap();
        }

        drop(ev_tx);
        task.await.unwrap();

        // Exactly MAX_CONSECUTIVE_SHORT_FORWARDS reach the peer; the rest are
        // suppressed by the breaker.
        let mut forwarded = 0;
        while peer_rx.try_recv().is_ok() {
            forwarded += 1;
        }
        assert_eq!(forwarded, MAX_CONSECUTIVE_SHORT_FORWARDS as usize);

        // All six still persisted for UI visibility.
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 6);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn substantive_message_resets_idle_breaker() {
        // A long (substantive) forward resets the short-streak counter, so a
        // later short chunk forwards normally instead of being suppressed.
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(64);
        let (peer_tx, mut peer_rx) = mpsc::channel(64);
        let task = tokio::spawn(pump_agent(
            fast_cfg(Author::Rain, Author::Brian),
            ev_rx,
            Some(peer_tx),
            storage.clone(),
            state.clone(),
        ));

        let long = "This is a substantive review comment that is clearly longer than the heartbeat length cap and carries real content.";
        // 5 shorts (5th suppressed), then a long (resets), then a short (forwards).
        let seq: [&str; 7] = ["a.", "b.", "c.", "d.", "e.", long, "f."];
        for chunk in seq {
            ev_tx.send(AgentEvent::Text(chunk.into())).await.unwrap();
            ev_tx
                .send(AgentEvent::TurnComplete { stop_reason: None, subtype: None, is_error: false, api_error_status: None })
                .await
                .unwrap();
        }
        drop(ev_tx);
        task.await.unwrap();

        let mut got = Vec::new();
        while let Ok(m) = peer_rx.try_recv() {
            got.push(m.message.content);
        }
        // 4 of the first 5 shorts + the long + the trailing short = 6.
        assert_eq!(got.len(), 6, "got: {got:?}");
        assert!(got.iter().any(|c| c.contains("substantive review comment")));
        assert!(got.iter().any(|c| c.contains("f.")), "post-reset short should forward");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn investigate_phase_buffers_text() {
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (peer_tx, mut peer_rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_agent(
            fast_cfg(Author::Brian, Author::Rain),
            ev_rx,
            Some(peer_tx),
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
            Some(peer_tx),
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
                is_error: false,
                api_error_status: None,
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
            Some(peer_tx),
            storage,
            state,
        ));

        ev_tx.send(AgentEvent::Text("quick".into())).await.unwrap();
        // Turn complete fires immediately — should flush without waiting 1.5s.
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: Some("end_turn".into()),
                subtype: None,
                is_error: false,
                api_error_status: None,
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
        let task = tokio::spawn(pump_agent(cfg, ev_rx, Some(peer_tx), storage.clone(), state));

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
            Some(peer_tx),
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

    #[test]
    fn is_atomic_command_matrix() {
        use serde_json::json;
        // Bash + atomic git ops / migrations → true.
        assert!(is_atomic_command("Bash", &json!({"command": "git commit -m x"})));
        assert!(is_atomic_command(
            "Bash",
            &json!({"command": "git push origin main"})
        ));
        assert!(is_atomic_command(
            "Bash",
            &json!({"command": "cd repo && git commit -F /tmp/m"})
        ));
        assert!(is_atomic_command("Bash", &json!({"command": "sqlx migrate run"})));
        assert!(is_atomic_command(
            "Bash",
            &json!({"command": "php artisan migrate"})
        ));
        // action_gate: the real wire name is MCP-prefixed; a bare alias also matches.
        assert!(is_atomic_command(
            "mcp__bot-hq-signaling__action_gate",
            &json!({"command": "git push"})
        ));
        assert!(is_atomic_command(
            "action_gate",
            &json!({"command": "git commit -m x"})
        ));
        // Non-atomic commands on a command surface → false.
        assert!(!is_atomic_command("Bash", &json!({"command": "git status"})));
        assert!(!is_atomic_command("Bash", &json!({"command": "ls -la"})));
        assert!(!is_atomic_command(
            "mcp__bot-hq-signaling__action_gate",
            &json!({"command": "git diff"})
        ));
        // Non-command tool surfaces → false even with a command-ish field.
        assert!(!is_atomic_command("Edit", &json!({"command": "git commit"})));
        assert!(!is_atomic_command("Read", &json!({})));
        // Missing / null command → false (no panic).
        assert!(!is_atomic_command("Bash", &json!({})));
        assert!(!is_atomic_command("Bash", &json!({"command": null})));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn atomic_tool_sets_and_clears_flag() {
        // An atomic ToolUse sets the shared flag; a NON-matching ToolResult does
        // NOT clear it (parallel-call safety); the id-matching ToolResult clears.
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let flag = Arc::new(AtomicBool::new(false));
        let cfg = DuoConfig {
            in_atomic_tool: Some(Arc::clone(&flag)),
            ..fast_cfg(Author::Brian, Author::Rain)
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, None, storage, state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_commit".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command": "git commit -m x"}),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(flag.load(Ordering::Acquire), "atomic ToolUse sets the flag");

        ev_tx
            .send(AgentEvent::ToolResult {
                tool_use_id: "tu_other".into(),
                content: "ok".into(),
                is_error: false,
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            flag.load(Ordering::Acquire),
            "a non-matching ToolResult must NOT clear the flag"
        );

        ev_tx
            .send(AgentEvent::ToolResult {
                tool_use_id: "tu_commit".into(),
                content: "ok".into(),
                is_error: false,
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !flag.load(Ordering::Acquire),
            "the id-matching ToolResult clears the flag"
        );

        drop(ev_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn turn_complete_safety_clears_atomic_flag() {
        // A turn that ends with an atomic op still "in flight" (no ToolResult)
        // must not strand the flag — TurnComplete safety-clears it.
        let (storage, state) = setup().await;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let flag = Arc::new(AtomicBool::new(false));
        let cfg = DuoConfig {
            in_atomic_tool: Some(Arc::clone(&flag)),
            ..fast_cfg(Author::Brian, Author::Rain)
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, None, storage, state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_push".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command": "git push"}),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(flag.load(Ordering::Acquire));

        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !flag.load(Ordering::Acquire),
            "TurnComplete safety-clears a stranded atomic flag"
        );

        drop(ev_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn edit_during_investigate_self_nudges_brian() {
        // A3a: Brian editing in Investigate gets a one-time self-nudge on his
        // OWN stdin (cfg.self_input_tx), pointing him at Apply.
        let (storage, state) = setup().await; // default phase = Investigate
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (self_tx, mut self_rx) = mpsc::channel(8);
        let cfg = DuoConfig {
            self_input_tx: Some(self_tx),
            ..fast_cfg(Author::Brian, Author::Rain)
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, None, storage, state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu1".into(),
                name: "Edit".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();

        let nudge = self_rx.recv().await.expect("self-nudge delivered");
        assert!(nudge.message.content.contains("Apply"));

        drop(ev_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn edit_during_apply_does_not_nudge() {
        // A3a: editing in Apply is correct — no nudge.
        let (storage, state) = setup().await;
        state.lock().await.current_phase = IpavPhase::Apply;
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let (self_tx, mut self_rx) = mpsc::channel(8);
        let cfg = DuoConfig {
            self_input_tx: Some(self_tx),
            ..fast_cfg(Author::Brian, Author::Rain)
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, None, storage, state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu1".into(),
                name: "Write".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(self_rx.try_recv().is_err(), "no nudge in Apply");

        drop(ev_tx);
        task.await.unwrap();
    }
}
