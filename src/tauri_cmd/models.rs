//! Models registry + app settings (default model) commands. Backs the
//! Settings → Models subtab and the session-create model pickers.

use crate::storage::{Model, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::Emitter;

/// Frontend-facing shape of a saved model. `auth_token` is exposed (the desktop
/// UI is local + trusted, like the AgentCard token field).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ModelView {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub model_name: String,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Context window in tokens, or `null` when unknown.
    ///
    /// The meter still takes its denominator from the CLI's own `contextWindow`
    /// report; this value is what the pump compares that report AGAINST, so a
    /// model the CLI does not recognise (200k by default) is called out in the
    /// channel instead of silently compacting every few turns.
    pub context_window: Option<i64>,
    /// claude-code settings merged into every participant's `--settings` at
    /// spawn — a JSON object as text, or `null`. `{"modelOverrides":{"claude-fable-5":
    /// "claude-fable-5-1"}}` is the shape that gives a model id newer than the
    /// installed CLI's catalog its real window.
    pub cli_settings: Option<String>,
}

/// `cli_settings` must be a JSON object (or absent). Anything else would be
/// merged into `--settings` as garbage, and claude-code silently ignores a
/// malformed `--settings` in `-p` mode — the whole hook block would vanish
/// with it. Empty / whitespace-only text is normalised to `None`.
fn validate_cli_settings(raw: Option<String>) -> Result<Option<String>, AppError> {
    let Some(text) = raw else { return Ok(None) };
    if text.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Object(_)) => Ok(Some(text)),
        Ok(_) => Err(AppError::Validation(
            "CLI settings must be a JSON object, e.g. {\"modelOverrides\":{…}}".into(),
        )),
        Err(e) => Err(AppError::Validation(format!("CLI settings is not valid JSON: {e}"))),
    }
}

impl From<Model> for ModelView {
    fn from(m: Model) -> Self {
        Self {
            id: m.id,
            display_name: m.display_name,
            provider: m.provider,
            model_name: m.model_name,
            base_url: m.base_url,
            auth_token: m.auth_token,
            created_at: m.created_at,
            updated_at: m.updated_at,
            context_window: m.context_window,
            cli_settings: m.cli_settings,
        }
    }
}

impl From<ModelView> for Model {
    fn from(v: ModelView) -> Self {
        Self {
            id: v.id,
            display_name: v.display_name,
            provider: v.provider,
            model_name: v.model_name,
            base_url: v.base_url,
            auth_token: v.auth_token,
            created_at: v.created_at,
            updated_at: v.updated_at,
            context_window: v.context_window,
            cli_settings: v.cli_settings,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_models(
    storage: tauri::State<'_, Arc<Storage>>,
) -> Result<Vec<ModelView>, AppError> {
    storage
        .list_models()
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::DbError(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_model(
    storage: tauri::State<'_, Arc<Storage>>,
    app: tauri::AppHandle,
    model: ModelView,
) -> Result<(), AppError> {
    let mut m: Model = model.into();
    m.cli_settings = validate_cli_settings(m.cli_settings.take())?;
    storage
        .upsert_model(&m)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    let _ = app.emit(crate::tauri_events::types::MODEL_CHANGED, ());
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_model(
    storage: tauri::State<'_, Arc<Storage>>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), AppError> {
    storage
        .delete_model(&id)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    let _ = app.emit(crate::tauri_events::types::MODEL_CHANGED, ());
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_app_setting(
    storage: tauri::State<'_, Arc<Storage>>,
    key: String,
) -> Result<Option<String>, AppError> {
    storage
        .get_setting(&key)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn set_app_setting(
    storage: tauri::State<'_, Arc<Storage>>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    storage
        .set_setting(&key, &value)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn view_roundtrips_through_model() {
        let view = ModelView {
            id: "m1".into(),
            display_name: "Opus".into(),
            provider: "anthropic".into(),
            model_name: "claude-opus-4-8".into(),
            base_url: Some("https://example/anthropic".into()),
            auth_token: Some("sk".into()),
            created_at: "2026-06-03T00:00:00.000Z".into(),
            updated_at: "2026-06-03T00:00:00.000Z".into(),
            context_window: Some(200_000),
            cli_settings: None,
        };
        let back: ModelView = Model::from(view.clone()).into();
        assert_eq!(back, view);
    }

    #[test]
    fn cli_settings_accepts_an_object_and_nothing_else() {
        let obj = r#"{"modelOverrides":{"claude-fable-5":"claude-fable-5-1"}}"#;
        assert_eq!(
            validate_cli_settings(Some(obj.to_string())).unwrap(),
            Some(obj.to_string())
        );
        assert_eq!(validate_cli_settings(None).unwrap(), None);
        assert_eq!(validate_cli_settings(Some("  \n".into())).unwrap(), None);
        // A JSON array or scalar is valid JSON and still refused: only an
        // object can be merged into `--settings`.
        assert!(validate_cli_settings(Some("[1,2]".into())).is_err());
        assert!(validate_cli_settings(Some("\"modelOverrides\"".into())).is_err());
        assert!(validate_cli_settings(Some("{not json".into())).is_err());
    }
}
