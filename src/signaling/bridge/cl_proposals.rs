//! Bridge methods for project-scoped Context Library proposals.
//! Proposal creation is non-mutating: agents file suggestions, and host approval
//! performs any eventual write-back.

use super::*;
use crate::storage::{ClProposal, ClProposalStatus};
use anyhow::Context;
use uuid::Uuid;

/// Outcome of filing a proposal: the new uid plus how many OTHER open
/// proposals already target the same file — competing suggestions the user
/// reviews together in the queue.
#[derive(Debug, Clone)]
pub struct ClProposeOutcome {
    pub uid: String,
    pub open_siblings: u32,
}

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
    ) -> Result<ClProposeOutcome> {
        validate_proposal_shape(
            &project,
            &file_path,
            &kind,
            target_excerpt.as_deref(),
            &proposed_body,
            &evidence,
        )?;
        let storage = self.cl_proposals_storage().await?;
        // Ground the proposal against the live CL BEFORE inserting: reject
        // shapes approval could never apply (add on an existing file,
        // correct/delete on a missing one) while the AGENT can still fix them,
        // and snapshot the base content hash so approval can flag drift.
        // Best-effort: without a resolvable project root (no data_dir, or the
        // CL dir doesn't exist yet) filing proceeds unvalidated and approval
        // stays the hard gate.
        let base_hash = match self.cl_project_root(&project).await {
            Some(root) => validate_against_cl(&root, &file_path, &kind).await?,
            None => None,
        };
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
                base_hash.as_deref(),
            )
            .await?;
        let open_siblings = storage
            .count_open_cl_proposals_for_file(&project, &file_path, &uid)
            .await?
            .try_into()
            .unwrap_or(0);
        // Proposing engages the CL — lift the close-out nudge gate so an agent
        // that files its learnings delta as a proposal isn't re-nudged at close.
        self.mark_cl_rescan(&session_id).await;
        // Filing is a DB-only write the CL fs-watcher can't see — tell the UI
        // so Context Manager badges update the moment a proposal lands.
        let _ = self
            .event_tx
            .send(SignalingEvent::ClProposalsChanged { project_id: project });
        Ok(ClProposeOutcome { uid, open_siblings })
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

    /// Approve an open proposal, writing it back to the CL. `force` is the
    /// explicit user override for a DETECTED conflict: replace an existing
    /// file (add), create a missing one (correct), or proceed past base-hash
    /// drift. Without `force`, a conflicted proposal stays open and the error
    /// names the resolution path — the queue never dead-ends.
    pub async fn approve_cl_proposal(&self, proposal_uid: String, force: bool) -> Result<String> {
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
            "add" | "correct" | "delete" => {}
            other => anyhow::bail!("unknown proposal kind '{other}'"),
        }
        let project_root = self
            .cl_project_root(&proposal.project_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("bridge data_dir is not configured"))?;
        let conflict = detect_conflict(
            &project_root,
            &proposal.file_path,
            &proposal.kind,
            proposal.base_hash.as_deref(),
        )
        .await?;
        // delete + target already gone: the intent is satisfied — resolve
        // without touching disk (no force needed).
        if proposal.kind == "delete" && conflict == Some(ClProposalConflict::Missing) {
            let Some(_claimed) = storage
                .resolve_cl_proposal(&proposal_uid, ClProposalStatus::Approved.as_str())
                .await?
            else {
                return Ok(format!("no-op: proposal '{proposal_uid}' is no longer open"));
            };
            let _ = self.event_tx.send(SignalingEvent::ClProposalsChanged {
                project_id: proposal.project_id.clone(),
            });
            return Ok(format!(
                "proposal '{proposal_uid}' approved — '{}' was already absent",
                proposal.file_path
            ));
        }
        if let Some(conflict) = &conflict {
            if !force {
                anyhow::bail!("{}", conflict.blocking_message(&proposal.file_path));
            }
        }
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
        let write_res = match claimed.kind.as_str() {
            // add: creating is the point; force additionally allows replacing
            // a file that appeared since filing.
            "add" => write_cl_file(&project_root, &claimed.file_path, &claimed.proposed_body, force, true).await,
            // correct: replacing is the point; force additionally allows
            // creating a file that vanished since filing.
            "correct" => write_cl_file(&project_root, &claimed.file_path, &claimed.proposed_body, true, force).await,
            // delete — the only other kind the validation above lets through.
            _ => delete_cl_file(&project_root, &claimed.file_path).await,
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

/// Filing-time ground check for `cl_propose`. Returns the sha256 base hash
/// for correct/delete when the target exists; None for add (no base file) or
/// when the CL root itself isn't on disk yet (validation degrades to
/// approval-time, which stays the hard gate).
async fn validate_against_cl(
    project_root: &Path,
    file_path: &str,
    kind: &str,
) -> Result<Option<String>> {
    let root = project_root.to_path_buf();
    let file_path = file_path.to_string();
    let kind = kind.to_string();
    tokio::task::spawn_blocking(move || {
        let Ok(root_real) = root.canonicalize() else {
            return Ok(None);
        };
        match root_real.join(&file_path).canonicalize() {
            Ok(existing) => {
                if !existing.starts_with(&root_real) {
                    anyhow::bail!("path traversal rejected — resolves outside project root");
                }
                if kind == "add" {
                    anyhow::bail!(
                        "'{file_path}' already exists in the CL — read the current file and \
                         file kind='correct' with the full replacement body instead"
                    );
                }
                if !existing.is_file() {
                    anyhow::bail!("'{file_path}' is not a regular file");
                }
                let bytes = std::fs::read(&existing)
                    .with_context(|| format!("reading '{file_path}' for the base snapshot"))?;
                Ok(Some(sha256_hex(&bytes)))
            }
            Err(_) => match kind.as_str() {
                "correct" => anyhow::bail!(
                    "'{file_path}' not found in the CL — use kind='add' to create a new file"
                ),
                "delete" => {
                    anyhow::bail!("'{file_path}' not found in the CL — nothing to delete")
                }
                _ => Ok(None),
            },
        }
    })
    .await
    .context("proposal validation task panicked")?
}

/// Lowercase-hex sha256 — the `base_hash` format stored on proposals and
/// recomputed at approval for drift detection.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// A detected divergence between what a proposal assumed about its target
/// file and the live CL. Computed at approval (and by the review-queue view)
/// so the user resolves it explicitly instead of hitting a dead-end write
/// error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClProposalConflict {
    /// `add` whose target now exists — approving would clobber content the
    /// proposer never read.
    Exists,
    /// `correct`/`delete` whose target is gone.
    Missing,
    /// `correct`/`delete` whose target changed since filing (base-hash drift)
    /// — approving would silently discard whatever changed it.
    StaleBase,
}

