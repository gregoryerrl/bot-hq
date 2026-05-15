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
    },
    /// Forward-compat for new system subtypes.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantEvent {
    pub message: AssistantMessage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantMessage {
    pub id: String,
    #[serde(default)]
    pub model: Option<String>,
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub stop_reason: Option<String>,
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
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub usage: Option<Value>,
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
}
