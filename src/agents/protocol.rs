//! Wire types for `claude-code`'s `--output-format stream-json` and the
//! matching `--input-format stream-json` envelope we write to stdin.
//!
//! Schema is empirical (see `docs/stream-json-events.md`). We deliberately use
//! `serde_json::Value` for fields we don't currently consume (rate-limit info,
//! hook metadata) so a future field added by claude-code doesn't break us.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---- inbound (stdout) -----------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    System(SystemEvent),
    Assistant(AssistantEvent),
    User(UserStreamEvent),
    #[serde(rename = "rate_limit_event")]
    RateLimit(Value),
    Result(ResultEvent),
    /// Anything else (forward-compatible).
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub enum SystemEvent {
    HookStarted {
        #[serde(default)]
        hook_name: Option<String>,
    },
    HookResponse {
        #[serde(default)]
        hook_name: Option<String>,
        #[serde(default)]
        outcome: Option<String>,
        /// What the hook printed. Read for one purpose: recognising bot-hq's
        /// own post-compaction hook by its marker (`agents::handoff`).
        #[serde(default)]
        output: Option<String>,
    },
    Init {
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        mcp_servers: Option<Value>,
        /// Where the CLI took its credential from (`docs/stream-json-events.md`):
        /// `none` for a subscription login (measured 2026-10-07 on 2.1.284 —
        /// an OAuth sign-in is not an "API key" source), else the variable,
        /// helper or descriptor that supplied a key. Read by the pump to say
        /// when a subscription participant is billing an API key instead.
        #[serde(default, rename = "apiKeySource")]
        api_key_source: Option<String>,
    },
    /// claude-code's list of this process's RUNNING background tasks
    /// (background agents, background shells), sent whenever the set changes:
    /// one entry at a launch, `[]` once the last one ends — before the
    /// completion's `task_notification` and the turn it starts. Probed on CLI
    /// 2.1.281 (s-5482dfff): `{"type":"system","subtype":
    /// "background_tasks_changed","tasks":[{"task_id":…,"task_type":
    /// "local_agent","description":…}]}`. Only the count is read.
    BackgroundTasksChanged {
        #[serde(default)]
        tasks: Vec<Value>,
    },
    /// claude-code compacted this process's context: everything before is now
    /// a summary. Probed on CLI 2.1.284 (s-d43b3630) with `/compact` on a
    /// stream-json `-p` process; it follows a `status: "compacting"` line and
    /// the `SessionStart:compact` hook events:
    /// `{"type":"system","subtype":"compact_boundary","compact_metadata":
    /// {"trigger":"manual","pre_tokens":23527,"post_tokens":3056,
    /// "cumulative_dropped_tokens":20471,"duration_ms":11667},…}`. A real
    /// session's auto-compaction carries `"trigger":"auto"`. The TRANSCRIPT
    /// spells the same fields in camelCase; stdout does not.
    CompactBoundary {
        #[serde(default)]
        compact_metadata: Option<CompactMetadata>,
    },
    /// Forward-compat for new system subtypes.
    #[serde(other)]
    Other,
}

