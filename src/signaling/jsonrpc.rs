//! Pure-function JSON-RPC dispatch for our MCP-subset endpoint.
//!
//! Separated from the HTTP layer so we can unit-test method handling without
//! standing up hyper.

use crate::policy::{ViolationKind, ViolationOutcome};
use crate::signaling::bridge::{ApprovalContext, SignalingBridge};
use crate::signaling::protocol::*;
use crate::signaling::response::{internal_err_no_prefix, ok_response, result_json};
use crate::signaling::tool_args::{arg_opt_str, arg_required_str, arg_required_str_array};
use serde_json::{json, Value};
use std::sync::Arc;

/// Identity of the (session, agent) pair making the call. Comes from the
/// URL path the agent's mcp-config points at.
#[derive(Debug, Clone)]
pub struct CallerIdentity {
    pub session_id: String,
    pub agent: String,
}

/// Dispatch one JSON-RPC request. Returns the response value (which the HTTP
/// layer wraps in `JsonRpcResponse::ok` / `err`).
///
/// Notifications (no id) return `Ok(None)` — caller writes a 202 with no body.
pub async fn dispatch(
    req: JsonRpcRequest,
    caller: &CallerIdentity,
    bridge: &Arc<SignalingBridge>,
) -> Result<Option<JsonRpcResponse>, JsonRpcError> {
    let id = match req.id.clone() {
        Some(v) => v,
        None => {
            // notification — execute (if relevant) and drop the response.
            return Ok(None);
        }
    };

    match req.method.as_str() {
        "initialize" => Ok(Some(JsonRpcResponse::ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "serverInfo": {
                    "name": "bot-hq-signaling",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "tools": { "listChanged": false }
                }
            }),
        ))),
        "ping" => Ok(Some(JsonRpcResponse::ok(id, json!({})))),
        "tools/list" => {
            let tools = tool_descriptors();
            Ok(Some(JsonRpcResponse::ok(id, json!({ "tools": tools }))))
        }
        "tools/call" => {
            let params = req
                .params
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing params"))?;
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing tool name")
                })?
                .to_string();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            let result = call_tool(&name, args, caller, bridge).await?;
            Ok(Some(JsonRpcResponse::ok(
                id,
                serde_json::to_value(result).unwrap_or(json!(null)),
            )))
        }
        _ => Err(JsonRpcError::new(
            JsonRpcError::METHOD_NOT_FOUND,
            format!("unknown method {}", req.method),
        )),
    }
}

/// Tools that affect the user (block on a choice, set "awaiting" state, or
/// request approval) must come from Brian — the HANDS role. Rain is EYES, a
/// read-only reviewer. Letting both agents race on these tools produced
/// duplicate prompts and bounced awaiting state.
const HANDS_ONLY_TOOLS: &[&str] = &[
    "ask_user_choice",
    "mark_awaiting_user",
    "request_approval",
    "action_gate",
    "supersede_question",
];

/// Tools that mutate CL annotations (folder descriptions, etc.). Brian (HANDS)
/// owns mutations; Rain (EYES) reviews via the read
/// counterparts (`cl_folder_search`, `cl_index_search`) and should not write.
const CL_MUTATE_TOOLS: &[&str] = &["cl_register_folder_description"];

/// Valid IPAV phase tags accepted by `session_doc_write` / `session_doc_search`.
/// Kept in sync with the `phase` enums in `protocol.rs` ToolDescriptors.
const VALID_PHASES: [&str; 4] = ["investigate", "plan", "apply", "verify"];

/// Parse + validate the optional `phase` arg shared by session_doc_write and
/// session_doc_search. Returns Ok(None) when absent; Err with INVALID_PARAMS
/// when present but outside the enum.
fn parse_optional_phase(args: &Value) -> Result<Option<String>, JsonRpcError> {
    let raw = args.get("phase").and_then(Value::as_str);
    if let Some(p) = raw {
        if !VALID_PHASES.contains(&p) {
            return Err(JsonRpcError::new(
                JsonRpcError::INVALID_PARAMS,
                format!("phase must be one of {:?}, got {:?}", VALID_PHASES, p),
            ));
        }
    }
    Ok(raw.map(str::to_string))
}

