//! Typed event structs for the Tauri events surface.
//!
//! Each event carries an associated string `EVENT_NAME` that matches the
//! `app.emit(name, &payload)` call. Frontend `useTauriEvent("event.name", …)`
//! subscribes to the same name. The Tauri-specta exporter picks up these
//! types via `specta::Type` so the TypeScript bindings stay in sync.

use crate::storage::Message;
use serde::{Deserialize, Serialize};
use specta::Type;

/// One chat message in the chronological stream. Mirrors `storage::Message`
/// with `created_at` left as a string (ISO) so the frontend can parse with
/// whatever date library it picks.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct AgentMessage {
    pub id: i64,
    pub session_id: String,
    pub author: String,
    pub kind: String,
    pub content: String,
    pub created_at: String,
}

impl AgentMessage {
    /// The event name used for batched message emits. Frontend subscribes
    /// with `listen("agent:messages:batch", handler)`. Tauri 2 event names
    /// disallow dots (alphanumeric / `-` / `/` / `:` / `_` only) — emits
    /// with a dotted name return IllegalEventName and the event is dropped.
    pub const EVENT_NAME_BATCH: &'static str = "agent:messages:batch";
}

impl From<Message> for AgentMessage {
    fn from(m: Message) -> Self {
        Self {
            id: m.id,
            session_id: m.session_id,
            author: m.author,
            kind: m.kind,
            content: m.content,
            created_at: m.created_at,
        }
    }
}

/// Emitted when an agent self-advances the IPAV phase (via `advance_phase`
/// MCP tool). The dashboard chip moves; the session header subtitle updates.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct PhaseChangedEvent {
    pub session_id: String,
    pub agent: String,
    pub target: String,
}

impl PhaseChangedEvent {
    pub const EVENT_NAME: &'static str = "session:phase_changed";
}

/// Emitted when a session enters or leaves "awaiting user input" state
/// (mark_awaiting_user / ask_user_choice / request_approval set the flag).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct AwaitingUser {
    pub session_id: String,
    pub agent: String,
    pub reason: String,
}

impl AwaitingUser {
    pub const EVENT_NAME: &'static str = "session:awaiting_user";
}

/// Emitted when a parked choice resolves (user picked, or agent withdrew).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ChoiceResolvedEvent {
    pub choice_id: String,
    pub picked: String,
}

impl ChoiceResolvedEvent {
    pub const EVENT_NAME: &'static str = "session:choice_resolved";
}

/// Emitted when an agent parks a choice/question for the user (a direct-emit
/// nudge; the tray polls `list_pending_choices` as the source of truth).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct PendingChoiceEvent {
    pub choice_id: String,
    pub session_id: String,
    pub agent: String,
    pub question: String,
    pub options: Vec<String>,
}

impl PendingChoiceEvent {
    pub const EVENT_NAME: &'static str = "session:pending_choice";
}

/// Emitted when a session document is written/updated (`session_doc_write`),
/// so the doc pane refreshes without a manual tab-switch.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct DocChangedEvent {
    pub session_id: String,
}

impl DocChangedEvent {
    pub const EVENT_NAME: &'static str = "session:doc_changed";
}

/// Emitted when a session finished closing, so the UI can navigate away from
/// the now-closed session and refresh its session lists.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SessionClosedEvent {
    pub session_id: String,
}

impl SessionClosedEvent {
    pub const EVENT_NAME: &'static str = "session:closed";
}

/// Plugin lifecycle event names emitted to the frontend PluginManager, which
/// listens for the same strings. Centralized so an emit site and the listener
/// can't drift independently.
pub const PLUGIN_STATE_CHANGED: &str = "plugin:state-changed";
pub const PLUGIN_UNINSTALLED: &str = "plugin:uninstalled";
pub const PLUGIN_CRASHED: &str = "plugin:crashed";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_message_event_name_is_batch_path() {
        assert_eq!(AgentMessage::EVENT_NAME_BATCH, "agent:messages:batch");
    }

    #[test]
    fn agent_message_serializes_to_expected_shape() {
        let msg = AgentMessage {
            id: 1,
            session_id: "s1".to_string(),
            author: "brian".to_string(),
            kind: "text".to_string(),
            content: "hello".to_string(),
            created_at: "2026-05-26T18:00:00Z".to_string(),
        };
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["session_id"], "s1");
        assert_eq!(v["author"], "brian");
        assert_eq!(v["content"], "hello");
    }

    #[test]
    fn agent_message_from_storage_message_preserves_fields() {
        let m = Message {
            id: 7,
            session_id: "s1".into(),
            author: "rain".into(),
            kind: "text".into(),
            content: "looks clean".into(),
            created_at: "2026-05-26T18:01:00Z".into(),
        };
        let am: AgentMessage = m.clone().into();
        assert_eq!(am.id, m.id);
        assert_eq!(am.session_id, m.session_id);
        assert_eq!(am.author, m.author);
        assert_eq!(am.kind, m.kind);
        assert_eq!(am.content, m.content);
        assert_eq!(am.created_at, m.created_at);
    }

    #[test]
    fn phase_changed_event_name() {
        assert_eq!(PhaseChangedEvent::EVENT_NAME, "session:phase_changed");
    }

    #[test]
    fn awaiting_user_event_name() {
        assert_eq!(AwaitingUser::EVENT_NAME, "session:awaiting_user");
    }
}
