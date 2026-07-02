//! Bridge methods for project-scoped Context Library proposals.
//! Proposal creation is non-mutating: agents file suggestions, and host approval
//! performs any eventual write-back.

use super::*;
use crate::storage::{ClProposal, ClProposalStatus};
use anyhow::Context;
use uuid::Uuid;

impl SignalingBridge {
    async fn cl_proposals_storage(&self) -> Result<Storage> {
        self.storage
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("cl_proposals: storage is not wired into this bridge"))
    }

    pub async fn cl_propose(
        &self,
        session_id: String,
        agent: String,
        project: String,
        file_path: String,
        kind: String,
        target_excerpt: Option<String>,
        proposed_body: String,
        evidence: String,
    ) -> Result<String> {
        validate_proposal_shape(
            &project,
            &file_path,
            &kind,
            target_excerpt.as_deref(),
            &proposed_body,
            &evidence,
        )?;
        let storage = self.cl_proposals_storage().await?;
        let uid = Uuid::new_v4().to_string();
        storage
            .create_cl_proposal(
                &uid,
                &project,
                &file_path,
                &kind,
                target_excerpt.as_deref(),
                &proposed_body,
                &evidence,
                &agent,
                Some(&session_id),
            )
            .await?;
        // Proposing engages the CL — lift the close-out nudge gate so an agent
        // that files its learnings delta as a proposal isn't re-nudged at close.
        self.mark_cl_rescan(&session_id).await;
        // Filing is a DB-only write the CL fs-watcher can't see — tell the UI
        // so Context Manager badges update the moment a proposal lands.
        let _ = self
            .event_tx
            .send(SignalingEvent::ClProposalsChanged { project_id: project });
        Ok(uid)
    }

    pub async fn cl_list_proposals(
        &self,
        project: String,
        status: Option<String>,
    ) -> Result<Vec<ClProposal>> {
        let status = match status {
            Some(s) => Some(
                ClProposalStatus::parse(&s)
                    .ok_or_else(|| anyhow::anyhow!("status must be 'open', 'approved', or 'rejected'"))?
                    .as_str()
                    .to_string(),
            ),
            None => None,
        };
        let storage = self.cl_proposals_storage().await?;
        storage.list_cl_proposals(&project, status.as_deref()).await
    }

    pub async fn approve_cl_proposal(&self, proposal_uid: String) -> Result<String> {
        let storage = self.cl_proposals_storage().await?;
        let proposal = storage
            .get_cl_proposal(&proposal_uid)
            .await?
            .ok_or_else(|| anyhow::anyhow!("proposal '{proposal_uid}' not found"))?;
        if proposal.status != "open" {
            return Ok(format!("no-op: proposal '{proposal_uid}' is already {}", proposal.status));
        }
        // Validate the kind BEFORE any mutation so an unsupported kind errors
        // with the row still open (and never claims the proposal below).
        match proposal.kind.as_str() {
            "add" | "correct" => {}
            "delete" => anyhow::bail!("delete proposal approval is not supported in the MVP"),
            other => anyhow::bail!("unknown proposal kind '{other}'"),
        }
        let project_root = self
            .cl_project_root(&proposal.project_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("bridge data_dir is not configured"))?;
        // Claim the proposal FIRST (open→approved CAS), then write. Writing
        // before the claim let an approve/reject race replace the file and then
        // report "no-op" — the disk mutation had already happened. Losing the
        // CAS therefore guarantees no write occurred.
        let Some(claimed) = storage
            .resolve_cl_proposal(&proposal_uid, ClProposalStatus::Approved.as_str())
            .await?
        else {
            return Ok(format!("no-op: proposal '{proposal_uid}' is no longer open"));
        };
        let write_res = if claimed.kind == "add" {
            write_new_cl_file(&project_root, &claimed.file_path, &claimed.proposed_body).await
        } else {
            // "correct" — the only other kind the validation above lets through.
            replace_existing_cl_file(&project_root, &claimed.file_path, &claimed.proposed_body).await
        };
        if let Err(err) = write_res {
            // Compensate: flip the claim back to open so the queue matches the
            // disk (nothing was written). Best-effort — reopen only touches
            // `approved` rows, so it can't resurrect a concurrent rejection.
            if let Err(revert_err) = storage.reopen_cl_proposal(&proposal_uid).await {
                tracing::warn!(
                    %revert_err,
                    proposal_uid,
                    "failed to reopen CL proposal after write-back failure"
                );
            }
            return Err(err);
        }
        if let Err(err) = self.cl_rescan(&proposal.project_id).await {
            tracing::warn!(
                %err,
                proposal_uid,
                project = %proposal.project_id,
                "cl_rescan failed after CL proposal approval; index may be stale"
            );
        }
        let _ = self.event_tx.send(SignalingEvent::ClProposalsChanged {
            project_id: proposal.project_id.clone(),
        });
        Ok(format!("proposal '{proposal_uid}' approved"))
    }

    pub async fn reject_cl_proposal(&self, proposal_uid: String) -> Result<String> {
        let storage = self.cl_proposals_storage().await?;
        let resolved = storage
            .resolve_cl_proposal(&proposal_uid, ClProposalStatus::Rejected.as_str())
            .await?;
        let Some(resolved) = resolved else {
            return Ok(format!("no-op: proposal '{proposal_uid}' is not open (unknown id, or already resolved)"));
        };
        // Rejection is DB-only (no file write, no fs-watcher event) — emit so
        // the badge count drops immediately.
        let _ = self.event_tx.send(SignalingEvent::ClProposalsChanged {
            project_id: resolved.project_id,
        });
        Ok(format!("proposal '{proposal_uid}' rejected"))
    }
}