async fn call_tool(
    name: &str,
    args: Value,
    caller: &CallerIdentity,
    bridge: &Arc<SignalingBridge>,
) -> Result<ToolCallResult, JsonRpcError> {
    if HANDS_ONLY_TOOLS.contains(&name) && caller.agent != "brian" {
        return Ok(ToolCallResult::error(format!(
            "tool '{name}' is reserved for the HANDS agent (brian); {} is the EYES role and should not invoke user-facing tools",
            caller.agent
        )));
    }
    if CL_MUTATE_TOOLS.contains(&name) && caller.agent == "rain" {
        return Ok(ToolCallResult::error(format!(
            "tool '{name}' is reserved for HANDS (brian); rain is EYES — read folder descriptions via cl_folder_search instead",
        )));
    }
    match name {
        "ask_user_choice" => {
            let question = arg_required_str(&args, "question")?;
            let options = arg_required_str_array(&args, "options")?;
            if options.is_empty() {
                return Err(JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    "options must be a non-empty array of strings",
                ));
            }
            let picked = bridge
                .ask_user_choice(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    question,
                    options,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(picked))
        }
        "mark_awaiting_user" => {
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            bridge
                .mark_awaiting_user(caller.session_id.clone(), caller.agent.clone(), reason)
                .await;
            Ok(ToolCallResult::text("ok"))
        }
        "advance_phase" => {
            let target = arg_required_str(&args, "target")?;
            parse_phase_arg("target", &target)?;
            bridge.agent_advance_phase(caller.session_id.clone(), caller.agent.clone(), target);
            Ok(ToolCallResult::text("phase advanced"))
        }
        "web_search" => {
            let query = arg_required_str(&args, "query")?;
            let num_results = args.get("num_results").and_then(Value::as_u64).map(|n| n as usize);
            let app = bridge
                .app_handle()
                .ok_or_else(|| {
                    JsonRpcError::new(
                        JsonRpcError::INTERNAL_ERROR,
                        "Tauri AppHandle not yet initialized".to_string(),
                    )
                })?
                .clone();
            match crate::signaling::web_search::run_search(app, &query, num_results).await {
                Ok(hits) => Ok(result_json(&hits, "[]")),
                Err(e) => Ok(ToolCallResult::error(e)),
            }
        }
        "request_phase_advance" => {
            let target = arg_required_str(&args, "target")?;
            parse_phase_arg("target", &target)?;
            let reason = arg_required_str(&args, "reason")?;
            bridge
                .request_phase_advance(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    target,
                    reason,
                )
                .await;
            Ok(ToolCallResult::text(
                "request submitted — awaiting user. They will advance the phase chip or reply.",
            ))
        }
        "request_approval" => {
            let kind_str = args
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing kind"))?;
            let kind = parse_violation_kind(kind_str).ok_or_else(|| {
                JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    format!("unknown kind '{kind_str}'"),
                )
            })?;
            let action = arg_required_str(&args, "action")?;
            let question = arg_required_str(&args, "question")?;
            let options = arg_required_str_array(&args, "options")?;
            if options.len() < 2 {
                return Err(JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    "options must have at least 2 entries",
                ));
            }
            let detail = arg_opt_str(&args, "detail");
            let ctx = ApprovalContext {
                kind,
                action,
                detail,
            };
            let picked = bridge
                .request_approval(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    question,
                    options,
                    ctx,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(picked))
        }
        "action_gate" => {
            let command = arg_required_str(&args, "command")?;
            let output = bridge
                .action_gate(caller.session_id.clone(), caller.agent.clone(), command)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(output))
        }
        "close_session" => {
            let archive = args
                .get("archive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            bridge.request_session_close(caller.session_id.clone(), caller.agent.clone(), archive);
            Ok(ToolCallResult::text(
                "session close requested — your subprocess will be terminated shortly",
            ))
        }
        "check_commit_message" => {
            let message = arg_required_str(&args, "message")?;
            // Audit the policy files BEFORE resolving — if the agent has
            // quietly modified policy.yaml to remove forbidden words,
            // PolicyMutation gets logged and the user sees it post-hoc.
            // v1 is audit-only; the check below still uses the new content.
            let _ = bridge
                .audit_policy_files_for_session(&caller.session_id, &caller.agent)
                .await;
            let policy = bridge
                .resolve_policy_for(&caller.session_id)
                .await
                .map_err(internal_err_no_prefix)?;
            match policy.first_forbidden_word(&message) {
                None => Ok(ToolCallResult::text("ok")),
                Some(word) => {
                    // Best-effort log: the user didn't decide anything, but
                    // bot-hq DID block (the agent will see the error and
                    // hopefully rewrite). Record as Denied so the audit
                    // trail captures the catch.
                    if let Some(log) = bridge.violations_log() {
                        let _ = log
                            .record(
                                caller.session_id.clone(),
                                caller.agent.clone(),
                                ViolationKind::CommitGrep,
                                "git commit".to_string(),
                                ViolationOutcome::Denied,
                                Some(format!("forbidden word '{word}' in proposed message")),
                            )
                            .await;
                    }
                    Ok(ToolCallResult::text(format!("forbidden_word: {word}")))
                }
            }
        }
        "list_my_pending_questions" => {
            let rows = bridge
                .list_questions_for_session(&caller.session_id)
                .await
                .map_err(internal_err_no_prefix)?;
            // Filter to this agent's still-pending questions and shape into
            // the documented contract.
            let mine: Vec<Value> = rows
                .iter()
                .filter(|r| r.agent == caller.agent && r.status == "pending")
                .map(|r| {
                    json!({
                        "choice_id": r.choice_id,
                        "kind": r.kind,
                        "prompt": r.prompt,
                        "options": r.options(),
                        "asked_at": r.asked_at,
                        "supersedes_id": r.supersedes_id,
                    })
                })
                .collect();
            Ok(result_json(&mine, "[]"))
        }
        "withdraw_question" => {
            let choice_id = arg_required_str(&args, "choice_id")?;
            let was_pending = bridge.withdraw_question(&choice_id).await;
            Ok(ToolCallResult::text(if was_pending {
                "withdrawn"
            } else {
                "no-op: choice_id was not pending"
            }))
        }
        "supersede_question" => {
            let stale_choice_id = arg_required_str(&args, "stale_choice_id")?;
            let question = arg_required_str(&args, "question")?;
            let options = arg_required_str_array(&args, "options")?;
            if options.is_empty() {
                return Err(JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    "options must have at least 1 entry",
                ));
            }
            let picked = bridge
                .supersede_question_with_new(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    stale_choice_id,
                    question,
                    options,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(picked))
        }
        "session_doc_write" => {
            let slug = arg_required_str(&args, "slug")?;
            let body = arg_required_str(&args, "body")?;
            let phase = parse_optional_phase(&args)?;
            let id = bridge
                .session_doc_write(&caller.session_id, &slug, &body, phase.as_deref())
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(
                json!({"id": id, "slug": slug}).to_string(),
            ))
        }
        "session_doc_search" => {
            let query = args.get("query").and_then(Value::as_str);
            let phase = parse_optional_phase(&args)?;
            let rows = bridge
                .session_doc_search(&caller.session_id, query, phase.as_deref())
                .await
                .map_err(internal_err_no_prefix)?;
            let trimmed: Vec<Value> = rows
                .into_iter()
                .map(|d| {
                    json!({
                        "id": d.id,
                        "slug": d.slug,
                        "body": d.body,
                        "phase": d.phase,
                        "created_at": d.created_at,
                        "updated_at": d.updated_at,
                    })
                })
                .collect();
            Ok(result_json(&trimmed, "[]"))
        }
        "session_doc_read" => {
            let slug = arg_required_str(&args, "slug")?;
            let row = bridge
                .session_doc_read(&caller.session_id, &slug)
                .await
                .map_err(internal_err_no_prefix)?;
            match row {
                Some(d) => Ok(ToolCallResult::text(
                    json!({
                        "id": d.id,
                        "slug": d.slug,
                        "body": d.body,
                        "created_at": d.created_at,
                        "updated_at": d.updated_at,
                    })
                    .to_string(),
                )),
                None => Ok(ToolCallResult::text("null".to_string())),
            }
        }
        "cl_index_search" => {
            let project = args.get("project").and_then(Value::as_str);
            let query = args.get("query").and_then(Value::as_str);
            let rows = bridge
                .cl_index_search(project, query)
                .await
                .map_err(internal_err_no_prefix)?;
            // Strip noisy fields; agents care about file_path, description,
            // tags, updated_at. Return as a compact JSON array.
            let trimmed: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "project": r.project_id,
                        "file_path": r.file_path,
                        "description": r.description,
                        "tags": r.tags,
                        "updated_at": r.updated_at,
                    })
                })
                .collect();
            Ok(result_json(&trimmed, "[]"))
        }
        "cl_register_read" => {
            let project = arg_required_str(&args, "project")?;
            let file_path = arg_required_str(&args, "file_path")?;
            // Fire-and-forget at the tool layer too — caller doesn't gain
            // anything by waiting on an audit insert.
            bridge
                .cl_register_read(
                    &caller.agent,
                    Some(&caller.session_id),
                    &project,
                    &file_path,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text("recorded"))
        }
        "cl_folder_search" => {
            let project = args.get("project").and_then(Value::as_str);
            let query = args.get("query").and_then(Value::as_str);
            let rows = bridge
                .cl_folder_search(project, query)
                .await
                .map_err(internal_err_no_prefix)?;
            let trimmed: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "project": r.project_id,
                        "folder_path": r.folder_path,
                        "description": r.description,
                        "tags": r.tags,
                        "updated_at": r.updated_at,
                    })
                })
                .collect();
            Ok(result_json(&trimmed, "[]"))
        }
        "cl_register_folder_description" => {
            let project = arg_required_str(&args, "project")?;
            let folder_path = arg_required_str(&args, "folder_path")?;
            let description = arg_required_str(&args, "description")?;
            let tags = arg_opt_str(&args, "tags");
            bridge
                .cl_register_folder_description(
                    &project,
                    &folder_path,
                    &description,
                    tags.as_deref(),
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text("ok"))
        }
        "cl_rescan" => {
            let project = arg_required_str(&args, "project")?;
            let report = bridge
                .cl_rescan(&project)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(result_json(&report, "{}"))
        }
        "webview_screenshot" => {
            let handle = bridge.app_handle().ok_or_else(|| {
                JsonRpcError::new(
                    JsonRpcError::INTERNAL_ERROR,
                    "Tauri AppHandle not yet initialized".to_string(),
                )
            })?;
            let data_dir = bridge.data_dir().ok_or_else(|| {
                JsonRpcError::new(
                    JsonRpcError::INTERNAL_ERROR,
                    "bridge data_dir not configured (test bridge?)".to_string(),
                )
            })?;
            let path = crate::tauri_cmd::screenshot::capture_main_window(handle, data_dir)
                .map_err(internal_err_no_prefix)?;
            Ok(result_json(
                &json!({ "path": path.display().to_string() }),
                "{}",
            ))
        }
        other => match super::webview_js::webview_tool_js(other, &args)? {
            Some(js) => {
                eval_in_webview(bridge, &js)?;
                Ok(ok_response())
            }
            None => Err(JsonRpcError::new(
                JsonRpcError::METHOD_NOT_FOUND,
                format!("unknown tool {other}"),
            )),
        },
    }
}