/// The `compact_metadata` of a [`SystemEvent::CompactBoundary`]. Every field
/// is optional: the boundary itself is the fact, the numbers are detail.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CompactMetadata {
    /// `"auto"` (the context filled) or `"manual"` (`/compact`).
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub pre_tokens: Option<u64>,
    #[serde(default)]
    pub post_tokens: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantEvent {
    pub message: AssistantMessage,
    /// Set when the message is a HELPER's — a subagent the participant launched
    /// with its Agent tool — to that Agent call's `tool_use_id`; `null` for the
    /// participant's own. claude-code streams a helper's messages on the
    /// parent's stdout, so without this they read as the parent speaking.
    #[serde(default)]
    pub parent_tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantMessage {
    pub id: String,
    #[serde(default)]
    pub model: Option<String>,
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    /// Per-API-call usage — the size of the prompt sent for THIS call.
    ///
    /// This is the only point-in-time reading in the stream, and therefore the
    /// only correct numerator for context occupancy. The `result` event's
    /// `usage` looks equivalent but is the **sum across every API call in the
    /// turn**, so on a turn with three tool calls it roughly triples:
    ///
    /// ```text
    /// assistant#1 usage=33,917
    /// assistant#2 usage=33,917
    /// assistant#3 usage=34,216   <- the real current context
    /// result      usage=68,133   <- 33,917 + 34,216, not a prompt size
    /// ```
    #[serde(default)]
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserStreamEvent {
    pub message: UserMessageEnvelope,
    /// See [`AssistantEvent::parent_tool_use_id`]: set on a helper's tool
    /// results.
    #[serde(default)]
    pub parent_tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserMessageEnvelope {
    #[serde(default)]
    pub role: Option<String>,
    pub content: UserContent,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    /// Plain string content (when we send prose to stdin).
    Text(String),
    /// Structured content blocks (e.g. `tool_result`).
    Blocks(Vec<UserContentBlock>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserContentBlock {
    ToolResult {
        tool_use_id: String,
        /// Can be a plain string or a structured value.
        #[serde(default)]
        content: Value,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResultEvent {
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    /// Turn-failure flag. `false` on success; `true` when the turn failed
    /// (API/permission error). See `docs/stream-json-events.md` "Errors".
    #[serde(default)]
    pub is_error: bool,
    /// HTTP status of an upstream API failure (e.g. `400` for the DeepSeek
    /// system-role rejection). `null`/absent on success. A second, explicit
    /// failure signal alongside `is_error`.
    #[serde(default)]
    pub api_error_status: Option<Value>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub usage: Option<Value>,
    /// Per-model usage map, keyed by model id — the ONLY place claude-code
    /// reports `contextWindow`, which is what makes a context *percentage*
    /// possible rather than a raw token count:
    ///
    /// ```jsonc
    /// "modelUsage": { "claude-opus-5": {
    ///   "inputTokens": 2, "cacheReadInputTokens": 11631,
    ///   "cacheCreationInputTokens": 12324, "contextWindow": 1000000,
    ///   "canonicalModel": "claude-opus-5", "provider": "firstParty" } }
    /// ```
    ///
    /// Note the on-disk `~/.claude/projects/**.jsonl` transcripts carry the
    /// token counts but NOT `contextWindow` — so the denominator is only ever
    /// available live, off this event. Capture it here or lose it.
    #[serde(default, rename = "modelUsage")]
    pub model_usage: Option<Value>,
}

// ---- outbound (stdin) -----------------------------------------------------

/// What we write to claude-code's stdin, one per line.
///
/// Empirically the same envelope claude-code uses for its own `user` events.
#[derive(Debug, Clone, Serialize)]
pub struct OutgoingUserMessage {
    #[serde(rename = "type")]
    pub typ: &'static str,
    pub message: OutgoingMessage,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingMessage {
    pub role: &'static str,
    pub content: String,
}

impl OutgoingUserMessage {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            typ: "user",
            message: OutgoingMessage {
                role: "user",
                content: content.into(),
            },
        }
    }
}

/// A stream-json `control_request` written to claude-code's stdin to ABORT the
/// in-flight turn WITHOUT killing the process (warm cache, no `--resume`). The
/// binary reads control requests on a channel separate from queued user input,
/// so this preempts even when user messages sit buffered; it ACKs with
/// `{"type":"control_response","response":{"subtype":"success","request_id":<id>}}`
/// then emits a `result` with `terminal_reason:"aborted_streaming"` (is_error:true).
/// Wire format verified live against claude-code v2.1.186. Used as the PRIMARY
/// cancel; SIGKILL process-group kill is the escalation fallback.
#[derive(Debug, Clone, Serialize)]
pub struct ControlRequest {
    #[serde(rename = "type")]
    pub typ: &'static str,
    pub request_id: String,
    pub request: ControlRequestBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlRequestBody {
    pub subtype: &'static str,
}

impl ControlRequest {
    /// An `interrupt` control request. `request_id` correlates the
    /// `control_response` ACK (any string unique enough per in-flight request).
    pub fn interrupt(request_id: impl Into<String>) -> Self {
        Self {
            typ: "control_request",
            request_id: request_id.into(),
            request: ControlRequestBody {
                subtype: "interrupt",
            },
        }
    }
}

// ---- tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_system_init() {
        let line = r#"{"type":"system","subtype":"init","cwd":"/x","model":"claude-opus-4-7"}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match ev {
            StreamEvent::System(SystemEvent::Init { cwd, model, .. }) => {
                assert_eq!(cwd.as_deref(), Some("/x"));
                assert_eq!(model.as_deref(), Some("claude-opus-4-7"));
            }
            other => panic!("expected System::Init, got {other:?}"),
        }
    }

    #[test]
    fn parses_assistant_text() {
        let line = r#"{
            "type":"assistant",
            "message":{
                "id":"msg_1",
                "model":"claude-opus-4-7",
                "content":[{"type":"text","text":"hi"}]
            }
        }"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match ev {
            StreamEvent::Assistant(a) => {
                assert_eq!(a.message.id, "msg_1");
                match &a.message.content[0] {
                    ContentBlock::Text { text } => assert_eq!(text, "hi"),
                    other => panic!("expected Text block, got {other:?}"),
                }
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parses_assistant_tool_use() {
        let line = r#"{
            "type":"assistant",
            "message":{
                "id":"msg_1",
                "content":[{"type":"tool_use","id":"tu_1","name":"ask_user_choice","input":{"question":"?","options":["a","b"]}}]
            }
        }"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        let StreamEvent::Assistant(a) = ev else { panic!() };
        match &a.message.content[0] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "ask_user_choice");
                assert_eq!(input["question"], "?");
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn parses_user_tool_result() {
        let line = r#"{
            "type":"user",
            "message":{
                "role":"user",
                "content":[{
                    "type":"tool_result",
                    "tool_use_id":"tu_1",
                    "content":"hello-from-claude",
                    "is_error":false
                }]
            }
        }"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        let StreamEvent::User(u) = ev else { panic!() };
        let UserContent::Blocks(blocks) = u.message.content else { panic!() };
        match &blocks[0] {
            UserContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "tu_1");
                assert_eq!(content.as_str(), Some("hello-from-claude"));
                assert!(!is_error);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn parses_result_event() {
        let line = r#"{
            "type":"result",
            "stop_reason":"end_turn",
            "subtype":"success",
            "cost_usd":0.01
        }"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match ev {
            StreamEvent::Result(r) => {
                assert_eq!(r.stop_reason.as_deref(), Some("end_turn"));
                assert_eq!(r.subtype.as_deref(), Some("success"));
            }
            other => panic!("expected Result, got {other:?}"),
        }
    }

    /// The stdout line captured from CLI 2.1.284 (s-d43b3630), verbatim.
    #[test]
    fn parses_the_compact_boundary_claude_code_writes_to_stdout() {
        let line = r#"{"type":"system","subtype":"compact_boundary","session_id":"06358c13-6c6d-465e-9c16-4f48d7bf541a","uuid":"181375ae-a3bb-4c41-9f1b-3bec3e875902","compact_metadata":{"trigger":"manual","pre_tokens":23527,"post_tokens":3056,"cumulative_dropped_tokens":20471,"duration_ms":11667},"logical_parent_uuid":"f97e2bd5-9d3c-429f-b5ce-97f99a53d0a9"}"#;
        match serde_json::from_str::<StreamEvent>(line).unwrap() {
            StreamEvent::System(SystemEvent::CompactBoundary { compact_metadata: Some(m) }) => {
                assert_eq!(m.trigger.as_deref(), Some("manual"));
                assert_eq!(m.pre_tokens, Some(23527));
                assert_eq!(m.post_tokens, Some(3056));
            }
            other => panic!("expected System::CompactBoundary, got {other:?}"),
        }
        // A boundary with no metadata is still a boundary.
        let bare = r#"{"type":"system","subtype":"compact_boundary"}"#;
        assert!(matches!(
            serde_json::from_str::<StreamEvent>(bare).unwrap(),
            StreamEvent::System(SystemEvent::CompactBoundary { compact_metadata: None })
        ));
        // The `status` lines around it stay unmodelled.
        let status = r#"{"type":"system","subtype":"status","status":"compacting","session_id":"s","uuid":"u"}"#;
        assert!(matches!(
            serde_json::from_str::<StreamEvent>(status).unwrap(),
            StreamEvent::System(SystemEvent::Other)
        ));
    }

    /// The shape of a `hook_response` on the same capture: the hook's output
    /// rides `output`.
    #[test]
    fn a_hook_response_carries_what_the_hook_printed() {
        let line = r#"{"type":"system","subtype":"hook_response","hook_id":"16503b8d","hook_name":"SessionStart:compact","hook_event":"SessionStart","output":"MARKER\nbody","stdout":"MARKER\nbody","stderr":"","exit_code":0,"outcome":"success","uuid":"aedda2cc","session_id":"06358c13"}"#;
        match serde_json::from_str::<StreamEvent>(line).unwrap() {
            StreamEvent::System(SystemEvent::HookResponse { hook_name, outcome, output }) => {
                assert_eq!(hook_name.as_deref(), Some("SessionStart:compact"));
                assert_eq!(outcome.as_deref(), Some("success"));
                assert_eq!(output.as_deref(), Some("MARKER\nbody"));
            }
            other => panic!("expected System::HookResponse, got {other:?}"),
        }
    }

    #[test]
    fn unknown_event_doesnt_panic() {
        let line = r#"{"type":"future_event","payload":42}"#;
        let ev: StreamEvent = serde_json::from_str(line).unwrap();
        match ev {
            StreamEvent::Unknown => {}
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn serializes_outgoing_user_message() {
        let m = OutgoingUserMessage::text("hello");
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"type\":\"user\""));
        assert!(s.contains("\"role\":\"user\""));
        assert!(s.contains("\"content\":\"hello\""));
    }

    #[test]
    fn serializes_control_request_interrupt() {
        // Must match the exact envelope verified live against claude-code v2.1.186:
        // {"type":"control_request","request_id":"r1","request":{"subtype":"interrupt"}}
        let c = ControlRequest::interrupt("r1");
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"type\":\"control_request\""));
        assert!(s.contains("\"request_id\":\"r1\""));
        assert!(s.contains("\"request\":{\"subtype\":\"interrupt\"}"));
    }
}
