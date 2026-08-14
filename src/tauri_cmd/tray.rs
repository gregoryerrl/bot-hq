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

/// One staged tray pick, as the composer's Send hands it over (rc3 D34).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StagedPick {
    pub choice_id: String,
    pub picked: String,
}

/// **The composer's Send when tray picks are staged (rc3 D34):** the typed
/// message plus every staged answer, delivered as ONE user response. Answers
/// record first, the message posts last (framing the turn), and exactly one
/// ring release fires. `text` may be empty when at least one pick is staged —
/// answering without commentary is a complete response.
#[tauri::command]
#[specta::specta]
pub async fn send_user_response(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    text: String,
    picks: Vec<StagedPick>,
) -> Result<(), AppError> {
    core.send_user_response(
        &session_id,
        &text,
        picks.into_iter().map(|p| (p.choice_id, p.picked)).collect(),
    )
    .await?;
    Ok(())
}

/// One durable `session_tray` row, projected for the session-view Tray tab.
/// Unlike the live in-memory pending view (`list_pending_choices`), this
/// surfaces every tray item for the session — pending AND resolved history —
/// so the tab shows what accumulated even across restarts.
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
    /// Set `stale` for a pending gated-command row: true when the prompt is
    /// older than the stale-gate window. Age-based, not receiver-based —
    /// action_gate parks immediately, so no gate ever has a live-waiting
    /// receiver and the old liveness key marked every pending gate stale.
    fn with_staleness(mut self) -> Self {
        use crate::signaling::{gate_age_secs, STALE_GATE_MAX_AGE_SECS};
        self.stale = self.status == "pending"
            && self.command_text.is_some()
            && !gate_age_secs(&self.asked_at).is_some_and(|a| a <= STALE_GATE_MAX_AGE_SECS);
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
    Ok(rows
        .into_iter()
        .map(|e| SessionTrayView::from(e).with_staleness())
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
    Ok(rows
        .into_iter()
        .map(|e| SessionTrayView::from(e).with_staleness())
        .collect())
}

/// Discard a tray row from the UI without answering it — the user's bin for
/// stale questions they no longer want to answer.
///
/// Deliberately NOT `resolve_choice` with some sentinel pick: nothing is
/// delivered to the agent. Since `ask_user_choice` / `action_gate` /
/// `request_approval` all PARK, the requesting agent already holds its ack and
/// is not awaiting a value, so dropping the row tells it nothing and costs it
/// nothing.
///
/// The one caller that genuinely does await in-band is the pre-push git hook
/// (`signaling::server::handle_pre_push` → the BLOCKING `request_approval`).
/// Discarding that row drops the parked oneshot, its await returns a cancel
/// error, and `handle_pre_push` takes its `Err` branch → `approved = false`.
/// So a discarded push gate DENIES the push; it never hangs.
///
/// Returns true if a pending row was actually discarded, false if the id was
/// unknown or already resolved.
#[tauri::command]
#[specta::specta]
pub async fn discard_choice(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    choice_id: String,
) -> Result<bool, AppError> {
    Ok(bridge.withdraw_question(&choice_id).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discard_drops_the_row_without_answering_the_agent() {
        // The user's trash button must not look like an answer. A parked
        // question's agent already has its ack, so discarding delivers nothing
        // — assert no ChoiceResolved event escapes and the row stops being
        // pending.
        let bridge = SignalingBridge::new();
        let storage = crate::storage::Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();

        let ack = bridge
            .ask_user_choice(
                "s1".into(),
                "brian".into(),
                "pick".into(),
                vec!["Yes".into(), "No".into()],
            )
            .await
            .unwrap();
        let cid = serde_json::from_str::<serde_json::Value>(&ack).unwrap()["choice_id"]
            .as_str()
            .unwrap()
            .to_string();

        let mut sub = bridge.subscribe();
        assert!(bridge.withdraw_question(&cid).await, "row was pending");

        // Nothing resolution-shaped may be emitted by a discard.
        match sub.try_recv() {
            Err(_) => {}
            Ok(ev) => panic!("discard must not emit a resolution event, got {ev:?}"),
        }
        let rows = bridge.list_questions_for_session("s1").await.unwrap();
        let row = rows.iter().find(|r| r.choice_id == cid).expect("row exists");
        assert_eq!(row.status, "withdrawn");
        assert!(
            row.picked_option.is_none(),
            "a discard records no pick: {:?}",
            row.picked_option
        );
    }

    #[tokio::test]
    async fn discard_of_an_unknown_id_is_false_not_an_error() {
        let bridge = SignalingBridge::new();
        assert!(!bridge.withdraw_question("nope").await);
    }

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
