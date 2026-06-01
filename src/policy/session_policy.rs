//! Canonical per-session policy snapshot.
//!
//! At session spawn, the resolved general+project [`Policy`] PLUS the global
//! Tool-Gate keyword list is frozen into
//! `<data_dir>/.local/session-policies/<sid>.yaml`. From then on THAT file is
//! the SOLE policy for the session — read verbatim, never re-merged against
//! the blueprints. The user edits it through the gear-tab UI; the blueprints
//! (`general-policy.yaml`, `projects/<p>/policy.yaml`) only seed the snapshot.
//!
//! Lifecycle (mirrors the deleted `session_permissions` disk helpers):
//! - Seeded WRITE-IF-ABSENT by the session spawn path (so re-opens keep edits).
//! - Read verbatim by [`crate::policy::Policy::resolve_at_root`] (short-circuit)
//!   and by the Tool Gate hook (`run_tool_gate`) for its keyword source.
//! - Deleted by `close_session`.
//! - Purged on bot-hq startup (a leftover file would leak one session's
//!   resolved policy into a fresh session that should re-seed from blueprints).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The canonical session snapshot: a resolved [`Policy`] plus the Tool-Gate
/// keyword list, both frozen at spawn. `#[serde(flatten)]` keeps the YAML flat
/// — the policy fields sit at the top level alongside `tool_gate`, so a
/// hand-edited file reads like a `policy.yaml` with an extra `tool_gate:` block.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionPolicy {
    #[serde(flatten)]
    pub policy: crate::policy::Policy,
    #[serde(default)]
    pub tool_gate: Vec<crate::policy::GatedKeyword>,
}

/// Path for one session's policy snapshot. Lives under `.local/` alongside
/// other runtime state (the SQLite db), NOT in a working repo — disguise-safe.
pub fn session_policy_path(data_dir: &Path, session_id: &str) -> PathBuf {
    data_dir
        .join(".local")
        .join("session-policies")
        .join(format!("{session_id}.yaml"))
}

/// Write the session's policy snapshot to disk (YAML). Overwrites any prior
/// snapshot. A blank `session_id` is a no-op (returns Ok).
pub fn write_session_policy(
    data_dir: &Path,
    session_id: &str,
    policy: &SessionPolicy,
) -> Result<()> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(());
    }
    let path = session_policy_path(data_dir, session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dir for {}", path.display()))?;
    }
    let body = serde_yaml::to_string(policy).with_context(|| "serializing session policy")?;
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Read the session's policy snapshot if one exists. Returns `Ok(None)` when
/// the file is absent (no snapshot seeded yet). Read + parse errors surface as
/// Err so callers can log them (and fail open where appropriate).
pub fn read_session_policy(
    data_dir: &Path,
    session_id: &str,
) -> Result<Option<SessionPolicy>> {
    let path = session_policy_path(data_dir, session_id);
    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let policy: SessionPolicy = serde_yaml::from_str(&body)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(policy))
}

/// Delete the session's policy snapshot. Idempotent — absent file is fine.
pub fn delete_session_policy(data_dir: &Path, session_id: &str) -> Result<()> {
    let path = session_policy_path(data_dir, session_id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("removing {}", path.display())),
    }
}

/// Wipe every session policy snapshot. Called at bot-hq startup — a leftover
/// file would leak one session's resolved policy into a fresh session that
/// should re-seed from the current blueprints.
pub fn purge_all_session_policies(data_dir: &Path) -> Result<()> {
    let dir = data_dir.join(".local").join("session-policies");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::tool_gate::{GateMode, GatedKeyword};
    use crate::policy::{ForcePushMode, Policy, PushGateMode};
    use tempfile::tempdir;

    fn sample() -> SessionPolicy {
        SessionPolicy {
            policy: Policy {
                forbidden_in_commits: vec!["bot-hq".into(), "Claude".into()],
                push_gate: PushGateMode::Ask,
                force_push: ForcePushMode::Blocked,
                per_action_approval: vec!["terraform apply".into()],
                branch_pattern: "feature/.*".into(),
                commit_style: "imperative".into(),
            },
            tool_gate: vec![
                GatedKeyword {
                    keyword: "gh".into(),
                    mode: GateMode::Gate,
                },
                GatedKeyword {
                    keyword: "git push".into(),
                    mode: GateMode::AutoAllow,
                },
            ],
        }
    }

    #[test]
    fn write_read_delete_roundtrip() {
        let dir = tempdir().unwrap();
        let sp = sample();
        write_session_policy(dir.path(), "sess-A", &sp).unwrap();

        let loaded = read_session_policy(dir.path(), "sess-A").unwrap().unwrap();
        assert_eq!(loaded, sp);

        delete_session_policy(dir.path(), "sess-A").unwrap();
        assert!(read_session_policy(dir.path(), "sess-A").unwrap().is_none());
    }

    #[test]
    fn missing_file_returns_none() {
        let dir = tempdir().unwrap();
        assert!(read_session_policy(dir.path(), "ghost").unwrap().is_none());
    }

    #[test]
    fn delete_missing_is_ok() {
        let dir = tempdir().unwrap();
        delete_session_policy(dir.path(), "ghost").unwrap();
    }

    #[test]
    fn blank_session_id_is_noop() {
        let dir = tempdir().unwrap();
        write_session_policy(dir.path(), "   ", &sample()).unwrap();
        // Nothing was written under a real session id.
        assert!(read_session_policy(dir.path(), "   ").unwrap().is_none());
    }

    #[test]
    fn purge_clears_everything() {
        let dir = tempdir().unwrap();
        for sid in ["a", "b", "c"] {
            write_session_policy(dir.path(), sid, &sample()).unwrap();
        }
        purge_all_session_policies(dir.path()).unwrap();
        for sid in ["a", "b", "c"] {
            assert!(read_session_policy(dir.path(), sid).unwrap().is_none());
        }
    }

    #[test]
    fn yaml_is_flat_policy_plus_tool_gate() {
        // The flatten attr keeps policy fields at the top level, with tool_gate
        // as a sibling block — a hand-editable shape for the gear tab.
        let dir = tempdir().unwrap();
        write_session_policy(dir.path(), "flat", &sample()).unwrap();
        let body =
            std::fs::read_to_string(session_policy_path(dir.path(), "flat")).unwrap();
        assert!(body.contains("forbidden_in_commits"), "body: {body}");
        assert!(body.contains("tool_gate"), "body: {body}");
        // tool_gate is NOT nested under a `policy:` key.
        assert!(!body.contains("policy:"), "should be flat, got: {body}");
    }
}
