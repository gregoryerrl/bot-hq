//! Session-scoped commit/push permission grants. The in-memory cache is the
//! source of truth; every mutation mirrors to
//! `<data_dir>/.local/session-permissions/<sid>.json` so the git pre-push hook
//! (a separate subprocess) can read the grant without an HTTP roundtrip.

use super::*;

impl SignalingBridge {
    /// Lock the session-permissions cache, apply `mutator` to the entry
    /// for `session_id` (creating a default entry if absent), then mirror
    /// the resulting state to `<data_dir>/.local/session-permissions/<sid>.json`
    /// so the pre-push hook can see it. Single source of truth for the
    /// lock→mutate→snapshot→mirror dance shared by every public grant /
    /// revoke / branch-add path.
    async fn mutate_session_permission(
        &self,
        session_id: &str,
        mutator: impl FnOnce(&mut crate::policy::SessionPermissions),
    ) -> Result<()> {
        let mut map = self.session_permissions.lock().await;
        let perm = map
            .entry(session_id.to_string())
            .or_insert_with(crate::policy::SessionPermissions::default);
        mutator(perm);
        let snapshot = perm.clone();
        drop(map);
        if let Some(data_dir) = &self.data_dir {
            crate::policy::session_permissions::write_session_permission(
                data_dir,
                session_id,
                &snapshot,
            )?;
        }
        Ok(())
    }

    /// Set the grant scope for `action` on this session. Overwrites any prior
    /// grant for the same action. Mirrors the new cache state to
    /// `<data_dir>/.local/session-permissions/<session_id>.json` so the
    /// pre-push hook can see it.
    pub async fn grant_session_permission(
        &self,
        session_id: &str,
        action: crate::policy::PermissionAction,
        scope: crate::policy::GrantScope,
    ) -> Result<()> {
        self.mutate_session_permission(session_id, |perm| perm.set(action, scope))
            .await
    }

    /// Revoke (reset to None) the grant for `action`. Idempotent on absent
    /// grants. Re-mirrors the file.
    pub async fn revoke_session_permission(
        &self,
        session_id: &str,
        action: crate::policy::PermissionAction,
    ) -> Result<()> {
        self.mutate_session_permission(session_id, |perm| {
            perm.set(action, crate::policy::GrantScope::None)
        })
        .await
    }

    /// Read the current permissions for this session. Returns the default
    /// (no grants) if nothing has been recorded.
    pub async fn list_session_permissions(
        &self,
        session_id: &str,
    ) -> crate::policy::SessionPermissions {
        self.session_permissions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Drop the cache entry + delete the mirrored file. Called by
    /// `core::state::close_session` when the session closes.
    pub async fn cleanup_session_permissions(&self, session_id: &str) -> Result<()> {
        self.session_permissions
            .lock()
            .await
            .remove(session_id);
        if let Some(data_dir) = &self.data_dir {
            crate::policy::session_permissions::delete_session_permission(
                data_dir,
                session_id,
            )?;
        }
        Ok(())
    }

    /// Internal helper: add `branch` to the existing push grant's Specific
    /// branch list, or upgrade None → Specific{[branch]}. AllBranches is left
    /// untouched (no point narrowing). Used by `resolve_choice` to record
    /// per-push approvals as a session-level grant.
    pub(super) async fn add_branch_to_session_grant(
        &self,
        session_id: &str,
        action: crate::policy::PermissionAction,
        branch: String,
    ) -> Result<()> {
        self.mutate_session_permission(session_id, move |perm| {
            use crate::policy::GrantScope;
            let current = match action {
                crate::policy::PermissionAction::Commit => &mut perm.commit,
                crate::policy::PermissionAction::Push => &mut perm.push,
            };
            match current {
                GrantScope::None => {
                    *current = GrantScope::Specific {
                        branches: vec![branch],
                    };
                }
                GrantScope::Specific { branches } => {
                    if !branches.iter().any(|b| b == &branch) {
                        branches.push(branch);
                    }
                }
                GrantScope::AllBranches => { /* already broader */ }
            }
        })
        .await
    }
}
