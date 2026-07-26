//! The native agent loop.
//!
//! Owns the turn cycle a `claude-code` subprocess owns today: take an input,
//! call `POST /v1/messages`, emit events, run the tools the model asked for,
//! feed the results back, repeat until the model stops. Returns an
//! [`AgentHandle`], so `supervise`, the duo pump, the router and the UI see
//! exactly what they see for a CLI agent.
//!
//! ## What it does NOT do yet
//!
//! - **No compaction** (B6). Instead there is a hard ceiling — see
//!   [`CONTEXT_CEILING`]. Loud beats silent.
//! - **No conversation persistence** (B7). History lives in this task, so a
//!   supervisor respawn starts a fresh conversation. On the CLI path `--resume`
//!   restores it from claude-code's own store; we have no equivalent yet.
//! - **`read_file` only** (B5 adds `Grep`/`Glob`/`Bash` + the write-verb deny
//!   matcher `Bash` requires).

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use super::mcp_client::{mcp_tools_to_anthropic, McpClient};
use super::profile::{AuthStyle, ProviderProfile, ANTHROPIC_VERSION};
use super::wire::{
    self, build_request, parse_turn, tool_results_message, RequestSpec, ToolCall, ToolOutcome,
    TurnStep,
};
use super::tools;
use crate::agents::protocol::{ControlRequest, OutgoingUserMessage};
use crate::agents::spawn::{AgentEvent, AgentHandle, SpawnConfig};

/// Refuse to start a turn once the window is this full.
///
/// claude-code auto-compacts and is silently rescuing long sessions today; a
/// native loop that neither compacts nor stops would instead hard-fail deep in a
/// request with an opaque upstream error. Stopping at a known threshold with a
/// visible message is the honest interim behaviour until B6.
pub const CONTEXT_CEILING: f64 = 0.85;

/// Runaway-loop guard: tool cycles allowed for a single user input.
pub const MAX_TURNS_PER_INPUT: usize = 50;

/// Appended to the assembled system prompt for a native agent.
///
/// The role prompts are written for claude-code and promise a tool surface the
/// native loop does not implement — `prompts.rs` tells EYES she has `Read`,
/// `Grep`, `Glob`, `WebFetch`, `ToolSearch`, `TodoWrite` and read-only `Bash`.
/// Without this correction a native agent spends its turns calling tools that
/// come back "unknown tool". Everything upstream of this addendum still applies;
/// only the tool inventory changes.
pub const NATIVE_TOOL_ADDENDUM: &str = "\n\n\
---\n\n\
# Your actual tool surface (native loop — this OVERRIDES the tool list above)\n\n\
You are running on bot-hq's own agent loop, not claude-code. The tools named \
earlier in this prompt do NOT all exist here. What you actually have:\n\n\
- **`read_file`** — read one UTF-8 text file, path relative to the repository \
root. This replaces `Read`. Paths outside the repository are refused, so you \
cannot read anything above the working repo.\n\
- **Every `mcp__bot-hq-signaling__*` tool** — the full bot-hq surface \
(`cl_index_search`, `cl_retrieve`, `session_doc_*`, `eyes_flag`, `peer_ack`, \
`web_search`, `terminal_read`, …). Role enforcement is unchanged: HANDS-only \
tools are still refused to you.\n\n\
**Not available:** `Grep`, `Glob`, `Bash`, `WebFetch`, `WebSearch`, `Edit`, \
`Write`, `Task`, `TodoWrite`. Do not call them — they return an error and waste \
the turn. In particular you have **no `Bash`**, so you cannot run `git log`, \
`git diff`, `git status` or any other shell command. To see what changed, read \
Brian's `session_doc_search(phase=\"apply\")` doc and `read_file` the files he \
names; ask him to paste output you cannot obtain yourself.\n\n\
`ToolSearch` does not exist and is not needed: every tool you have is passed on \
every request, so there is nothing deferred to search for.\n\n\
For anything outside the repository, use `mcp__bot-hq-signaling__web_search`.\n";

/// A `POST /v1/messages` failure.
#[derive(Debug, Clone)]
pub struct ApiFailure {
    /// Upstream HTTP status, when there was a response at all. `None` for a
    /// transport-level failure (DNS, connect, TLS). The supervisor classifies
    /// on this exact value via `is_transient_api_error`.
    pub status: Option<u16>,
    pub detail: String,
}

/// The HTTP call, abstracted so the loop is testable without a network.
pub trait MessagesTransport: Send + Sync + 'static {
    fn send(&self, body: Value) -> BoxFuture<'_, Result<Value, ApiFailure>>;
}

/// The real transport.
pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
    auth: AuthStyle,
    token: Option<String>,
}

impl HttpTransport {
    pub fn new(url: String, auth: AuthStyle, token: Option<String>) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .build()
                .context("building Messages HTTP client")?,
            url,
            auth,
            token,
        })
    }
}

