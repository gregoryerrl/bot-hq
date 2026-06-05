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

/// Parse an IPAV phase target from an MCP argument. Returns INVALID_PARAMS
/// with the canonical error_hint message on failure. `field` is the
/// argument name as the dispatcher exposes it on the wire (`"target"` for
/// internal `advance_phase`/`request_phase_advance`, `"phase"` for
/// external `advance_phase`) — preserved in the error string for caller
/// clarity. Single source of truth so the three dispatch sites can't
/// drift on what they tell agents.
pub(crate) fn parse_phase_arg(
    field: &str,
    value: &str,
) -> Result<crate::core::ipav::IpavPhase, JsonRpcError> {
    crate::core::ipav::IpavPhase::parse(value).ok_or_else(|| {
        JsonRpcError::new(
            JsonRpcError::INVALID_PARAMS,
            format!(
                "unknown {field} '{value}' (expected {})",
                crate::core::ipav::IpavPhase::error_hint()
            ),
        )
    })
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
pub fn tool_descriptors() -> &'static [ToolDescriptor] {
    use std::sync::LazyLock;
    static TOOLS: LazyLock<Vec<ToolDescriptor>> = LazyLock::new(|| {
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
            name: "advance_phase",
            description: "Move the IPAV phase chip yourself (no user gate). Use this whenever your work crosses a phase boundary during a substantive task — investigation done -> Plan, plan stated -> Apply, mutation done -> Verify. The dashboard chip updates; both agents receive a [PHASE: X] transition notice on stdin. Phase is a self-discipline signal, not a permission gate. Use exact phase names: Investigate, Plan, Apply, Verify.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "enum": ["Investigate", "Plan", "Apply", "Verify"],
                        "description": "The phase you're moving to."
                    }
                },
                "required": ["target"]
            }),
        },
        ToolDescriptor {
            name: "request_phase_advance",
            description: "OPT-IN gated phase advance — use ONLY when you want to pause for explicit user acknowledgment before crossing a boundary. Most phase transitions should use `advance_phase` (self-advance, no gate). Reserve this for irreversible / destructive Apply work (force-push, prod writes, large rewrites). Adds a chat message + halt row; the duo's peer-forward halts until the user advances the chip OR replies in chat (implicit decline).",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "enum": ["Investigate", "Plan", "Apply", "Verify"],
                        "description": "The phase you want to move to."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why this transition needs explicit user ack (e.g., 'about to force-push to main')."
                    }
                },
                "required": ["target", "reason"]
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
            name: "action_gate",
            description: "Execute a Bash command that the bot-hq Tool Gate blocked (the PreToolUse hook returned a blocking error telling you to route the command here). bot-hq classifies the command against the GLOBAL gated-keyword list: an `auto_allow`/unmatched command runs immediately, while a `gate` command surfaces an Approve/Reject prompt to the user — and ON APPROVE bot-hq EXECUTES the command in this session's working repo and returns its stdout/stderr/exit code. This is an ACTION request, not a permission request: you do NOT re-run the command yourself afterward — the output you get back IS the result. On reject, the command is not run.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The exact Bash command the gate blocked (e.g., 'gh issue comment 41 --body-file /tmp/x'). bot-hq runs it verbatim in your working repo on approval."
                    }
                },
                "required": ["command"]
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
            name: "session_doc_write",
            description: "Upsert a per-session scratch document (plan, investigation findings, notes — any free-form text). Isolated to THIS session; does NOT appear in cl_index_search and won't pollute the CL. **Tag with `phase` (one of `investigate`/`plan`/`apply`/`verify`) and the doc is keyed BY PHASE — exactly ONE rewritable doc per phase. Writing that phase again (even under a different slug) overwrites the single doc; if you found new info, rewrite the whole doc, never create a `-v2`.** Phase-tagged docs surface in the matching IPAV document tab and are retrievable via `session_doc_search(phase=...)`. Untagged docs are keyed by `slug` (many allowed) — session-scoped scratch, invisible to tabs and phase searches. Promote to CL by writing the body to a CL path with Write/Bash + cl_rescan(project) ONLY when the user asks.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "description": "Identifier for UNTAGGED scratch docs (many allowed per session). For phase-tagged writes the doc is keyed by `phase`, so slug is only a human label — use the phase name (e.g. 'plan')."
                    },
                    "body": {
                        "type": "string",
                        "description": "The document body, free-form markdown / text."
                    },
                    "phase": {
                        "type": "string",
                        "enum": ["investigate", "plan", "apply", "verify"],
                        "description": "Optional. Tags the doc AND keys it: exactly one rewritable doc per phase. Surfaces in the matching IPAV tab and is retrievable via session_doc_search(phase=...)."
                    }
                },
                "required": ["slug", "body"]
            }),
        },
        ToolDescriptor {
            name: "session_doc_search",
            description: "List this session's scratch documents. Returns lightweight rows {id, slug, body, phase, updated_at} ordered newest-first. Use BEFORE session_doc_read to find what's worth opening. **Use the `phase` filter to pull prior-phase context (e.g., Brian in Apply: `session_doc_search(phase=\"plan\")` finds the plan to implement; Rain in Verify: `session_doc_search(phase=\"apply\")` finds the apply summary).** Prefer this over scrolling chat history.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional case-insensitive substring filter over slug + body."
                    },
                    "phase": {
                        "type": "string",
                        "enum": ["investigate", "plan", "apply", "verify"],
                        "description": "Optional. Filter results to docs tagged with this IPAV phase. Use for cross-phase context retrieval."
                    }
                },
                "required": []
            }),
        },
        ToolDescriptor {
            name: "session_doc_read",
            description: "Fetch one session-scratch document by slug. Returns {id, slug, body, created_at, updated_at} or null when the slug isn't in this session.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "description": "Slug from session_doc_search."
                    }
                },
                "required": ["slug"]
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
            name: "cl_folder_search",
            description: "Parallel to cl_index_search but for FOLDERS (directories) instead of files. Returns {project, folder_path, description, tags, updated_at} rows so agents can decide whether a folder is worth exploring without reading every file inside. `folder_path = \"\"` means the project root itself (the project-level description). Filter by project and/or substring query just like cl_index_search.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name. Use the session's project for project-scoped folders, '_globals' for system-wide directories under the bot-hq root, or omit to search across all projects."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional case-insensitive substring filter over folder_path/description/tags."
                    }
                },
                "required": []
            }),
        },
        ToolDescriptor {
            name: "cl_register_folder_description",
            description: "Upsert a description for a CL folder. Mirrors cl_register_read's role for files but writes a stored description instead of an audit row. HANDS (brian) can call this; Rain (EYES) is denied — Rain reviews via cl_folder_search. `folder_path = \"\"` writes the project-root description.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project the folder belongs to (use '_globals' for the bot-hq root tree)."
                    },
                    "folder_path": {
                        "type": "string",
                        "description": "Folder path relative to the project's CL root. Empty string targets the project root itself."
                    },
                    "description": {
                        "type": "string",
                        "description": "One-sentence description. Saved as-is into cl_folders.description."
                    },
                    "tags": {
                        "type": "string",
                        "description": "Optional comma-separated search hooks (e.g. 'PR-382, schema-v3')."
                    }
                },
                "required": ["project", "folder_path", "description"]
            }),
        },
        // Webview automation — agents test the bot-hq UI on their own.
        // Mirror of the external-MCP equivalents in external_jsonrpc.rs.
        ToolDescriptor {
            name: "webview_screenshot",
            description: "Capture the bot-hq main window to a PNG under `<data_dir>/screenshots/<ts>.png`. Returns `{path}`. Open the file with your built-in Read tool (which supports PNG). Use this AS YOUR EYES on what the user sees. macOS Screen Recording permission required.",
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        },
        ToolDescriptor {
            name: "webview_click",
            description: "Synthesize a click on the first DOM element matching the CSS selector in the bot-hq webview. Fire-and-forget — verify with a follow-up `webview_screenshot`. Examples: `a[href=\"/sessions/abc\"]`, `button[title^=\"Capture\"]`, `[data-testid=foo]`.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for the target element." }
                },
                "required": ["selector"]
            }),
        },
        ToolDescriptor {
            name: "webview_type",
            description: "Set the value of an input/textarea matched by the CSS selector + dispatch input/change events the React-friendly way (via the prototype value setter so React state actually updates). Element is also focused.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for an input or textarea." },
                    "text": { "type": "string", "description": "Text to set." }
                },
                "required": ["selector", "text"]
            }),
        },
        ToolDescriptor {
            name: "webview_scroll",
            description: "Scroll an element (if selector given) or the page. `y` is the destination scrollTop / scrollY in pixels.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "Optional CSS selector. Omit for window scroll." },
                    "y": { "type": "integer", "description": "Destination in pixels." }
                },
                "required": ["y"]
            }),
        },
        ToolDescriptor {
            name: "webview_press_key",
            description: "Synthesize keydown + keypress + keyup on a target element (or document.activeElement if no selector). Key names follow DOM KeyboardEvent.key: `Enter`, `Tab`, `Escape`, `ArrowDown`, `a`, etc.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "DOM key name." },
                    "selector": { "type": "string", "description": "Optional CSS selector for the target element." }
                },
                "required": ["key"]
            }),
        },
    ]
    });
    &TOOLS
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
