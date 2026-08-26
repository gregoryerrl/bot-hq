//! Tauri commands for the three-tier policy editors (global / project /
//! session). **User-only by construction** — these are Tauri commands, not MCP
//! tools, so agents (which only reach the JSON-RPC tool registry) can never
//! invoke them. Each tier edits a [`crate::policy::Policy`]; the `Policy`-only
//! boundary keeps the `#[serde(flatten)]` [`SessionPolicy`] off the wire (and
//! out of the specta bindings).
//!
//! - **Global** → `<data_dir>/config/general-policy.yaml`.
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

use crate::policy::tool_gate::GatedKeyword;
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

/// Read the full enforcement audit trail (`<data_dir>/.local/violations.jsonl`)
/// for the Settings → Violations viewer. Parse-tolerant (the reader skips
/// malformed lines); empty when the log doesn't exist yet.
#[tauri::command]
#[specta::specta]
pub async fn read_violations(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
) -> Result<Vec<policy::violations::ViolationRecord>, AppError> {
    let dd = data_dir(&bridge)?;
    Ok(policy::violations::ViolationsLog::new(&dd).read_all()?)
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

/// Resolve the one-time starter-policy offer (1.0.1; mirrors
/// `resolve_role_preset_offer`). Split from the command so the F6 assertion
/// (an existing file survives byte-identical) can run in a unit test.
///
/// Writes the RAW commented starter (`presets::STARTER_GENERAL_POLICY_YAML`),
/// not a serialized `Policy` — the comments teaching the format are the
/// point, and `write_policy_file` would strip them.
pub(crate) async fn resolve_policy_offer_inner(
    storage: &Storage,
    dd: &std::path::Path,
    install: bool,
) -> Result<(), AppError> {
    let stamp = if install {
        let path = policy::general_policy_path(dd);
        // A hand-written policy wins over the starter — but a missing or
        // whitespace-only file configures nothing, and stamping 'installed'
        // over it would lie (EYES 120806f3, the gate twin). Semantic
        // defaults are NOT second-guessed: a file that says `push_gate:
        // auto` is a real choice and is kept.
        let effectively_empty = match std::fs::read_to_string(&path) {
            Ok(body) => body.trim().is_empty(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false, // unreadable ≠ empty: don't overwrite what we can't see
        };
        if effectively_empty {
            policy::write_config_atomically(&path, policy::presets::STARTER_GENERAL_POLICY_YAML)?;
            policy::audit::record_policy_write(dd, &path)?;
            "installed"
        } else {
            "kept_existing"
        }
    } else {
        "declined"
    };
    storage
        .set_setting("policy_preset_offer", stamp)
        .await
        .map_err(|e| AppError::DbError(e.to_string()))?;
    Ok(())
}

/// The Settings → Policies card's resolver. Renders only while
/// `get_app_setting("policy_preset_offer")` is the literal `pending`; an
/// absent key means no offer.
#[tauri::command]
#[specta::specta]
pub async fn resolve_policy_preset_offer(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    storage: tauri::State<'_, Arc<Storage>>,
    install: bool,
) -> Result<(), AppError> {
    let dd = data_dir(&bridge)?;
    resolve_policy_offer_inner(&storage, &dd, install).await
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

/// Read the session's frozen Tool-Gate keyword list. Mirrors
/// [`get_session_policy`]'s fallback: the snapshot's `tool_gate` when seeded,
/// else the GLOBAL `tool-gate.json` (what a fresh spawn would seed + what the
/// hook falls back to). User-only.
#[tauri::command]
#[specta::specta]
pub async fn get_session_tool_gate(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
) -> Result<Vec<GatedKeyword>, AppError> {
    let dd = data_dir(&bridge)?;
    Ok(
        match policy::session_policy::read_session_policy(&dd, &session_id)? {
            Some(sp) => sp.tool_gate,
            None => policy::tool_gate::load(&dd),
        },
    )
}

/// Override the session's Tool-Gate keywords for THIS session only — the exact
/// mirror of [`set_session_policy`], swapping the preserved field: it keeps the
/// snapshot's [`Policy`] and replaces `tool_gate`. When no snapshot exists yet,
/// the Policy is seeded from the resolved blueprint (NOT defaulted) so the
/// inherited push/force/forbidden values aren't lost. Blank keywords are
/// dropped, matching the global Tool-Gate editor. The enforcement hook sources
/// from this snapshot first, so the change is live on the next Bash call.
#[tauri::command]
#[specta::specta]
pub async fn set_session_tool_gate(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
    keywords: Vec<GatedKeyword>,
) -> Result<(), AppError> {
    let dd = data_dir(&bridge)?;
    let policy = match policy::session_policy::read_session_policy(&dd, &session_id)? {
        Some(sp) => sp.policy,
        None => bridge.resolve_policy_for(&session_id).await?,
    };
    let tool_gate: Vec<GatedKeyword> = keywords
        .into_iter()
        .filter(|k| !k.keyword.trim().is_empty())
        .collect();
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
            commit_style: "house-style".into(),
            // Non-`None` so the file round-trips below cover it — `None` is
            // what a key dropped on write reads back as.
            round_cap: Some(250),
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
        let path = dir
            .path()
            .join("library")
            .join("projects")
            .join("foo")
            .join("policy.yaml");
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

    #[test]
    fn set_session_tool_gate_swaps_gate_preserves_policy() {
        // The inverse invariant of set_session_policy: editing the session
        // tool_gate must replace ONLY the keywords and keep the Policy intact.
        let dir = tempdir().unwrap();
        write_session_policy(
            dir.path(),
            "s1",
            &SessionPolicy {
                policy: sample_policy(),
                tool_gate: vec![GatedKeyword { keyword: "sql".into(), mode: GateMode::Gate }],
            },
        )
        .unwrap();

        // Mirror set_session_tool_gate's read-preserve-write with new keywords.
        let existing = read_session_policy(dir.path(), "s1").unwrap().unwrap();
        let next = SessionPolicy {
            policy: existing.policy,
            tool_gate: vec![
                GatedKeyword { keyword: "rm -rf".into(), mode: GateMode::Gate },
                GatedKeyword { keyword: "gh issue comment".into(), mode: GateMode::Gate },
            ],
        };
        write_session_policy(dir.path(), "s1", &next).unwrap();

        let loaded = read_session_policy(dir.path(), "s1").unwrap().unwrap();
        assert_eq!(loaded.policy, sample_policy(), "policy must be preserved");
        assert_eq!(loaded.tool_gate.len(), 2);
        assert_eq!(loaded.tool_gate[0].keyword, "rm -rf");
        assert_eq!(loaded.tool_gate[1].keyword, "gh issue comment");
    }

    #[tokio::test]
    async fn the_policy_offer_never_touches_an_existing_file() {
        // F6: an install=true against a hand-written general-policy.yaml must
        // leave it byte-identical — the starter only lands on a bare dir.
        let dir = tempfile::tempdir().unwrap();
        let s = crate::storage::Storage::memory().await.unwrap();
        let path = crate::policy::general_policy_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "push_gate: auto\nforce_push: allowed\n").unwrap();
        let before = std::fs::read(&path).unwrap();
        super::resolve_policy_offer_inner(&s, dir.path(), true).await.unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "an existing general-policy.yaml must survive byte-identical"
        );
        // EYES 120806f3: kept ≠ installed — the stamp says which happened.
        assert_eq!(
            s.get_setting("policy_preset_offer").await.unwrap().as_deref(),
            Some("kept_existing")
        );

        // A whitespace-only file configures nothing: the starter lands.
        let dir_ws = tempfile::tempdir().unwrap();
        let ws_path = crate::policy::general_policy_path(dir_ws.path());
        std::fs::create_dir_all(ws_path.parent().unwrap()).unwrap();
        std::fs::write(&ws_path, "\n  \n").unwrap();
        super::resolve_policy_offer_inner(&s, dir_ws.path(), true).await.unwrap();
        let p_ws = crate::policy::Policy::resolve(dir_ws.path(), None, None).unwrap();
        assert_eq!(p_ws.push_gate, crate::policy::PushGateMode::Ask);
        assert_eq!(
            s.get_setting("policy_preset_offer").await.unwrap().as_deref(),
            Some("installed")
        );

        // Bare dir: the commented starter lands and resolves to the safe
        // basics the card promises.
        let dir2 = tempfile::tempdir().unwrap();
        super::resolve_policy_offer_inner(&s, dir2.path(), true).await.unwrap();
        let p = crate::policy::Policy::resolve(dir2.path(), None, None).unwrap();
        assert_eq!(p.push_gate, crate::policy::PushGateMode::Ask);
        assert_eq!(p.force_push, crate::policy::ForcePushMode::Blocked);
        assert!(p.forbidden_in_commits.is_empty());

        // Decline writes nothing.
        let dir3 = tempfile::tempdir().unwrap();
        super::resolve_policy_offer_inner(&s, dir3.path(), false).await.unwrap();
        assert!(!crate::policy::general_policy_path(dir3.path()).exists());
    }
}
