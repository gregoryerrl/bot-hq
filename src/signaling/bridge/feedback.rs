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

/// The project whose sessions may resolve the queue: it is bot-hq's own
/// queue, worked by the sessions that develop bot-hq.
pub const FEEDBACK_PROJECT: &str = "bot-hq";

/// The two closing states `resolve_feedback` takes. Reopening is the user's,
/// from the Feedback panel.
pub const FEEDBACK_RESOLUTIONS: &[&str] = &["done", "dismissed"];

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

impl SignalingBridge {
    /// Close one feedback row with the reason in words — `done` naming the
    /// commit that shipped it, `dismissed` naming the decision that declined
    /// it — recorded with who closed it. Refused outside a session on the
    /// bot-hq project, for an unknown id, a status other than done/dismissed,
    /// or an empty reason. Not capability-gated, like `file_feedback`: the
    /// project scope is the boundary, `resolved_by` says who, and the user can
    /// reopen the row from the panel.
    pub async fn resolve_feedback(
        &self,
        session_id: &str,
        agent: &str,
        id: i64,
        status: &str,
        reason: &str,
    ) -> Result<()> {
        let status = status.trim().to_ascii_lowercase();
        if !FEEDBACK_RESOLUTIONS.contains(&status.as_str()) {
            return Err(anyhow::anyhow!(
                "unknown status '{status}' — expected one of {FEEDBACK_RESOLUTIONS:?} \
                 (reopening a row is the user's, from the Feedback panel)"
            ));
        }
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(anyhow::anyhow!(
                "a reason is required — the commit that shipped it, or whose decision declined it"
            ));
        }
        let project = self.project_for_session(session_id).await;
        if project.as_deref() != Some(FEEDBACK_PROJECT) {
            return Err(anyhow::anyhow!(
                "resolve_feedback works only in a session on the {FEEDBACK_PROJECT} project \
                 (this session's project: {}) — the queue is bot-hq's own",
                project.as_deref().unwrap_or("none")
            ));
        }
        let storage = self
            .storage
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("storage not configured"))?;
        let n = storage
            .resolve_feedback(id, &status, reason, &format!("{agent} \u{b7} {session_id}"))
            .await?;
        if n == 0 {
            return Err(anyhow::anyhow!("no feedback #{id}"));
        }
        Ok(())
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

    /// `resolve_feedback` closes a row with its reason and who closed it — in a
    /// bot-hq session only, for done or dismissed with a reason, on a real id;
    /// any move from the panel clears the reason.
    #[tokio::test]
    async fn a_bot_hq_session_resolves_a_row_with_its_reason() {
        let (bridge, storage) = bridge_with_storage().await;
        let id = bridge.file_feedback("s1", "eyes", "issue", "gate unreadable", "…").await.unwrap();
        let err = bridge.resolve_feedback("s1", "hands", id, "done", "abc1234").await.unwrap_err();
        assert!(err.to_string().contains("only in a session on the bot-hq project"), "{err}");
        bridge.register_session("s1".into(), Some("stryka".into())).await;
        let err = bridge.resolve_feedback("s1", "hands", id, "done", "abc1234").await.unwrap_err();
        assert!(err.to_string().contains("this session's project: stryka"), "{err}");

        bridge.register_session("s1".into(), Some(FEEDBACK_PROJECT.into())).await;
        for (status, reason, needle) in [
            ("open", "reopened", "unknown status"),
            ("fixed", "abc1234", "unknown status"),
            ("done", "   ", "a reason is required"),
        ] {
            let err = bridge.resolve_feedback("s1", "hands", id, status, reason).await.unwrap_err();
            assert!(err.to_string().contains(needle), "{status}/{reason:?}: {err}");
        }
        let err = bridge.resolve_feedback("s1", "hands", id + 99, "done", "abc1234").await.unwrap_err();
        assert!(err.to_string().contains(&format!("no feedback #{}", id + 99)), "{err}");
        assert_eq!(storage.list_feedback(Some("open")).await.unwrap().len(), 1, "nothing moved yet");

        bridge
            .resolve_feedback("s1", "hands", id, " Dismissed ", " declined by the user, 2026-10-12 ")
            .await
            .unwrap();
        let row = &storage.list_feedback(None).await.unwrap()[0];
        assert_eq!(row.status, "dismissed");
        assert_eq!(row.resolution.as_deref(), Some("declined by the user, 2026-10-12"));
        assert_eq!(row.resolved_by.as_deref(), Some("hands \u{b7} s1"));

        // Any move from the panel clears the reason (EYES a226929f): closed by
        // the tool as done, then dismissed by the user, must not read
        // "Dismissed: shipped in …".
        bridge.resolve_feedback("s1", "hands", id, "done", "shipped in abc1234").await.unwrap();
        storage.set_feedback_status(id, "dismissed").await.unwrap();
        let row = &storage.list_feedback(None).await.unwrap()[0];
        assert_eq!((row.status.as_str(), row.resolution.as_deref(), row.resolved_by.as_deref()), ("dismissed", None, None));
        bridge.resolve_feedback("s1", "hands", id, "done", "shipped in abc1234").await.unwrap();
        storage.set_feedback_status(id, "open").await.unwrap();
        let row = &storage.list_feedback(None).await.unwrap()[0];
        assert_eq!((row.status.as_str(), row.resolution.as_deref(), row.resolved_by.as_deref()), ("open", None, None));
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