async fn write_new_cl_file(project_root: &Path, file_path: &str, content: &str) -> Result<()> {
    let root = project_root.to_path_buf();
    let file_path = file_path.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let root_real = root
            .canonicalize()
            .with_context(|| format!("canonicalizing CL project root {}", root.display()))?;
        let candidate = resolve_new_path(&root_real, &file_path)?;
        atomic_write(&candidate, &content)
    })
    .await
    .context("proposal add write task panicked")?
}

async fn replace_existing_cl_file(project_root: &Path, file_path: &str, content: &str) -> Result<()> {
    let root = project_root.to_path_buf();
    let file_path = file_path.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let root_real = root
            .canonicalize()
            .with_context(|| format!("canonicalizing CL project root {}", root.display()))?;
        let candidate = resolve_existing_file(&root_real, &file_path)?;
        atomic_write(&candidate, &content)
    })
    .await
    .context("proposal correct write task panicked")?
}

fn resolve_new_path(project_root_real: &Path, rel_path: &str) -> Result<PathBuf> {
    let joined = project_root_real.join(rel_path);
    let parent = joined
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid path: no parent"))?;
    let file_name = joined
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid path: no final segment"))?;
    let parent_real = parent
        .canonicalize()
        .with_context(|| format!("parent directory not found for {rel_path}"))?;
    if !parent_real.starts_with(project_root_real) {
        anyhow::bail!("path traversal rejected — resolves outside project root");
    }
    let target = parent_real.join(file_name);
    if target.exists() {
        anyhow::bail!("'{rel_path}' already exists");
    }
    Ok(target)
}

fn resolve_existing_file(project_root_real: &Path, rel_path: &str) -> Result<PathBuf> {
    let candidate = project_root_real
        .join(rel_path)
        .canonicalize()
        .with_context(|| format!("file '{rel_path}' not found"))?;
    if !candidate.starts_with(project_root_real) {
        anyhow::bail!("path traversal rejected — file resolves outside project root");
    }
    let meta = std::fs::metadata(&candidate).context("reading CL target metadata")?;
    if !meta.is_file() {
        anyhow::bail!("not a regular file");
    }
    Ok(candidate)
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".bot-hq-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, content.as_bytes())
        .with_context(|| format!("writing temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming temp file into {}", path.display()))?;
    Ok(())
}

