//! External-driver MCP tool dispatch.
//!
//! Auth is enforced at the HTTP layer (bearer token); by the time a request
//! reaches `dispatch_external`, the caller is authenticated. Tools talk
//! directly to `core::AppState` — no per-agent identity, no HANDS-only gating.
//!
//! See `external_server.rs` for the listener / auth half.

use crate::core::ipav::IpavPhase;
use crate::core::AppState as CoreAppState;
use crate::signaling::protocol::{
    JsonRpcError, JsonRpcRequest, JsonRpcResponse, ToolCallResult, ToolDescriptor,
};
use serde_json::{json, Value};
use std::sync::Arc;

const PROTOCOL_VERSION: &str = "2024-11-05";

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
            let tools: Vec<_> = external_tool_descriptors();
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

/// Iter 1+2 toolset: enough to drive a session end-to-end (create, send,
/// read, list) PLUS phase control, choice resolution, lifecycle (close,
/// restart-emma), Emma read, pending-choices visibility, and status. Iter 3
/// adds: get_violations, get/set_agent_configs.
pub fn external_tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
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
    ]
}

async fn call_external_tool(
    name: &str,
    args: Value,
    core: &Arc<CoreAppState>,
) -> Result<ToolCallResult, JsonRpcError> {
    match name {
        "list_sessions" => {
            let sessions = core.list_active_sessions().await.map_err(|e| {
                JsonRpcError::new(
                    JsonRpcError::INTERNAL_ERROR,
                    format!("list_active_sessions: {e}"),
                )
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
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "sessions": arr })).unwrap_or_default(),
            ))
        }
        "create_session" => {
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing title"))?
                .to_string();
            let working_repo_path = args
                .get("working_repo_path")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from);
            let session_id = core
                .open_session(title, working_repo_path)
                .await
                .map_err(|e| {
                    JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, format!("open_session: {e}"))
                })?;
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "session_id": session_id })).unwrap_or_default(),
            ))
        }
        "send_message" => {
            let session_id = args
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing session_id")
                })?
                .to_string();
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing text"))?
                .to_string();
            core.broadcast(&session_id, &text).await.map_err(|e| {
                JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, format!("broadcast: {e}"))
            })?;
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "ok": true })).unwrap_or_default(),
            ))
        }
        "get_session_messages" => {
            let session_id = args
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing session_id")
                })?
                .to_string();
            let since_id = args.get("since_id").and_then(Value::as_i64);
            let msgs = core
                .storage
                .messages_for_session(&session_id, since_id)
                .await
                .map_err(|e| {
                    JsonRpcError::new(
                        JsonRpcError::INTERNAL_ERROR,
                        format!("messages_for_session: {e}"),
                    )
                })?;
            let arr: Vec<_> = msgs
                .into_iter()
                .map(|m| {
                    json!({
                        "id": m.id,
                        "author": m.author,
                        "kind": m.kind,
                        "content": m.content,
                        "created_at": m.created_at,
                    })
                })
                .collect();
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "messages": arr })).unwrap_or_default(),
            ))
        }
        "advance_phase" => {
            let session_id = args
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing session_id")
                })?
                .to_string();
            let phase_str = args
                .get("phase")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing phase")
                })?;
            let phase = match phase_str {
                "I" => IpavPhase::Investigate,
                "P" => IpavPhase::Plan,
                "A" => IpavPhase::Apply,
                "V" => IpavPhase::Verify,
                other => {
                    return Err(JsonRpcError::new(
                        JsonRpcError::INVALID_PARAMS,
                        format!("phase must be one of I/P/A/V, got {other}"),
                    ))
                }
            };
            core.advance_phase(&session_id, phase).await.map_err(|e| {
                JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, format!("advance_phase: {e}"))
            })?;
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "ok": true })).unwrap_or_default(),
            ))
        }
        "resolve_choice" => {
            let choice_id = args
                .get("choice_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing choice_id")
                })?
                .to_string();
            let picked = args
                .get("picked")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing picked")
                })?
                .to_string();
            core.resolve_choice(&choice_id, picked).await.map_err(|e| {
                JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, format!("resolve_choice: {e}"))
            })?;
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "ok": true })).unwrap_or_default(),
            ))
        }
        "close_session" => {
            let session_id = args
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing session_id")
                })?
                .to_string();
            let archive = args.get("archive").and_then(Value::as_bool).unwrap_or(true);
            core.close_session(&session_id, archive).await.map_err(|e| {
                JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, format!("close_session: {e}"))
            })?;
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "ok": true })).unwrap_or_default(),
            ))
        }
        "restart_emma" => {
            core.restart_emma().await.map_err(|e| {
                JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, format!("restart_emma: {e}"))
            })?;
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "ok": true })).unwrap_or_default(),
            ))
        }
        "get_emma_messages" => {
            let since_id = args.get("since_id").and_then(Value::as_i64);
            let msgs = core
                .storage
                .messages_for_session("emma", since_id)
                .await
                .map_err(|e| {
                    JsonRpcError::new(
                        JsonRpcError::INTERNAL_ERROR,
                        format!("messages_for_session(emma): {e}"),
                    )
                })?;
            let arr: Vec<_> = msgs
                .into_iter()
                .map(|m| {
                    json!({
                        "id": m.id,
                        "author": m.author,
                        "kind": m.kind,
                        "content": m.content,
                        "created_at": m.created_at,
                    })
                })
                .collect();
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "messages": arr })).unwrap_or_default(),
            ))
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
            Ok(ToolCallResult::text(
                serde_json::to_string(&json!({ "pending_choices": arr })).unwrap_or_default(),
            ))
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
            Ok(ToolCallResult::text(
                serde_json::to_string(&payload).unwrap_or_default(),
            ))
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
    fn descriptors_include_iter1_and_iter2_tools() {
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
        assert_eq!(names.len(), 11);
    }
}
