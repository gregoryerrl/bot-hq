//! External-driver MCP tool dispatch.
//!
//! Auth is enforced at the HTTP layer (bearer token); by the time a request
//! reaches `dispatch_external`, the caller is authenticated. Tools talk
//! directly to `core::AppState` — no per-agent identity, no HANDS-only gating.
//!
//! See `external_server.rs` for the listener / auth half.

use crate::core::AppState as CoreAppState;
use crate::signaling::bridge::SignalingEvent;
use crate::signaling::protocol::{
    parse_phase_arg, JsonRpcError, JsonRpcRequest, JsonRpcResponse, PROTOCOL_VERSION,
    ToolCallResult, ToolDescriptor,
};
use crate::signaling::response::{ok_response, result_json};
use crate::signaling::tool_args::arg_required_str;
use crate::storage::{AgentConfig as DbAgentConfig, Message};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

fn message_to_json(m: &Message) -> Value {
    json!({
        "id": m.id,
        "author": m.author,
        "kind": m.kind,
        "content": m.content,
        "created_at": m.created_at,
    })
}

fn internal_err(op: &str, e: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, format!("{op}: {e}"))
}

/// Top-level dispatch. Mirrors the internal `signaling::jsonrpc::dispatch`
/// shape so the same hyper plumbing pattern works for both servers.
pub async fn dispatch_external(
    req: JsonRpcRequest,
    core: &Arc<CoreAppState>,
) -> Result<Option<JsonRpcResponse>, JsonRpcError> {
    let id = req.id.clone().unwrap_or(json!(null));
    match req.method.as_str() {
        "initialize" => Ok(Some(JsonRpcResponse::ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "bot-hq",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        ))),
        m if m.starts_with("notifications/") => Ok(None),
        "tools/list" => {
            let tools = external_tool_descriptors();
            Ok(Some(JsonRpcResponse::ok(id, json!({ "tools": tools }))))
        }
        "tools/call" => {
            let params = req
                .params
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing params"))?;
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing tool name"))?
                .to_string();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let result = call_external_tool(&name, args, core).await?;
            Ok(Some(JsonRpcResponse::ok(
                id,
                serde_json::to_value(result).unwrap_or(json!(null)),
            )))
        }
        "ping" => Ok(Some(JsonRpcResponse::ok(id, json!({})))),
        _ => Err(JsonRpcError::new(
            JsonRpcError::METHOD_NOT_FOUND,
            format!("unknown method {}", req.method),
        )),
    }
}