impl ClProposalConflict {
    /// Stable wire tag consumed by the review-queue UI.
    pub fn as_str(&self) -> &'static str {
        match self {
            ClProposalConflict::Exists => "exists",
            ClProposalConflict::Missing => "missing",
            ClProposalConflict::StaleBase => "stale_base",
        }
    }

    /// User-actionable refusal for a non-forced approval of this conflict.
    fn blocking_message(&self, file_path: &str) -> String {
        match self {
            ClProposalConflict::Exists => format!(
                "'{file_path}' already exists — review the current file, then approve with \
                 force to replace it, or reject"
            ),
            ClProposalConflict::Missing => format!(
                "'{file_path}' no longer exists — approve with force to create it with the \
                 proposed body, or reject"
            ),
            ClProposalConflict::StaleBase => format!(
                "'{file_path}' changed since this proposal was filed — review the current \
                 content, then approve with force to proceed, or reject"
            ),
        }
    }
}

/// Compare a proposal's assumptions against the live CL. `Ok(None)` = no
/// conflict. Shared by approval and the review-queue listing so both report
/// the same state.
pub(crate) async fn detect_conflict(
    project_root: &Path,
    file_path: &str,
    kind: &str,
    base_hash: Option<&str>,
) -> Result<Option<ClProposalConflict>> {
    let root = project_root.to_path_buf();
    let file_path = file_path.to_string();
    let kind = kind.to_string();
    let base_hash = base_hash.map(str::to_string);
    tokio::task::spawn_blocking(move || {
        let root_real = root
            .canonicalize()
            .with_context(|| format!("canonicalizing CL project root {}", root.display()))?;
        match root_real.join(&file_path).canonicalize() {
            Ok(existing) => {
                if !existing.starts_with(&root_real) {
                    anyhow::bail!("path traversal rejected — resolves outside project root");
                }
                if kind == "add" {
                    return Ok(Some(ClProposalConflict::Exists));
                }
                if !existing.is_file() {
                    anyhow::bail!("'{file_path}' is not a regular file");
                }
                // NULL base_hash (add proposals, pre-0033 rows) = no drift
                // detection possible; only a stored snapshot can prove drift.
                match base_hash {
                    Some(base) => {
                        let bytes = std::fs::read(&existing)
                            .with_context(|| format!("reading '{file_path}' for drift check"))?;
                        if sha256_hex(&bytes) != base {
                            Ok(Some(ClProposalConflict::StaleBase))
                        } else {
                            Ok(None)
                        }
                    }
                    None => Ok(None),
                }
            }
            Err(_) => {
                if kind == "add" {
                    Ok(None)
                } else {
                    Ok(Some(ClProposalConflict::Missing))
                }
            }
        }
    })
    .await
    .context("proposal conflict check task panicked")?
}

