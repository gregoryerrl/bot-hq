//! Session-level permission grant commands.
//!
//! HANDS-only at the agent MCP layer — but Tauri commands are user-driven,
//! so the role gate doesn't apply here. The frontend's Settings UI uses
//! these to surface current grants and let the user revoke from outside the
//! chat flow.

use crate::policy::{GrantScope, PermissionAction, SessionPermissions};
use crate::signaling::SignalingBridge;
use crate::tauri_cmd::error::AppError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GrantScopeView {
    None,
    AllBranches,
    Specific { branches: Vec<String> },
}

impl From<GrantScope> for GrantScopeView {
    fn from(s: GrantScope) -> Self {
        match s {
            GrantScope::None => GrantScopeView::None,
            GrantScope::AllBranches => GrantScopeView::AllBranches,
            GrantScope::Specific { branches } => GrantScopeView::Specific { branches },
        }
    }
}

impl From<GrantScopeView> for GrantScope {
    fn from(v: GrantScopeView) -> Self {
        match v {
            GrantScopeView::None => GrantScope::None,
            GrantScopeView::AllBranches => GrantScope::AllBranches,
            GrantScopeView::Specific { branches } => GrantScope::Specific { branches },
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionActionView {
    Commit,
    Push,
}

impl From<PermissionActionView> for PermissionAction {
    fn from(v: PermissionActionView) -> Self {
        match v {
            PermissionActionView::Commit => PermissionAction::Commit,
            PermissionActionView::Push => PermissionAction::Push,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SessionPermissionsView {
    pub commit: GrantScopeView,
    pub push: GrantScopeView,
}

impl From<SessionPermissions> for SessionPermissionsView {
    fn from(p: SessionPermissions) -> Self {
        Self {
            commit: p.commit.into(),
            push: p.push.into(),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn grant_session_permission(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
    action: PermissionActionView,
    scope: GrantScopeView,
) -> Result<(), AppError> {
    bridge
        .grant_session_permission(&session_id, action.into(), scope.into())
        .await?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn revoke_session_permission(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
    action: PermissionActionView,
) -> Result<(), AppError> {
    bridge
        .revoke_session_permission(&session_id, action.into())
        .await?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn list_session_permissions(
    bridge: tauri::State<'_, Arc<SignalingBridge>>,
    session_id: String,
) -> Result<SessionPermissionsView, AppError> {
    let perms = bridge.list_session_permissions(&session_id).await;
    Ok(perms.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn grant_and_list_session_permission_roundtrip() {
        let bridge = SignalingBridge::new();
        bridge
            .grant_session_permission(
                "s1",
                PermissionAction::Push,
                GrantScope::Specific {
                    branches: vec!["tauri-v2-migration".to_string()],
                },
            )
            .await
            .unwrap();
        let perms = bridge.list_session_permissions("s1").await;
        let view: SessionPermissionsView = perms.into();
        assert!(matches!(view.push, GrantScopeView::Specific { branches } if branches == vec!["tauri-v2-migration".to_string()]));
        assert!(matches!(view.commit, GrantScopeView::None));
    }

    #[test]
    fn grant_scope_view_serializes_with_kind_tag() {
        let v = GrantScopeView::Specific {
            branches: vec!["main".to_string()],
        };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["kind"], "specific");
        assert_eq!(j["branches"][0], "main");
    }
}
