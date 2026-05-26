//! Pending choice / question commands.

use crate::signaling::{PendingChoice, SignalingBridge};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct PendingChoiceView {
    pub choice_id: String,
    pub session_id: String,
    pub agent: String,
    pub question: String,
    pub options: Vec<String>,
}

impl From<PendingChoice> for PendingChoiceView {
    fn from(p: PendingChoice) -> Self {
        Self {
            choice_id: p.choice_id,
            session_id: p.session_id,
            agent: p.agent,
            question: p.question,
            options: p.options,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_pending_choices(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
) -> Result<Vec<PendingChoiceView>, AppError> {
    let v = bridge.list_pending_choices().await;
    Ok(v.into_iter().map(Into::into).collect())
}

#[tauri::command]
#[specta::specta]
pub async fn resolve_choice(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    choice_id: String,
    picked: String,
) -> Result<(), AppError> {
    bridge
        .resolve_choice(&choice_id, picked)
        .await
        .map(|_| ())
        .map_err(|e| AppError::Internal(e.to_string()))
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
}
