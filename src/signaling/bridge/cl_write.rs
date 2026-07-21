//! Direct Context Library writes for agents. `cl_write_file` is the
//! single-call agent write path: a guarded create-or-replace inside the
//! project's CL root that rescans the index and lifts the close-out gate
//! itself, so no separate `cl_rescan` call is needed.

use super::*;
use crate::storage::Project;
use anyhow::Context;

/// Hard cap on a single CL write. The CL is study notes, not a data store —
/// anything larger than the UI editor's 1 MB read cap (tauri_cmd/cl.rs)
/// would be unreadable there anyway.
const MAX_WRITE_BYTES: usize = 1_048_576; // 1 MiB

impl SignalingBridge {
    /// Create or replace `file_path` under `project`'s CL root with `content`.
    /// Missing parent folders are created; the write is atomic (tmp+rename in
    /// the same directory). On success the project index is rescanned
    /// (warn-on-fail — the write itself already landed) and the session's
    /// close-out gate is marked, so persisting a learnings delta through this
    /// tool suppresses the close nudge exactly like `cl_rescan` does.
    pub async fn cl_write_file(
        &self,
        session_id: String,
        project: String,
        file_path: String,
        content: String,
    ) -> Result<String> {
        if project.trim().is_empty() {
            anyhow::bail!("project is required");
        }
        if file_path.trim().is_empty() || file_path.starts_with('/') || file_path.contains("..") {
            anyhow::bail!("file_path must be a relative CL path within the project");
        }
        if content.len() > MAX_WRITE_BYTES {
            anyhow::bail!(
                "content is {} bytes — the CL write cap is 1 MiB. CL files are \
                 high-signal study notes; trim or split instead",
                content.len()
            );
        }
        let project_root = self
            .cl_project_root(&project)
            .await
            .ok_or_else(|| anyhow::anyhow!("bridge data_dir is not configured"))?;
        let fp = file_path.clone();
        let proj = project.clone();
        let replaced = tokio::task::spawn_blocking(move || -> Result<bool> {
            let root_real = project_root.canonicalize().with_context(|| {
                format!("canonicalizing CL project root {}", project_root.display())
            })?;
            let (target, replaced) = if root_real.join(&fp).exists() {
                (resolve_existing_file(&root_real, &fp)?, true)
            } else {
                (resolve_new_path(&root_real, &fp)?, false)
            };
            assert_not_protected_globals_write(&proj, &root_real, &target)?;
            atomic_write(&target, &content)?;
            Ok(replaced)
        })
        .await
        .context("cl_write_file task panicked")??;
        if let Err(err) = self.cl_rescan(&project).await {
            tracing::warn!(
                %err,
                project = %project,
                file_path,
                "cl_rescan failed after cl_write_file; index may be stale"
            );
        }
        // Writing a CL delta lifts the close-out nudge, same as cl_rescan.
        self.mark_cl_rescan(&session_id).await;
        Ok(format!(
            "{} '{file_path}' in project '{project}'",
            if replaced { "replaced" } else { "created" }
        ))
    }
}

