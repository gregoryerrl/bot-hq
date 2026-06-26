//! Per-agent event pump. Persists agent events to storage, fans text chunks
//! out to the peer with the IPAV buffer rule.

use crate::agents::{AgentEvent, AgentHealth, OutgoingUserMessage};
use crate::core::activity::ActivityTracker;
use crate::core::ipav::{IpavPhase, IpavState};
use crate::core::router::RouterCommand;
use crate::signaling::SignalingBridge;
use crate::storage::{Author, MessageKind, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
// Test-only since Batch 6 removed the buffered-window timer (the sole non-test
// `Duration` user); the test sleeps below still need it.
#[cfg(test)]
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, warn};

#[derive(Clone)]
pub struct DuoConfig {
    pub session_id: String,
    pub author: Author,
    /// Sender to the central peer-forward router (`core::router`). The pump emits
    /// a `RouterCommand::Forward` here on each completed turn that buffered prose;
    /// the router is the single decision point (forward / suppress / break the
    /// volley) and owns the interleaved convergence stream + the await-halt check.
    /// `None` = solo session (no peer, no router) or tests that don't route.
    pub router_tx: Option<mpsc::Sender<RouterCommand>>,
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
    /// Per-agent liveness for the Batch 7 stall watchdog — the pump touches it on
    /// every event and tracks tools-in-flight. `None` in tests / solo configs
    /// that don't run the watchdog.
    pub liveness: Option<Arc<crate::core::watchdog::AgentLiveness>>,
}

