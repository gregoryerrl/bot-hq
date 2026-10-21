//! app-server notifications → [`AgentEvent`]s: the Codex side of the seam
//! the pump reads, mapped onto the vocabulary claude-code's stream fills so
//! nothing past `AgentHandle` knows which connector it has.
//!
//! The names are claude's on purpose: a shell command is a `Bash` tool use
//! (the pump's atomic-command check and the Tool Gate's matcher key on it),
//! a file change is `Edit` (the edit-before-Apply nudge), an MCP tool is
//! `mcp__bot-hq-signaling__<tool>` (the pump's `__peer_ack` suffix match and
//! the frontend's running-tool label). The context reading rides each
//! `thread/tokenUsage/updated` as a mid-turn usage AND is remembered for the
//! turn's completion, where the meter reads it: `last.inputTokens` is the
//! prompt size of the LAST MODEL CALL (measured across a 4-call turn:
//! 14,603 → 14,911 while `total` summed to 59,082 — plan-eyes P4), so it is
//! the numerator and `total` never is.

use crate::agents::spawn::{AgentEvent, ContextReport, ContextVerdict};
use serde_json::Value;

use super::client::{AnsweredRequest, Incoming, Notification, ServerAnswer};

/// The MCP server name bot-hq registers with Codex (a TOML key, so no
/// hyphen) and the name claude-code's config gives the same server — the
/// tool names the pump and the UI know are built from the latter.
pub const CODEX_MCP_SERVER_NAME: &str = "bot_hq";
pub const CLAUDE_MCP_SERVER_NAME: &str = "bot-hq-signaling";

/// Per-participant translation state.
#[derive(Debug, Default)]
pub struct Translator {
    /// The latest `thread/tokenUsage/updated`: (model call prompt size,
    /// context window), for the next turn completion's context report.
    usage: Option<(u64, Option<u64>)>,
    /// The model the thread runs (for the context report's label).
    pub model: String,
    /// Where to send the user for a fresh sign-in when Codex says
    /// `unauthorized` (the row's home, for the notice).
    pub login_command: String,
}

impl Translator {
    pub fn new(model: impl Into<String>, login_command: impl Into<String>) -> Self {
        Self { usage: None, model: model.into(), login_command: login_command.into() }
    }

    /// The context report for a turn completing now.
    fn context_report(&self) -> ContextReport {
        match self.usage {
            Some((used, Some(window))) if window > 0 => {
                // The same plausibility band the claude path applies: a
                // numerator far over the denominator means the window is
                // wrong, not the agent.
                let verdict = if used as f64 > window as f64 * 1.05 {
                    ContextVerdict::ImplausibleWindow
                } else {
                    ContextVerdict::Usable
                };
                ContextReport {
                    model: Some(self.model.clone()),
                    used_tokens: Some(used),
                    reported_window: Some(window),
                    verdict,
                }
            }
            Some((used, _)) => ContextReport {
                model: Some(self.model.clone()),
                used_tokens: Some(used),
                reported_window: None,
                verdict: ContextVerdict::NoWindow,
            },
            None => ContextReport::none(ContextVerdict::NoUsage),
        }
    }

    /// Translate one incoming message into the events it means (often none).
    pub fn translate(&mut self, incoming: &Incoming) -> Vec<AgentEvent> {
        match incoming {
            Incoming::Notification(n) => self.notification(n),
            Incoming::Answered(a) => answered(a),
            Incoming::Closed => Vec::new(),
        }
    }

