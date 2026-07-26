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
    /// Carried from the saved model the user assigned. This is the ONLY way a
    /// native agent can be reached from any path that doesn't name a model id
    /// on the session row — "Maintain CL", the plugin proxy, and a driver
    /// `create_session` without ids all resolve through here.
    pub native: bool,
    /// Likewise: the context window the user supplied for the assigned model.
    /// `None` = unknown, which keeps the meter a visible gap.
    pub context_window: Option<i64>,
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
            native: c.native,
            context_window: c.context_window,
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
            native: v.native,
            context_window: v.context_window,
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
            native: false,
            context_window: None,
        };
        storage.upsert_agent_config(&cfg).await.unwrap();

        let fetched = storage.get_agent_config("brian").await.unwrap().unwrap();
        assert_eq!(fetched.provider, "anthropic");
        let view: AgentConfigView = fetched.into();
        assert_eq!(view.model_name, "fast-thinker-1");
    }

    #[test]
    fn the_view_round_trips_the_native_flag_and_context_window() {
        // The view is the ONLY channel between the Agents tab and the saved
        // config, so a field missing here is a control the user can set and never
        // persist — which is exactly how a native model assigned to Rain silently
        // spawned claude-code.
        let cfg = AgentConfig {
            agent_name: "rain".into(),
            provider: "deepseek".into(),
            model_name: "deepseek-v4-pro".into(),
            base_url: Some("https://api.deepseek.com/anthropic".into()),
            auth_token: Some("tok".into()),
            updated_at: "2026-07-27T00:00:00.000Z".into(),
            native: true,
            context_window: Some(1_000_000),
        };
        let back: AgentConfig = AgentConfigView::from(cfg.clone()).into();

        assert!(back.native);
        assert_eq!(back.context_window, Some(1_000_000));
        assert_eq!(back.model_name, cfg.model_name);
        assert_eq!(back.auth_token, cfg.auth_token);
    }
}