impl MessagesTransport for HttpTransport {
    fn send(&self, body: Value) -> BoxFuture<'_, Result<Value, ApiFailure>> {
        Box::pin(async move {
            // `reqwest` is `default-features = false` here, so `.json()` does
            // not exist — serialize by hand.
            let payload = serde_json::to_vec(&body).map_err(|e| ApiFailure {
                status: None,
                detail: format!("serializing request: {e}"),
            })?;

            let mut req = self
                .client
                .post(&self.url)
                .header("content-type", "application/json")
                .header("anthropic-version", ANTHROPIC_VERSION);

            if let Some(token) = &self.token {
                req = match self.auth {
                    AuthStyle::XApiKey => req.header("x-api-key", token),
                    AuthStyle::Bearer => req.header("authorization", format!("Bearer {token}")),
                };
            }

            let resp = req.body(payload).send().await.map_err(|e| ApiFailure {
                status: e.status().map(|s| s.as_u16()),
                detail: format!("POST /v1/messages failed: {e}"),
            })?;

            let status = resp.status().as_u16();
            let text = resp.text().await.map_err(|e| ApiFailure {
                status: Some(status),
                detail: format!("reading response body: {e}"),
            })?;

            if !(200..300).contains(&status) {
                // Body verbatim — the API's error messages name the exact
                // offending field, which is most of the debugging value.
                return Err(ApiFailure {
                    status: Some(status),
                    detail: text,
                });
            }

            serde_json::from_str(&text).map_err(|e| ApiFailure {
                status: Some(status),
                detail: format!("parsing response JSON: {e}"),
            })
        })
    }
}

/// Everything the loop needs that isn't a channel.
pub struct LoopConfig {
    pub agent_name: String,
    pub model: String,
    pub profile: ProviderProfile,
    pub system_prompt: String,
    /// Canonicalized read root for the built-in tools.
    pub root: PathBuf,
    /// Anthropic-shaped `tools` entries: bot-hq's MCP tools plus the built-ins.
    pub tools: Vec<Value>,
}

/// Loop state that survives across inputs.
struct State {
    history: Vec<Value>,
    /// Latched once the context ceiling is breached. The loop stays alive and
    /// keeps refusing — it does NOT close its event channel, because closure is
    /// the supervisor's end-of-incarnation signal and would trigger a respawn
    /// with no history (worse than stopping, until B7).
    ceiling_reached: bool,
}

/// Run the loop until input closes or a kill arrives.
#[allow(clippy::too_many_arguments)]
pub async fn run_loop(
    cfg: LoopConfig,
    transport: Arc<dyn MessagesTransport>,
    mcp: Option<Arc<McpClient>>,
    event_tx: mpsc::Sender<AgentEvent>,
    mut input_rx: mpsc::Receiver<OutgoingUserMessage>,
    mut control_rx: mpsc::Receiver<ControlRequest>,
    mut kill_rx: oneshot::Receiver<()>,
) {
    // No claude-code session UUID exists for a native agent. `None` is honest:
    // the supervisor simply never sets `resume_session_id`, which is correct
    // until B7 gives us our own persistence.
    if event_tx
        .send(AgentEvent::Init { session_id: None })
        .await
        .is_err()
    {
        return;
    }

    let mut state = State {
        history: Vec::new(),
        ceiling_reached: false,
    };
    let mut control_open = true;

    loop {
        let msg = tokio::select! {
            biased;
            _ = &mut kill_rx => return,
            ctl = control_rx.recv(), if control_open => {
                // Nothing is in flight here — an interrupt between turns is a
                // no-op, exactly as it is for an idle subprocess.
                if ctl.is_none() { control_open = false; }
                continue;
            }
            msg = input_rx.recv() => match msg {
                Some(m) => m,
                // Handle dropped → tear down. This is the ONE place the loop
                // ends by closing its event channel, which is what the
                // supervisor reads as end-of-incarnation.
                None => return,
            },
        };

        push_user_text(&mut state.history, &msg.message.content);

        if state.ceiling_reached {
            emit_ceiling_refusal(&cfg, &event_tx).await;
            continue;
        }

        // Inner scope so the `&mut state` borrow held by `turns` ends before the
        // interrupt repair below needs it again.
        let outcome = {
            let turns = run_turns(&cfg, &transport, mcp.as_ref(), &mut state, &event_tx);
            tokio::pin!(turns);

            tokio::select! {
                biased;
                _ = &mut kill_rx => Outcome::Killed,
                ctl = control_rx.recv(), if control_open => match ctl {
                    Some(_) => Outcome::Interrupted,
                    None => Outcome::ControlClosed,
                },
                done = &mut turns => match done {
                    Ok(()) => Outcome::Finished,
                    Err(()) => Outcome::EventChannelClosed,
                },
            }
        };

        match outcome {
            Outcome::Killed | Outcome::EventChannelClosed => return,
            Outcome::Finished => {}
            Outcome::ControlClosed => control_open = false,
            Outcome::Interrupted => {
                // The abandoned turn can leave a trailing assistant `tool_use`
                // whose results never arrived, which 400s the NEXT request.
                repair_dangling_tool_use(&mut state);
                info!(agent = %cfg.agent_name, "native turn interrupted");
                // Matches the CLI path's abort shape: an errored turn, so
                // partial text is not peer-forwarded.
                let _ = event_tx
                    .send(wire::turn_complete_err(None, "aborted_streaming"))
                    .await;
            }
        }
    }
}

