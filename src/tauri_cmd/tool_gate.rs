//! Tauri commands for the global **Tool Gate** keyword config (Settings page).
//!
//! Thin wrappers over [`crate::policy::tool_gate`] load/save against the
//! bridge's data dir. The same `<data_dir>/config/tool-gate.json` is also read by the
//! PreToolUse hook subprocess and the `action_gate` MCP tool, so the Settings
//! UI edits one global list every session honors.

use crate::policy::tool_gate::{self, GatedKeyword};
use crate::signaling::SignalingBridge;
use crate::tauri_cmd::error::AppError;
use std::sync::Arc;

#[tauri::command]
#[specta::specta]
pub async fn get_tool_gate_keywords(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
) -> Result<Vec<GatedKeyword>, AppError> {
    let data_dir = bridge
        .data_dir()
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))?
        .clone();
    Ok(tool_gate::load(&data_dir))
}

#[tauri::command]
#[specta::specta]
pub async fn set_tool_gate_keywords(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    keywords: Vec<GatedKeyword>,
) -> Result<(), AppError> {
    let data_dir = bridge
        .data_dir()
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))?
        .clone();
    tool_gate::save(&data_dir, &keywords)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::policy::tool_gate::{self, GateMode, GatedKeyword};
    use tempfile::tempdir;

    // The Tauri command wrappers are thin (data_dir → load/save); the load/save
    // + matcher logic is unit-tested in `policy::tool_gate`. Here we assert the
    // on-disk round-trip the commands depend on against a real data dir.
    #[test]
    fn keywords_persist_through_data_dir() {
        let dir = tempdir().unwrap();
        let kws = vec![
            GatedKeyword { keyword: "gh".into(), mode: GateMode::Gate },
            GatedKeyword { keyword: "git push".into(), mode: GateMode::AutoAllow },
        ];
        tool_gate::save(dir.path(), &kws).unwrap();
        assert_eq!(tool_gate::load(dir.path()), kws);
    }
}
