//! Bridge surface for `file_feedback` — an agent's issue or idea about bot-hq
//! ITSELF, filed from whatever project it happened to be working on.
//!
//! Available to EVERY participant (unlike the user-facing tray tools, which
//! are capability-gated): filing is not a repo mutation and not a user
//! interruption, and a reviewer hits bot-hq friction as often as an executor
//! does. It writes a row and
//! returns — nothing is surfaced to the user mid-session, so it cannot become
//! another way to interrupt them.

use super::SignalingBridge;
use crate::storage::FEEDBACK_KINDS;
use anyhow::Result;

impl SignalingBridge {
    /// File one issue/idea about bot-hq. Returns the reference the agent can
    /// quote back ("filed as feedback #12").
    pub async fn file_feedback(
        &self,
        session_id: &str,
        agent: &str,
        kind: &str,
        title: &str,
        body: &str,
    ) -> Result<i64> {
        let kind = kind.trim().to_ascii_lowercase();
        if !FEEDBACK_KINDS.contains(&kind.as_str()) {
            return Err(anyhow::anyhow!(
                "unknown kind '{kind}' — expected one of {FEEDBACK_KINDS:?}"
            ));
        }
        if title.trim().is_empty() {
            return Err(anyhow::anyhow!("title is required"));
        }
        // Provenance only — where the friction was hit, not what it is about
        // (that is always bot-hq). Best-effort: an unregistered session still
        // files, it just carries no project.
        let project = self.project_for_session(session_id).await;
        let storage = self
            .storage
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("storage not configured"))?;
        storage
            .insert_feedback(session_id, project.as_deref(), agent, &kind, title, body)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    async fn bridge_with_storage() -> (std::sync::Arc<SignalingBridge>, Storage) {
        let bridge = SignalingBridge::new();
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("s1", "t", None).await.unwrap();
        (bridge, storage)
    }

    #[tokio::test]
    async fn both_agents_can_file() {
        let (bridge, storage) = bridge_with_storage().await;
        bridge
            .file_feedback("s1", "hands", "issue", "gate unreadable", "…")
            .await
            .unwrap();
        bridge
            .file_feedback("s1", "eyes", "idea", "batch approvals", "…")
            .await
            .unwrap();
        let all = storage.list_feedback(None).await.unwrap();
        assert_eq!(all.len(), 2, "EYES must be able to file too");
    }

    #[tokio::test]
    async fn unknown_kind_is_refused_rather_than_stored() {
        let (bridge, storage) = bridge_with_storage().await;
        let err = bridge
            .file_feedback("s1", "hands", "complaint", "x", "y")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown kind"), "{err}");
        assert!(storage.list_feedback(None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn kind_is_normalized_not_rejected_on_case() {
        let (bridge, storage) = bridge_with_storage().await;
        bridge
            .file_feedback("s1", "hands", "  Idea ", "x", "y")
            .await
            .unwrap();
        assert_eq!(storage.list_feedback(None).await.unwrap()[0].kind, "idea");
    }

    #[tokio::test]
    async fn empty_title_is_refused() {
        let (bridge, _s) = bridge_with_storage().await;
        assert!(bridge
            .file_feedback("s1", "hands", "issue", "   ", "body")
            .await
            .is_err());
    }
}