/// Full driver toolset: session lifecycle + phase control + choice resolution
/// + Emma + status + admin (agent configs, violations log).
pub fn external_tool_descriptors() -> &'static [ToolDescriptor] {
    use std::sync::LazyLock;
    static TOOLS: LazyLock<Vec<ToolDescriptor>> = LazyLock::new(|| vec![
        ToolDescriptor {
            name: "list_sessions",
            description: "List active bot-hq sessions (not archived, not closed). Each entry includes id, title, working_repo_path, created_at, and the brian_model_at_spawn / rain_model_at_spawn fields if recorded.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDescriptor {
            name: "create_session",
            description: "Open a new bot-hq session. Spawns Brian (HANDS) + Rain (EYES) subprocesses; returns the session id. The call blocks until both agents have spawned (typically 1-3 seconds). `working_repo_path` is optional — if set, the project name is derived from the path's last component and project-specific policy.yaml is resolved.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Human-readable label shown in the session tile." },
                    "working_repo_path": { "type": "string", "description": "Optional absolute path to a git repo. Drives project-specific policy + system-prompt context." }
                },
                "required": ["title"]
            }),
        },
        ToolDescriptor {
            name: "send_message",
            description: "Send a user-authored message to a session. The message is persisted, fed to both agents (Brian + Rain), and clears any 'awaiting user' halt. For Emma, use session_id=\"emma\".",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Target session id (UUID for duos, literal \"emma\" for the singleton helper)." },
                    "text": { "type": "string", "description": "Message body. No formatting required." }
                },
                "required": ["session_id", "text"]
            }),
        },
        ToolDescriptor {
            name: "get_session_messages",
            description: "Read messages for a session in chronological order. Optional `since_id` returns only messages with id > since_id — use for polling. Each message has id, author (user|emma|brian|rain), kind (text|tool_use|tool_result|phase_change), content, created_at.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id (or \"emma\")." },
                    "since_id": { "type": "integer", "description": "Optional: return only messages with id > this value." }
                },
                "required": ["session_id"]
            }),
        },
        ToolDescriptor {
            name: "advance_phase",
            description: "Move a session to a new IPAV phase. Emits a synthetic phase-change message both agents see. Phases: I (Investigate), P (Plan), A (Apply), V (Verify).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "phase": { "type": "string", "enum": ["I", "P", "A", "V"] }
                },
                "required": ["session_id", "phase"]
            }),
        },
        ToolDescriptor {
            name: "resolve_choice",
            description: "Answer a parked `ask_user_choice` / `request_approval` prompt. Look up choice_id via `get_pending_choices`. The agent's blocking tool-call returns with the picked option as its result.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "choice_id": { "type": "string", "description": "UUID for the parked choice." },
                    "picked": { "type": "string", "description": "The option string to return to the agent." }
                },
                "required": ["choice_id", "picked"]
            }),
        },
        ToolDescriptor {
            name: "close_session",
            description: "Close a session — kills Brian + Rain subprocesses and marks the session row closed/archived. Idempotent: closing an already-closed session is a no-op.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "archive": { "type": "boolean", "description": "If true (default), also archive — removes the session from the dashboard's active list." }
                },
                "required": ["session_id"]
            }),
        },
        ToolDescriptor {
            name: "restart_emma",
            description: "Kill Emma's subprocess and respawn with the current agent_configs row. Use after editing Emma's model/auth via the database directly. Returns ok on success; error if the spawn fails (e.g., missing `claude` binary).",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDescriptor {
            name: "get_emma_messages",
            description: "Read Emma's chat in chronological order. Same shape as get_session_messages but always targets the singleton emma session row.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "since_id": { "type": "integer", "description": "Optional: return only messages with id > this value." }
                }
            }),
        },
        ToolDescriptor {
            name: "get_pending_choices",
            description: "List every choice currently parked in the signaling bridge — choices that an agent is blocking on. Each entry includes choice_id (needed for resolve_choice), session_id, agent, question, and the picker options.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDescriptor {
            name: "get_status",
            description: "Snapshot of bot-hq runtime state — version, signaling address, external MCP address, count of active duo sessions, whether Emma is spawned, and a millisecond-resolution wall-clock timestamp. Useful for client health checks.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDescriptor {
            name: "get_agent_configs",
            description: "List all three agent configs (emma, brian, rain) — provider, model_name, base_url, updated_at. The auth_token is REDACTED: returned as `<unset>` if empty, or `<set:****abcd>` showing only the last 4 chars to confirm which key is loaded. Full secret retrieval is intentionally not exposed.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDescriptor {
            name: "set_agent_config",
            description: "Upsert an agent config row. agent_name must be one of emma/brian/rain. Pass auth_token to set a new credential; pass empty string to clear. Other fields (provider, model_name, base_url) are optional — omit to keep the current value.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_name": { "type": "string", "enum": ["emma", "brian", "rain"] },
                    "provider": { "type": "string", "description": "Optional. e.g. 'anthropic'. Omit to keep current value." },
                    "model_name": { "type": "string", "description": "Optional. e.g. 'claude-opus-4-7'. Omit to keep current value." },
                    "base_url": { "type": "string", "description": "Optional. e.g. 'https://api.anthropic.com/v1'. Empty string clears. Omit to keep current value." },
                    "auth_token": { "type": "string", "description": "Optional. Empty string clears. Omit to keep current value." }
                },
                "required": ["agent_name"]
            }),
        },
        ToolDescriptor {
            name: "get_violations",
            description: "Read recent entries from violations.jsonl. Each record: ts (RFC 3339), session_id, agent, kind, action, outcome (Approved/Denied/Abandoned/Detected), detail. Use `limit` to cap response size; default 100 most-recent.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max records returned. Default 100. Most-recent first." }
                }
            }),
        },
        ToolDescriptor {
            name: "wait_for_change",
            description: "Long-poll for new messages in a session. Returns immediately if storage already has messages with id > since_id; otherwise blocks server-side until a new message arrives (via the bridge's MessagePersisted event) or the timeout expires. Saves AI clients from busy-polling get_session_messages. Returns `{messages: [...]}` (possibly empty on timeout).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id (or \"emma\")." },
                    "since_id": { "type": "integer", "description": "Wait for messages with id > this value. Omit to wait for any new message starting from current state." },
                    "timeout_ms": { "type": "integer", "description": "Max time to block server-side, milliseconds. Default 30000, clamped to [100, 60000]." }
                },
                "required": ["session_id"]
            }),
        },
        ToolDescriptor {
            name: "get_session_snapshot",
            description: "One-shot aggregate read: session metadata + last N messages + current IPAV phase + awaiting-user flag + pending choices for this session. Saves AI clients from chaining list_sessions + get_session_messages + get_pending_choices into a single round trip.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "msg_limit": { "type": "integer", "description": "Max messages returned, most-recent kept. Default 50, clamped to [1, 500]." }
                },
                "required": ["session_id"]
            }),
        },
    ]);
    &*TOOLS
}