fn eval_in_webview(bridge: &Arc<SignalingBridge>, js: &str) -> Result<(), JsonRpcError> {
    use tauri::Manager;
    let handle = bridge.app_handle().ok_or_else(|| {
        JsonRpcError::new(
            JsonRpcError::INTERNAL_ERROR,
            "Tauri AppHandle not yet initialized".to_string(),
        )
    })?;
    let window = handle.get_webview_window("main").ok_or_else(|| {
        JsonRpcError::new(
            JsonRpcError::INTERNAL_ERROR,
            "main webview not found".to_string(),
        )
    })?;
    window.eval(js).map_err(internal_err_no_prefix)?;
    Ok(())
}

fn parse_violation_kind(s: &str) -> Option<ViolationKind> {
    // Parse through serde so the wire names can't drift from `ViolationKind`'s
    // own `#[serde(rename_all = "snake_case")]` derive (a hand-written match
    // had to be kept in lockstep with the enum). Unknown string → None.
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signaling::bridge::SignalingEvent;

    fn caller() -> CallerIdentity {
        CallerIdentity {
            session_id: "s1".into(),
            agent: "brian".into(),
        }
    }

    fn rain_caller() -> CallerIdentity {
        CallerIdentity {
            session_id: "s1".into(),
            agent: "rain".into(),
        }
    }

    fn req(method: &str, params: Value, id: i64) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: method.into(),
            params: Some(params),
        }
    }

    #[tokio::test]
    async fn initialize_returns_capabilities() {
        let bridge = SignalingBridge::new();
        let res = dispatch(req("initialize", json!({}), 1), &caller(), &bridge)
            .await
            .unwrap()
            .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(v["result"]["serverInfo"]["name"], "bot-hq-signaling");
    }

    #[tokio::test]
    async fn tools_list_returns_all_tools() {
        let bridge = SignalingBridge::new();
        let res = dispatch(req("tools/list", json!({}), 1), &caller(), &bridge)
            .await
            .unwrap()
            .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let tools = v["result"]["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"ask_user_choice"));
        assert!(names.contains(&"mark_awaiting_user"));
        assert!(names.contains(&"request_approval"));
        assert!(names.contains(&"action_gate"));
        assert!(names.contains(&"check_commit_message"));
        assert!(names.contains(&"close_session"));
        assert!(names.contains(&"list_my_pending_questions"));
        assert!(names.contains(&"withdraw_question"));
        assert_eq!(
            tools.len(),
            names.iter().collect::<std::collections::HashSet<_>>().len(),
            "tool names should be unique"
        );
    }

    #[tokio::test]
    async fn close_session_emits_event() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "close_session", "arguments": {}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("close requested"));
        let ev = sub.recv().await.unwrap();
        match ev {
            SignalingEvent::SessionCloseRequest {
                session_id,
                agent,
                archive,
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(agent, "brian");
                assert!(!archive);
            }
            other => panic!("expected SessionCloseRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let bridge = SignalingBridge::new();
        let err = dispatch(req("garbage", json!({}), 1), &caller(), &bridge)
            .await
            .unwrap_err();
        assert_eq!(err.code, JsonRpcError::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn rain_rejected_from_hands_only_tools() {
        let bridge = SignalingBridge::new();
        for tool in &[
            "mark_awaiting_user",
            "ask_user_choice",
            "request_approval",
            "action_gate",
        ] {
            let res = dispatch(
                req(
                    "tools/call",
                    json!({
                        "name": tool,
                        "arguments": {
                            "reason": "x",
                            "question": "?",
                            "options": ["a", "b"],
                            "kind": "push_gate",
                            "action": "y",
                        }
                    }),
                    1,
                ),
                &rain_caller(),
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
            let v = serde_json::to_value(&res).unwrap();
            assert_eq!(
                v["result"]["isError"],
                json!(true),
                "tool {tool} should return is_error=true for rain"
            );
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            assert!(
                text.contains("reserved for the HANDS"),
                "tool {tool} should explain HANDS-only restriction, got: {text}"
            );
        }
    }

    #[tokio::test]
    async fn mark_awaiting_user_dispatch_works() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "mark_awaiting_user", "arguments": {"reason": "wait"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("ok"));
        let ev = sub.recv().await.unwrap();
        assert!(matches!(ev, SignalingEvent::AwaitingUser { reason, .. } if reason == "wait"));
    }

    #[tokio::test]
    async fn advance_phase_self_dispatch_emits_event() {
        // Self-advance path: agent moves the chip without user gate. Bridge
        // fires AgentAdvancePhase; AppState's subscriber routes to
        // core.advance_phase. We only assert the event here.
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "advance_phase",
                    "arguments": {"target": "Apply"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "phase advanced");
        let ev = sub.recv().await.unwrap();
        match ev {
            SignalingEvent::AgentAdvancePhase { target, agent, .. } => {
                assert_eq!(target, "Apply");
                assert_eq!(agent, "brian");
            }
            other => panic!("expected AgentAdvancePhase, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn advance_phase_self_rejects_bogus_target() {
        let bridge = SignalingBridge::new();
        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "advance_phase",
                    "arguments": {"target": "Wander"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(err.message.contains("unknown target"));
    }

    #[tokio::test]
    async fn rain_can_self_advance_phase() {
        // Self-advance is not HANDS-only — either agent can move the chip.
        // The user retains override via the dashboard chip click.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "advance_phase",
                    "arguments": {"target": "Plan"}
                }),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(false));
    }

    #[tokio::test]
    async fn request_phase_advance_dispatch_emits_event() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_phase_advance",
                    "arguments": {"target": "Apply", "reason": "plan done"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("awaiting user"));
        let ev = sub.recv().await.unwrap();
        match ev {
            SignalingEvent::AwaitingUser { reason, .. } => {
                assert!(
                    reason.contains("PHASE REQUEST -> Apply"),
                    "reason: {reason}"
                );
                assert!(reason.contains("plan done"), "reason: {reason}");
            }
            other => panic!("expected AwaitingUser, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn request_phase_advance_rejects_bogus_target() {
        let bridge = SignalingBridge::new();
        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_phase_advance",
                    "arguments": {"target": "Coffee", "reason": "x"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(err.message.contains("unknown target"));
    }

    #[tokio::test]
    async fn rain_can_call_request_phase_advance() {
        // Phase requests are not HANDS-only — Rain (EYES) should also be able
        // to ask the user to back off to Investigate when Brian is about to
        // mutate without a plan.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_phase_advance",
                    "arguments": {"target": "Investigate", "reason": "need to reassess"}
                }),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(
            v["result"]["isError"],
            json!(false),
            "rain should be allowed to call request_phase_advance"
        );
    }

    #[tokio::test]
    async fn request_phase_advance_accepts_chip_form() {
        // F12 regression guard: chip-form targets (I/P/A/V) must reach the
        // bridge — same leniency `advance_phase` already had. Previously
        // request_phase_advance used a hardcoded matches!() against full
        // names only and returned INVALID_PARAMS for "A".
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_phase_advance",
                    "arguments": {"target": "A", "reason": "ready to mutate"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(false));
    }

    #[tokio::test]
    async fn advance_phase_self_accepts_chip_form() {
        // Parity with request_phase_advance_accepts_chip_form — both paths
        // route through IpavPhase::parse so chip form should work here too.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "advance_phase",
                    "arguments": {"target": "A"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "phase advanced");
    }

    #[tokio::test]
    async fn ask_user_choice_dispatches_and_resolves() {
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let call = tokio::spawn(async move {
            dispatch(
                req(
                    "tools/call",
                    json!({
                        "name": "ask_user_choice",
                        "arguments": {"question": "?", "options": ["a", "b"]}
                    }),
                    1,
                ),
                &caller(),
                &bridge_clone,
            )
            .await
        });
        let ev = sub.recv().await.unwrap();
        let pending = match ev {
            SignalingEvent::PendingChoice(p) => p,
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge
            .resolve_choice(&pending.choice_id, "a".into())
            .await
            .unwrap();
        let res = call.await.unwrap().unwrap().unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "a");
    }

    #[tokio::test]
    async fn notification_returns_no_response() {
        let bridge = SignalingBridge::new();
        let mut r = req("ping", json!({}), 1);
        r.id = None;
        let out = dispatch(r, &caller(), &bridge).await.unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn session_doc_write_then_read_round_trip() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        let write_res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "plan-v1", "body": "the plan body"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&write_res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("plan-v1"), "write returned: {text}");

        let read_res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_read",
                    "arguments": {"slug": "plan-v1"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&read_res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("\"body\":\"the plan body\""),
            "read returned: {text}"
        );
    }

    #[tokio::test]
    async fn session_doc_read_unknown_slug_returns_null() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_read",
                    "arguments": {"slug": "nope"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "null");
    }

    #[tokio::test]
    async fn session_doc_write_with_phase_then_search_by_phase() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        // Two writes under phase="plan" (even with different slugs) collapse to
        // ONE rewritable doc keyed by phase — the latest body wins. A different
        // phase keeps its own doc.
        for (slug, body, phase) in [
            ("plan-v1", "first", "plan"),
            ("plan-v2", "second", "plan"),
            ("find-1", "x", "investigate"),
        ] {
            dispatch(
                req(
                    "tools/call",
                    json!({
                        "name": "session_doc_write",
                        "arguments": {"slug": slug, "body": body, "phase": phase}
                    }),
                    1,
                ),
                &caller(),
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
        }

        // Search filtered by phase="plan" returns the single consolidated doc.
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_search",
                    "arguments": {"phase": "plan"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let rows: Vec<Value> = serde_json::from_str(text).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "phase docs collapse to one per phase, got: {text}"
        );
        assert_eq!(rows[0]["phase"], "plan");
        assert_eq!(
            rows[0]["slug"], "plan",
            "phase-tagged doc is keyed by phase name"
        );
        assert_eq!(rows[0]["body"], "second", "latest write wins");
    }

    #[tokio::test]
    async fn session_doc_write_rejects_invalid_phase() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "doc", "body": "x", "phase": "garbage"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .expect_err("invalid phase enum should return Err(JsonRpcError)");
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(
            err.message.contains("phase must be one of"),
            "msg: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn session_doc_search_rejects_invalid_phase() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_search",
                    "arguments": {"phase": "garbage"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .expect_err("invalid phase enum should return Err(JsonRpcError)");
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(
            err.message.contains("phase must be one of"),
            "msg: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn check_commit_message_no_policy_returns_ok() {
        // Default bridge has no data_dir → policy resolves to default → ok.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "check_commit_message",
                    "arguments": {"message": "anything with Claude inside"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "ok");
    }

    #[tokio::test]
    async fn check_commit_message_finds_forbidden_word() {
        let tmp = tempfile::tempdir().unwrap();
        // Write a project policy and register the session.
        std::fs::create_dir_all(tmp.path().join("projects/foo")).unwrap();
        std::fs::write(
            tmp.path().join("projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - bot-hq\n  - Claude\n",
        )
        .unwrap();
        let log = crate::policy::ViolationsLog::new(tmp.path());
        let bridge = SignalingBridge::with_policy(log.clone(), tmp.path().to_path_buf());
        bridge
            .register_session("s1".into(), Some("foo".into()))
            .await;

        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "check_commit_message",
                    "arguments": {"message": "fix: pass bot-hq tests"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("forbidden_word:"), "got: {text}");
        assert!(text.contains("bot-hq"));

        // Violation logged.
        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, crate::policy::ViolationKind::CommitGrep);
        assert_eq!(recs[0].outcome, crate::policy::ViolationOutcome::Denied);
    }

    #[tokio::test]
    async fn request_approval_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let log = crate::policy::ViolationsLog::new(tmp.path());
        let bridge = SignalingBridge::with_violations_log(log.clone());
        let mut sub = bridge.subscribe();
        let bridge_clone = Arc::clone(&bridge);
        let call = tokio::spawn(async move {
            dispatch(
                req(
                    "tools/call",
                    json!({
                        "name": "request_approval",
                        "arguments": {
                            "kind": "push_gate",
                            "action": "git push origin main",
                            "question": "Approve push to main?",
                            "options": ["Approve once", "Deny"],
                            "detail": "first push to this branch"
                        }
                    }),
                    1,
                ),
                &caller(),
                &bridge_clone,
            )
            .await
        });
        let ev = sub.recv().await.unwrap();
        let pending = match ev {
            SignalingEvent::PendingChoice(p) => {
                assert!(p.approval.is_some());
                p
            }
            other => panic!("expected PendingChoice, got {other:?}"),
        };
        bridge
            .resolve_choice(&pending.choice_id, "Approve once".into())
            .await
            .unwrap();
        let res = call.await.unwrap().unwrap().unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "Approve once");
        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, crate::policy::ViolationKind::PushGate);
        assert_eq!(recs[0].outcome, crate::policy::ViolationOutcome::Approved);
    }

    #[tokio::test]
    async fn request_approval_rejects_unknown_kind() {
        let bridge = SignalingBridge::new();
        let err = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "request_approval",
                    "arguments": {
                        "kind": "bogus_kind",
                        "action": "x",
                        "question": "?",
                        "options": ["a", "b"]
                    }
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(err.message.contains("unknown kind"));
    }
}