impl DuoConfig {
    pub fn new(session_id: impl Into<String>, author: Author) -> Self {
        Self {
            session_id: session_id.into(),
            author,
            router_tx: None,
            bridge: None,
            self_input_tx: None,
            activity: None,
            in_atomic_tool: None,
            liveness: None,
        }
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

/// True for the `peer_ack` MCP tool call — the bare alias (tests) or the
/// MCP-prefixed wire name (`mcp__bot-hq-signaling__peer_ack`). When the pump
/// sees this ToolUse, it suppresses the turn's peer-forward: the agent
/// explicitly acknowledged its peer without wanting to wake it for a full turn.
/// Behavioral happy-path layer ON TOP of the L2 volley-breaker, never a
/// replacement (weak models that never call it still hit L2).
fn is_peer_ack_tool(name: &str) -> bool {
    name == "peer_ack" || name.ends_with("__peer_ack")
}

/// Pump events from one agent. Each text chunk is persisted; the peer-forward
/// path depends on the current IPAV phase. `TurnComplete` flushes pending
/// buffered text immediately regardless of phase.
pub async fn pump_agent(
    cfg: DuoConfig,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    storage: Storage,
    ipav_state: Arc<Mutex<IpavState>>,
) {
    let mut buffer = String::new();
    // peer_ack (behavioral layer): set when the agent calls the `peer_ack` tool
    // during this turn; consumed at the turn's flush to suppress that turn's
    // peer-forward. Per-turn — reset after every TurnComplete (success OR error).
    let mut peer_ack_pending = false;
    // A3a: one-shot guard so Brian gets at most one "you're mutating before
    // Apply" nudge per session (delivered to his own stdin via self_input_tx).
    let mut mutate_nudged = false;
    // Batch 3.1 Part 1: the tool_use_id of an in-flight atomic op (git commit/
    // push/migration), so a cancel can defer the kill until it completes. We
    // match the clearing ToolResult by id — claude-code can emit parallel tool
    // calls, so clearing on ANY result would race a still-running commit.
    let mut atomic_tool_id: Option<String> = None;

    loop {
        let Some(event) = event_rx.recv().await else { break };

        // Batch 7: any event means the agent is alive — reset the stall timer.
        if let Some(liveness) = &cfg.liveness {
            liveness.touch();
        }

        match event {
            AgentEvent::Text(text) => {
                match storage
                    .insert_message(&cfg.session_id, cfg.author, MessageKind::Text, &text)
                    .await
                {
                    Ok(id) => cfg.notify_persisted(id),
                    Err(e) => warn!(?e, "persisting text"),
                }

                buffer.push_str(&text);
                buffer.push('\n');
            }
            AgentEvent::ToolUse { id, name, input } => {
                // peer_ack: the agent explicitly acked its peer this turn — flag it
                // so this turn's Forward tells the router to suppress the wake.
                if is_peer_ack_tool(&name) {
                    peer_ack_pending = true;
                }
                // Batch 7: a tool call started — suppress stall detection until
                // its ToolResult (a long build/install emits no events meanwhile).
                if let Some(liveness) = &cfg.liveness {
                    liveness.tool_started();
                }
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
                // Batch 7: tool result returned — one fewer tool in flight.
                if let Some(liveness) = &cfg.liveness {
                    liveness.tool_finished();
                }
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
                // The router owns self-idle on the forward path (it sequences
                // peer-busy BEFORE this agent's idle → no momentary Idle flicker).
                // The pump owns self-idle only when it does NOT hand a Forward to
                // the router: an errored turn, an empty buffer, or a solo session.
                let mut router_owns_idle = false;
                if is_error {
                    // Failed turn (API/permission error). The error text is already
                    // persisted per-chunk above for UI visibility, but must NOT be
                    // peer-forwarded: forwarding it bounces the error to the peer,
                    // the peer replies, and that re-triggers this failing agent — an
                    // unbounded error-spam loop (Rain on the DeepSeek gateway,
                    // 2026-05-29). Drain silently.
                    debug!(agent = ?cfg.author, "errored turn; draining buffer without router-forward");
                    buffer.clear();
                } else {
                    // Hand the turn's buffered prose to the central router (the
                    // single forward decision point). Empty/whitespace buffers and
                    // solo sessions (no router_tx) forward nothing.
                    let body = std::mem::take(&mut buffer);
                    if !body.trim().is_empty() {
                        if let Some(router_tx) = &cfg.router_tx {
                            // Only delegate self-idle to the router if the Forward
                            // actually reached it. If the router is gone (channel
                            // closed — e.g. it panicked), fall through to clear our
                            // own busy below, so a dead router can't strand this
                            // agent Busy with the chat input locked.
                            match router_tx
                                .send(RouterCommand::Forward {
                                    from: cfg.author,
                                    body,
                                    peer_ack: peer_ack_pending,
                                })
                                .await
                            {
                                Ok(()) => router_owns_idle = true,
                                // The router task is gone (channel closed — panic or
                                // an early exit). This agent's whole turn of prose is
                                // DROPPED and never reaches its peer. Was silent —
                                // the exact invisible one-way break. Make it loud.
                                Err(_) => warn!(
                                    agent = ?cfg.author,
                                    "peer-forward DROPPED: router channel closed (router task gone) — this turn's prose did not reach the peer"
                                ),
                            }
                        }
                    }
                }
                // peer_ack is per-turn — reset after BOTH branches so an errored
                // turn (which skips the router) can't leak the flag into the next.
                peer_ack_pending = false;
                // Turn ended → this agent is idle, UNLESS we handed off to the
                // router (which clears it after setting the peer busy, avoiding the
                // momentary `Idle` flicker that would unlock the input mid-handoff).
                if !router_owns_idle {
                    if let Some(activity) = &cfg.activity {
                        activity.set_busy(cfg.author, false);
                    }
                }
                // Batch 7: turn done → no tools can still be in flight; reset so a
                // stranded ToolUse-without-ToolResult can't wedge stall detection.
                if let Some(liveness) = &cfg.liveness {
                    liveness.reset_tools();
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
                // Best-effort: forward any trailing buffered prose to the peer
                // before we go.
                let body = std::mem::take(&mut buffer);
                if !body.trim().is_empty() {
                    if let Some(router_tx) = &cfg.router_tx {
                        if router_tx
                            .send(RouterCommand::Forward {
                                from: cfg.author,
                                body,
                                peer_ack: peer_ack_pending,
                            })
                            .await
                            .is_err()
                        {
                            warn!(
                                agent = ?cfg.author,
                                "peer-forward DROPPED on exit: router channel closed — trailing prose did not reach the peer"
                            );
                        }
                    }
                }
                // The agent is dying → force self-idle unconditionally (the
                // post-loop cleanup also clears it; idempotent).
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

    fn fast_cfg(author: Author) -> DuoConfig {
        DuoConfig {
            session_id: "s1".into(),
            author,
            router_tx: None,
            bridge: None,
            self_input_tx: None,
            activity: None,
            in_atomic_tool: None,
            liveness: None,
        }
    }

    /// A cfg wired to a fresh router command channel, returning the receiver so a
    /// test can assert WHICH `RouterCommand`s the pump emits. The pump's contract
    /// is "emit the right Forward on a completed turn"; the router's decision logic
    /// (forward / suppress / break) is tested in `core::router`.
    fn cfg_with_route(author: Author) -> (DuoConfig, mpsc::Receiver<RouterCommand>) {
        let (route_tx, route_rx) = mpsc::channel(16);
        let cfg = DuoConfig {
            router_tx: Some(route_tx),
            ..fast_cfg(author)
        };
        (cfg, route_rx)
    }

    /// Pull one `RouterCommand::Forward` from `rx` → (from, body, peer_ack).
    fn next_forward(rx: &mut mpsc::Receiver<RouterCommand>) -> Option<(Author, String, bool)> {
        match rx.try_recv() {
            Ok(RouterCommand::Forward {
                from,
                body,
                peer_ack,
            }) => Some((from, body, peer_ack)),
            Err(_) => None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn errored_turn_emits_no_forward() {
        // Regression (Rain on the DeepSeek gateway, 2026-05-29): a turn that ends
        // in an API error must NOT be forwarded. Forwarding the error bounces it to
        // the peer, the peer replies, and that re-triggers the failing agent — an
        // unbounded error-spam loop. The pump must emit NO RouterCommand; the error
        // text is still persisted (UI visibility).
        let (storage, state) = setup().await;
        let (cfg, mut route_rx) = cfg_with_route(Author::Rain);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

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

        drop(ev_tx);
        task.await.unwrap();
        assert!(
            next_forward(&mut route_rx).is_none(),
            "errored turn must emit no Forward (would re-trigger the loop)"
        );
        // Persisted for UI visibility even though not forwarded.
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].content.contains("API Error"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn text_emits_forward_only_on_turn_complete() {
        // I/P is turn-based: text does NOT emit a Forward mid-turn; the pump emits
        // exactly one Forward on TurnComplete carrying the buffered text.
        let (storage, state) = setup().await; // default phase = Investigate
        let (cfg, mut route_rx) = cfg_with_route(Author::Brian);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        ev_tx.send(AgentEvent::Text("hello".into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            next_forward(&mut route_rx).is_none(),
            "must not emit a Forward mid-turn (before TurnComplete)"
        );

        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let (from, body, peer_ack) =
            next_forward(&mut route_rx).expect("Forward on turn complete");
        assert_eq!(from, Author::Brian);
        assert!(body.contains("hello"));
        assert!(!peer_ack);
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].author, "brian");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_phase_coalesces_into_one_forward() {
        let (storage, state) = setup().await;
        state.lock().await.current_phase = IpavPhase::Apply;

        let (cfg, mut route_rx) = cfg_with_route(Author::Brian);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state.clone()));

        ev_tx.send(AgentEvent::Text("step 1".into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            next_forward(&mut route_rx).is_none(),
            "no Forward mid-turn in Apply"
        );

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
        drop(ev_tx);
        task.await.unwrap();

        let (_, body, _) = next_forward(&mut route_rx).expect("Forward on turn complete");
        assert!(body.contains("step 1"));
        assert!(body.contains("step 2"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn turn_complete_emits_forward() {
        let (storage, state) = setup().await;
        let (cfg, mut route_rx) = cfg_with_route(Author::Brian);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage, state));

        ev_tx.send(AgentEvent::Text("quick".into())).await.unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: Some("end_turn".into()),
                subtype: None,
                is_error: false,
                api_error_status: None,
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let (_, body, _) = next_forward(&mut route_rx).expect("flushed to router");
        assert!(body.contains("quick"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_use_persists_but_emits_no_forward() {
        let (storage, state) = setup().await;
        let (cfg, mut route_rx) = cfg_with_route(Author::Brian);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu1".into(),
                name: "ask_user_choice".into(),
                input: serde_json::json!({"question":"?","options":["a","b"]}),
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        assert!(
            next_forward(&mut route_rx).is_none(),
            "tool use alone emits no Forward"
        );
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].kind, "tool_use");
    }

    #[test]
    fn is_peer_ack_tool_matches_bare_and_prefixed() {
        // Bare alias (tests) + the real MCP-prefixed wire name both match.
        assert!(is_peer_ack_tool("peer_ack"));
        assert!(is_peer_ack_tool("mcp__bot-hq-signaling__peer_ack"));
        // Other tools + near-misses without the MCP `__` separator do NOT match.
        assert!(!is_peer_ack_tool("ask_user_choice"));
        assert!(!is_peer_ack_tool("Edit"));
        assert!(!is_peer_ack_tool("keeper_ack"));
        assert!(!is_peer_ack_tool("speer_ack"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn peer_ack_sets_flag_in_forward() {
        // peer_ack is PASSED THROUGH to the router (which suppresses the wake): the
        // pump emits a Forward with peer_ack=true. The text is still persisted.
        let (storage, state) = setup().await;
        let (cfg, mut route_rx) = cfg_with_route(Author::Brian);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        ev_tx
            .send(AgentEvent::Text("Agreed — nothing to add.".into()))
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_ack".into(),
                // The real wire name is MCP-prefixed.
                name: "mcp__bot-hq-signaling__peer_ack".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let (_, _, peer_ack) = next_forward(&mut route_rx).expect("Forward emitted");
        assert!(peer_ack, "peer_ack tool must set peer_ack=true in the Forward");
        // The agent's text is still persisted for the user.
        let msgs = storage.messages_for_session("s1", None).await.unwrap();
        assert!(
            msgs.iter()
                .any(|m| m.content.contains("Agreed — nothing to add.")),
            "peer_ack must still persist the agent's text"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn peer_ack_flag_is_per_turn() {
        // The peer_ack flag applies only to the turn it was called in: turn 1's
        // Forward carries peer_ack=true, turn 2's (no ack) carries peer_ack=false.
        let (storage, state) = setup().await;
        let (cfg, mut route_rx) = cfg_with_route(Author::Brian);
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(8);
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage.clone(), state));

        // Turn 1: peer_ack.
        ev_tx.send(AgentEvent::Text("acked".into())).await.unwrap();
        ev_tx
            .send(AgentEvent::ToolUse {
                id: "tu_ack".into(),
                name: "peer_ack".into(),
                input: serde_json::json!({}),
            })
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
            })
            .await
            .unwrap();

        // Turn 2: no peer_ack.
        ev_tx
            .send(AgentEvent::Text("real follow-up".into()))
            .await
            .unwrap();
        ev_tx
            .send(AgentEvent::TurnComplete {
                stop_reason: None,
                subtype: None,
                is_error: false,
                api_error_status: None,
            })
            .await
            .unwrap();
        drop(ev_tx);
        task.await.unwrap();

        let (_, b1, ack1) = next_forward(&mut route_rx).expect("turn 1 Forward");
        assert!(ack1 && b1.contains("acked"));
        let (_, b2, ack2) = next_forward(&mut route_rx).expect("turn 2 Forward");
        assert!(!ack2 && b2.contains("real follow-up"));
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
            ..fast_cfg(Author::Brian)
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage, state));

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
            ..fast_cfg(Author::Brian)
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage, state));

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
            ..fast_cfg(Author::Brian)
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage, state));

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
            ..fast_cfg(Author::Brian)
        };
        let task = tokio::spawn(pump_agent(cfg, ev_rx, storage, state));

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
