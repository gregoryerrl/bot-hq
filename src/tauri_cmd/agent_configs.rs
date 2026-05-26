//! Settings page per-agent provider/model/auth_token rows.

use crate::storage::{AgentConfig, Storage};
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct AgentConfigView {
    pub agent_name: String,
    pub provider: String,
    pub model_name: String,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub updated_at: String,
}

impl From<AgentConfig> for AgentConfigView {
    fn from(c: AgentConfig) -> Self {
        Self {
            agent_name: c.agent_name,
            provider: c.provider,
            model_name: c.model_name,
            base_url: c.base_url,
            auth_token: c.auth_token,
            updated_at: c.updated_at,
        }
    }
}

impl From<AgentConfigView> for AgentConfig {
    fn from(v: AgentConfigView) -> Self {
        Self {
            agent_name: v.agent_name,
            provider: v.provider,
            model_name: v.model_name,
            base_url: v.base_url,
            auth_token: v.auth_token,
            updated_at: v.updated_at,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_config(
    storage: tauri::State<'_, Arc<Storage>>,
    agent_name: String,
) -> Result<Option<AgentConfigView>, AppError> {
    storage
        .get_agent_config(&agent_name)
        .await
        .map(|opt| opt.map(Into::into))
        .map_err(|e| AppError::DbError(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn list_agent_configs(
    storage: tauri::State<'_, Arc<Storage>>,
) -> Result<Vec<AgentConfigView>, AppError> {
    storage
        .list_agent_configs()
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map_err(|e| AppError::DbError(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_agent_config(
    storage: tauri::State<'_, Arc<Storage>>,
    cfg: AgentConfigView,
) -> Result<(), AppError> {
    let model: AgentConfig = cfg.into();
    storage
        .upsert_agent_config(&model)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upsert_and_get_agent_config_roundtrip() {
        let storage = Arc::new(Storage::memory().await.unwrap());
        let cfg = AgentConfig {
            agent_name: "brian".to_string(),
            provider: "anthropic".to_string(),
            model_name: "fast-thinker-1".to_string(),
            base_url: None,
            auth_token: Some("secret".to_string()),
            updated_at: String::new(),
        };
        storage.upsert_agent_config(&cfg).await.unwrap();

        let fetched = storage.get_agent_config("brian").await.unwrap().unwrap();
        assert_eq!(fetched.provider, "anthropic");
        let view: AgentConfigView = fetched.into();
        assert_eq!(view.model_name, "fast-thinker-1");
    }
}
