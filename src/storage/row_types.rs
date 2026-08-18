//! Row types for the storage layer.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

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
    /// Host-emitted notice (e.g. the idle-unflagged watchdog nudging HANDS).
    /// Persisted as synthetic `author=user` like `PhaseChange` — the CHECK
    /// constraint on `messages.author` has no host author, and the chat should
    /// attribute host actions to neither agent.
    SystemNotice,
    /// Orientation, from the BOOT phase before the ring starts (rc3 **D21**):
    /// the primer the host hands every participant, and whatever each says while
    /// reading it.
    ///
    /// **Persisted and shown to the USER, but never delivered to a peer.** D21:
    /// *"Three near-identical 'CL loaded for cognotify' rows are exactly the
    /// noise the channel does not need, and a peer reading them learns
    /// nothing."* The exclusion is one entry in `channel_page`'s existing
    /// `kind NOT IN (…)` filter (rc3 D19a), which applies only to a peer's
    /// backlog read — the UI reads the whole channel and sees these.
    ///
    /// Needs no migration: `messages.kind` carries no CHECK constraint, unlike
    /// `messages.author`.
    Boot,
}

impl MessageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageKind::Text => "text",
            MessageKind::ToolUse => "tool_use",
            MessageKind::ToolResult => "tool_result",
            MessageKind::PhaseChange => "phase_change",
            MessageKind::SystemNotice => "system_notice",
            MessageKind::Boot => "boot",
        }
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
    /// The model each SPAWN SLOT ran with, frozen at spawn so the chat header
    /// shows what the session actually started on. Slot-indexed, and only two
    /// of them: a third participant's spawn model is recorded nowhere, which is
    /// a real limit of this shape rather than an oversight — see
    /// `spawn_model_slots_round_trip_and_slot_two_is_a_silent_no_op`.
    pub slot0_model_at_spawn: Option<String>,
    pub slot1_model_at_spawn: Option<String>,
    /// Does this session run more than one participant?
    ///
    /// **Computed in SQL from `session_participants`, not stored.** It used to
    /// be a `rain_enabled` column written from `seeded > 1` at one site — a
    /// cached count of the roster, and therefore a second source of truth free
    /// to disagree with the rows it summarised. That is the shape behind round
    /// 2's B3, where a boolean that could not carry a count made rosters of 2,
    /// 3 and 8 produce identical sessions. Derived, the roster is the only
    /// place the answer lives.
    pub multi_participant: bool,
    /// The user's main repo when this session runs in an isolated git
    /// worktree (then `working_repo_path` is the worktree). NULL = session
    /// runs directly in `working_repo_path`.
    pub base_repo_path: Option<String>,
    /// The plugin that created this session via the `plugin_sessions`
    /// capability, or NULL for user/agent/driver-created sessions. The
    /// ownership anchor: a plugin may drive ONLY sessions whose
    /// `created_by_plugin` equals its own id (see `require_owned_session`).
    pub created_by_plugin: Option<String>,
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
    /// Author of that latest text message: `'user'` or a participant slug
    /// (`'hands'`, `'eyes'`, `'eyes-2'`, …). Was `'user' | 'brian' | 'rain'`
    /// when a two-party `Author` enum bounded it.
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

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AgentConfig {
    pub agent_name: String,
    pub provider: String,
    pub model_name: String,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub updated_at: String,
    /// Context window in tokens, when the user has told us. `None` = unknown.
    /// Set from the chosen [`Model`]; persisted on `agent_configs` since 0038.
    ///
    /// **Nothing reads this today.** Its only consumer was the native loop's own
    /// context accounting, deleted with that runtime (rc3 D9) — on claude-code
    /// the meter comes from the CLI's own `contextWindow` report
    /// (`agents::events`), not from here. Kept because the column is still
    /// written and this phase writes no migration.
    #[serde(default)]
    #[sqlx(default)]
    pub context_window: Option<i64>,
}

/// A saved model in the user-managed registry (`models` table). Bundles the
/// provider + model id + optional gateway (`base_url`) and credential
/// (`auth_token`) an agent spawns with. Referenced by id from session-create.
///
/// **`models.native` is deliberately absent.** The column still exists (0036)
/// and still carries whatever the user last ticked, but rc3 D9 deleted the
/// runtime it selected, so nothing reads it and this struct does not project it.
/// Dropping the column would need a migration; the user is starting the database
/// over, so it is left in place and unread rather than migrated away.
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
    /// Context window in tokens. `None` = unknown — never guessed.
    ///
    /// **Nothing reads this today** — see [`AgentConfig::context_window`].
    #[serde(default)]
    pub context_window: Option<i64>,
}

/// Surface type of a question parked for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    /// `ask_user_choice` — has a fixed set of options.
    Choice,
    /// An Approve/Reject GATE — a parked `action_gate` command, a push gate,
    /// a reviewer-down override request: policy-initiated, menu exactly
    /// `["Approve","Reject"]`, answered on the spot in the gate slot rather
    /// than the tray. Written at insert since round 8; rows parked before
    /// that carry `choice` with the gate menu, which is why every reader goes
    /// through [`is_gate_row`](crate::storage::is_gate_row) (kind OR menu),
    /// not the kind alone.
    Approval,
    /// Free-text open question — user types a reply via normal chat input.
    OpenAsk,
    /// `mark_awaiting_user` — informational halt. **Nothing writes this since
    /// rc3 D35** (the halt is a session-state slot, `sessions.halt_*`); kept so
    /// the rows parked before D35 still parse.
    Halt,
}

