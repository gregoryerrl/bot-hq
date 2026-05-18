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

/// Surface type of a question parked for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    /// `ask_user_choice` — has a fixed set of options.
    Choice,
    /// Free-text open question — user types a reply via normal chat input.
    OpenAsk,
    /// `mark_awaiting_user` — informational halt; no user input needed,
    /// the next chat message implicitly resumes.
    Halt,
}

impl QuestionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            QuestionKind::Choice => "choice",
            QuestionKind::OpenAsk => "open_ask",
            QuestionKind::Halt => "halt",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "choice" => QuestionKind::Choice,
            "open_ask" => QuestionKind::OpenAsk,
            "halt" => QuestionKind::Halt,
            _ => return None,
        })
    }
}

/// Lifecycle status of a question row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionStatus {
    Pending,
    Answered,
    Withdrawn,
    Superseded,
}

impl QuestionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            QuestionStatus::Pending => "pending",
            QuestionStatus::Answered => "answered",
            QuestionStatus::Withdrawn => "withdrawn",
            QuestionStatus::Superseded => "superseded",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => QuestionStatus::Pending,
            "answered" => QuestionStatus::Answered,
            "withdrawn" => QuestionStatus::Withdrawn,
            "superseded" => QuestionStatus::Superseded,
            _ => return None,
        })
    }
}

/// A row from the `session_questions` table. Mirrors a question the agent
/// has surfaced to the user via the per-session questions tray.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SessionQuestion {
    pub id: i64,
    pub session_id: String,
    pub choice_id: String,
    pub agent: String,
    pub kind: String,
    pub prompt: String,
    /// JSON-encoded `Vec<String>` for kind=choice; NULL for open_ask / halt.
    pub options_json: Option<String>,
    pub status: String,
    pub picked_option: Option<String>,
    pub asked_at: String,
    pub answered_at: Option<String>,
    pub supersedes_id: Option<i64>,
}

impl SessionQuestion {
    pub fn kind_typed(&self) -> Option<QuestionKind> {
        QuestionKind::parse(&self.kind)
    }

    pub fn status_typed(&self) -> Option<QuestionStatus> {
        QuestionStatus::parse(&self.status)
    }

    /// Decode `options_json` into a Vec<String>. Returns None for non-choice
    /// kinds or when the column is null/empty.
    pub fn options(&self) -> Option<Vec<String>> {
        let raw = self.options_json.as_deref()?;
        serde_json::from_str(raw).ok()
    }
}

/// A registered project. The special name `_globals` is the bot-hq root
/// bucket (general-rules.md, etc.) and has NULL working_repo_path.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub display_name: String,
    pub working_repo_path: Option<String>,
    pub description: Option<String>,
    pub created_at: String,
}

impl Project {
    pub const GLOBALS: &'static str = "_globals";

    pub fn is_globals(&self) -> bool {
        self.name == Self::GLOBALS
    }
}

/// A row in the CL index. Agents query this BEFORE reading files to decide
/// what's relevant from descriptions alone.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ClIndexEntry {
    pub id: i64,
    pub project_id: String,
    pub file_path: String,
    pub description: String,
    pub tags: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// One audit row recorded each time an agent reads a CL file.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ClRead {
    pub id: i64,
    pub cl_index_id: i64,
    pub session_id: Option<String>,
    pub agent: String,
    pub read_at: String,
}