    fn notification(&mut self, n: &Notification) -> Vec<AgentEvent> {
        let p = &n.params;
        match n.method.as_str() {
            "item/started" => item_started(&p["item"]),
            "item/completed" => item_completed(&p["item"]),
            "thread/tokenUsage/updated" => {
                let usage = &p["tokenUsage"];
                let used = usage["last"]["inputTokens"].as_u64();
                let window = usage["modelContextWindow"].as_u64();
                match used {
                    Some(used) => {
                        self.usage = Some((used, window));
                        vec![AgentEvent::AssistantUsage { used_tokens: used }]
                    }
                    None => Vec::new(),
                }
            }
            "thread/compacted" => vec![AgentEvent::Compacted { trigger: Some("auto".into()), pre_tokens: None, post_tokens: None }],
            "turn/completed" => self.turn_completed(&p["turn"]),
            "error" => {
                // Codex also announces a failing turn's error here, before
                // `turn/completed`; a transient one it will retry itself.
                if p["willRetry"].as_bool() == Some(true) {
                    return Vec::new();
                }
                error_notice(&p["error"], &self.login_command).into_iter().map(AgentEvent::Notice).collect()
            }
            "hook/completed" => {
                let run = &p["run"];
                let is_gate = run["eventName"].as_str() == Some("preToolUse");
                if is_gate && run["status"].as_str() == Some("failed") {
                    let detail = run["entries"]
                        .as_array()
                        .map(|es| es.iter().filter_map(|e| e["text"].as_str()).collect::<Vec<_>>().join(" "))
                        .unwrap_or_default();
                    vec![AgentEvent::Notice(format!(
                        "bot-hq's gate hook FAILED to run in this participant's Codex home ({}){} — Codex lets a \
                         call through when its hook fails, so this participant is unguarded until the hook runs.",
                        run["sourcePath"].as_str().unwrap_or("?"),
                        if detail.is_empty() { String::new() } else { format!(": {detail}") }
                    ))]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn turn_completed(&mut self, turn: &Value) -> Vec<AgentEvent> {
        let status = turn["status"].as_str().unwrap_or("completed").to_string();
        let failed = status == "failed";
        let mut out = Vec::new();
        let mut api_error_status = None;
        if failed {
            let err = &turn["error"];
            api_error_status = transient_status(&err["codexErrorInfo"]);
            out.extend(error_notice(err, &self.login_command).into_iter().map(AgentEvent::Notice));
        }
        out.push(AgentEvent::TurnComplete {
            stop_reason: Some(status.clone()),
            subtype: Some(status),
            is_error: failed,
            api_error_status,
            context: self.context_report(),
        });
        out
    }
}

/// The HTTP status the supervisor's transient-retry ladder understands, for
/// the Codex errors that are transient: a rate limit (429), an overloaded
/// server (529, the Anthropic "Overloaded" code the ladder was built on), a
/// connection or stream failure (503, or the status Codex saw).
fn transient_status(info: &Value) -> Option<u16> {
    match info.as_str() {
        Some("rateLimitExceeded") => Some(429),
        Some("serverOverloaded") | Some("flexUnavailable") => Some(529),
        Some("internalServerError") => Some(500),
        Some(_) => None,
        None => {
            let obj = info.as_object()?;
            let (kind, body) = obj.iter().next()?;
            match kind.as_str() {
                "httpConnectionFailed" | "responseStreamConnectionFailed" | "responseStreamDisconnected" => {
                    Some(body["httpStatusCode"].as_u64().map(|c| c as u16).unwrap_or(503))
                }
                _ => None,
            }
        }
    }
}

/// The notice a failed turn posts: Codex's message, prefixed so the pump's
/// limit detector (which keys on claude's wording) recognises a usage limit,
/// and the sign-in command for an `unauthorized`.
fn error_notice(err: &Value, login_command: &str) -> Option<String> {
    let message = err["message"].as_str().unwrap_or("Codex reported an error").trim().to_string();
    let info = err["codexErrorInfo"].as_str().unwrap_or("");
    Some(match info {
        "usageLimitExceeded" => {
            if message.starts_with("You've hit your") {
                message
            } else {
                format!("You've hit your usage limit. {message}")
            }
        }
        "unauthorized" => format!(
            "{message} — the Codex home is no longer signed in; sign in again in your terminal: \
             {login_command}"
        ),
        "contextWindowExceeded" => format!("{message} — the context window is full; the participant needs a compaction or a fresh thread."),
        _ => message,
    })
}

/// A started item: the tool uses the transcript shows while they run.
fn item_started(item: &Value) -> Vec<AgentEvent> {
    let id = item["id"].as_str().unwrap_or("").to_string();
    match item["type"].as_str() {
        Some("commandExecution") => vec![AgentEvent::ToolUse {
            id,
            name: "Bash".into(),
            input: serde_json::json!({ "command": item["command"].as_str().unwrap_or("") }),
        }],
        Some("mcpToolCall") => vec![AgentEvent::ToolUse {
            id,
            name: mcp_tool_name(item["server"].as_str().unwrap_or(""), item["tool"].as_str().unwrap_or("")),
            input: item["arguments"].clone(),
        }],
        Some("fileChange") => vec![AgentEvent::ToolUse {
            id,
            name: "Edit".into(),
            input: serde_json::json!({ "changes": changed_paths(item) }),
        }],
        _ => Vec::new(),
    }
}

/// A completed item: the tool results, the model's own text, a notice.
fn item_completed(item: &Value) -> Vec<AgentEvent> {
    let id = item["id"].as_str().unwrap_or("").to_string();
    match item["type"].as_str() {
        Some("agentMessage") => {
            let text = item["text"].as_str().unwrap_or("").trim();
            if text.is_empty() {
                Vec::new()
            } else {
                vec![AgentEvent::Text(text.to_string())]
            }
        }
        Some("commandExecution") => {
            let status = item["status"].as_str().unwrap_or("completed");
            let exit = item["exitCode"].as_i64();
            let is_error = matches!(status, "failed" | "declined") || exit.is_some_and(|c| c != 0);
            let mut content = item["aggregatedOutput"].as_str().unwrap_or("").to_string();
            if status == "declined" {
                content = format!("(declined){}", if content.is_empty() { String::new() } else { format!("\n{content}") });
            } else if let Some(code) = exit.filter(|c| *c != 0) {
                content = format!("{content}\n(exit code {code})").trim_start().to_string();
            }
            vec![AgentEvent::ToolResult { tool_use_id: id, content, is_error }]
        }
        Some("mcpToolCall") => {
            let is_error = item["status"].as_str() == Some("failed") || !item["error"].is_null();
            let content = if let Some(msg) = item["error"]["message"].as_str() {
                msg.to_string()
            } else {
                mcp_result_text(&item["result"])
            };
            vec![AgentEvent::ToolResult { tool_use_id: id, content, is_error }]
        }
        Some("fileChange") => {
            let status = item["status"].as_str().unwrap_or("completed");
            let paths = changed_paths(item);
            let is_error = matches!(status, "failed" | "declined");
            let content = if is_error {
                format!("file change {status}: {}", paths.join(", "))
            } else {
                format!("{} file(s) changed: {}", paths.len(), paths.join(", "))
            };
            vec![AgentEvent::ToolResult { tool_use_id: id, content, is_error }]
        }
        Some("error") => item["message"]
            .as_str()
            .map(|m| vec![AgentEvent::Notice(m.to_string())])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The transcript line for a server request bot-hq answered on the
/// participant's behalf — an approval it declined, a question it redirected.
fn answered(a: &AnsweredRequest) -> Vec<AgentEvent> {
    let what = match a.method.as_str() {
        "item/commandExecution/requestApproval" => format!(
            "Codex asked to approve a command (`{}`); bot-hq declined it — approvals are never \
             asked for in this harness, the Tool Gate decides.",
            a.params["command"].as_str().unwrap_or("?")
        ),
        "item/fileChange/requestApproval" => "Codex asked to approve a file change; bot-hq declined it.".to_string(),
        "item/tool/requestUserInput" => "Codex's own user-input request was answered with a redirect to `ask_user_choice`.".to_string(),
        _ => match &a.answer {
            ServerAnswer::Error { .. } => format!("Codex sent `{}`, which bot-hq does not answer; refused.", a.method),
            ServerAnswer::Result(_) => return Vec::new(),
        },
    };
    vec![AgentEvent::Notice(what)]
}

/// `mcp__<server>__<tool>`, with bot-hq's own server under the name claude's
/// config gives it so the pump's and the UI's matching keep working.
fn mcp_tool_name(server: &str, tool: &str) -> String {
    let server = if server == CODEX_MCP_SERVER_NAME { CLAUDE_MCP_SERVER_NAME } else { server };
    format!("mcp__{server}__{tool}")
}

/// An MCP result's text: its text blocks joined, else the JSON.
fn mcp_result_text(result: &Value) -> String {
    let blocks = result["content"].as_array();
    match blocks {
        Some(blocks) => {
            let texts: Vec<&str> = blocks.iter().filter_map(|b| b["text"].as_str()).collect();
            if texts.is_empty() {
                result.to_string()
            } else {
                texts.join("\n")
            }
        }
        None if result.is_null() => String::new(),
        None => result.to_string(),
    }
}

fn changed_paths(item: &Value) -> Vec<String> {
    item["changes"]
        .as_array()
        .map(|cs| cs.iter().filter_map(|c| c["path"].as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn note(method: &str, params: Value) -> Incoming {
        Incoming::Notification(Notification { method: method.into(), params })
    }

    /// Fixtures are the shapes measured on 0.163.0 (s-de77e2f0).
    #[test]
    fn a_command_is_a_bash_tool_use_and_its_output_the_result() {
        let mut t = Translator::new("gpt-6.1-sol", "CODEX_HOME=/h codex login");
        let started = t.translate(&note("item/started", json!({"item": {"type": "commandExecution", "id": "exec-1", "command": "/bin/zsh -lc 'echo plain-probe'", "cwd": "/tmp", "status": "inProgress"}})));
        assert!(matches!(&started[..], [AgentEvent::ToolUse { id, name, input }] if id == "exec-1" && name == "Bash" && input["command"] == "/bin/zsh -lc 'echo plain-probe'"));
        let done = t.translate(&note("item/completed", json!({"item": {"type": "commandExecution", "id": "exec-1", "command": "/bin/zsh -lc 'echo plain-probe'", "aggregatedOutput": "plain-probe\n", "exitCode": 0, "status": "completed"}})));
        assert!(matches!(&done[..], [AgentEvent::ToolResult { tool_use_id, content, is_error: false }] if tool_use_id == "exec-1" && content == "plain-probe\n"));
        let failed = t.translate(&note("item/completed", json!({"item": {"type": "commandExecution", "id": "exec-2", "command": "false", "aggregatedOutput": "", "exitCode": 1, "status": "failed"}})));
        assert!(matches!(&failed[..], [AgentEvent::ToolResult { is_error: true, content, .. }] if content.contains("exit code 1")));
        let declined = t.translate(&note("item/completed", json!({"item": {"type": "commandExecution", "id": "exec-3", "command": "rm", "aggregatedOutput": null, "exitCode": null, "status": "declined"}})));
        assert!(matches!(&declined[..], [AgentEvent::ToolResult { is_error: true, content, .. }] if content.starts_with("(declined)")));
    }

    #[test]
    fn an_mcp_call_keeps_claudes_tool_name_so_the_pump_still_matches_it() {
        let mut t = Translator::default();
        let started = t.translate(&note("item/started", json!({"item": {"type": "mcpToolCall", "id": "call_1", "server": "bot_hq", "tool": "peer_ack", "arguments": {"final": true}, "status": "inProgress"}})));
        assert!(matches!(&started[..], [AgentEvent::ToolUse { name, input, .. }] if name == "mcp__bot-hq-signaling__peer_ack" && input["final"] == true));
        let other = t.translate(&note("item/started", json!({"item": {"type": "mcpToolCall", "id": "call_2", "server": "discord", "tool": "whoami", "arguments": {}}})));
        assert!(matches!(&other[..], [AgentEvent::ToolUse { name, .. }] if name == "mcp__discord__whoami"));
        let done = t.translate(&note("item/completed", json!({"item": {"type": "mcpToolCall", "id": "call_1", "server": "bot_hq", "tool": "peer_ack", "result": {"content": [{"type": "text", "text": "acked"}, {"type": "text", "text": "done"}]}, "error": null, "status": "completed"}})));
        assert!(matches!(&done[..], [AgentEvent::ToolResult { tool_use_id, content, is_error: false }] if tool_use_id == "call_1" && content == "acked\ndone"));
        let err = t.translate(&note("item/completed", json!({"item": {"type": "mcpToolCall", "id": "call_3", "server": "bot_hq", "tool": "x", "result": null, "error": {"message": "user cancelled MCP tool call"}, "status": "failed"}})));
        assert!(matches!(&err[..], [AgentEvent::ToolResult { is_error: true, content, .. }] if content == "user cancelled MCP tool call"));
    }

    #[test]
    fn a_file_change_is_an_edit_and_the_models_text_is_speech() {
        let mut t = Translator::default();
        let started = t.translate(&note("item/started", json!({"item": {"type": "fileChange", "id": "fc-1", "changes": [{"path": "src/a.rs", "kind": {"type": "update"}, "diff": "..."}], "status": "inProgress"}})));
        assert!(matches!(&started[..], [AgentEvent::ToolUse { name, input, .. }] if name == "Edit" && input["changes"][0] == "src/a.rs"));
        let done = t.translate(&note("item/completed", json!({"item": {"type": "fileChange", "id": "fc-1", "changes": [{"path": "src/a.rs", "kind": {"type": "update"}, "diff": "..."}], "status": "completed"}})));
        assert!(matches!(&done[..], [AgentEvent::ToolResult { is_error: false, content, .. }] if content == "1 file(s) changed: src/a.rs"));
        let text = t.translate(&note("item/completed", json!({"item": {"type": "agentMessage", "id": "m", "text": "Step 1: done.\n", "phase": "final_answer"}})));
        assert!(matches!(&text[..], [AgentEvent::Text(s)] if s == "Step 1: done."));
        assert!(t.translate(&note("item/completed", json!({"item": {"type": "agentMessage", "id": "m", "text": "  ", "phase": "commentary"}}))).is_empty());
        assert!(t.translate(&note("item/completed", json!({"item": {"type": "reasoning", "id": "r", "summary": ["thinking"]}}))).is_empty());
        assert!(t.translate(&note("item/started", json!({"item": {"type": "userMessage", "id": "u", "content": []}}))).is_empty());
        assert!(t.translate(&note("item/agentMessage/delta", json!({"delta": "x"}))).is_empty());
    }

    /// P4: the meter's numerator is the LAST model call's prompt size, never
    /// the turn's running total, and the window is the reported one. Three
    /// updates in one turn (the 4-call fixture's shape) must leave the last.
    #[test]
    fn token_usage_feeds_the_meter_per_model_call_and_the_turn_report() {
        let mut t = Translator::new("gpt-6.1-sol", "");
        let mut update = |last_input: u64, total_input: u64| {
            t.translate(&note("thread/tokenUsage/updated", json!({"threadId": "x", "turnId": "y", "tokenUsage": {"total": {"totalTokens": total_input + 100, "inputTokens": total_input, "cachedInputTokens": 0, "outputTokens": 100, "reasoningOutputTokens": 0}, "last": {"totalTokens": last_input + 20, "inputTokens": last_input, "cachedInputTokens": 14_000, "outputTokens": 20, "reasoningOutputTokens": 0}, "modelContextWindow": 475_000}})))
        };
        assert!(matches!(&update(14_603, 14_603)[..], [AgentEvent::AssistantUsage { used_tokens: 14_603 }]));
        assert!(matches!(&update(14_742, 29_345)[..], [AgentEvent::AssistantUsage { used_tokens: 14_742 }]));
        assert!(matches!(&update(14_911, 59_082)[..], [AgentEvent::AssistantUsage { used_tokens: 14_911 }]));
        let done = t.translate(&note("turn/completed", json!({"threadId": "x", "turn": {"id": "y", "status": "completed", "items": [], "error": null}})));
        let [AgentEvent::TurnComplete { is_error: false, api_error_status: None, context, stop_reason, .. }] = &done[..] else {
            panic!("{done:?}");
        };
        assert_eq!(stop_reason.as_deref(), Some("completed"));
        let usage = context.usable().expect("a usable reading");
        assert_eq!((usage.used_tokens, usage.context_window, usage.model.as_str()), (14_911, 475_000, "gpt-6.1-sol"));
        assert_ne!(usage.used_tokens, 59_082, "the running total is not the numerator");
        // No usage yet → no reading, and the pump records why.
        let mut fresh = Translator::new("gpt-6-astra", "");
        let done = fresh.translate(&note("turn/completed", json!({"turn": {"id": "y", "status": "completed"}})));
        assert!(matches!(&done[..], [AgentEvent::TurnComplete { context, .. }] if context.verdict == ContextVerdict::NoUsage));
    }

    #[test]
    fn a_failed_turn_is_an_error_with_a_notice_and_the_limit_wording_the_pump_knows() {
        let mut t = Translator::new("gpt-6-astra", "CODEX_HOME='/Users/me/.codex-bot-hq' codex login");
        let limit = t.translate(&note("turn/completed", json!({"turn": {"id": "y", "status": "failed", "error": {"message": "You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at Oct 9th, 2026 11:19 PM.", "codexErrorInfo": "usageLimitExceeded"}}})));
        assert!(matches!(&limit[..], [AgentEvent::Notice(n), AgentEvent::TurnComplete { is_error: true, api_error_status: None, .. }] if n.starts_with("You've hit your usage limit")));
        let limit2 = t.translate(&note("turn/completed", json!({"turn": {"status": "failed", "error": {"message": "Usage exhausted", "codexErrorInfo": "usageLimitExceeded"}}})));
        assert!(matches!(&limit2[..], [AgentEvent::Notice(n), ..] if n.starts_with("You've hit your usage limit. Usage exhausted")));
        let rate = t.translate(&note("turn/completed", json!({"turn": {"status": "failed", "error": {"message": "slow down", "codexErrorInfo": "rateLimitExceeded"}}})));
        assert!(matches!(&rate[..], [AgentEvent::Notice(_), AgentEvent::TurnComplete { is_error: true, api_error_status: Some(429), .. }]));
        let over = t.translate(&note("turn/completed", json!({"turn": {"status": "failed", "error": {"message": "busy", "codexErrorInfo": "serverOverloaded"}}})));
        assert!(matches!(&over[..], [_, AgentEvent::TurnComplete { api_error_status: Some(529), .. }]));
        let conn = t.translate(&note("turn/completed", json!({"turn": {"status": "failed", "error": {"message": "no route", "codexErrorInfo": {"httpConnectionFailed": {"httpStatusCode": 502}}}}})));
        assert!(matches!(&conn[..], [_, AgentEvent::TurnComplete { api_error_status: Some(502), .. }]));
        let auth = t.translate(&note("turn/completed", json!({"turn": {"status": "failed", "error": {"message": "401 Unauthorized", "codexErrorInfo": "unauthorized"}}})));
        assert!(matches!(&auth[..], [AgentEvent::Notice(n), AgentEvent::TurnComplete { is_error: true, api_error_status: None, .. }] if n.contains("codex login")));
        let interrupted = t.translate(&note("turn/completed", json!({"turn": {"status": "interrupted", "error": null}})));
        assert!(matches!(&interrupted[..], [AgentEvent::TurnComplete { is_error: false, stop_reason: Some(s), .. }] if s == "interrupted"));
        // Codex's own `error` notification: a retry it handles is silent; a
        // final one is the notice (the turn/completed after it carries the
        // same message again — the pump dedups nothing, so only the final).
        assert!(t.translate(&note("error", json!({"error": {"message": "stream reset"}, "willRetry": true}))).is_empty());
        assert!(matches!(&t.translate(&note("error", json!({"error": {"message": "stream reset"}, "willRetry": false})))[..], [AgentEvent::Notice(n)] if n == "stream reset"));
    }

    #[test]
    fn compaction_hook_failure_and_answered_requests_become_notices() {
        let mut t = Translator::default();
        assert!(matches!(&t.translate(&note("thread/compacted", json!({"threadId": "x", "turnId": "y"})))[..], [AgentEvent::Compacted { .. }]));
        let failed_hook = t.translate(&note("hook/completed", json!({"run": {"eventName": "preToolUse", "status": "failed", "sourcePath": "/h/hooks.json", "entries": [{"kind": "stderr", "text": "No such file"}]}})));
        assert!(matches!(&failed_hook[..], [AgentEvent::Notice(n)] if n.contains("FAILED") && n.contains("No such file")));
        assert!(t.translate(&note("hook/completed", json!({"run": {"eventName": "preToolUse", "status": "blocked", "sourcePath": "/h/hooks.json"}}))).is_empty(), "a block is the gate working");
        assert!(t.translate(&note("hook/completed", json!({"run": {"eventName": "preToolUse", "status": "completed"}}))).is_empty());
        let answered = t.translate(&Incoming::Answered(AnsweredRequest { method: "item/commandExecution/requestApproval".into(), params: json!({"command": "rm -rf x"}), answer: ServerAnswer::Result(json!({"decision": "decline"})) }));
        assert!(matches!(&answered[..], [AgentEvent::Notice(n)] if n.contains("rm -rf x") && n.contains("declined")));
        let refused = t.translate(&Incoming::Answered(AnsweredRequest { method: "attestation/generate".into(), params: json!({}), answer: ServerAnswer::Error { code: -32601, message: "no".into() } }));
        assert!(matches!(&refused[..], [AgentEvent::Notice(n)] if n.contains("attestation/generate")));
        assert!(t.translate(&Incoming::Closed).is_empty());
        for quiet in ["turn/started", "thread/started", "thread/status/changed", "account/rateLimits/updated", "mcpServer/startupStatus/updated", "item/reasoning/textDelta", "hook/started", "remoteControl/status/changed"] {
            assert!(t.translate(&note(quiet, json!({}))).is_empty(), "{quiet}");
        }
    }
}