/// Write a proposal body into the CL. `allow_existing` permits replacing a
/// file that's already there (correct's normal path; add only under force);
/// `allow_create` permits creating one that isn't (add's normal path; correct
/// only under force). Creation mkdir-p's missing parent folders inside the
/// root, so proposals can target new subfolders.
async fn write_cl_file(
    project_root: &Path,
    file_path: &str,
    content: &str,
    allow_existing: bool,
    allow_create: bool,
) -> Result<()> {
    let root = project_root.to_path_buf();
    let file_path = file_path.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let root_real = root
            .canonicalize()
            .with_context(|| format!("canonicalizing CL project root {}", root.display()))?;
        if root_real.join(&file_path).exists() {
            if !allow_existing {
                anyhow::bail!("'{file_path}' already exists");
            }
            let existing = resolve_existing_file(&root_real, &file_path)?;
            atomic_write(&existing, &content)
        } else {
            if !allow_create {
                anyhow::bail!("file '{file_path}' not found");
            }
            let target = resolve_new_path(&root_real, &file_path)?;
            atomic_write(&target, &content)
        }
    })
    .await
    .context("proposal write task panicked")?
}

async fn delete_cl_file(project_root: &Path, file_path: &str) -> Result<()> {
    let root = project_root.to_path_buf();
    let file_path = file_path.to_string();
    tokio::task::spawn_blocking(move || {
        let root_real = root
            .canonicalize()
            .with_context(|| format!("canonicalizing CL project root {}", root.display()))?;
        let existing = resolve_existing_file(&root_real, &file_path)?;
        std::fs::remove_file(&existing)
            .with_context(|| format!("deleting '{file_path}'"))
    })
    .await
    .context("proposal delete task panicked")?
}

