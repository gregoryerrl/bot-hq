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
            description: "Abandon a question you previously parked for the user (you figured it out, the context changed). Removes the prompt from the user's questions tray + the dashboard counter. If you want to REPLACE the question with a rephrased version, prefer `supersede_question` over withdraw+ask — the former is one tool call AND links the old row to the new via `supersedes_id` so the history is traceable.",
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
        ToolDescriptor {
            name: "supersede_question",
            description: "Replace a stale question you parked for the user with a rephrased version. The old row gets status='superseded' (drops from the tray); the new row links to it via `supersedes_id` so the history is traceable. Same blocking semantics as `ask_user_choice` — returns the user's pick of an option from the NEW question.\n\nNote: `ask_user_choice` and `request_approval` already auto-supersede the MOST RECENT pending question from this agent in this session. Use `supersede_question` when you need to explicitly target a SPECIFIC stale row that isn't the most recent (e.g. multiple pending choices from different topics, and you want to rephrase a particular one without disturbing others).",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "stale_choice_id": {
                        "type": "string",
                        "description": "The choice_id of the question being replaced (from `list_my_pending_questions`)."
                    },
                    "question": {
                        "type": "string",
                        "description": "The rephrased question."
                    },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Concrete options the user can pick on the new question."
                    }
                },
                "required": ["stale_choice_id", "question", "options"]
            }),
        },
        ToolDescriptor {
            name: "cl_index_search",
            description: "Search the Context Library (CL) index for relevant files BEFORE reading any CL file. The index returns lightweight {file_path, description, tags, updated_at} rows — read descriptions to decide which files are worth opening. This saves context vs eagerly reading everything. The `_globals` project holds cross-project system files (general-rules.md, etc.); for project-scoped work pass your session's working project name. Optional `query` does a case-insensitive substring match across file_path/description/tags.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name. Use the session's project for project-scoped files, '_globals' for system-wide rules, or omit to search across all projects."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional case-insensitive substring filter. Matches file_path, description, and tags."
                    }
                },
                "required": []
            }),
        },
        ToolDescriptor {
            name: "cl_register_read",
            description: "Record that this agent read a CL file. Powers the audit trail (cl_reads) — answers 'what context did this agent see before making a decision?'. Optional but encouraged on important reads. Fire-and-forget; failures are silently logged.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name the file belongs to."
                    },
                    "file_path": {
                        "type": "string",
                        "description": "Path relative to the project root (from cl_index_search results)."
                    }
                },
                "required": ["project", "file_path"]
            }),
        },
        ToolDescriptor {
            name: "cl_rescan",
            description: "Diff a project's filesystem against the index. Auto-registers new .md files (description = first H1 or first 80 chars), refreshes updated_at where mtime has moved, and removes orphan index rows for files that no longer exist. Use after creating CL files via Bash so the index stays in sync. Cheap to call.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name to rescan. Required."
                    }
                },
                "required": ["project"]
            }),
        },
        ToolDescriptor {
            name: "grant_session_permission",
            description: "Record a SESSION-LEVEL grant so subsequent commits or pushes don't have to ask for approval. Call this when the user says something like 'you can commit', 'you can push', 'you can commit and push', or 'you can push on this branch'. The grant lives in the bridge's in-memory cache + a mirrored JSON file the git hooks read. It is wiped when the session closes (next session starts fresh). For permanent grants across sessions, the user has to hand-edit policy.yaml — there's no tool for that yet. `scope` controls breadth: 'all' grants every branch for the rest of the session; 'specific' grants only the listed branches.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["commit", "push"],
                        "description": "Which action the grant applies to."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["all", "specific"],
                        "description": "'all' = any branch in this session; 'specific' = only the branches in `branches`."
                    },
                    "branches": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Required when scope='specific'. List of branch names to grant. Ignored when scope='all'."
                    }
                },
                "required": ["action", "scope"]
            }),
        },
        ToolDescriptor {
            name: "revoke_session_permission",
            description: "Reset a session-level grant back to None — subsequent commits/pushes for `action` will require approval again. Call this when the user explicitly takes back permission ('stop pushing on your own', 'ask before committing').",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["commit", "push"],
                        "description": "Which action to revoke."
                    }
                },
                "required": ["action"]
            }),
        },
        ToolDescriptor {
            name: "list_session_permissions",
            description: "Return the current session permissions (commit + push grant scopes). Useful for the agent to introspect before deciding whether to call `request_approval`. Returns { commit: {kind, branches?}, push: {kind, branches?} }.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
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