/// Block server-side until the session has at least one new message past
/// `since_id`, or `timeout_ms` elapses. On timeout returns an empty Vec.
async fn wait_for_change(
    core: &Arc<CoreAppState>,
    session_id: &str,
    since_id: Option<i64>,
    timeout_ms: u64,
) -> anyhow::Result<Vec<crate::storage::Message>> {
    // Fast path: storage might already have new messages past since_id.
    let initial = core
        .storage
        .messages_for_session(session_id, since_id)
        .await?;
    if !initial.is_empty() {
        return Ok(initial);
    }
    // Subscribe BEFORE the post-subscribe re-check so we don't miss an event
    // that fires between the initial query and the subscribe call.
    let mut rx = core.bridge.subscribe();
    let recheck = core
        .storage
        .messages_for_session(session_id, since_id)
        .await?;
    if !recheck.is_empty() {
        return Ok(recheck);
    }
    let deadline = tokio::time::sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return Ok(Vec::new()),
            ev = rx.recv() => {
                match ev {
                    Ok(SignalingEvent::MessagePersisted { session_id: sid, .. }) if sid == session_id => {
                        let msgs = core.storage.messages_for_session(session_id, since_id).await?;
                        if !msgs.is_empty() {
                            return Ok(msgs);
                        }
                    }
                    // Receiver lag (broadcast channel full) is also a possible signal —
                    // poll once to be safe.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let msgs = core.storage.messages_for_session(session_id, since_id).await?;
                        if !msgs.is_empty() {
                            return Ok(msgs);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Ok(Vec::new());
                    }
                    _ => {} // other event types — ignore, keep waiting
                }
            }
        }
    }
}

/// Redact an auth_token field for read-side display. Returns `<unset>` for
/// empty/None, or `<set:****abcd>` showing only the last 4 chars so the user
/// can verify which credential is loaded without exposing the full secret.
fn redact_auth_token(t: &Option<String>) -> String {
    match t.as_deref() {
        None | Some("") => "<unset>".to_string(),
        Some(s) => {
            let suffix = if s.len() >= 4 { &s[s.len() - 4..] } else { s };
            format!("<set:****{suffix}>")
        }
    }
}

