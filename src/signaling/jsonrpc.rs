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
    // EYES files findings; HANDS resolves them — so disposition is HANDS-only.
    "disposition_finding",
    // HANDS overrides the reviewer-down commit block (fail-closed escape valve).
    "override_reviewer_block",
    // `halt` yields the session to the USER (sets awaiting) — user-facing, so it
    // follows mark_awaiting_user's HANDS-only precedent. EYES converges via
    // peer_ack (not HANDS-only) instead of yielding to the user.
    "halt",
];

/// The inverse of [`HANDS_ONLY_TOOLS`]: tools only EYES (rain) may call.
/// `eyes_flag` is EYES's state-writing tool — HANDS must not file blocking
/// findings against its own work, or the independent-review gate is meaningless.
/// `approve_finding` is EYES's sign-off that an escalated fix is real — only the
/// reviewer who raised it can clear the escalation, so HANDS can't self-approve.
const EYES_ONLY_TOOLS: &[&str] = &["eyes_flag", "approve_finding"];

/// Tools that mutate CL annotations (folder descriptions, etc.). Brian (HANDS)
/// owns mutations; Rain (EYES) reviews via the read
/// counterparts (`cl_folder_search`, `cl_index_search`) and should not write.
/// `cl_register_read` is deliberately NOT gated: it writes an AUDIT row (who
/// read what), not CL content, and Rain recording her own reads is correct —
/// the gate is about content authorship, not any write to a CL table.
const CL_MUTATE_TOOLS: &[&str] = &["cl_register_folder_description"];

