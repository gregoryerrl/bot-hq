//! Tauri commands for the **Claude Config** Settings subtab.
//!
//! - [`claude_config_read`] resolves the user's real `~/.claude` config into a
//!   masked, inheritance-annotated view (no state needed — reads env + fs).
//! - [`get_claude_overrides`] / [`set_claude_overrides`] load/save the per-agent
//!   override store at `<data_dir>/config/claude-overrides.json`, which the spawn path
//!   merges into the `--settings`/env/mcp-config it injects.

use crate::claude_config::{
    config_dir, load_overrides_checked, read_claude_config, save_overrides, set_bool, set_plugin_enabled,
    set_string, ClaudeConfigView, ClaudeOverrides,
};
use crate::signaling::SignalingBridge;
use crate::tauri_cmd::error::AppError;
use std::sync::Arc;

#[tauri::command]
#[specta::specta]
pub async fn claude_config_read() -> Result<ClaudeConfigView, AppError> {
    // A directory walk + a file read per skill + two JSON reads + stats — off
    // the 2-worker reactor (round 10), like `cl.rs` / `docs.rs` / `files.rs`.
    tokio::task::spawn_blocking(read_claude_config)
        .await
        .map_err(|e| AppError::Internal(format!("claude_config_read task panicked: {e}")))
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
    overrides_for_editor(&data_dir)
}

/// The store as the editors read it before they write it back whole: the
/// checked read, so an unreadable or malformed file is an error the editors
/// refuse to save over — never an empty store (EYES `5e7ef17e`).
fn overrides_for_editor(data_dir: &std::path::Path) -> Result<ClaudeOverrides, AppError> {
    Ok(load_overrides_checked(data_dir)?)
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
            .per_role
            .entry("hands".into())
            .or_default()
            .skills
            .insert("note".into(), SkillVisibility::UserInvocableOnly);
        save_overrides(dir.path(), &store).unwrap();
        assert_eq!(load_overrides(dir.path()), store);
    }

    /// EYES `5e7ef17e`: the editors write the store back WHOLE, so their read
    /// must not turn a file it could not read into an empty store — that
    /// empty store would be saved over it. Missing is a fresh, empty store; a
    /// valid file reads as itself; a malformed or unreadable one is an error
    /// naming the file, and its bytes are left as they were. (Spawn keeps the
    /// tolerant loader: a bad file must not brick a participant.)
    #[test]
    fn the_editor_read_refuses_a_store_it_cannot_read() {
        use crate::claude_config::overrides::config_path;
        let dir = tempdir().unwrap();
        assert_eq!(super::overrides_for_editor(dir.path()).unwrap(), ClaudeOverrides::default(), "missing: a fresh store");

        let mut store = ClaudeOverrides::default();
        store.per_role.entry("eyes".into()).or_default().mcp.insert("discord".into(), true);
        save_overrides(dir.path(), &store).unwrap();
        assert_eq!(super::overrides_for_editor(dir.path()).unwrap(), store, "valid: itself");

        let path = config_path(dir.path());
        let garbled = b"{\"per_role\": {\"eyes\": {\"mcp\": {\"discord\": tru";
        std::fs::write(&path, garbled).unwrap();
        let err = super::overrides_for_editor(dir.path()).expect_err("a malformed store is not an empty one");
        assert!(format!("{err:?}").contains("claude-overrides.json"), "names the file: {err:?}");
        assert_eq!(std::fs::read(&path).unwrap(), garbled, "the bytes are left as they were");
        assert_eq!(load_overrides(dir.path()), ClaudeOverrides::default(), "spawn's loader stays tolerant");

        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        let err = super::overrides_for_editor(dir.path()).expect_err("an unreadable store is not an empty one");
        assert!(format!("{err:?}").contains("claude-overrides.json"), "{err:?}");
    }
}
