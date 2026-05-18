//! MCP JSON-RPC types.
//!
//! Minimal subset of [Model Context Protocol] we serve: `initialize`,
//! `tools/list`, `tools/call`, `ping`. Notifications (`notifications/*`) are
//! accepted but discarded.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// MCP protocol version we advertise.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    /// Notifications omit `id`. Requests must have one (string or number).
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const PARSE_ERROR: i32 = -32700;

    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl JsonRpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: Value, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
        }
    }
}

// ---- MCP-specific shapes ----

#[derive(Debug, Clone, Serialize)]
pub struct ToolDescriptor {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Hand-built JSON Schemas for our tools. We don't pull in `schemars`.
pub fn tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor {
            name: "ask_user_choice",
            description: "Ask the user to pick one option from a list. Blocks the agent's turn until the user picks. Use this whenever a decision belongs to the user.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "The question to show the user." },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Concrete options the user can pick."
                    }
                },
                "required": ["question", "options"]
            }),
        },
        ToolDescriptor {
            name: "mark_awaiting_user",
            description: "Flag this session as awaiting user input (non-blocking). The session's [Need User Input] badge is set; it clears the next time the user sends a message.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "reason": { "type": "string", "description": "Short reason why we're waiting on the user." }
                },
                "required": ["reason"]
            }),
        },
        ToolDescriptor {
            name: "request_approval",
            description: "Request user approval for a policy-gated action (push_gate, force_push, tool_blocklist, per_action). Blocks until the user approves or denies in the bot-hq UI. The outcome is written to violations.jsonl. Call this BEFORE running the action (e.g., before `git push`).",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": [
                            "push_gate",
                            "commit_grep",
                            "force_push",
                            "tool_blocklist",
                            "per_action",
                            "generic_approval"
                        ],
                        "description": "Which policy class triggered this request."
                    },
                    "action": {
                        "type": "string",
                        "description": "The concrete action being requested (e.g., 'git push origin main')."
                    },
                    "question": {
                        "type": "string",
                        "description": "Prompt shown to the user. Be specific about WHAT and WHY."
                    },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 2,
                        "description": "Options. Convention: include at least one starting with 'Approve' and one 'Deny'."
                    },
                    "detail": {
                        "type": "string",
                        "description": "Optional context for the violations log (e.g., 'forbidden word: bot-hq')."
                    }
                },
                "required": ["kind", "action", "question", "options"]
            }),
        },
        ToolDescriptor {
            name: "check_commit_message",
            description: "Pre-commit grep against project policy.forbidden_in_commits. Returns 'ok' if clean, or 'forbidden_word: <word>' if the message contains a disallowed phrase. Always call this BEFORE `git commit`. If the result is anything other than 'ok', rewrite the message — do not bypass.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The proposed commit message (and optionally the diff content) to scan."
                    }
                },
                "required": ["message"]
            }),
        },
        ToolDescriptor {
            name: "close_session",
            description: "Close the session this agent is running in. Kills both agents (the duo) and marks the session row closed (or archived). Use this when the user asks you to close the session and the conversation has reached a natural stopping point. Fire-and-forget — your subprocess will be terminated shortly after this call returns.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "archive": {
                        "type": "boolean",
                        "default": false,
                        "description": "If true, mark the session archived (hidden from dashboard) rather than just closed."
                    }
                },
                "required": []
            }),
        },
        ToolDescriptor {
            name: "list_my_pending_questions",
            description: "List the questions YOU (this agent) have currently parked for the user in this session. Includes ask_user_choice prompts, mark_awaiting_user halts, and request_approval gates that haven't been resolved yet. **Call this BEFORE issuing a new `ask_user_choice` to avoid duplicate retries** — if you already have a pending one on the same topic, supersede or withdraw it first. Returns a JSON array of { choice_id, kind, prompt, options, asked_at, supersedes_id }.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDescriptor {
            name: "withdraw_question",
            description: "Abandon a question you previously parked for the user (you figured it out, the context changed, or you want to rephrase it). Removes the prompt from the user's questions tray + the dashboard counter. If you intend to ask a fresh version, just call `ask_user_choice` again after withdrawing — don't accumulate duplicates.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "choice_id": {
                        "type": "string",
                        "description": "The choice_id from `list_my_pending_questions`."
                    }
                },
                "required": ["choice_id"]
            }),
        },
    ]
}

/// What an MCP server returns from `tools/call`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallResult {
    pub content: Vec<ToolContentBlock>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContentBlock {
    Text { text: String },
}

impl ToolCallResult {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContentBlock::Text { text: s.into() }],
            is_error: false,
        }
    }
    pub fn error(s: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContentBlock::Text { text: s.into() }],
            is_error: true,
        }
    }
}