/// Parse + validate the optional `phase` arg shared by session_doc_write and
/// session_doc_search. Returns Ok(None) when absent; Err with INVALID_PARAMS
/// when present but unparseable. Routed through `IpavPhase::parse` (the single
/// source of truth, shared with `advance_phase`) and normalized to the canonical
/// lowercase `tag()` — so the same phase string can't be valid for one phase
/// tool and rejected by another (the old `VALID_PHASES` drift), and any accepted
/// casing/chip stores as a consistent tag the IPAV tabs can match.
fn parse_optional_phase(args: &Value) -> Result<Option<String>, JsonRpcError> {
    let raw = args.get("phase").and_then(Value::as_str);
    match raw {
        None => Ok(None),
        Some(p) => match crate::core::ipav::IpavPhase::parse(p) {
            Some(phase) => Ok(Some(phase.tag().to_string())),
            None => Err(JsonRpcError::new(
                JsonRpcError::INVALID_PARAMS,
                format!(
                    "phase must be one of {}, got {:?}",
                    crate::core::ipav::IpavPhase::error_hint(),
                    p
                ),
            )),
        },
    }
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
    if EYES_ONLY_TOOLS.contains(&name) && caller.agent != "rain" {
        return Ok(ToolCallResult::error(format!(
            "tool '{name}' is reserved for the EYES agent (rain); {} is HANDS — EYES files review findings, HANDS resolves them via disposition_finding",
            caller.agent
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
            // ask_user_choice is non-blocking: this returns a parked ack
            // (`{"status":"parked","choice_id"}`) immediately, NOT the pick. The
            // user's choice arrives later as an out-of-band user message.
            let parked = bridge
                .ask_user_choice(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    question,
                    options,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(parked))
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
        "peer_ack" => {
            // The effect is realized in the duo pump: it observes THIS ToolUse
            // event and suppresses the turn's peer-forward (duo.rs::pump_agent).
            // Nothing to do bridge-side — the call just needs to succeed so the
            // agent's turn proceeds. Either agent may call it.
            Ok(ToolCallResult::text(
                "peer_ack noted — this turn won't be forwarded to your peer.",
            ))
        }
        "halt" => {
            // Yield to the user: reuse mark_awaiting_user's machinery (set the
            // awaiting flag + Halt tray row + AwaitingUser event). `awaiting`
            // outranks `busy` in SessionActivity::derive, so the input unlocks
            // immediately — no busy-flag poking needed. HANDS-only (gated above).
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Agent yielded — your move.")
                .to_string();
            bridge
                .mark_awaiting_user(caller.session_id.clone(), caller.agent.clone(), reason)
                .await;
            Ok(ToolCallResult::text(
                "halted — yielded to the user; input unlocked.",
            ))
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
            let engine = args.get("engine").and_then(Value::as_str).map(str::to_string);
            let app = bridge
                .app_handle()
                .ok_or_else(JsonRpcError::app_handle_missing)?
                .clone();
            match crate::signaling::web_search::run_search(app, &query, num_results, engine).await {
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
            // A3b (adherence): soft-gate the FIRST close with no CL learnings
            // delta this session — nudge to run propose-don't-mutate, then close on
            // the retry. The UI force-close path (tauri_cmd) is separate + ungated.
            if bridge.should_nudge_close(&caller.session_id).await {
                Ok(ToolCallResult::text(
                    "Before closing: PROPOSE this session's bounded learnings delta via \
                     cl_propose (read the project's notes.md, append under ## Learnings, and \
                     propose kind=correct with the full body), so the next session doesn't \
                     re-discover what this one learned. Then call close_session again. (If \
                     there's genuinely nothing to persist, just call close_session again and \
                     it will close.)",
                ))
            } else {
                bridge.request_session_close(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    archive,
                );
                Ok(ToolCallResult::text(
                    "session close requested — your subprocess will be terminated shortly",
                ))
            }
        }
        "check_commit_message" => {
            let message = arg_required_str(&args, "message")?;
            // Audit the policy files BEFORE resolving — if the agent has
            // quietly modified policy.yaml to remove forbidden words,
            // PolicyMutation gets logged and the user sees it post-hoc.
            // v1 is audit-only; the check below still uses the new content.
            if let Err(err) = bridge
                .audit_policy_files_for_session(&caller.session_id, &caller.agent)
                .await
            {
                tracing::warn!(%err, session_id = %caller.session_id, "policy-file audit failed");
            }
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
                        if let Err(err) = log
                            .record(
                                caller.session_id.clone(),
                                caller.agent.clone(),
                                ViolationKind::CommitGrep,
                                "git commit".to_string(),
                                ViolationOutcome::Denied,
                                Some(format!("forbidden word '{word}' in proposed message")),
                            )
                            .await
                        {
                            // The block still lands (the agent sees the error
                            // either way) — but a hole in the audit trail must
                            // not be invisible.
                            tracing::warn!(%err, session_id = %caller.session_id, "violation-log write failed");
                        }
                    }
                    Ok(ToolCallResult::text(format!("forbidden_word: {word}")))
                }
            }
        }
        "eyes_flag" => {
            let severity_str = arg_required_str(&args, "severity")?;
            let severity = crate::storage::FindingSeverity::parse(&severity_str).ok_or_else(|| {
                JsonRpcError::new(
                    JsonRpcError::INVALID_PARAMS,
                    format!("unknown severity '{severity_str}' (expected 'blocking' or 'advisory')"),
                )
            })?;
            let summary = arg_required_str(&args, "summary")?;
            let code_ref = arg_opt_str(&args, "code_ref");
            let uid = bridge
                .eyes_flag(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    severity,
                    summary,
                    code_ref,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(format!("finding filed: {uid}")))
        }
        "disposition_finding" => {
            let finding_id = arg_required_str(&args, "finding_id")?;
            let status_str = arg_required_str(&args, "status")?;
            // Agent dispositions are fixed | rebutted only; `open` isn't a
            // resolution (and there is no agent-driven "stale" disposition).
            let status = crate::storage::FindingStatus::parse(&status_str)
                .filter(|s| {
                    matches!(
                        s,
                        crate::storage::FindingStatus::Fixed
                            | crate::storage::FindingStatus::Rebutted
                    )
                })
                .ok_or_else(|| {
                    JsonRpcError::new(
                        JsonRpcError::INVALID_PARAMS,
                        format!("status must be 'fixed' or 'rebutted', got '{status_str}'"),
                    )
                })?;
            let reason = arg_required_str(&args, "reason")?;
            let result = bridge
                .disposition_finding(finding_id, status, reason, caller.agent.clone())
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(result))
        }
        "check_open_findings" => {
            let result = bridge
                .check_open_findings(&caller.session_id)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(result))
        }
        "override_reviewer_block" => {
            let reason = arg_required_str(&args, "reason")?;
            let result = bridge.override_reviewer_block(&caller.session_id, &reason);
            Ok(ToolCallResult::text(result))
        }
        "approve_finding" => {
            let finding_id = arg_required_str(&args, "finding_id")?;
            let result = bridge
                .approve_finding(finding_id)
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(result))
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
            // Non-blocking, like ask_user_choice: returns a parked ack, not the
            // pick — the user's choice on the new question arrives out-of-band.
            let parked = bridge
                .supersede_question_with_new(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    stale_choice_id,
                    question,
                    options,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(parked))
        }
        "session_doc_write" => {
            let slug = arg_required_str(&args, "slug")?;
            let body = arg_required_str(&args, "body")?;
            let phase = parse_optional_phase(&args)?;
            // EYES (rain) contributing to a phase doc must not overwrite Brian's
            // single per-phase doc. Route a phase-tagged rain write to a
            // co-located, attributed `<phase>-eyes` doc (same phase tag → same
            // IPAV tab). Untagged rain scratch writes fall through to the normal
            // overwrite path.
            match (caller.agent.as_str(), phase.as_deref()) {
                ("rain", Some(p)) => {
                    let (id, eyes_slug) = bridge
                        .session_doc_write_eyes(&caller.session_id, p, &body)
                        .await
                        .map_err(internal_err_no_prefix)?;
                    Ok(ToolCallResult::text(
                        json!({"id": id, "slug": eyes_slug}).to_string(),
                    ))
                }
                _ => {
                    let id = bridge
                        .session_doc_write(&caller.session_id, &slug, &body, phase.as_deref())
                        .await
                        .map_err(internal_err_no_prefix)?;
                    Ok(ToolCallResult::text(
                        json!({"id": id, "slug": slug}).to_string(),
                    ))
                }
            }
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
        "cl_retrieve" => {
            let project = arg_required_str(&args, "project")?;
            let query = arg_required_str(&args, "query")?;
            let paths: Option<Vec<String>> = args
                .get("paths")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            let budget = args
                .get("budget_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(3000);
            let atoms = bridge
                .cl_retrieve(&project, &query, paths.as_deref(), budget)
                .await
                .map_err(internal_err_no_prefix)?;
            // Stage-4b measurement: log this retrieval (best-effort; never fails
            // the call). `caller` carries the session/agent context here.
            bridge
                .log_retrieval_event(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    &project,
                    &query,
                    &atoms,
                    budget,
                )
                .await;
            // Inline the atom bodies as readable `## file > heading` blocks — the
            // whole point is to hand the agent the CONTENT, not a TOC.
            let text = if atoms.is_empty() {
                // Failure-mode #5 (CL brief): an empty retrieval must never read
                // as "no constraints exist" — the fact may simply rank below the
                // match threshold or use different words.
                format!(
                    "(no matching CL atoms for: {query} — this does NOT mean no \
                     conventions/constraints exist; rephrase the query or check \
                     cl_index_search.)"
                )
            } else {
                let mut out = String::new();
                for atom in &atoms {
                    let flag = if atom.stale {
                        "⚠ possibly stale (cited code changed since indexed) — verify against the source.\n"
                    } else {
                        ""
                    };
                    out.push_str(&format!(
                        "## {} > {}\n{}{}\n\n",
                        atom.file_path, atom.heading_path, flag, atom.body
                    ));
                }
                out.trim_end().to_string()
            };
            Ok(ToolCallResult::text(text))
        }
        "cl_propose" => {
            let project = arg_required_str(&args, "project")?;
            let file_path = arg_required_str(&args, "file_path")?;
            let kind = arg_required_str(&args, "kind")?;
            let target_excerpt = arg_opt_str(&args, "target_excerpt");
            let proposed_body = arg_opt_str(&args, "proposed_body").unwrap_or_default();
            let evidence = arg_required_str(&args, "evidence")?;
            let uid = bridge
                .cl_propose(
                    caller.session_id.clone(),
                    caller.agent.clone(),
                    project,
                    file_path,
                    kind,
                    target_excerpt,
                    proposed_body,
                    evidence,
                )
                .await
                .map_err(internal_err_no_prefix)?;
            Ok(ToolCallResult::text(format!("proposal filed: {uid}")))
        }
        "cl_list_proposals" => {
            let project = arg_required_str(&args, "project")?;
            let status = arg_opt_str(&args, "status");
            let rows = bridge
                .cl_list_proposals(project, status)
                .await
                .map_err(internal_err_no_prefix)?;
            let trimmed: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|p| {
                    serde_json::json!({
                        "proposal_uid": p.proposal_uid,
                        "project": p.project_id,
                        "file_path": p.file_path,
                        "kind": p.kind,
                        "target_excerpt": p.target_excerpt,
                        "proposed_body": p.proposed_body,
                        "evidence": p.evidence,
                        "status": p.status,
                        "proposed_by": p.proposed_by,
                        "session_id": p.session_id,
                        "created_at": p.created_at,
                        "updated_at": p.updated_at,
                    })
                })
                .collect();
            Ok(result_json(&trimmed, "[]"))
        }
        "cl_register_read" => {
            let project = arg_required_str(&args, "project")?;
            let file_path = arg_required_str(&args, "file_path")?;
            // Awaited audit insert (cheap single-row write). Unknown paths
            // no-op inside the bridge; only real DB failures surface as errors.
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
            // A3b: a cl_rescan is the proxy for "the agent touched the CL" — it
            // lifts the close-delta gate so a later close_session won't nudge.
            bridge.mark_cl_rescan(&caller.session_id).await;
            Ok(result_json(&report, "{}"))
        }
        "webview_screenshot" => {
            let handle = bridge
                .app_handle()
                .ok_or_else(JsonRpcError::app_handle_missing)?;
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
    let handle = bridge
        .app_handle()
        .ok_or_else(JsonRpcError::app_handle_missing)?;
    let window = handle
        .get_webview_window("main")
        .ok_or_else(JsonRpcError::webview_missing)?;
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
        assert!(names.contains(&"peer_ack"));
        assert!(names.contains(&"halt"));
        assert!(names.contains(&"request_approval"));
        assert!(names.contains(&"action_gate"));
        assert!(names.contains(&"check_commit_message"));
        assert!(names.contains(&"close_session"));
        assert!(names.contains(&"list_my_pending_questions"));
        assert!(names.contains(&"withdraw_question"));
        assert!(names.contains(&"cl_propose"));
        assert!(names.contains(&"cl_list_proposals"));
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
    async fn close_session_nudges_for_cl_delta_then_closes() {
        // A3b: with storage wired (adherence on by default) and no cl_rescan this
        // session, the FIRST close_session returns a write-then-prune nudge and
        // does NOT request close; the SECOND closes.
        let bridge = SignalingBridge::new();
        bridge
            .set_storage(crate::storage::Storage::memory().await.unwrap())
            .await;
        let mut sub = bridge.subscribe();

        let first = dispatch(
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
        let v = serde_json::to_value(&first).unwrap();
        assert!(
            v["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("learnings"),
            "first close must nudge for the learnings delta"
        );
        assert!(
            sub.try_recv().is_err(),
            "nudged close must NOT request session close"
        );

        let second = dispatch(
            req(
                "tools/call",
                json!({"name": "close_session", "arguments": {}}),
                2,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v2 = serde_json::to_value(&second).unwrap();
        assert!(v2["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("close requested"));
        match sub.recv().await.unwrap() {
            SignalingEvent::SessionCloseRequest { .. } => {}
            other => panic!("expected SessionCloseRequest on retry, got {other:?}"),
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
            "halt",
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
    async fn brian_rejected_from_eyes_only_eyes_flag() {
        // eyes_flag is the inverse gate: EYES-only, so HANDS (brian) is rejected.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "eyes_flag", "arguments": {"severity": "blocking", "summary": "x"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true));
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("reserved for the EYES"), "got: {text}");
    }

    #[tokio::test]
    async fn rain_rejected_from_disposition_finding() {
        // disposition_finding joins HANDS_ONLY_TOOLS — EYES (rain) is rejected.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "disposition_finding", "arguments": {"finding_id": "f1", "status": "fixed", "reason": "x"}}),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true));
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("reserved for the HANDS"), "got: {text}");
    }

    #[tokio::test]
    async fn disposition_finding_rejects_non_disposition_status() {
        // `stale`/`open` are not agent dispositions — only fixed|rebutted.
        let bridge = SignalingBridge::new();
        let err = dispatch(
            req(
                "tools/call",
                json!({"name": "disposition_finding", "arguments": {"finding_id": "f1", "status": "stale", "reason": "x"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(err.message.contains("fixed' or 'rebutted"), "msg: {}", err.message);
    }

    #[tokio::test]
    async fn brian_rejected_from_approve_finding() {
        // approve_finding is EYES-only — only the reviewer who raised a finding
        // can sign off its fix; HANDS can't self-approve.
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "approve_finding", "arguments": {"finding_id": "f1"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        assert_eq!(v["result"]["isError"], json!(true));
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("reserved for the EYES"));
    }

    #[tokio::test]
    async fn findings_gate_round_trip_via_dispatch() {
        // The full gate, end-to-end through dispatch: rain files blocking →
        // check_open_findings blocks → brian dispositions → check returns ok.
        // This is the s-3cb39c76 scenario in miniature.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();

        let filed = dispatch(
            req(
                "tools/call",
                json!({"name": "eyes_flag", "arguments": {"severity": "blocking", "summary": "NPE on null id", "code_ref": "job.rs:42"}}),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&filed).unwrap();
        assert_eq!(v["result"]["isError"], json!(false));
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let uid = text.trim_start_matches("finding filed: ").to_string();
        assert!(!uid.is_empty(), "expected a finding uid, got: {text}");

        let blocked = dispatch(
            req("tools/call", json!({"name": "check_open_findings", "arguments": {}}), 1),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&blocked).unwrap();
        assert!(
            v["result"]["content"][0]["text"].as_str().unwrap().starts_with("blocked: 1"),
            "commit-time check must block while the finding is open"
        );

        dispatch(
            req(
                "tools/call",
                json!({"name": "disposition_finding", "arguments": {"finding_id": uid, "status": "fixed", "reason": "fixed in abc123"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();

        let ok = dispatch(
            req("tools/call", json!({"name": "check_open_findings", "arguments": {}}), 1),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "ok", "gate clears after disposition");
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
    async fn halt_dispatch_sets_awaiting() {
        // halt yields to the user: it routes through mark_awaiting_user's
        // machinery, so it emits AwaitingUser carrying the defaulted reason.
        let bridge = SignalingBridge::new();
        let mut sub = bridge.subscribe();
        let res = dispatch(
            req("tools/call", json!({"name": "halt", "arguments": {}}), 1),
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
            .contains("halted"));
        let ev = sub.recv().await.unwrap();
        assert!(
            matches!(ev, SignalingEvent::AwaitingUser { reason, .. } if reason.contains("yielded")),
            "halt should emit AwaitingUser with the default 'yielded' reason"
        );
    }

    #[tokio::test]
    async fn cl_retrieve_dispatch_inlines_bodies_and_handles_no_match() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .replace_atoms_for_file(
                "p",
                "notes.md",
                &[crate::storage::Atom {
                    heading_path: "Gotchas".into(),
                    body: "the migration is immutable".into(),
                    code_hash: None,
                }],
                "t",
            )
            .await
            .unwrap();
        bridge.set_storage(storage).await;

        // A real query inlines the matching atom body under a `## file > heading`.
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_retrieve", "arguments": {"project": "p", "query": "migration"}}),
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
        assert!(text.contains("## notes.md > Gotchas"), "header present: {text}");
        assert!(text.contains("the migration is immutable"), "body inlined: {text}");

        // A term-less query returns a friendly no-match string, not an error.
        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_retrieve", "arguments": {"project": "p", "query": "***"}}),
                2,
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
            .contains("no matching CL atoms"));
    }

    #[tokio::test]
    async fn cl_retrieve_dispatch_logs_a_retrieval_event() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .replace_atoms_for_file(
                "p",
                "notes.md",
                &[crate::storage::Atom {
                    heading_path: "Gotchas".into(),
                    body: "the migration is immutable".into(),
                    code_hash: None,
                }],
                "t",
            )
            .await
            .unwrap();
        // Storage is Clone (shared pool) — keep a probe to read the log after dispatch.
        let probe = storage.clone();
        bridge.set_storage(storage).await;

        dispatch(
            req(
                "tools/call",
                json!({"name": "cl_retrieve", "arguments": {"project": "p", "query": "migration"}}),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();

        // The Stage-4b hook wrote exactly one retrieval_events row for session "s1".
        let stats = probe.retrieval_stats(Some("p"), None).await.unwrap();
        assert_eq!(stats.event_count, 1, "one retrieval logged");
        assert_eq!(stats.distinct_sessions, 1);
        assert_eq!(stats.total_atoms, 1, "the one returned atom was recorded");
        assert!(stats.total_tokens > 0, "token estimate recorded: {}", stats.total_tokens);
        assert_eq!(stats.empty_returns, 0);
    }

    #[tokio::test]
    async fn cl_proposal_dispatch_allows_rain_to_create_and_list() {
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        storage
            .upsert_project("bot-hq", "bot-hq", None, None, None)
            .await
            .unwrap();
        storage.create_session("s1", "CL proposals", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;

        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_propose", "arguments": {
                    "project": "bot-hq",
                    "file_path": "notes.md",
                    "kind": "correct",
                    "target_excerpt": "old",
                    "proposed_body": "complete corrected body",
                    "evidence": "stale wording"
                }}),
                1,
            ),
            &rain_caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("proposal filed: "), "proposal response: {text}");

        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_propose", "arguments": {
                    "project": "bot-hq",
                    "file_path": "obsolete.md",
                    "kind": "delete",
                    "evidence": "obsolete note"
                }}),
                2,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("proposal filed: "), "Brian proposal response: {text}");

        let res = dispatch(
            req(
                "tools/call",
                json!({"name": "cl_list_proposals", "arguments": {
                    "project": "bot-hq",
                    "status": "open"
                }}),
                3,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();
        let v = serde_json::to_value(&res).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["project"], "bot-hq");
        assert_eq!(rows[0]["file_path"], "notes.md");
        assert_eq!(rows[0]["kind"], "correct");
        assert_eq!(rows[0]["status"], "open");
        assert_eq!(rows[0]["proposed_by"], "rain");
        assert_eq!(rows[1]["file_path"], "obsolete.md");
        assert_eq!(rows[1]["kind"], "delete");
        assert_eq!(rows[1]["proposed_body"], "");
        assert_eq!(rows[1]["proposed_by"], "brian");
    }

    #[tokio::test]
    async fn peer_ack_allowed_for_either_agent() {
        // peer_ack is NOT role-gated — both HANDS and EYES converge via it. (The
        // real suppression happens in the duo pump; here we just assert the
        // dispatch accepts the call from either agent.)
        let bridge = SignalingBridge::new();
        for c in [caller(), rain_caller()] {
            let agent = c.agent.clone();
            let res = dispatch(
                req("tools/call", json!({"name": "peer_ack", "arguments": {}}), 1),
                &c,
                &bridge,
            )
            .await
            .unwrap()
            .unwrap();
            let v = serde_json::to_value(&res).unwrap();
            assert_eq!(
                v["result"]["isError"],
                json!(false),
                "peer_ack must be allowed for {agent}"
            );
        }
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
    async fn ask_user_choice_dispatches_parked_ack() {
        // ask_user_choice is non-blocking at the dispatch layer too: the tool
        // call returns `{status:"parked", choice_id}` immediately, NOT the pick.
        // (No spawn needed — it doesn't wait on the user.)
        let bridge = SignalingBridge::new();
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "ask_user_choice",
                    "arguments": {"question": "?", "options": ["a", "b"]}
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
        assert!(text.contains("\"status\":\"parked\""), "text: {text}");
        assert!(text.contains("choice_id"), "text: {text}");
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
    async fn rain_phase_tagged_doc_write_creates_co_located_eyes_doc() {
        // EYES contributes to a phase doc WITHOUT clobbering Brian's single
        // per-phase doc. Brian authors `plan`; Rain's phase-tagged write lands
        // in a co-located `plan-eyes` doc (same phase tag → same IPAV tab).
        // Both persist; Brian's body is untouched; Rain's is attributed.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        // Brian authors the plan doc.
        dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "plan", "body": "brian's plan", "phase": "plan"}
                }),
                1,
            ),
            &caller(),
            &bridge,
        )
        .await
        .unwrap()
        .unwrap();

        // Rain contributes — must NOT error, and must land in `plan-eyes`.
        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "plan", "body": "rain's review", "phase": "plan"}
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
        assert_ne!(
            v["result"]["isError"],
            json!(true),
            "rain's phase-tagged write must be accepted now"
        );
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("plan-eyes"),
            "rain's write should report the co-located eyes slug, got: {text}"
        );

        // Both docs render under the Plan tab; Brian's body is not clobbered.
        let docs = bridge
            .session_doc_search("s1", None, Some("plan"))
            .await
            .unwrap();
        assert_eq!(docs.len(), 2, "Brian's plan + Rain's plan-eyes both persist");
        let brian = docs
            .iter()
            .find(|d| d.slug == "plan")
            .expect("brian's plan doc");
        assert_eq!(brian.body, "brian's plan", "Brian's doc must be untouched");
        let eyes = docs
            .iter()
            .find(|d| d.slug == "plan-eyes")
            .expect("rain's eyes doc");
        assert!(eyes.body.contains("### EYES findings (Rain)"));
        assert!(eyes.body.contains("rain's review"));
    }

    #[tokio::test]
    async fn rain_untagged_doc_write_allowed() {
        // The gate is narrow: Rain may still keep her own UNTAGGED scratch doc
        // (RAIN_ROLE explicitly permits this). Only the phase-tagged form is
        // HANDS-only.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "test", None).await.unwrap();

        let res = dispatch(
            req(
                "tools/call",
                json!({
                    "name": "session_doc_write",
                    "arguments": {"slug": "rain-scratch", "body": "my notes"}
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
        assert_ne!(
            v["result"]["isError"],
            json!(true),
            "rain's untagged scratch doc must be allowed"
        );
        let read = bridge
            .session_doc_read("s1", "rain-scratch")
            .await
            .unwrap();
        assert!(read.is_some(), "untagged scratch doc should persist");
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
                    "arguments": {"message": "anything with Acme inside"}
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
        std::fs::create_dir_all(tmp.path().join("library/projects/foo")).unwrap();
        std::fs::write(
            tmp.path().join("library/projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - bot-hq\n  - Acme\n",
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
