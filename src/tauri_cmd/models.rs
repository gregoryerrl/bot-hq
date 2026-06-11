//! Models registry + app settings (default model) commands. Backs the
//! Settings → Models subtab and the session-create model pickers.

use crate::storage::{Model, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

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
    model: ModelView,
) -> Result<(), AppError> {
    let m: Model = model.into();
    storage
        .upsert_model(&m)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn delete_model(
    storage: tauri::State<'_, Arc<Storage>>,
    id: String,
) -> Result<(), AppError> {
    storage
        .delete_model(&id)
        .await
        .map(|_| ())
        .map_err(|e| AppError::DbError(e.to_string()))
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
        };
        let back: ModelView = Model::from(view.clone()).into();
        assert_eq!(back, view);
    }
}