/// How one input's turn sequence ended.
enum Outcome {
    Finished,
    Interrupted,
    Killed,
    ControlClosed,
    EventChannelClosed,
}

/// One input's worth of turns. `Err(())` means the event channel closed.
async fn run_turns(
    cfg: &LoopConfig,
    transport: &Arc<dyn MessagesTransport>,
    mcp: Option<&Arc<McpClient>>,
    state: &mut State,
    event_tx: &mpsc::Sender<AgentEvent>,
) -> Result<(), ()> {
    for _turn in 1..=MAX_TURNS_PER_INPUT {
        let body = build_request(&RequestSpec {
            model: &cfg.model,
            max_tokens: cfg.profile.default_max_tokens,
            system: Some(&cfg.system_prompt),
            tools: &cfg.tools,
            messages: &state.history,
        });

        let resp = match transport.send(body).await {
            Ok(v) => v,
            Err(f) => {
                warn!(agent = %cfg.agent_name, status = ?f.status, detail = %f.detail, "native turn failed");
                send(event_tx, AgentEvent::Error(f.detail)).await?;
                send(event_tx, wire::turn_complete_err(f.status, "api_error")).await?;
                return Ok(());
            }
        };

        let parsed = parse_turn(&resp, &cfg.model, cfg.profile.context_window);

        // Absolute occupancy, logged every turn whether or not a window is
        // known. No provider currently declares one (`profile.rs`), so
        // `parsed.context` is `None` and the UI meter shows a gap — this line is
        // then the ONLY record of how fast a real session fills, which is the
        // measurement B6's compaction design depends on.
        if let Some(used) = parsed.used_tokens {
            info!(
                agent = %cfg.agent_name,
                model = %cfg.model,
                used_tokens = used,
                history_messages = state.history.len(),
                window = ?cfg.profile.context_window,
                "native turn accounting"
            );
        }

        for ev in parsed.events {
            send(event_tx, ev).await?;
        }
        if let Some(m) = parsed.assistant_message {
            state.history.push(m);
        }

        // Checked AFTER the turn is accounted for, so the reading that trips the
        // ceiling is itself reported.
        let over_ceiling = parsed
            .context
            .as_ref()
            .is_some_and(|c| c.fraction() >= CONTEXT_CEILING);

        match parsed.step {
            TurnStep::ToolUse { calls } => {
                let outcomes = exec_calls(cfg, mcp, &calls).await;
                for o in &outcomes {
                    send(
                        event_tx,
                        AgentEvent::ToolResult {
                            tool_use_id: o.tool_use_id.clone(),
                            content: o.content.clone(),
                            is_error: o.is_error,
                        },
                    )
                    .await?;
                }
                state.history.push(tool_results_message(&outcomes));

                if over_ceiling {
                    state.ceiling_reached = true;
                    emit_ceiling_refusal(cfg, event_tx).await;
                    return Ok(());
                }
                continue;
            }
            TurnStep::End { .. } => {
                if over_ceiling {
                    state.ceiling_reached = true;
                }
                send(
                    event_tx,
                    wire::turn_complete_ok("end_turn", parsed.context),
                )
                .await?;
                if state.ceiling_reached {
                    emit_ceiling_refusal(cfg, event_tx).await;
                }
                return Ok(());
            }
            TurnStep::Refusal { details } => {
                send(
                    event_tx,
                    AgentEvent::Error(format!("model declined the request ({details})")),
                )
                .await?;
                send(event_tx, wire::turn_complete_err(None, "refusal")).await?;
                return Ok(());
            }
            TurnStep::MaxTokens => {
                send(
                    event_tx,
                    AgentEvent::Error(format!(
                        "hit max_tokens ({}); on Opus-class models thinking and response \
                         text share this budget, so raise it rather than assuming the \
                         answer was long",
                        cfg.profile.default_max_tokens
                    )),
                )
                .await?;
                send(event_tx, wire::turn_complete_err(None, "max_tokens")).await?;
                return Ok(());
            }
            TurnStep::Unhandled { stop_reason } => {
                send(
                    event_tx,
                    AgentEvent::Error(format!("unhandled stop_reason {stop_reason:?}")),
                )
                .await?;
                send(event_tx, wire::turn_complete_err(None, "unhandled_stop")).await?;
                return Ok(());
            }
        }
    }

    send(
        event_tx,
        AgentEvent::Error(format!(
            "gave up after {MAX_TURNS_PER_INPUT} tool cycles without finishing the turn"
        )),
    )
    .await?;
    send(event_tx, wire::turn_complete_err(None, "max_turns")).await?;
    Ok(())
}

