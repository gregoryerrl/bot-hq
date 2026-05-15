//! Row types for the storage layer.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Who authored a message. ARCHITECTURE.md "Author enum" — no `system` author.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    User,
    Emma,
    Brian,
    Rain,
}

impl Author {
    pub fn as_str(&self) -> &'static str {
        match self {
            Author::User => "user",
            Author::Emma => "emma",
            Author::Brian => "brian",
            Author::Rain => "rain",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user" => Author::User,
            "emma" => Author::Emma,
            "brian" => Author::Brian,
            "rain" => Author::Rain,
            _ => return None,
        })
    }
}

/// What kind of payload a message holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// Plain prose. `content` is the text.
    Text,
    /// Agent invoked a tool. `content` is JSON: `{ "name": "...", "args": {...}, "tool_use_id": "..." }`.
    ToolUse,
    /// Tool returned. `content` is JSON: `{ "tool_use_id": "...", "output": "..." }`.
    ToolResult,
    /// IPAV phase advance, persisted as synthetic `author=user` message so the chat reads coherently.
    PhaseChange,
}

impl MessageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageKind::Text => "text",
            MessageKind::ToolUse => "tool_use",
            MessageKind::ToolResult => "tool_result",
            MessageKind::PhaseChange => "phase_change",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => MessageKind::Text,
            "tool_use" => MessageKind::ToolUse,
            "tool_result" => MessageKind::ToolResult,
            "phase_change" => MessageKind::PhaseChange,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub working_repo_path: Option<String>,
    pub created_at: String,
    pub closed_at: Option<String>,
    pub archived: i64,
    pub brian_model_at_spawn: Option<String>,
    pub rain_model_at_spawn: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub session_id: String,
    pub author: String,
    pub kind: String,
    pub content: String,
    pub created_at: String,
}

impl Message {
    pub fn author_typed(&self) -> Option<Author> {
        Author::parse(&self.author)
    }
    pub fn kind_typed(&self) -> Option<MessageKind> {
        MessageKind::parse(&self.kind)
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AgentConfig {
    pub agent_name: String,
    pub provider: String,
    pub model_name: String,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub updated_at: String,
}