/// Resolve a not-yet-existing target for creation, mkdir-p'ing missing parent
/// folders. Traversal is guarded against the deepest EXISTING ancestor before
/// anything is created, and re-checked on the final parent after creation (in
/// case an intermediate symlink pointed outside the root).
fn resolve_new_path(project_root_real: &Path, rel_path: &str) -> Result<PathBuf> {
    let joined = project_root_real.join(rel_path);
    let parent = joined
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid path: no parent"))?;
    let file_name = joined
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid path: no final segment"))?;
    let mut probe = parent.to_path_buf();
    while !probe.exists() {
        probe = probe
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid path: no existing ancestor"))?
            .to_path_buf();
    }
    let probe_real = probe
        .canonicalize()
        .with_context(|| format!("resolving existing ancestor of {rel_path}"))?;
    if !probe_real.starts_with(project_root_real) {
        anyhow::bail!("path traversal rejected — resolves outside project root");
    }
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating parent folders for {rel_path}"))?;
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

        let outcome = bridge
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
        assert_eq!(outcome.open_siblings, 0);

        let proposal = storage.get_cl_proposal(&outcome.uid).await.unwrap().unwrap();
        assert_eq!(proposal.project_id, "bot-hq");
        assert_eq!(proposal.file_path, "notes.md");
        assert_eq!(proposal.kind, "correct");
        assert_eq!(proposal.target_excerpt.as_deref(), Some("old"));
        assert_eq!(proposal.proposed_body, "complete corrected body");
        assert_eq!(proposal.evidence, "stale wording");
        assert_eq!(proposal.status, "open");
        assert_eq!(proposal.proposed_by, "rain");
        assert_eq!(proposal.session_id.as_deref(), Some("s1"));
        // bridge_with_storage has no data_dir → filing skips fs validation and
        // stores no base snapshot.
        assert_eq!(proposal.base_hash, None);
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
                "p1", "bot-hq", "notes.md", "add", None, "body", "evidence", "brian", Some("s1"), None,
            )
            .await
            .unwrap();
        storage
            .create_cl_proposal(
                "p2", "other", "notes.md", "add", None, "body", "evidence", "rain", Some("s1"), None,
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
                "p-add", "bot-hq", "notes.md", "add", None, "new body", "new file", "rain", Some("s1"), None,
            )
            .await
            .unwrap();

        let result = bridge.approve_cl_proposal("p-add".to_string(), false).await.unwrap();
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
                Some("s1"), None,
            )
            .await
            .unwrap();

        let err = bridge.approve_cl_proposal("p-add-existing".to_string(), false).await.unwrap_err();
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
                Some("s1"), None,
            )
            .await
            .unwrap();

        let result = bridge.approve_cl_proposal("p-correct".to_string(), false).await.unwrap();
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
                "p-reject", "bot-hq", "notes.md", "add", None, "new body", "not needed", "rain", Some("s1"), None,
            )
            .await
            .unwrap();

        let result = bridge.reject_cl_proposal("p-reject".to_string()).await.unwrap();
        assert!(result.contains("rejected"));
        assert!(!tmp.path().join("library/projects/bot-hq/notes.md").exists());
        assert_eq!(storage.get_cl_proposal("p-reject").await.unwrap().unwrap().status, "rejected");
    }

    #[tokio::test]
    async fn approve_delete_removes_existing_file() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "obsolete content").unwrap();
        let base = sha256_hex(b"obsolete content");
        storage
            .create_cl_proposal(
                "p-delete", "bot-hq", "notes.md", "delete", None, "", "obsolete", "rain", Some("s1"),
                Some(&base),
            )
            .await
            .unwrap();

        let result = bridge.approve_cl_proposal("p-delete".to_string(), false).await.unwrap();
        assert!(result.contains("approved"));
        assert!(!path.exists());
        assert_eq!(storage.get_cl_proposal("p-delete").await.unwrap().unwrap().status, "approved");
    }

    #[tokio::test]
    async fn approve_delete_on_missing_file_resolves_as_satisfied() {
        let (bridge, storage, _tmp) = bridge_with_data_dir().await;
        storage
            .create_cl_proposal(
                "p-delete-gone", "bot-hq", "ghost.md", "delete", None, "", "obsolete", "rain",
                Some("s1"), None,
            )
            .await
            .unwrap();

        let result = bridge.approve_cl_proposal("p-delete-gone".to_string(), false).await.unwrap();
        assert!(result.contains("already absent"), "got: {result}");
        assert_eq!(
            storage.get_cl_proposal("p-delete-gone").await.unwrap().unwrap().status,
            "approved"
        );
    }

    #[tokio::test]
    async fn approve_delete_with_stale_base_requires_force() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "changed after filing").unwrap();
        let base = sha256_hex(b"content at filing time");
        storage
            .create_cl_proposal(
                "p-delete-stale", "bot-hq", "notes.md", "delete", None, "", "obsolete", "rain",
                Some("s1"), Some(&base),
            )
            .await
            .unwrap();

        let err = bridge
            .approve_cl_proposal("p-delete-stale".to_string(), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("changed since"));
        assert!(path.exists());
        assert_eq!(
            storage.get_cl_proposal("p-delete-stale").await.unwrap().unwrap().status,
            "open"
        );

        let result = bridge.approve_cl_proposal("p-delete-stale".to_string(), true).await.unwrap();
        assert!(result.contains("approved"));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn approve_add_with_force_replaces_existing_file() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "already here").unwrap();
        storage
            .create_cl_proposal(
                "p-add-force", "bot-hq", "notes.md", "add", None, "new body", "new file", "rain",
                Some("s1"), None,
            )
            .await
            .unwrap();

        let result = bridge.approve_cl_proposal("p-add-force".to_string(), true).await.unwrap();
        assert!(result.contains("approved"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new body");
        assert_eq!(
            storage.get_cl_proposal("p-add-force").await.unwrap().unwrap().status,
            "approved"
        );
    }

    #[tokio::test]
    async fn approve_add_creates_missing_parent_folders() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        storage
            .create_cl_proposal(
                "p-add-nested", "bot-hq", "plans/2026/handoff.md", "add", None, "nested body",
                "new plan file", "brian", Some("s1"), None,
            )
            .await
            .unwrap();

        let result = bridge.approve_cl_proposal("p-add-nested".to_string(), false).await.unwrap();
        assert!(result.contains("approved"));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("library/projects/bot-hq/plans/2026/handoff.md"))
                .unwrap(),
            "nested body"
        );
    }

    #[tokio::test]
    async fn approve_correct_on_missing_file_requires_force_then_creates() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        storage
            .create_cl_proposal(
                "p-correct-gone", "bot-hq", "ghost.md", "correct", None, "revived body",
                "file vanished", "rain", Some("s1"), None,
            )
            .await
            .unwrap();

        let err = bridge
            .approve_cl_proposal("p-correct-gone".to_string(), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no longer exists"));
        assert_eq!(
            storage.get_cl_proposal("p-correct-gone").await.unwrap().unwrap().status,
            "open"
        );

        let result = bridge.approve_cl_proposal("p-correct-gone".to_string(), true).await.unwrap();
        assert!(result.contains("approved"));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("library/projects/bot-hq/ghost.md")).unwrap(),
            "revived body"
        );
    }

    #[tokio::test]
    async fn approve_correct_with_stale_base_requires_force() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "session B approved first").unwrap();
        let base = sha256_hex(b"the original both sessions read");
        storage
            .create_cl_proposal(
                "p-correct-stale", "bot-hq", "notes.md", "correct", None, "session A full body",
                "A's delta", "brian", Some("s1"), Some(&base),
            )
            .await
            .unwrap();

        // Without force: blocked, file untouched, proposal still open — the
        // silent-clobber path is closed.
        let err = bridge
            .approve_cl_proposal("p-correct-stale".to_string(), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("changed since"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "session B approved first");
        assert_eq!(
            storage.get_cl_proposal("p-correct-stale").await.unwrap().unwrap().status,
            "open"
        );

        // With force: the user reviewed and explicitly proceeded.
        let result = bridge.approve_cl_proposal("p-correct-stale".to_string(), true).await.unwrap();
        assert!(result.contains("approved"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "session A full body");
    }

    #[tokio::test]
    async fn approve_correct_with_matching_base_needs_no_force() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "unchanged since filing").unwrap();
        let base = sha256_hex(b"unchanged since filing");
        storage
            .create_cl_proposal(
                "p-correct-clean", "bot-hq", "notes.md", "correct", None, "updated full body",
                "routine update", "rain", Some("s1"), Some(&base),
            )
            .await
            .unwrap();

        let result = bridge.approve_cl_proposal("p-correct-clean".to_string(), false).await.unwrap();
        assert!(result.contains("approved"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "updated full body");
    }

    #[tokio::test]
    async fn cl_propose_rejects_add_on_existing_file_at_filing() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        std::fs::write(tmp.path().join("library/projects/bot-hq/notes.md"), "already here")
            .unwrap();

        let err = bridge
            .cl_propose(
                "s1".to_string(),
                "brian".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "add".to_string(),
                None,
                "new body".to_string(),
                "evidence".to_string(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert!(err.to_string().contains("kind='correct'"));
        // Rejected at filing — nothing inserted for the user to wade through.
        assert!(storage.list_cl_proposals("bot-hq", Some("open")).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cl_propose_rejects_correct_and_delete_on_missing_file_at_filing() {
        let (bridge, storage, _tmp) = bridge_with_data_dir().await;

        let err = bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "ghost.md".to_string(),
                "correct".to_string(),
                None,
                "body".to_string(),
                "evidence".to_string(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
        assert!(err.to_string().contains("kind='add'"));

        let err = bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "ghost.md".to_string(),
                "delete".to_string(),
                None,
                String::new(),
                "obsolete".to_string(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nothing to delete"));
        assert!(storage.list_cl_proposals("bot-hq", Some("open")).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cl_propose_snapshots_base_hash_for_correct_but_not_add() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        std::fs::write(tmp.path().join("library/projects/bot-hq/notes.md"), "old body").unwrap();

        let outcome = bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "correct".to_string(),
                None,
                "new full body".to_string(),
                "stale".to_string(),
            )
            .await
            .unwrap();
        let row = storage.get_cl_proposal(&outcome.uid).await.unwrap().unwrap();
        assert_eq!(row.base_hash.as_deref(), Some(sha256_hex(b"old body").as_str()));

        let outcome = bridge
            .cl_propose(
                "s1".to_string(),
                "rain".to_string(),
                "bot-hq".to_string(),
                "fresh.md".to_string(),
                "add".to_string(),
                None,
                "body".to_string(),
                "new file".to_string(),
            )
            .await
            .unwrap();
        let row = storage.get_cl_proposal(&outcome.uid).await.unwrap().unwrap();
        assert_eq!(row.base_hash, None);
    }

    #[tokio::test]
    async fn cl_propose_counts_open_siblings_on_same_file() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        std::fs::write(tmp.path().join("library/projects/bot-hq/notes.md"), "current").unwrap();
        storage
            .create_cl_proposal(
                "p-competing",
                "bot-hq",
                "notes.md",
                "correct",
                None,
                "competing body",
                "their delta",
                "rain",
                Some("s1"),
                None,
            )
            .await
            .unwrap();

        let outcome = bridge
            .cl_propose(
                "s1".to_string(),
                "brian".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "correct".to_string(),
                None,
                "my body".to_string(),
                "my delta".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(outcome.open_siblings, 1);
    }
}