/// bot-hq-owned `_globals` paths agents must not write: an agent rewriting
/// `custom-instructions.md` / `custom-general-rules.md` (or the legacy
/// `agents/` subtree) would be editing its own standing rules. The user edits
/// these in the Library UI; mirror of `assert_not_protected_globals_path`
/// (tauri_cmd/cl.rs), which guards the user-side rename/delete instead.
fn assert_not_protected_globals_write(
    project: &str,
    root_real: &Path,
    candidate: &Path,
) -> Result<()> {
    if project != Project::GLOBALS {
        return Ok(());
    }
    let agents = root_real.join("agents");
    if candidate == root_real.join("custom-general-rules.md")
        || candidate == root_real.join("custom-instructions.md")
        || candidate.starts_with(&agents)
    {
        anyhow::bail!(
            "protected bot-hq system file — agents may not rewrite their own \
             instructions; ask the user to edit it in the Context Library"
        );
    }
    Ok(())
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
    Ok(parent_real.join(file_name))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use std::sync::Arc;

    async fn bridge_with_data_dir() -> (Arc<SignalingBridge>, Storage, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("library/projects/bot-hq")).unwrap();
        let bridge = SignalingBridge::new_with(None, Some(tmp.path().to_path_buf()));
        let storage = Storage::memory().await.unwrap();
        storage
            .upsert_project("bot-hq", "bot-hq", None, None, None)
            .await
            .unwrap();
        storage.create_session("s1", "CL write", None).await.unwrap();
        bridge.set_storage(storage.clone()).await;
        (bridge, storage, tmp)
    }

    #[tokio::test]
    async fn cl_write_file_creates_nested_file_and_indexes_it() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;

        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "bot-hq".to_string(),
                "plans/2026/handoff.md".to_string(),
                "nested body".to_string(),
            )
            .await
            .unwrap();
        assert!(msg.starts_with("created"), "got: {msg}");
        assert_eq!(
            std::fs::read_to_string(
                tmp.path().join("library/projects/bot-hq/plans/2026/handoff.md")
            )
            .unwrap(),
            "nested body"
        );
        // The follow-up rescan indexed it — no separate cl_rescan needed.
        assert!(storage
            .get_cl_index("bot-hq", "plans/2026/handoff.md")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn cl_write_file_replaces_existing_content() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "old body").unwrap();

        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "new full body".to_string(),
            )
            .await
            .unwrap();
        assert!(msg.starts_with("replaced"), "got: {msg}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new full body");
    }

    #[tokio::test]
    async fn cl_write_file_rejects_bad_shapes_and_traversal() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;

        for bad in ["/abs.md", "../escape.md", "a/../../b.md", "  "] {
            let err = bridge
                .cl_write_file(
                    "s1".to_string(),
                    "bot-hq".to_string(),
                    bad.to_string(),
                    "body".to_string(),
                )
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("relative CL path"),
                "path {bad:?} should be rejected, got: {err}"
            );
        }
        // Nothing escaped the root.
        assert!(!tmp.path().join("escape.md").exists());
        assert!(!tmp.path().join("b.md").exists());

        let err = bridge
            .cl_write_file(
                "s1".to_string(),
                "bot-hq".to_string(),
                "big.md".to_string(),
                "x".repeat(MAX_WRITE_BYTES + 1),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("1 MiB"), "got: {err}");
    }

    #[tokio::test]
    async fn cl_write_file_blocks_protected_globals_but_allows_loose_files() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        std::fs::write(tmp.path().join("library/custom-instructions.md"), "rules").unwrap();
        std::fs::create_dir_all(tmp.path().join("library/agents")).unwrap();

        // Existing protected file: refuse the rewrite.
        let err = bridge
            .cl_write_file(
                "s1".to_string(),
                "_globals".to_string(),
                "custom-instructions.md".to_string(),
                "agent-authored rules".to_string(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("protected"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("library/custom-instructions.md")).unwrap(),
            "rules"
        );

        // New file under agents/: refused too (legacy subtree is bot-hq-owned).
        let err = bridge
            .cl_write_file(
                "s1".to_string(),
                "_globals".to_string(),
                "agents/sneaky.md".to_string(),
                "x".to_string(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("protected"), "got: {err}");

        // Loose cross-project files stay writable (eod.md, tasks.md live here).
        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "_globals".to_string(),
                "eod.md".to_string(),
                "today: shipped cl_write_file".to_string(),
            )
            .await
            .unwrap();
        assert!(msg.starts_with("created"), "got: {msg}");
    }

    #[tokio::test]
    async fn cl_write_file_marks_close_gate_so_no_nudge() {
        let (bridge, _storage, _tmp) = bridge_with_data_dir().await;

        // Control: an untouched session is nudged on first close.
        assert!(bridge.should_nudge_close("s2").await);

        bridge
            .cl_write_file(
                "s1".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "a learning".to_string(),
            )
            .await
            .unwrap();
        assert!(
            !bridge.should_nudge_close("s1").await,
            "a cl_write_file should lift the close-out nudge"
        );
    }
}
