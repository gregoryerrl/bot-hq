//! Row types for the storage layer.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Who authored a message. ARCHITECTURE.md "Author enum" — no `system` author.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    User,
    Brian,
    Rain,
}

impl Author {
    pub fn as_str(&self) -> &'static str {
        match self {
            Author::User => "user",
            Author::Brian => "brian",
            Author::Rain => "rain",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user" => Author::User,
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
    pub brian_claude_session_id: Option<String>,
    pub rain_claude_session_id: Option<String>,
    /// 0 = solo Brian (Rain disabled for this session); 1 = duo. Default 1.
    pub rain_enabled: i64,
    /// Saved-model ids chosen at create time (NULL = fall back to agent config).
    pub brian_model_id: Option<String>,
    pub rain_model_id: Option<String>,
    /// Per-session effort/ultracode overrides chosen at create time (NULL =
    /// inherit the Settings → Claude Config defaults). ultracode applies to
    /// Brian only (EYES gets no --settings).
    pub brian_effort: Option<String>,
    pub rain_effort: Option<String>,
    pub brian_ultracode: Option<bool>,
    pub rain_ultracode: Option<bool>,
    /// The user's main repo when this session runs in an isolated git
    /// worktree (then `working_repo_path` is the worktree). NULL = session
    /// runs directly in `working_repo_path`.
    pub base_repo_path: Option<String>,
}

/// `Session` plus a cheap latest-text-message preview, for the dashboard
/// session list (Quickview). Built only by
/// `list_active_sessions_with_preview`; the plain `Session` row type stays
/// unchanged so the other `query_as::<_, Session>` sites (`get_session`,
/// `list_closed_sessions`) keep compiling without a preview column.
#[derive(Debug, Clone, FromRow)]
pub struct SessionWithPreview {
    #[sqlx(flatten)]
    pub session: Session,
    /// First 200 chars of the most recent `kind='text'` message, or None when
    /// the session has no text messages yet.
    pub last_message: Option<String>,
    /// Author of that latest text message ('user' | 'brian' | 'rain').
    pub last_author: Option<String>,
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

/// A saved model in the user-managed registry (`models` table). Bundles the
/// provider + model id + optional gateway (`base_url`) and credential
/// (`auth_token`) an agent spawns with. Referenced by id from session-create.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub model_name: String,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub created_at: String,
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

/// A row from the `session_tray` table. Mirrors a tray item the agent has
/// surfaced to the user — a question, an approval, an action_gate gated
/// command, or a `mark_awaiting_user` halt — via the per-session tray.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SessionTrayEntry {
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
    /// The command to run on approve, for an action_gate (ToolBlocklist)
    /// approval. Persisted so the command executes at resolve time even after
    /// the in-memory oneshot is gone (client timeout / restart). NULL for any
    /// non-executing tray item.
    #[serde(default)]
    pub command_text: Option<String>,
}

impl SessionTrayEntry {
    /// Decode `options_json` into a Vec<String>. Returns None for non-choice
    /// kinds or when the column is null/empty.
    pub fn options(&self) -> Option<Vec<String>> {
        let raw = self.options_json.as_deref()?;
        serde_json::from_str(raw).ok()
    }
}

/// Severity of an EYES finding. Only `Blocking` gates a commit; `Advisory`
/// (nits) never blocks — over-gating would train HANDS to ignore the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Blocking,
    Advisory,
}

impl FindingSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            FindingSeverity::Blocking => "blocking",
            FindingSeverity::Advisory => "advisory",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "blocking" => FindingSeverity::Blocking,
            "advisory" => FindingSeverity::Advisory,
            _ => return None,
        })
    }
}

/// Lifecycle status of an EYES finding. `Open` gates (when blocking);
/// `Fixed`/`Rebutted` are HANDS dispositions (both carry a reason and surface
/// to the user); `Stale` is auto-marked when the referenced code is gone, or
/// set by an explicit user override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Open,
    Fixed,
    Rebutted,
    Stale,
}

impl FindingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FindingStatus::Open => "open",
            FindingStatus::Fixed => "fixed",
            FindingStatus::Rebutted => "rebutted",
            FindingStatus::Stale => "stale",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "open" => FindingStatus::Open,
            "fixed" => FindingStatus::Fixed,
            "rebutted" => FindingStatus::Rebutted,
            "stale" => FindingStatus::Stale,
            _ => return None,
        })
    }
}

/// A row from the `findings` table — an EYES review finding on a session.
/// `severity`/`status` are stored as text; use [`FindingSeverity`]/
/// [`FindingStatus`] `::parse` to type them. `finding_uid` is the public
/// handle agents pass to `disposition_finding`.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Finding {
    pub id: i64,
    pub session_id: String,
    pub finding_uid: String,
    pub agent: String,
    pub severity: String,
    pub summary: String,
    pub code_ref: Option<String>,
    pub status: String,
    pub disposition_reason: Option<String>,
    pub disposed_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// How many times EYES has raised this finding (re-raise dedup). `>= 2` =
    /// "escalated" — the frontend banner emphasizes it. Default 1.
    pub raise_count: i64,
    /// `1` once EYES confirmed the resolution via `approve_finding` (clears the
    /// escalation signal); `0` otherwise. Orthogonal to `status`.
    pub eyes_approved: i64,
}

/// A registered project. The special name `_globals` is the bot-hq root
/// bucket (general-rules.md, etc.) and has NULL working_repo_path.
///
/// `cl_path` overrides the default CL root convention
/// `<data_dir>/projects/<name>/`. NULL means use the convention.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub display_name: String,
    pub working_repo_path: Option<String>,
    pub description: Option<String>,
    pub created_at: String,
    pub cl_path: Option<String>,
}

impl Project {
    pub const GLOBALS: &'static str = "_globals";
}

/// A bot-hq plugin row. The plugin runtime is scaffolded in `src/plugin/`;
/// the registry persists installed plugins so they survive restart.
///
/// `manifest_json` is the raw JSON content of `bot-hq-plugin.json` from the
/// plugin's directory at install time; `dir_path` is the absolute path where
/// the plugin's files live on disk. `enabled` toggles whether the runtime
/// loads it on startup (the row is preserved either way).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub manifest_json: String,
    pub dir_path: String,
    pub installed_at: String,
}

/// Per-folder description (parallel to ClIndexEntry but for directories).
/// `folder_path` is relative to the project's CL root; `''` is the project
/// root itself.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ClFolder {
    pub id: i64,
    pub project_id: String,
    pub folder_path: String,
    pub description: String,
    pub tags: Option<String>,
    pub created_at: String,
    pub updated_at: String,
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

/// Per-session free-form document. Agents use this for plans, investigation
/// findings, and scratch notes that should NOT pollute the CL. Isolated per
/// session; archived with the session row on close. `slug` is the agent-
/// chosen identifier; UNIQUE per (session_id, slug) so writes are idempotent.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SessionDocument {
    pub id: i64,
    pub session_id: String,
    pub slug: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub phase: Option<String>,
}
