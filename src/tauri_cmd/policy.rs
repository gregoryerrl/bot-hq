//! Tauri commands for the three-tier policy editors (global / project /
//! session). **User-only by construction** — these are Tauri commands, not MCP
//! tools, so agents (which only reach the JSON-RPC tool registry) can never
//! invoke them. Each tier edits a [`crate::policy::Policy`]; the `Policy`-only
//! boundary keeps the `#[serde(flatten)]` [`SessionPolicy`] off the wire (and
//! out of the specta bindings).
//!
//! - **Global** → `<data_dir>/general-policy.yaml`.
//! - **Project** → `<cl_path>/policy.yaml` (resolved via the projects row so a
//!   non-default `cl_path` is honored, matching the resolver + auditor).
//! - **Session** → `.local/session-policies/<sid>.yaml`, the canonical snapshot.
//!   `get` returns the snapshot verbatim when seeded, else the resolved
//!   general+project blueprint (so the gear tab shows real values even before
//!   the agents finish spawning). `set` preserves the snapshot's frozen
//!   `tool_gate` (tool_gate stays global-only via Settings → Tool Gate).
//!
//! Global + project writes call [`crate::policy::audit::record_policy_write`]
//! so an authorized edit doesn't read back as an unauthorized `PolicyMutation`
//! on the next audit pass.

use crate::policy::{self, Policy, SessionPolicy};
use crate::signaling::SignalingBridge;
use crate::storage::Storage;
use crate::tauri_cmd::error::AppError;
use std::path::PathBuf;
use std::sync::Arc;

fn data_dir(bridge: &SignalingBridge) -> Result<PathBuf, AppError> {
    bridge
        .data_dir()
        .ok_or_else(|| AppError::Internal("bridge data_dir not configured".into()))
        .cloned()
}

// --- Global tier -------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_general_policy(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
) -> Result<Policy, AppError> {
    let dd = data_dir(&bridge)?;
    Ok(policy::read_policy_file(&policy::general_policy_path(&dd))?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_general_policy(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    policy: Policy,
) -> Result<(), AppError> {
    let dd = data_dir(&bridge)?;
    let path = policy::general_policy_path(&dd);
    policy::write_policy_file(&path, &policy)?;
    policy::audit::record_policy_write(&dd, &path)?;
    Ok(())
}

// --- Project tier ------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_project_policy(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    storage: tauri::State<'_, Arc<Storage>>,
    project: String,
) -> Result<Policy, AppError> {
    let dd = data_dir(&bridge)?;
    let root = storage
        .cl_path_for_project(&dd, &project)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(policy::read_policy_file(&root.join("policy.yaml"))?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_project_policy(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    storage: tauri::State<'_, Arc<Storage>>,
    project: String,
    policy: Policy,
) -> Result<(), AppError> {
    let dd = data_dir(&bridge)?;
    let root = storage
        .cl_path_for_project(&dd, &project)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let path = root.join("policy.yaml");
    policy::write_policy_file(&path, &policy)?;
    policy::audit::record_policy_write(&dd, &path)?;
    Ok(())
}

// --- Session tier ------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_session_policy(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
) -> Result<Policy, AppError> {
    // resolve_policy_for returns the canonical snapshot verbatim when seeded,
    // else the resolved general+project overlay — exactly what the gear tab
    // should display before the snapshot exists.
    Ok(bridge.resolve_policy_for(&session_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_policy(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
    policy: Policy,
) -> Result<(), AppError> {
    let dd = data_dir(&bridge)?;
    // Preserve the snapshot's frozen tool_gate; seed from the global list if no
    // snapshot exists yet (matches the spawn-time seed in core/session.rs).
    let tool_gate = match policy::session_policy::read_session_policy(&dd, &session_id)? {
        Some(sp) => sp.tool_gate,
        None => policy::tool_gate::load(&dd),
    };
    let sp = SessionPolicy { policy, tool_gate };
    policy::session_policy::write_session_policy(&dd, &session_id, &sp)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    // The Tauri wrappers are thin (bridge/storage → policy file helpers); the
    // file load/save + audit-record logic is unit-tested in `policy::mod` and
    // `policy::audit`. Here we assert the on-disk round-trips the commands
    // depend on, including the session-set tool_gate-preservation invariant.
    use crate::policy::session_policy::{read_session_policy, write_session_policy};
    use crate::policy::tool_gate::{GateMode, GatedKeyword};
    use crate::policy::{
        general_policy_path, read_policy_file, write_policy_file, ForcePushMode, Policy,
        PushGateMode, SessionPolicy,
    };
    use tempfile::tempdir;

    fn sample_policy() -> Policy {
        Policy {
            forbidden_in_commits: vec!["bot-hq".into()],
            push_gate: PushGateMode::Ask,
            force_push: ForcePushMode::Blocked,
            per_action_approval: vec!["terraform apply".into()],
            branch_pattern: "feature/.*".into(),
            commit_style: "imperative".into(),
        }
    }

    #[test]
    fn general_policy_round_trip() {
        let dir = tempdir().unwrap();
        let path = general_policy_path(dir.path());
        write_policy_file(&path, &sample_policy()).unwrap();
        assert_eq!(read_policy_file(&path).unwrap(), sample_policy());
    }

    #[test]
    fn absent_general_policy_reads_default() {
        let dir = tempdir().unwrap();
        let path = general_policy_path(dir.path());
        assert_eq!(read_policy_file(&path).unwrap(), Policy::default());
    }

    #[test]
    fn project_policy_round_trip_at_convention_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("projects").join("foo").join("policy.yaml");
        write_policy_file(&path, &sample_policy()).unwrap();
        assert_eq!(read_policy_file(&path).unwrap(), sample_policy());
    }

    #[test]
    fn session_set_preserves_frozen_tool_gate() {
        // Seed a snapshot whose tool_gate was frozen at spawn, then "set" a new
        // Policy via the same read-preserve-write path set_session_policy uses.
        // The tool_gate must survive — the per-session form never touches it.
        let dir = tempdir().unwrap();
        let frozen = vec![
            GatedKeyword { keyword: "gh".into(), mode: GateMode::Gate },
            GatedKeyword { keyword: "git push".into(), mode: GateMode::AutoAllow },
        ];
        write_session_policy(
            dir.path(),
            "s1",
            &SessionPolicy { policy: Policy::default(), tool_gate: frozen.clone() },
        )
        .unwrap();

        let existing = read_session_policy(dir.path(), "s1").unwrap().unwrap();
        let next = SessionPolicy { policy: sample_policy(), tool_gate: existing.tool_gate };
        write_session_policy(dir.path(), "s1", &next).unwrap();

        let loaded = read_session_policy(dir.path(), "s1").unwrap().unwrap();
        assert_eq!(loaded.policy, sample_policy());
        assert_eq!(loaded.tool_gate, frozen, "frozen tool_gate must be preserved");
    }
}