/// Run every requested tool. Built-ins are handled locally; everything else goes
/// to bot-hq's signaling MCP server, where role enforcement lives.
async fn exec_calls(
    cfg: &LoopConfig,
    mcp: Option<&Arc<McpClient>>,
    calls: &[ToolCall],
) -> Vec<ToolOutcome> {
    let mut out = Vec::with_capacity(calls.len());
    for call in calls {
        if tools::handles(&call.name) {
            out.push(tools::exec(call, &cfg.root));
        } else if let Some(mcp) = mcp {
            out.push(mcp.call_tool(&call.id, &call.name, call.input.clone()).await);
        } else {
            out.push(ToolOutcome {
                tool_use_id: call.id.clone(),
                content: format!("tool {:?} is not available to this agent", call.name),
                is_error: true,
            });
        }
    }
    out
}

/// Append user text to the history, MERGING into a trailing user message rather
/// than stacking a second one.
///
/// Two `role: "user"` entries in a row is a shape the Messages API is not
/// guaranteed to accept, and the loop can produce one without this: the
/// interrupt-repair path appends synthetic `tool_result`s (which are a user
/// message), and the very next input would otherwise push a second user message
/// straight after. The ceiling-refusal path can stack them too. Merging is
/// cheap and removes the whole class.
fn push_user_text(history: &mut Vec<Value>, text: &str) {
    let block = json!({ "type": "text", "text": text });

    if let Some(last) = history.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some("user") {
            match last.get_mut("content") {
                Some(Value::Array(blocks)) => {
                    blocks.push(block);
                    return;
                }
                Some(existing @ Value::String(_)) => {
                    let prior = existing.take();
                    *existing = json!([{ "type": "text", "text": prior }, block]);
                    return;
                }
                _ => {}
            }
        }
    }
    history.push(json!({ "role": "user", "content": [block] }));
}