fn validate_proposal_shape(
    project: &str,
    file_path: &str,
    kind: &str,
    target_excerpt: Option<&str>,
    proposed_body: &str,
    evidence: &str,
) -> Result<()> {
    if project.trim().is_empty() {
        anyhow::bail!("project is required");
    }
    if file_path.trim().is_empty() || file_path.starts_with('/') || file_path.contains("..") {
        anyhow::bail!("file_path must be a relative CL path within the project");
    }
    if evidence.trim().is_empty() {
        anyhow::bail!("evidence is required");
    }
    match kind {
        "add" => {
            if proposed_body.trim().is_empty() {
                anyhow::bail!("proposed_body is required for add proposals");
            }
            if target_excerpt.is_some_and(|s| !s.trim().is_empty()) {
                anyhow::bail!("target_excerpt is not used for add proposals in the MVP");
            }
        }
        "correct" => {
            if proposed_body.trim().is_empty() {
                anyhow::bail!("proposed_body is required for correct proposals");
            }
        }
        "delete" => {}
        _ => anyhow::bail!("kind must be 'add', 'correct', or 'delete'"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use std::sync::Arc;

    async fn bridge_with_storage() -> (Arc<SignalingBridge>, Storage) {
        let bridge = SignalingBridge::new();
        let storage = Storage::memory().await.unwrap();
        storage
            .upsert_project("bot-hq", "bot-hq", None, None, None)
            .await
            .unwrap();
        storage.create_session("s1", "CL proposals", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;
        (bridge, storage)
    }

    #[tokio::test]
    async fn cl_propose_creates_project_scoped_open_proposal() {
        let (bridge, storage) = bridge_with_storage().await;

        let uid = bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "correct".to_string(),
                Some("old".to_string()),
                "complete corrected body".to_string(),
                "stale wording".to_string(),
            )
            .await
            .unwrap();

        let proposal = storage.get_cl_proposal(&uid).await.unwrap().unwrap();
        assert_eq!(proposal.project_id, "bot-hq");
        assert_eq!(proposal.file_path, "notes.md");
        assert_eq!(proposal.kind, "correct");
        assert_eq!(proposal.target_excerpt.as_deref(), Some("old"));
        assert_eq!(proposal.proposed_body, "complete corrected body");
        assert_eq!(proposal.evidence, "stale wording");
        assert_eq!(proposal.status, "open");
        assert_eq!(proposal.proposed_by, "rain");
        assert_eq!(proposal.session_id.as_deref(), Some("s1"));
    }

    #[tokio::test]
    async fn cl_propose_marks_close_gate_so_no_nudge() {
        let (bridge, _storage) = bridge_with_storage().await;

        // Control: a session that engaged the CL in no way is nudged on its
        // first close (adherence nudges are on by default).
        assert!(
            bridge.should_nudge_close("s2").await,
            "a session with no CL activity should be nudged"
        );

        // Filing a proposal counts as engaging the CL, so the close-out nudge
        // is lifted — the agent persisted its learnings via cl_propose.
        bridge
            .cl_propose(
                "s1".to_string(),
                "brian".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "add".to_string(),
                None,
                "a session learning".to_string(),
                "non-obvious discovery".to_string(),
            )
            .await
            .unwrap();
        assert!(
            !bridge.should_nudge_close("s1").await,
            "filing a cl_propose should mark the close gate and suppress the nudge"
        );
    }

    #[tokio::test]
    async fn cl_propose_emits_proposals_changed_event() {
        let (bridge, _storage) = bridge_with_storage().await;
        // Filing is DB-only (invisible to the CL fs-watcher) — the badge
        // freshness contract is this explicit broadcast.
        let mut rx = bridge.subscribe();
        bridge
            .cl_propose(
                "s1".to_string(),
                "brian".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "add".to_string(),
                None,
                "a learning".to_string(),
                "evidence".to_string(),
            )
            .await
            .unwrap();
        loop {
            match rx.try_recv() {
                Ok(SignalingEvent::ClProposalsChanged { project_id }) => {
                    assert_eq!(project_id, "bot-hq");
                    break;
                }
                Ok(_) => continue, // unrelated events are fine; keep draining
                Err(e) => panic!("expected ClProposalsChanged on the bus, got {e:?}"),
            }
        }
    }

    #[tokio::test]
    async fn cl_list_proposals_filters_by_project_and_status() {
        let (bridge, storage) = bridge_with_storage().await;
        storage.upsert_project("other", "other", None, None, None).await.unwrap();
        storage
            .create_cl_proposal(
                "p1", "bot-hq", "notes.md", "add", None, "body", "evidence", "brian", Some("s1"),
            )
            .await
            .unwrap();
        storage
            .create_cl_proposal(
                "p2", "other", "notes.md", "add", None, "body", "evidence", "rain", Some("s1"),
            )
            .await
            .unwrap();
        storage.resolve_cl_proposal("p1", "rejected").await.unwrap();

        assert!(bridge
            .cl_list_proposals("bot-hq".to_string(), Some("open".to_string()))
            .await
            .unwrap()
            .is_empty());
        let rejected = bridge
            .cl_list_proposals("bot-hq".to_string(), Some("rejected".to_string()))
            .await
            .unwrap();
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].proposal_uid, "p1");
        assert_eq!(
            bridge
                .cl_list_proposals("other".to_string(), Some("open".to_string()))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn cl_propose_rejects_invalid_mvp_shapes() {
        let (bridge, _) = bridge_with_storage().await;

        assert!(bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "append".to_string(),
                None,
                "body".to_string(),
                "evidence".to_string(),
            )
            .await
            .is_err());
        assert!(bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "add".to_string(),
                Some("not used".to_string()),
                "body".to_string(),
                "evidence".to_string(),
            )
            .await
            .is_err());
        assert!(bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "correct".to_string(),
                None,
                "   ".to_string(),
                "evidence".to_string(),
            )
            .await
            .is_err());
        assert!(bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "correct".to_string(),
                None,
                "body".to_string(),
                "   ".to_string(),
            )
            .await
            .is_err());
        assert!(bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "delete".to_string(),
                None,
                "".to_string(),
                "obsolete".to_string(),
            )
            .await
            .is_ok());
    }

    async fn bridge_with_data_dir() -> (Arc<SignalingBridge>, Storage, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("library/projects/bot-hq");
        std::fs::create_dir_all(&project_root).unwrap();
        let bridge = SignalingBridge::new_with(None, Some(tmp.path().to_path_buf()));
        let storage = Storage::memory().await.unwrap();
        storage
            .upsert_project("bot-hq", "bot-hq", None, None, None)
            .await
            .unwrap();
        storage.create_session("s1", "CL proposals", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;
        (bridge, storage, tmp)
    }

    #[tokio::test]
    async fn approve_add_creates_new_file_and_rescans() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        storage
            .create_cl_proposal(
                "p-add", "bot-hq", "notes.md", "add", None, "new body", "new file", "rain", Some("s1"),
            )
            .await
            .unwrap();

        let result = bridge.approve_cl_proposal("p-add".to_string()).await.unwrap();
        assert!(result.contains("approved"));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("library/projects/bot-hq/notes.md")).unwrap(),
            "new body"
        );
        assert_eq!(storage.get_cl_proposal("p-add").await.unwrap().unwrap().status, "approved");
        assert!(storage.get_cl_index("bot-hq", "notes.md").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn approve_add_rejects_existing_file_without_resolving() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "already here").unwrap();
        storage
            .create_cl_proposal(
                "p-add-existing",
                "bot-hq",
                "notes.md",
                "add",
                None,
                "new body",
                "new file",
                "rain",
                Some("s1"),
            )
            .await
            .unwrap();

        let err = bridge.approve_cl_proposal("p-add-existing".to_string()).await.unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "already here");
        assert_eq!(storage.get_cl_proposal("p-add-existing").await.unwrap().unwrap().status, "open");
    }

    #[tokio::test]
    async fn approve_correct_replaces_existing_file_and_rescans() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "old body").unwrap();
        bridge.cl_rescan("bot-hq").await.unwrap();
        storage
            .create_cl_proposal(
                "p-correct",
                "bot-hq",
                "notes.md",
                "correct",
                Some("old body"),
                "complete corrected body",
                "fix stale content",
                "rain",
                Some("s1"),
            )
            .await
            .unwrap();

        let result = bridge.approve_cl_proposal("p-correct".to_string()).await.unwrap();
        assert!(result.contains("approved"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "complete corrected body");
        assert_eq!(storage.get_cl_proposal("p-correct").await.unwrap().unwrap().status, "approved");
        let indexed = storage.get_cl_index("bot-hq", "notes.md").await.unwrap().unwrap();
        assert!(indexed.description.contains("complete corrected body"));
    }

    #[tokio::test]
    async fn reject_proposal_marks_rejected_without_mutation() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        storage
            .create_cl_proposal(
                "p-reject", "bot-hq", "notes.md", "add", None, "new body", "not needed", "rain", Some("s1"),
            )
            .await
            .unwrap();

        let result = bridge.reject_cl_proposal("p-reject".to_string()).await.unwrap();
        assert!(result.contains("rejected"));
        assert!(!tmp.path().join("library/projects/bot-hq/notes.md").exists());
        assert_eq!(storage.get_cl_proposal("p-reject").await.unwrap().unwrap().status, "rejected");
    }

    #[tokio::test]
    async fn approve_delete_is_unsupported_in_mvp() {
        let (bridge, storage, _) = bridge_with_data_dir().await;
        storage
            .create_cl_proposal(
                "p-delete", "bot-hq", "notes.md", "delete", None, "delete notes", "obsolete", "rain", Some("s1"),
            )
            .await
            .unwrap();

        let err = bridge.approve_cl_proposal("p-delete".to_string()).await.unwrap_err();
        assert!(err.to_string().contains("delete proposal approval is not supported"));
        assert_eq!(storage.get_cl_proposal("p-delete").await.unwrap().unwrap().status, "open");
    }
}
