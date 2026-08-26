//! Tauri commands for the global **Tool Gate** keyword config (Settings page).
//!
//! Thin wrappers over [`crate::policy::tool_gate`] load/save against the
//! bridge's data dir. The same `<data_dir>/config/tool-gate.json` is also read by the
//! PreToolUse hook subprocess and the `action_gate` MCP tool, so the Settings
//! UI edits one global list every session honors.

use crate::policy::tool_gate::{self, GatedKeyword};
use crate::signaling::SignalingBridge;
use crate::storage::Storage;
use crate::tauri_cmd::error::AppError;
use std::path::Path;
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

/// Resolve the one-time starter-gates offer (1.0.1; mirrors
/// `resolve_role_preset_offer`). Split from the command so a unit test can
/// reach it — the F6 assertion (an existing file survives byte-identical)
/// cannot run through `tauri::State`.
pub(crate) async fn resolve_gate_offer_inner(
    storage: &Storage,
    data_dir: &Path,
    install: bool,
) -> Result<(), AppError> {
    if install {
        // NEVER overwrite: a hand-written list wins over the starter. The key
        // is stamped either way, so the card retires.
        if !tool_gate::config_path(data_dir).exists() {
            tool_gate::save(data_dir, &crate::policy::presets::starter_gate_keywords())?;
        }
    }
    storage
        .set_setting("gate_preset_offer", if install { "installed" } else { "declined" })
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    Ok(())
}

/// The Settings → Tool Gate card's resolver. Renders only while
/// `get_app_setting("gate_preset_offer")` is the literal `pending`; an absent
/// key means no offer.
#[tauri::command]
#[specta::specta]
pub async fn resolve_gate_preset_offer(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    storage: tauri::State<'_, Arc<Storage>>,
    install: bool,
) -> Result<(), AppError> {
    let data_dir = bridge
        .data_dir()
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))?
        .clone();
    resolve_gate_offer_inner(&storage, &data_dir, install).await
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

    #[tokio::test]
    async fn the_gate_offer_never_touches_an_existing_list() {
        // F6: the most damaging thing this feature could do is replace the
        // user's own tailored list. Byte-identical before/after is the claim —
        // and the key still stamps, so the card retires without a write.
        let dir = tempdir().unwrap();
        let s = crate::storage::Storage::memory().await.unwrap();
        let own = vec![GatedKeyword { keyword: "gh api".into(), mode: GateMode::Gate }];
        tool_gate::save(dir.path(), &own).unwrap();
        let path = tool_gate::config_path(dir.path());
        let before = std::fs::read(&path).unwrap();
        super::resolve_gate_offer_inner(&s, dir.path(), true).await.unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "an existing tool-gate.json must survive byte-identical"
        );
        assert_eq!(
            s.get_setting("gate_preset_offer").await.unwrap().as_deref(),
            Some("installed")
        );

        // On a bare dir the starter lands and parses back.
        let dir2 = tempdir().unwrap();
        super::resolve_gate_offer_inner(&s, dir2.path(), true).await.unwrap();
        assert_eq!(
            tool_gate::load(dir2.path()),
            crate::policy::presets::starter_gate_keywords()
        );

        // Decline writes nothing anywhere.
        let dir3 = tempdir().unwrap();
        super::resolve_gate_offer_inner(&s, dir3.path(), false).await.unwrap();
        assert!(!tool_gate::config_path(dir3.path()).exists());
        assert_eq!(
            s.get_setting("gate_preset_offer").await.unwrap().as_deref(),
            Some("declined")
        );
    }
}
