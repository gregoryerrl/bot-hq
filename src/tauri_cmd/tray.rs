//! Pending choice / question commands.

use crate::core::AppState as CoreAppState;
use crate::signaling::SignalingBridge;
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

/// Outcome of resolving a choice. `NeedsStaleConfirm` means the pick would run a
/// gated command whose requesting agent has moved on (client timeout / restart)
/// — nothing ran; the UI must confirm (the command may be invalid/destructive
/// against a changed repo) and re-call with `confirm_stale = true`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolveResult {
    Resolved,
    NeedsStaleConfirm {
        command: String,
        asked_at: Option<String>,
    },
}

#[tauri::command]
#[specta::specta]
pub async fn resolve_choice(
    core: tauri::State<'_, Arc<CoreAppState>>,
    choice_id: String,
    picked: String,
    confirm_stale: bool,
) -> Result<ResolveResult, AppError> {
    use crate::signaling::ResolveOutcome;
    let outcome = core.resolve_choice(&choice_id, picked, confirm_stale).await?;
    Ok(match outcome {
        ResolveOutcome::StaleGateNeedsConfirm { command, asked_at } => {
            ResolveResult::NeedsStaleConfirm { command, asked_at }
        }
        _ => ResolveResult::Resolved,
    })
}

/// One durable `session_tray` row, projected for the session-view Tray tab.
/// Unlike [`PendingChoiceView`] (live in-memory pending only), this surfaces
/// every tray item for the session — pending AND resolved history — so the tab
/// shows what accumulated even across restarts.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SessionTrayView {
    pub id: i64,
    pub session_id: String,
    pub choice_id: String,
    pub agent: String,
    pub kind: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub status: String,
    pub picked_option: Option<String>,
    /// The gated command (action_gate / ToolBlocklist approvals); null otherwise.
    pub command_text: Option<String>,
    pub asked_at: String,
    pub answered_at: Option<String>,
    /// True when this is a PENDING gated command whose requesting agent has moved
    /// on (client timeout / restart) — approving runs the command blind, so the
    /// UI warns + requires confirm. Computed at list time from the live in-memory
    /// pending map (false from the bare `From` conversion; the list commands set it).
    pub stale: bool,
}

impl From<crate::storage::SessionTrayEntry> for SessionTrayView {
    fn from(e: crate::storage::SessionTrayEntry) -> Self {
        let options = e.options().unwrap_or_default();
        Self {
            id: e.id,
            session_id: e.session_id,
            choice_id: e.choice_id,
            agent: e.agent,
            kind: e.kind,
            prompt: e.prompt,
            options,
            status: e.status,
            picked_option: e.picked_option,
            command_text: e.command_text,
            asked_at: e.asked_at,
            answered_at: e.answered_at,
            stale: false,
        }
    }
}

impl SessionTrayView {
    /// Set `stale` for a pending gated-command row: true unless its requesting
    /// agent is still live-waiting (`live` = `bridge.live_waiting_gates()`).
    fn with_staleness(mut self, live: &std::collections::HashSet<String>) -> Self {
        self.stale =
            self.status == "pending" && self.command_text.is_some() && !live.contains(&self.choice_id);
        self
    }
}

/// All tray rows for a session, oldest-first (the Tab filters/render decide
/// what to show). Reads the durable table via the bridge, so it survives
/// restarts and includes resolved history.
#[tauri::command]
#[specta::specta]
pub async fn list_session_tray(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
) -> Result<Vec<SessionTrayView>, AppError> {
    let rows = bridge.list_questions_for_session(&session_id).await?;
    let live = bridge.live_waiting_gates().await;
    Ok(rows
        .into_iter()
        .map(|e| SessionTrayView::from(e).with_staleness(&live))
        .collect())
}

/// All pending tray rows for OPEN sessions across the whole app — powers the
/// header notifier's per-session "needs your input [N]" counts. Durable, so it
/// survives a restart (unlike the in-memory `list_pending_choices`). Closed
/// sessions are excluded so dead-session pending isn't
/// surfaced as noise.
#[tauri::command]
#[specta::specta]
pub async fn list_pending_tray(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
) -> Result<Vec<SessionTrayView>, AppError> {
    let rows = bridge.list_pending_tray_open().await?;
    let live = bridge.live_waiting_gates().await;
    Ok(rows
        .into_iter()
        .map(|e| SessionTrayView::from(e).with_staleness(&live))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_pending_choices_empty_initially() {
        let bridge = SignalingBridge::new();
        let v = bridge.list_pending_choices().await;
        assert!(v.is_empty());
    }

    #[test]
    fn tray_view_decodes_options_and_keeps_command() {
        let entry = crate::storage::SessionTrayEntry {
            id: 1,
            session_id: "s".into(),
            choice_id: "c".into(),
            agent: "brian".into(),
            kind: "choice".into(),
            prompt: "Run gated command?".into(),
            options_json: Some(r#"["Approve","Reject"]"#.into()),
            status: "pending".into(),
            picked_option: None,
            asked_at: "t0".into(),
            answered_at: None,
            supersedes_id: None,
            command_text: Some("gh api user".into()),
        };
        let view: SessionTrayView = entry.into();
        assert_eq!(view.options, vec!["Approve", "Reject"]);
        assert_eq!(view.command_text.as_deref(), Some("gh api user"));
        assert_eq!(view.status, "pending");
        assert_eq!(view.kind, "choice");
    }
}
