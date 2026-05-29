//! Session-level permission grants for the duo.
//!
//! Each session can be granted permissions to skip the per-action approval
//! dance for `commit` and/or `push`. Default = ask for everything (no grant).
//!
//! Storage: bridge in-memory cache is the source of truth. We MIRROR each
//! grant to `<data_dir>/.local/session-permissions/<session_id>.json` so the
//! git pre-push hook (a separate subprocess invoked by git) can read the
//! grant without making an HTTP call back into the daemon.
//!
//! Lifecycle:
//! - Created/updated by the bridge when an agent calls `grant_session_permission`.
//! - Deleted by `close_session` (cache + file both go).
//! - All files purged on bot-hq startup (cache is gone after restart anyway —
//!   any leftover file would let a fresh session inherit grants it never earned).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPermissions {
    #[serde(default)]
    pub commit: GrantScope,
    #[serde(default)]
    pub push: GrantScope,
}

/// How broadly a grant applies. `None` is the default — ask every time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GrantScope {
    /// No grant — agent must call `request_approval`.
    #[default]
    None,
    /// Granted for any branch in this session.
    AllBranches,
    /// Granted only for the listed branches.
    Specific { branches: Vec<String> },
}

impl GrantScope {
    /// True if the scope grants permission for the given branch.
    pub fn allows(&self, branch: &str) -> bool {
        match self {
            Self::None => false,
            Self::AllBranches => true,
            Self::Specific { branches } => branches.iter().any(|b| b == branch),
        }
    }
}

/// Which action a permission applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionAction {
    Commit,
    Push,
}

impl SessionPermissions {
    /// NOTE: the commit grant is plumbed end-to-end (set via
    /// `PermissionAction::Commit`, persisted, mirrored, shown in the UI) but
    /// NOT yet enforced — there is no commit-side git hook gate that reads
    /// this, only the pre-push hook reads `allows_push`. Keep until a
    /// commit-msg/pre-commit grant gate lands.
    pub fn allows_commit(&self, branch: &str) -> bool {
        self.commit.allows(branch)
    }

    pub fn allows_push(&self, branch: &str) -> bool {
        self.push.allows(branch)
    }

    pub fn set(&mut self, action: PermissionAction, scope: GrantScope) {
        match action {
            PermissionAction::Commit => self.commit = scope,
            PermissionAction::Push => self.push = scope,
        }
    }
}

/// Path for one session's permissions JSON. Lives under `.local/` alongside
/// other runtime state (the SQLite db).
pub fn session_permission_path(data_dir: &Path, session_id: &str) -> PathBuf {
    data_dir
        .join(".local")
        .join("session-permissions")
        .join(format!("{session_id}.json"))
}

/// Write the session's current permissions to disk. Overwrites any prior
/// snapshot — the cache is the source of truth, the file is just a mirror.
pub fn write_session_permission(
    data_dir: &Path,
    session_id: &str,
    perm: &SessionPermissions,
) -> Result<()> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(());
    }
    let path = session_permission_path(data_dir, session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dir for {}", path.display()))?;
    }
    let body = serde_json::to_string_pretty(perm)
        .with_context(|| "serializing session permissions")?;
    std::fs::write(&path, body)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Read the session's permissions if a snapshot exists. Returns `Ok(None)`
/// when the file is absent (i.e. no grants have been recorded yet). Parse
/// errors surface as Err so they're caught + logged.
pub fn read_session_permission(
    data_dir: &Path,
    session_id: &str,
) -> Result<Option<SessionPermissions>> {
    let path = session_permission_path(data_dir, session_id);
    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let perm: SessionPermissions = serde_json::from_str(&body)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(perm))
}

/// Delete the session's permissions file. Idempotent — absent file is fine.
pub fn delete_session_permission(data_dir: &Path, session_id: &str) -> Result<()> {
    let path = session_permission_path(data_dir, session_id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("removing {}", path.display())),
    }
}

/// Wipe every session permissions file. Called at bot-hq startup — bridge
/// memory is gone, leftover files would be stale.
pub fn purge_all_session_permissions(data_dir: &Path) -> Result<()> {
    let dir = data_dir.join(".local").join("session-permissions");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_is_none() {
        let p = SessionPermissions::default();
        assert!(!p.allows_commit("main"));
        assert!(!p.allows_push("main"));
    }

    #[test]
    fn all_branches_grants_everything() {
        let mut p = SessionPermissions::default();
        p.set(PermissionAction::Push, GrantScope::AllBranches);
        assert!(p.allows_push("main"));
        assert!(p.allows_push("anything"));
        assert!(!p.allows_commit("main"));
    }

    #[test]
    fn specific_grants_only_listed_branches() {
        let mut p = SessionPermissions::default();
        p.set(
            PermissionAction::Push,
            GrantScope::Specific {
                branches: vec!["main".into(), "develop".into()],
            },
        );
        assert!(p.allows_push("main"));
        assert!(p.allows_push("develop"));
        assert!(!p.allows_push("feature-x"));
    }

    #[test]
    fn write_read_delete_roundtrip() {
        let dir = tempdir().unwrap();
        let mut p = SessionPermissions::default();
        p.set(PermissionAction::Commit, GrantScope::AllBranches);
        p.set(
            PermissionAction::Push,
            GrantScope::Specific {
                branches: vec!["main".into()],
            },
        );

        write_session_permission(dir.path(), "sess-A", &p).unwrap();

        let loaded = read_session_permission(dir.path(), "sess-A")
            .unwrap()
            .unwrap();
        assert_eq!(loaded, p);

        delete_session_permission(dir.path(), "sess-A").unwrap();
        assert!(read_session_permission(dir.path(), "sess-A")
            .unwrap()
            .is_none());
    }

    #[test]
    fn purge_clears_everything() {
        let dir = tempdir().unwrap();
        for sid in ["a", "b", "c"] {
            write_session_permission(dir.path(), sid, &SessionPermissions::default()).unwrap();
        }
        purge_all_session_permissions(dir.path()).unwrap();
        for sid in ["a", "b", "c"] {
            assert!(read_session_permission(dir.path(), sid).unwrap().is_none());
        }
    }

    #[test]
    fn missing_file_returns_none() {
        let dir = tempdir().unwrap();
        assert!(read_session_permission(dir.path(), "ghost")
            .unwrap()
            .is_none());
    }

    #[test]
    fn delete_missing_is_ok() {
        let dir = tempdir().unwrap();
        delete_session_permission(dir.path(), "ghost").unwrap();
    }
}