/// An interrupt can drop the turn between "assistant asked for tools" and "the
/// results were appended". The API requires every `tool_use` to be answered, so
/// leaving the gap 400s the next request. Answer them as interrupted instead.
fn repair_dangling_tool_use(state: &mut State) {
    let Some(last) = state.history.last() else {
        return;
    };
    if last.get("role").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    let ids: Vec<String> = last
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                .filter_map(|b| b.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    if ids.is_empty() {
        return;
    }
    let outcomes: Vec<ToolOutcome> = ids
        .into_iter()
        .map(|id| ToolOutcome {
            tool_use_id: id,
            content: "interrupted by the user before this tool ran".into(),
            is_error: true,
        })
        .collect();
    state.history.push(tool_results_message(&outcomes));
}

async fn emit_ceiling_refusal(cfg: &LoopConfig, event_tx: &mpsc::Sender<AgentEvent>) {
    let pct = (CONTEXT_CEILING * 100.0).round() as u32;
    let _ = event_tx
        .send(AgentEvent::Error(format!(
            "context window is over {pct}% full — this agent has stopped rather than \
             fail mid-request. Native agents do not auto-compact yet; start a new \
             session to continue."
        )))
        .await;
    let _ = event_tx
        .send(wire::turn_complete_err(None, "context_ceiling"))
        .await;
    warn!(agent = %cfg.agent_name, "native agent hit the context ceiling");
}

async fn send(tx: &mpsc::Sender<AgentEvent>, ev: AgentEvent) -> Result<(), ()> {
    tx.send(ev).await.map_err(|_| ())
}

/// Spawn a native agent from the same [`SpawnConfig`] the CLI path uses.
///
/// Signature-compatible with `spawn_agent`, which is what lets
/// `spawn_supervised_agent`'s `FnMut(SpawnConfig) -> Fut` retry machinery drive
/// it unchanged.
pub async fn spawn_native_agent(cfg: SpawnConfig) -> Result<AgentHandle> {
    let profile = ProviderProfile::for_provider(&cfg.config.provider);
    let url = profile.messages_url(cfg.config.base_url.as_deref());

    // claude-code can fall back to ambient auth (a logged-in CLI, `ANTHROPIC_API_KEY`
    // in the environment); this loop cannot — it only ever sends the token on the
    // model row. Without this check a token-less model spawns fine and then fails
    // every turn with a bare upstream 401 that names nothing actionable.
    if cfg
        .config
        .auth_token
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        anyhow::bail!(
            "model {:?} is set to the native loop but has no auth token. The native \
             loop has no ambient-auth fallback (unlike claude-code) — add the API key \
             to the saved model in Settings → Models, or untick \"Native loop\".",
            cfg.config.model_name
        );
    }

    let mut system_prompt = std::fs::read_to_string(&cfg.system_prompt_path).with_context(|| {
        format!(
            "reading system prompt at {}",
            cfg.system_prompt_path.display()
        )
    })?;
    // The assembled prompt promises claude-code's tool surface; correct it.
    system_prompt.push_str(NATIVE_TOOL_ADDENDUM);

    let root = cfg
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        .canonicalize()
        .context("canonicalizing the agent's read root")?;

    let mcp = match cfg.mcp_config_path.as_ref() {
        Some(p) => {
            let client = Arc::new(McpClient::from_mcp_config(p)?);
            client
                .initialize()
                .await
                .context("handshaking with the signaling MCP server")?;
            Some(client)
        }
        None => None,
    };

    let mut tool_defs = tools::tool_defs();
    if let Some(mcp) = mcp.as_ref() {
        tool_defs.extend(mcp_tools_to_anthropic(&mcp.list_tools().await?));
    }

    let transport = Arc::new(HttpTransport::new(
        url,
        profile.auth,
        cfg.config.auth_token.clone(),
    )?) as Arc<dyn MessagesTransport>;

    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(256);
    let (input_tx, input_rx) = mpsc::channel::<OutgoingUserMessage>(64);
    let (control_tx, control_rx) = mpsc::channel::<ControlRequest>(8);
    let (kill_tx, kill_rx) = oneshot::channel::<()>();

    let name = cfg.agent_name.clone();
    info!(agent = %name, model = %cfg.config.model_name, "spawning native agent");

    tokio::spawn(run_loop(
        LoopConfig {
            agent_name: name.clone(),
            model: cfg.config.model_name.clone(),
            profile,
            system_prompt,
            root,
            tools: tool_defs,
        },
        transport,
        mcp,
        event_tx,
        input_rx,
        control_rx,
        kill_rx,
    ));

    Ok(AgentHandle::from_parts(
        name, event_rx, input_tx, control_tx, kill_tx,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Returns pre-baked responses in order, recording each request body.
    struct ScriptedTransport {
        responses: Mutex<std::collections::VecDeque<Result<Value, ApiFailure>>>,
        seen: Mutex<Vec<Value>>,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<Result<Value, ApiFailure>>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses.into()),
                seen: Mutex::new(Vec::new()),
            })
        }
        fn requests(&self) -> Vec<Value> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl MessagesTransport for ScriptedTransport {
        fn send(&self, body: Value) -> BoxFuture<'_, Result<Value, ApiFailure>> {
            self.seen.lock().unwrap().push(body);
            let next = self.responses.lock().unwrap().pop_front();
            Box::pin(async move {
                next.unwrap_or_else(|| {
                    Err(ApiFailure {
                        status: None,
                        detail: "script exhausted".into(),
                    })
                })
            })
        }
    }

    struct Harness {
        events: mpsc::Receiver<AgentEvent>,
        input: mpsc::Sender<OutgoingUserMessage>,
        /// Held open so the loop's control arm stays live; the interrupt test
        /// wires its own channels rather than going through `start`.
        _control: mpsc::Sender<ControlRequest>,
        _kill: oneshot::Sender<()>,
        _dir: TempDir,
    }

    fn start(
        transport: Arc<dyn MessagesTransport>,
        window: Option<u64>,
    ) -> Harness {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let root = dir.path().canonicalize().unwrap();

        let (event_tx, events) = mpsc::channel(256);
        let (input, input_rx) = mpsc::channel(16);
        let (control, control_rx) = mpsc::channel(4);
        let (kill, kill_rx) = oneshot::channel();

        let mut profile = ProviderProfile::for_provider("anthropic");
        profile.context_window = window;

        tokio::spawn(run_loop(
            LoopConfig {
                agent_name: "rain".into(),
                model: "m".into(),
                profile,
                system_prompt: "sys".into(),
                root,
                tools: tools::tool_defs(),
            },
            transport,
            None,
            event_tx,
            input_rx,
            control_rx,
            kill_rx,
        ));

        Harness {
            events,
            input,
            _control: control,
            _kill: kill,
            _dir: dir,
        }
    }

    fn end_turn(text: &str) -> Value {
        json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": text }]
        })
    }

    async fn next(h: &mut Harness) -> AgentEvent {
        tokio::time::timeout(std::time::Duration::from_secs(5), h.events.recv())
            .await
            .expect("event within 5s")
            .expect("channel open")
    }

    #[tokio::test]
    async fn emits_init_then_text_then_turn_complete() {
        let t = ScriptedTransport::new(vec![Ok(end_turn("hi there"))]);
        let mut h = start(t, None);

        assert!(matches!(next(&mut h).await, AgentEvent::Init { .. }));
        h.input.send(OutgoingUserMessage::text("hello")).await.unwrap();

        assert!(matches!(next(&mut h).await, AgentEvent::Text(t) if t == "hi there"));
        match next(&mut h).await {
            AgentEvent::TurnComplete { is_error, .. } => assert!(!is_error),
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn runs_a_builtin_tool_and_feeds_the_result_back() {
        let t = ScriptedTransport::new(vec![
            Ok(json!({
                "stop_reason": "tool_use",
                "content": [{ "type": "tool_use", "id": "tu_1", "name": "read_file",
                              "input": { "path": "a.txt" } }]
            })),
            Ok(end_turn("the file says hello")),
        ]);
        let mut h = start(t.clone() as Arc<dyn MessagesTransport>, None);

        assert!(matches!(next(&mut h).await, AgentEvent::Init { .. }));
        h.input.send(OutgoingUserMessage::text("read a.txt")).await.unwrap();

        assert!(matches!(next(&mut h).await, AgentEvent::ToolUse { .. }));
        match next(&mut h).await {
            AgentEvent::ToolResult { content, is_error, .. } => {
                assert_eq!(content, "hello");
                assert!(!is_error);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
        assert!(matches!(next(&mut h).await, AgentEvent::Text(_)));

        // Second request must carry the assistant turn AND the tool results.
        let reqs = t.requests();
        assert_eq!(reqs.len(), 2);
        let msgs = reqs[1]["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3, "user, assistant(tool_use), user(tool_result)");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
    }

    #[tokio::test]
    async fn api_failure_surfaces_the_status_the_supervisor_classifies_on() {
        let t = ScriptedTransport::new(vec![Err(ApiFailure {
            status: Some(529),
            detail: "Overloaded".into(),
        })]);
        let mut h = start(t, None);

        assert!(matches!(next(&mut h).await, AgentEvent::Init { .. }));
        h.input.send(OutgoingUserMessage::text("hi")).await.unwrap();

        assert!(matches!(next(&mut h).await, AgentEvent::Error(_)));
        match next(&mut h).await {
            AgentEvent::TurnComplete { is_error, api_error_status, .. } => {
                assert!(is_error);
                assert_eq!(api_error_status, Some(529));
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_failed_turn_does_not_end_the_agent() {
        let t = ScriptedTransport::new(vec![
            Err(ApiFailure { status: Some(500), detail: "boom".into() }),
            Ok(end_turn("recovered")),
        ]);
        let mut h = start(t, None);

        assert!(matches!(next(&mut h).await, AgentEvent::Init { .. }));
        h.input.send(OutgoingUserMessage::text("one")).await.unwrap();
        assert!(matches!(next(&mut h).await, AgentEvent::Error(_)));
        assert!(matches!(next(&mut h).await, AgentEvent::TurnComplete { .. }));

        // Channel still open — the supervisor must not see end-of-incarnation.
        h.input.send(OutgoingUserMessage::text("two")).await.unwrap();
        assert!(matches!(next(&mut h).await, AgentEvent::Text(t) if t == "recovered"));
    }

    #[tokio::test]
    async fn context_ceiling_stops_the_agent_loudly_without_closing_the_channel() {
        // 900/1000 = 90% > the 85% ceiling.
        let over = json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "done" }],
            "usage": { "input_tokens": 900 }
        });
        let t = ScriptedTransport::new(vec![Ok(over)]);
        let mut h = start(t, Some(1_000));

        assert!(matches!(next(&mut h).await, AgentEvent::Init { .. }));
        h.input.send(OutgoingUserMessage::text("hi")).await.unwrap();

        assert!(matches!(next(&mut h).await, AgentEvent::Text(_)));
        assert!(matches!(next(&mut h).await, AgentEvent::TurnComplete { is_error: false, .. }));
        match next(&mut h).await {
            AgentEvent::Error(m) => assert!(m.contains("context window")),
            other => panic!("expected ceiling Error, got {other:?}"),
        }
        assert!(matches!(next(&mut h).await, AgentEvent::TurnComplete { is_error: true, .. }));

        // Latched: the next input is refused without an API call, and the
        // channel stays open so no amnesiac respawn is triggered.
        h.input.send(OutgoingUserMessage::text("again")).await.unwrap();
        match next(&mut h).await {
            AgentEvent::Error(m) => assert!(m.contains("context window")),
            other => panic!("expected latched refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unknown_window_never_trips_the_ceiling() {
        let t = ScriptedTransport::new(vec![Ok(json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "done" }],
            "usage": { "input_tokens": 9_000_000 }
        }))]);
        let mut h = start(t, None);

        assert!(matches!(next(&mut h).await, AgentEvent::Init { .. }));
        h.input.send(OutgoingUserMessage::text("hi")).await.unwrap();
        assert!(matches!(next(&mut h).await, AgentEvent::Text(_)));
        match next(&mut h).await {
            AgentEvent::TurnComplete { is_error, .. } => assert!(!is_error),
            other => panic!("expected a clean TurnComplete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn refusal_is_reported_and_the_agent_survives() {
        let t = ScriptedTransport::new(vec![
            Ok(json!({ "stop_reason": "refusal", "content": [] })),
            Ok(end_turn("ok now")),
        ]);
        let mut h = start(t, None);

        assert!(matches!(next(&mut h).await, AgentEvent::Init { .. }));
        h.input.send(OutgoingUserMessage::text("hi")).await.unwrap();
        match next(&mut h).await {
            AgentEvent::Error(m) => assert!(m.contains("declined")),
            other => panic!("expected Error, got {other:?}"),
        }
        assert!(matches!(next(&mut h).await, AgentEvent::TurnComplete { is_error: true, .. }));

        h.input.send(OutgoingUserMessage::text("retry")).await.unwrap();
        assert!(matches!(next(&mut h).await, AgentEvent::Text(_)));
    }

    #[tokio::test]
    async fn dropping_the_input_sender_closes_the_event_channel() {
        // This is the supervisor's end-of-incarnation signal, so it must work.
        let t = ScriptedTransport::new(vec![]);
        let mut h = start(t, None);
        assert!(matches!(next(&mut h).await, AgentEvent::Init { .. }));

        drop(h.input);
        assert!(h.events.recv().await.is_none(), "event channel must close");
    }

    #[tokio::test]
    async fn a_tool_using_agent_that_never_stops_is_capped() {
        let forever: Vec<Result<Value, ApiFailure>> = (0..MAX_TURNS_PER_INPUT + 2)
            .map(|i| {
                Ok(json!({
                    "stop_reason": "tool_use",
                    "content": [{ "type": "tool_use", "id": format!("tu_{i}"),
                                  "name": "read_file", "input": { "path": "a.txt" } }]
                }))
            })
            .collect();
        let t = ScriptedTransport::new(forever);
        let mut h = start(t, None);

        assert!(matches!(next(&mut h).await, AgentEvent::Init { .. }));
        h.input.send(OutgoingUserMessage::text("spin")).await.unwrap();

        let mut saw_cap = false;
        for _ in 0..(MAX_TURNS_PER_INPUT * 3 + 4) {
            if let AgentEvent::Error(m) = next(&mut h).await {
                if m.contains("gave up after") {
                    saw_cap = true;
                    break;
                }
            }
        }
        assert!(saw_cap, "runaway tool loop must be capped");
    }

    /// Never resolves — stands in for a turn still waiting on the API. Signals
    /// on `entered` first so a test can interrupt at a deterministic point.
    struct HangingTransport {
        entered: mpsc::Sender<()>,
    }

    impl MessagesTransport for HangingTransport {
        fn send(&self, _body: Value) -> BoxFuture<'_, Result<Value, ApiFailure>> {
            let tx = self.entered.clone();
            Box::pin(async move {
                let _ = tx.send(()).await;
                std::future::pending().await
            })
        }
    }

    #[tokio::test]
    async fn an_interrupt_aborts_the_in_flight_turn_and_the_agent_stays_alive() {
        let (event_tx, mut events) = mpsc::channel(64);
        let (input, input_rx) = mpsc::channel(16);
        let (control, control_rx) = mpsc::channel(4);
        let (_kill, kill_rx) = oneshot::channel();
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (entered_tx, mut entered) = mpsc::channel(1);

        tokio::spawn(run_loop(
            LoopConfig {
                agent_name: "rain".into(),
                model: "m".into(),
                profile: ProviderProfile::for_provider("anthropic"),
                system_prompt: "sys".into(),
                root,
                tools: vec![],
            },
            Arc::new(HangingTransport { entered: entered_tx }),
            None,
            event_tx,
            input_rx,
            control_rx,
            kill_rx,
        ));

        assert!(matches!(events.recv().await.unwrap(), AgentEvent::Init { .. }));
        input.send(OutgoingUserMessage::text("hang")).await.unwrap();

        // Wait until the request is genuinely in flight. Interrupting earlier is
        // absorbed by the outer select as a no-op — correct behaviour for an idle
        // agent (the CLI path behaves the same, and the cancel path escalates to
        // a kill if an interrupt doesn't take), but it makes the test race.
        entered.recv().await.expect("transport was called");

        control.send(ControlRequest::interrupt("r1")).await.unwrap();

        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .expect("interrupt must abort the turn within 5s")
            .unwrap();
        match ev {
            AgentEvent::TurnComplete { is_error, subtype, .. } => {
                // Errored so the aborted turn's partial text is not
                // peer-forwarded — same contract as the CLI abort path.
                assert!(is_error);
                assert_eq!(subtype.as_deref(), Some("aborted_streaming"));
            }
            other => panic!("expected an aborted TurnComplete, got {other:?}"),
        }

        // Still alive: the event channel is open, so the supervisor sees no
        // end-of-incarnation and does not respawn.
        assert!(!events.is_closed());
    }

    #[test]
    fn interrupt_repair_answers_a_dangling_tool_use() {
        let mut state = State {
            history: vec![json!({
                "role": "assistant",
                "content": [{ "type": "tool_use", "id": "tu_1", "name": "read_file", "input": {} }]
            })],
            ceiling_reached: false,
        };
        repair_dangling_tool_use(&mut state);

        assert_eq!(state.history.len(), 2);
        let repaired = &state.history[1];
        assert_eq!(repaired["role"], "user");
        assert_eq!(repaired["content"][0]["tool_use_id"], "tu_1");
        assert_eq!(repaired["content"][0]["is_error"], true);
    }

    #[test]
    fn user_text_merges_into_a_trailing_tool_result_message() {
        // The exact shape the interrupt-repair path leaves behind: synthetic
        // tool_results (a user message) followed immediately by fresh input.
        let mut history = vec![tool_results_message(&[ToolOutcome {
            tool_use_id: "tu_1".into(),
            content: "interrupted".into(),
            is_error: true,
        }])];
        push_user_text(&mut history, "try again");

        assert_eq!(history.len(), 1, "must not stack two user messages");
        let blocks = history[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[1]["text"], "try again");
    }

    #[test]
    fn user_text_after_an_assistant_message_starts_a_new_message() {
        let mut history = vec![json!({ "role": "assistant", "content": [] })];
        push_user_text(&mut history, "next");

        assert_eq!(history.len(), 2);
        assert_eq!(history[1]["role"], "user");
        assert_eq!(history[1]["content"][0]["text"], "next");
    }

    #[test]
    fn user_text_merges_into_a_string_content_user_message() {
        let mut history = vec![json!({ "role": "user", "content": "first" })];
        push_user_text(&mut history, "second");

        assert_eq!(history.len(), 1);
        let blocks = history[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["text"], "first");
        assert_eq!(blocks[1]["text"], "second");
    }

    fn spawn_cfg_with_token(token: Option<&str>) -> SpawnConfig {
        SpawnConfig {
            agent_name: "rain".into(),
            config: crate::storage::AgentConfig {
                agent_name: "rain".into(),
                provider: "anthropic".into(),
                model_name: "claude-opus-5".into(),
                base_url: None,
                auth_token: token.map(str::to_string),
                updated_at: String::new(),
                native: true,
            },
            // Deliberately nonexistent: the auth guard must fire before any IO,
            // so this path is never read on the failing case.
            system_prompt_path: std::path::PathBuf::from("/nonexistent/prompt.txt"),
            mcp_config_path: None,
            working_dir: None,
            claude_bin: None,
            session_id: "s-test".into(),
            resume_session_id: None,
            project: None,
            data_dir: std::path::PathBuf::from("/tmp"),
            session_effort: None,
            session_ultracode: None,
        }
    }

    #[tokio::test]
    async fn a_native_model_without_a_token_fails_with_an_actionable_message() {
        // claude-code would fall back to ambient auth; this loop cannot, so a
        // bare upstream 401 on every turn would name nothing useful.
        for token in [None, Some(""), Some("   ")] {
            // `AgentHandle` isn't Debug, so unwrap the Result by hand.
            let msg = match spawn_native_agent(spawn_cfg_with_token(token)).await {
                Ok(_) => panic!("a token-less native model must not spawn"),
                Err(e) => e.to_string(),
            };
            assert!(msg.contains("no auth token"), "got: {msg}");
            assert!(msg.contains("Native loop"), "must name the fix; got: {msg}");
        }
    }

    #[test]
    fn the_native_addendum_names_exactly_the_builtins_that_exist() {
        // Drift here is what sends the agent chasing tools that do not exist.
        for def in tools::tool_defs() {
            let name = def["name"].as_str().unwrap();
            assert!(
                NATIVE_TOOL_ADDENDUM.contains(name),
                "built-in {name} is missing from the addendum"
            );
        }
        for absent in ["Grep", "Glob", "Bash", "WebFetch", "TodoWrite"] {
            assert!(
                NATIVE_TOOL_ADDENDUM.contains(absent),
                "{absent} is promised by prompts.rs and must be retracted here"
            );
        }
    }

    #[test]
    fn interrupt_repair_is_a_noop_when_nothing_dangles() {
        let mut state = State {
            history: vec![json!({
                "role": "assistant",
                "content": [{ "type": "text", "text": "no tools here" }]
            })],
            ceiling_reached: false,
        };
        repair_dangling_tool_use(&mut state);
        assert_eq!(state.history.len(), 1);
    }
}