async fn call_external_tool(
    name: &str,
    args: Value,
    core: &Arc<CoreAppState>,
) -> Result<ToolCallResult, JsonRpcError> {
    match name {
        "list_sessions" => {
            let sessions = core.list_active_sessions().await.map_err(|e| {
                internal_err("list_active_sessions", e)
            })?;
            let arr: Vec<_> = sessions
                .into_iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "title": s.title,
                        "working_repo_path": s.working_repo_path,
                        "created_at": s.created_at,
                        "brian_model_at_spawn": s.brian_model_at_spawn,
                        "rain_model_at_spawn": s.rain_model_at_spawn,
                    })
                })
                .collect();
            Ok(result_json(&json!({ "sessions": arr }), "{}"))
        }
        "create_session" => {
            let title = arg_required_str(&args, "title")?;
            let working_repo_path = args
                .get("working_repo_path")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from);
            let session_id = core
                .open_session(title, working_repo_path)
                .await
                .map_err(|e| {
                    internal_err("open_session", e)
                })?;
            Ok(result_json(&json!({ "session_id": session_id }), "{}"))
        }
        "send_message" => {
            let session_id = arg_required_str(&args, "session_id")?;
            let text = arg_required_str(&args, "text")?;
            core.broadcast(&session_id, &text).await.map_err(|e| {
                internal_err("broadcast", e)
            })?;
            Ok(ok_response())
        }
        "get_session_messages" => {
            let session_id = arg_required_str(&args, "session_id")?;
            let since_id = args.get("since_id").and_then(Value::as_i64);
            let msgs = core
                .storage
                .messages_for_session(&session_id, since_id)
                .await
                .map_err(|e| {
                    internal_err("messages_for_session", e)
                })?;
            let arr: Vec<_> = msgs
                .iter()
                .map(message_to_json)
                .collect();
            Ok(result_json(&json!({ "messages": arr }), "{}"))
        }
        "advance_phase" => {
            let session_id = arg_required_str(&args, "session_id")?;
            let phase_str = arg_required_str(&args, "phase")?;
            let phase = parse_phase_arg("phase", &phase_str)?;
            core.advance_phase(&session_id, phase).await.map_err(|e| {
                internal_err("advance_phase", e)
            })?;
            Ok(ok_response())
        }
        "resolve_choice" => {
            let choice_id = arg_required_str(&args, "choice_id")?;
            let picked = arg_required_str(&args, "picked")?;
            core.resolve_choice(&choice_id, picked).await.map_err(|e| {
                internal_err("resolve_choice", e)
            })?;
            Ok(ok_response())
        }
        "close_session" => {
            let session_id = arg_required_str(&args, "session_id")?;
            let archive = args.get("archive").and_then(Value::as_bool).unwrap_or(true);
            core.close_session(&session_id, archive).await.map_err(|e| {
                internal_err("close_session", e)
            })?;
            Ok(ok_response())
        }
        "restart_emma" => {
            core.restart_emma().await.map_err(|e| {
                internal_err("restart_emma", e)
            })?;
            Ok(ok_response())
        }
        "get_emma_messages" => {
            let since_id = args.get("since_id").and_then(Value::as_i64);
            let msgs = core
                .storage
                .messages_for_session("emma", since_id)
                .await
                .map_err(|e| {
                    internal_err("messages_for_session(emma)", e)
                })?;
            let arr: Vec<_> = msgs
                .iter()
                .map(message_to_json)
                .collect();
            Ok(result_json(&json!({ "messages": arr }), "{}"))
        }
        "get_pending_choices" => {
            let choices = core.bridge.list_pending_choices().await;
            let arr: Vec<_> = choices
                .into_iter()
                .map(|c| {
                    json!({
                        "choice_id": c.choice_id,
                        "session_id": c.session_id,
                        "agent": c.agent,
                        "question": c.question,
                        "options": c.options,
                    })
                })
                .collect();
            Ok(result_json(&json!({ "pending_choices": arr }), "{}"))
        }
        "get_status" => {
            let session_count = core.sessions.lock().await.len();
            let emma_started = core.emma.lock().await.is_some();
            let external_addr = core
                .external_server
                .lock()
                .await
                .as_ref()
                .map(|s| s.local_addr.to_string());
            let payload = json!({
                "version": env!("CARGO_PKG_VERSION"),
                "signaling_addr": core.signaling_addr.to_string(),
                "external_mcp_addr": external_addr,
                "active_duo_sessions": session_count,
                "emma_started": emma_started,
                "now": chrono::Utc::now().to_rfc3339(),
            });
            Ok(result_json(&payload, "{}"))
        }
        "get_agent_configs" => {
            let cfgs = core.storage.list_agent_configs().await.map_err(|e| {
                internal_err("list_agent_configs", e)
            })?;
            let arr: Vec<_> = cfgs
                .into_iter()
                .map(|c| {
                    json!({
                        "agent_name": c.agent_name,
                        "provider": c.provider,
                        "model_name": c.model_name,
                        "base_url": c.base_url,
                        "auth_token": redact_auth_token(&c.auth_token),
                        "updated_at": c.updated_at,
                    })
                })
                .collect();
            Ok(result_json(&json!({ "agent_configs": arr }), "{}"))
        }
        "set_agent_config" => {
            let agent_name = arg_required_str(&args, "agent_name")?;
            if !["emma", "brian", "rain"].contains(&agent_name.as_str()) {
                return Err(JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    format!("agent_name must be emma/brian/rain, got {agent_name}"),
                ));
            }
            // Load current, then overlay any provided fields.
            let current = core
                .storage
                .get_agent_config(&agent_name)
                .await
                .map_err(|e| {
                    internal_err("get_agent_config", e)
                })?
                .unwrap_or_else(|| DbAgentConfig {
                    agent_name: agent_name.clone(),
                    provider: "anthropic".to_string(),
                    model_name: String::new(),
                    base_url: None,
                    auth_token: None,
                    updated_at: String::new(),
                });
            let provider = args
                .get("provider")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or(current.provider);
            let model_name = args
                .get("model_name")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or(current.model_name);
            // For base_url / auth_token: empty string clears (None), absent keeps
            // the existing value. This matches the descriptor's documented semantics.
            let base_url = match args.get("base_url").and_then(Value::as_str) {
                Some("") => None,
                Some(s) => Some(s.to_string()),
                None => current.base_url,
            };
            let auth_token = match args.get("auth_token").and_then(Value::as_str) {
                Some("") => None,
                Some(s) => Some(s.to_string()),
                None => current.auth_token,
            };
            let cfg = DbAgentConfig {
                agent_name,
                provider,
                model_name,
                base_url,
                auth_token,
                updated_at: String::new(), // upsert sets datetime('now')
            };
            core.storage.upsert_agent_config(&cfg).await.map_err(|e| {
                internal_err("upsert_agent_config", e)
            })?;
            Ok(ok_response())
        }
        "get_violations" => {
            let limit = args
                .get("limit")
                .and_then(Value::as_i64)
                .filter(|n| *n > 0)
                .unwrap_or(100) as usize;
            let log = core.bridge.violations_log().ok_or_else(|| {
                JsonRpcError::new(
                    JsonRpcError::INTERNAL_ERROR,
                    "violations log not configured (bridge built without policy)",
                )
            })?;
            let mut records = log.read_all().map_err(|e| {
                internal_err("violations read_all", e)
            })?;
            // Most-recent first; cap to `limit`.
            records.reverse();
            records.truncate(limit);
            Ok(result_json(&json!({ "violations": records }), "{}"))
        }
        "wait_for_change" => {
            let session_id = arg_required_str(&args, "session_id")?;
            let since_id = args.get("since_id").and_then(Value::as_i64);
            let timeout_ms = args
                .get("timeout_ms")
                .and_then(Value::as_i64)
                .unwrap_or(30_000)
                .clamp(100, 60_000) as u64;
            let messages = wait_for_change(core, &session_id, since_id, timeout_ms)
                .await
                .map_err(|e| {
                    internal_err("wait_for_change", e)
                })?;
            let arr: Vec<_> = messages
                .iter()
                .map(message_to_json)
                .collect();
            Ok(result_json(&json!({ "messages": arr }), "{}"))
        }
        "get_session_snapshot" => {
            let session_id = arg_required_str(&args, "session_id")?;
            let msg_limit = args
                .get("msg_limit")
                .and_then(Value::as_i64)
                .unwrap_or(50)
                .clamp(1, 500) as usize;

            let session_row = core.storage.get_session(&session_id).await.map_err(|e| {
                internal_err("get_session", e)
            })?;
            let mut messages = core
                .storage
                .messages_for_session(&session_id, None)
                .await
                .map_err(|e| {
                    internal_err("messages_for_session", e)
                })?;
            // Keep only last msg_limit entries (already in chronological order).
            if messages.len() > msg_limit {
                let drop = messages.len() - msg_limit;
                messages.drain(0..drop);
            }
            let phase = core
                .current_phase(&session_id)
                .await
                .map(|p| p.chip().to_string());
            let awaiting = {
                let sessions = core.sessions.lock().await;
                sessions
                    .get(&session_id)
                    .map(|h| h.awaiting.load(std::sync::atomic::Ordering::Acquire))
                    .unwrap_or(false)
            };
            let pending: Vec<_> = core
                .bridge
                .list_pending_choices()
                .await
                .into_iter()
                .filter(|c| c.session_id == session_id)
                .map(|c| {
                    json!({
                        "choice_id": c.choice_id,
                        "agent": c.agent,
                        "question": c.question,
                        "options": c.options,
                    })
                })
                .collect();
            let msg_arr: Vec<_> = messages
                .iter()
                .map(message_to_json)
                .collect();
            let snapshot = json!({
                "session": session_row.map(|s| json!({
                    "id": s.id,
                    "title": s.title,
                    "working_repo_path": s.working_repo_path,
                    "created_at": s.created_at,
                    "closed_at": s.closed_at,
                    "brian_model_at_spawn": s.brian_model_at_spawn,
                    "rain_model_at_spawn": s.rain_model_at_spawn,
                })),
                "phase": phase,
                "awaiting": awaiting,
                "pending_choices": pending,
                "messages": msg_arr,
            });
            Ok(result_json(&snapshot, "{}"))
        }
        unknown => Err(JsonRpcError::new(
            JsonRpcError::METHOD_NOT_FOUND,
            format!("unknown tool {unknown}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_include_all_iters() {
        let names: Vec<&str> = external_tool_descriptors()
            .iter()
            .map(|d| d.name)
            .collect();
        // iter 1
        assert!(names.contains(&"list_sessions"));
        assert!(names.contains(&"create_session"));
        assert!(names.contains(&"send_message"));
        assert!(names.contains(&"get_session_messages"));
        // iter 2
        assert!(names.contains(&"advance_phase"));
        assert!(names.contains(&"resolve_choice"));
        assert!(names.contains(&"close_session"));
        assert!(names.contains(&"restart_emma"));
        assert!(names.contains(&"get_emma_messages"));
        assert!(names.contains(&"get_pending_choices"));
        assert!(names.contains(&"get_status"));
        // iter 3
        assert!(names.contains(&"get_agent_configs"));
        assert!(names.contains(&"set_agent_config"));
        assert!(names.contains(&"get_violations"));
        // iter 4
        assert!(names.contains(&"wait_for_change"));
        assert!(names.contains(&"get_session_snapshot"));
        assert_eq!(names.len(), 16);
    }

    #[test]
    fn redact_auth_token_unset() {
        assert_eq!(redact_auth_token(&None), "<unset>");
        assert_eq!(redact_auth_token(&Some(String::new())), "<unset>");
    }

    #[test]
    fn redact_auth_token_shows_last_4() {
        assert_eq!(
            redact_auth_token(&Some("sk-ant-api03-abcdefghij1234".to_string())),
            "<set:****1234>"
        );
        // Short token (< 4 chars) reveals the whole thing — acceptable, it's
        // clearly not a real key.
        assert_eq!(redact_auth_token(&Some("ab".to_string())), "<set:****ab>");
    }
}
