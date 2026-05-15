//! Pure-function JSON-RPC dispatch for our MCP-subset endpoint.
//!
//! Separated from the HTTP layer so we can unit-test method handling without
//! standing up hyper.

use crate::policy::{ViolationKind, ViolationOutcome};
use crate::signaling::bridge::{ApprovalContext, SignalingBridge};
use crate::signaling::protocol::*;
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
            let tools: Vec<_> = tool_descriptors();
            Ok(Some(JsonRpcResponse::ok(
                id,
                json!({ "tools": tools }),
            )))
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
const HANDS_ONLY_TOOLS: &[&str] = &["ask_user_choice", "mark_awaiting_user", "request_approval"];

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
    match name {
        "ask_user_choice" => {
            let question = args
                .get("question")
                .and_then(Value::as_str)
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing question"))?
                .to_string();
            let options: Vec<String> = args
                .get("options")
                .and_then(Value::as_array)
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing options"))?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
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
                .map_err(|e| JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, e.to_string()))?;
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
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing action"))?
                .to_string();
            let question = args
                .get("question")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing question")
                })?
                .to_string();
            let options: Vec<String> = args
                .get("options")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing options")
                })?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if options.len() < 2 {
                return Err(JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    "options must have at least 2 entries",
                ));
            }
            let detail = args
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_string);
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
                .map_err(|e| JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, e.to_string()))?;
            Ok(ToolCallResult::text(picked))
        }
        "close_session" => {
            let archive = args
                .get("archive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            bridge.request_session_close(
                caller.session_id.clone(),
                caller.agent.clone(),
                archive,
            );
            Ok(ToolCallResult::text(
                "session close requested — your subprocess will be terminated shortly",
            ))
        }
        "check_commit_message" => {
            let message = args
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    JsonRpcError::new(JsonRpcError::INVALID_PARAMS, "missing message")
                })?
                .to_string();
            // Audit the policy files BEFORE resolving — if the agent has
            // quietly modified policy.yaml to remove forbidden words,
            // PolicyMutation gets logged and the user sees it post-hoc.
            // v1 is audit-only; the check below still uses the new content.
            if let Some(data_dir) = bridge.data_dir() {
                let project = bridge.project_for_session(&caller.session_id).await;
                let _ = crate::policy::audit_policy_files(
                    data_dir,
                    project.as_deref(),
                    bridge.violations_log(),
                    &caller.session_id,
                    &caller.agent,
                );
            }
            let policy = bridge
                .resolve_policy_for(&caller.session_id)
                .await
                .map_err(|e| JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, e.to_string()))?;
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
        other => Err(JsonRpcError::new(
            JsonRpcError::METHOD_NOT_FOUND,
            format!("unknown tool {other}"),
        )),
    }
}

fn parse_violation_kind(s: &str) -> Option<ViolationKind> {
    Some(match s {
        "push_gate" => ViolationKind::PushGate,
        "commit_grep" => ViolationKind::CommitGrep,
        "force_push" => ViolationKind::ForcePush,
        "tool_blocklist" => ViolationKind::ToolBlocklist,
        "per_action" => ViolationKind::PerAction,
        "generic_approval" => ViolationKind::GenericApproval,
        _ => return None,
    })
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
        assert_eq!(tools.len(), 5);
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"ask_user_choice"));
        assert!(names.contains(&"mark_awaiting_user"));
        assert!(names.contains(&"request_approval"));
        assert!(names.contains(&"check_commit_message"));
        assert!(names.contains(&"close_session"));
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
        assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("close requested"));
        let ev = sub.recv().await.unwrap();
        match ev {
            SignalingEvent::SessionCloseRequest { session_id, agent, archive } => {
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
        for tool in &["mark_awaiting_user", "ask_user_choice", "request_approval"] {
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
                v["result"]["isError"], json!(true),
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
        let bridge =
            SignalingBridge::with_policy(log.clone(), tmp.path().to_path_buf());
        bridge.register_session("s1".into(), Some("foo".into())).await;

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
