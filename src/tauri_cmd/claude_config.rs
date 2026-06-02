//! Tauri commands for the **Claude Config** Settings subtab.
//!
//! - [`claude_config_read`] resolves the user's real `~/.claude` config into a
//!   masked, inheritance-annotated view (no state needed — reads env + fs).
//! - [`get_claude_overrides`] / [`set_claude_overrides`] load/save the per-agent
//!   override store at `<data_dir>/claude-overrides.json`, which the spawn path
//!   merges into the `--settings`/env/mcp-config it injects.

use crate::claude_config::{
    config_dir, load_overrides, read_claude_config, save_overrides, set_bool, set_plugin_enabled,
    set_string, ClaudeConfigView, ClaudeOverrides,
};
use crate::signaling::SignalingBridge;
use crate::tauri_cmd::error::AppError;
use std::sync::Arc;

#[tauri::command]
#[specta::specta]
pub async fn claude_config_read() -> Result<ClaudeConfigView, AppError> {
    Ok(read_claude_config())
}

#[tauri::command]
#[specta::specta]
pub async fn get_claude_overrides(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
) -> Result<ClaudeOverrides, AppError> {
    let data_dir = bridge
        .data_dir()
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))?
        .clone();
    Ok(load_overrides(&data_dir))
}

#[tauri::command]
#[specta::specta]
pub async fn set_claude_overrides(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    overrides: ClaudeOverrides,
) -> Result<(), AppError> {
    let data_dir = bridge
        .data_dir()
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))?
        .clone();
    save_overrides(&data_dir, &overrides)?;
    Ok(())
}

// --- global write-back to the user's real settings.json ---------------------

/// Set/remove a string-valued settings.json key (e.g. `effortLevel`,
/// `editorMode`, `env.CLAUDE_CODE_MAX_OUTPUT_TOKENS`). `None` removes it.
#[tauri::command]
#[specta::specta]
pub async fn claude_config_set_string(
    key: String,
    value: Option<String>,
) -> Result<(), AppError> {
    set_string(&config_dir(), &key, value)?;
    Ok(())
}

/// Set/remove a bool-valued settings.json key (e.g. `alwaysThinkingEnabled`,
/// `voiceEnabled`). `None` removes it.
#[tauri::command]
#[specta::specta]
pub async fn claude_config_set_bool(key: String, value: Option<bool>) -> Result<(), AppError> {
    set_bool(&config_dir(), &key, value)?;
    Ok(())
}

/// Enable/disable a plugin globally in `enabledPlugins` (`None` removes the
/// entry → marketplace default).
#[tauri::command]
#[specta::specta]
pub async fn claude_config_set_plugin_enabled(
    key: String,
    enabled: Option<bool>,
) -> Result<(), AppError> {
    set_plugin_enabled(&config_dir(), &key, enabled)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::claude_config::{load_overrides, save_overrides, ClaudeOverrides, SkillVisibility};
    use tempfile::tempdir;

    // Command wrappers are thin (data_dir → load/save); the load/save + resolve
    // logic is unit-tested in `claude_config::overrides`. Assert the on-disk
    // round-trip the commands depend on.
    #[test]
    fn overrides_persist_through_data_dir() {
        let dir = tempdir().unwrap();
        let mut store = ClaudeOverrides::default();
        store
            .brian
            .skills
            .insert("note".into(), SkillVisibility::UserInvocableOnly);
        save_overrides(dir.path(), &store).unwrap();
        assert_eq!(load_overrides(dir.path()), store);
    }
}