impl QuestionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            QuestionKind::Choice => "choice",
            QuestionKind::Approval => "approval",
            QuestionKind::OpenAsk => "open_ask",
            QuestionKind::Halt => "halt",
        }
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
    /// JSON-encoded `Vec<String>` for kind=choice / approval; NULL for open_ask / halt.
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
/// to the user).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Open,
    Fixed,
    Rebutted,
}

impl FindingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FindingStatus::Open => "open",
            FindingStatus::Fixed => "fixed",
            FindingStatus::Rebutted => "rebutted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "open" => FindingStatus::Open,
            "fixed" => FindingStatus::Fixed,
            "rebutted" => FindingStatus::Rebutted,
            _ => return None,
        })
    }
}

/// A row from the `activity_events` table — one duo-activity transition.
///
/// `state` is the DERIVED session activity, which collapses both agents and is
/// outranked by `awaiting`/`paused`; the per-agent flags are what answer "was
/// anyone actually working at that moment?". See the migration.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub id: i64,
    pub session_id: String,
    pub recorded_at: String,
    /// `idle` | `busy` | `awaiting_user` | `cancelling` | `paused`.
    pub state: String,
    pub slot0_busy: i64,
    pub slot1_busy: i64,
}

/// A row from the `cancel_events` table — one Stop, with everything needed to
/// tell the candidate failure paths apart after the fact. See the migration for
/// why each field is here.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct CancelEvent {
    pub id: i64,
    pub session_id: String,
    pub pressed_at: String,
    pub settled_at: String,
    pub deferred_ms: i64,
    pub deferral_capped: i64,
    /// 1 = queued, 0 = dropped (full/closed channel), None = no such agent.
    pub slot0_interrupt_queued: Option<i64>,
    pub slot1_interrupt_queued: Option<i64>,
    pub both_idle: i64,
    pub cancel_superseded: i64,
    pub idled_since_cancel: i64,
    /// `honored` | `superseded` | `sigkill`.
    pub outcome: String,
}

/// A row from the `agent_feedback` table — an agent's issue or idea about
/// bot-hq ITSELF, filed from whatever project it happened to be working on.
/// `session_id`/`project` are provenance: they say where the friction was hit,
/// not what the feedback is about (that is always bot-hq).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AgentFeedback {
    pub id: i64,
    pub session_id: String,
    pub project: Option<String>,
    pub agent: String,
    /// `issue` (something is broken/annoying) | `idea` (something could be better).
    pub kind: String,
    pub title: String,
    pub body: String,
    /// `open` | `done` | `dismissed`.
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
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

/// Aggregated `retrieval_events` telemetry (Stage 4b measurement). Raw counts
/// come from SQL; the ratios are derived in Rust to dodge SQL float/NULL edge
/// cases. See [`Storage::retrieval_stats`](crate::storage::Storage::retrieval_stats).
#[derive(Debug, Clone, Serialize)]
pub struct RetrievalStats {
    /// Number of `cl_retrieve` calls logged in scope.
    pub event_count: i64,
    /// Distinct sessions that issued a retrieval.
    pub distinct_sessions: i64,
    /// Total atom-body tokens handed back across all events.
    pub total_tokens: i64,
    /// Total atoms handed back across all events.
    pub total_atoms: i64,
    /// Atoms returned with the ⚠ drift flag set.
    pub stale_hits: i64,
    /// Events that returned zero atoms (a retrieval miss).
    pub empty_returns: i64,
    /// `total_tokens / event_count` (0.0 when no events).
    pub avg_tokens_per_event: f64,
    /// `total_tokens / distinct_sessions` — the tokens-per-task proxy.
    pub avg_tokens_per_session: f64,
    /// `stale_hits / total_atoms` — should trend toward 0.
    pub stale_hit_rate: f64,
    /// `empty_returns / event_count` — retrieval-miss rate.
    pub empty_return_rate: f64,
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
    /// Consent-frozen CSP grant (serialized `CspExtraOrigins` approved at
    /// install). `None` = strict default CSP — including every install made
    /// by a pre-CSP host, which stored the manifest but never consented.
    pub csp_json: Option<String>,
    /// Dev-mode install: `dir_path` is the user's SOURCE directory, served
    /// directly (no copy) and NEVER deleted by uninstall.
    pub linked: bool,
    /// Copy-mode only: where the copy came from (local dir or manifest
    /// URL), for "Update from source" + Reinstall pre-fill. `None` for
    /// linked rows (dir_path IS the source) and pre-0032 installs.
    pub source_path: Option<String>,
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
    /// false = user-only (hidden from agent-facing search/retrieval and
    /// refused by agent cl_write_file); the Library UI always sees the file.
    pub agent_visible: bool,
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
