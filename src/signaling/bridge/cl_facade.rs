//! Context Library read surface for agents: index/folder search, read-audit,
//! folder-description writes, and the disk↔index reconciliation pass
//! (`cl_rescan`). Thin wrappers over storage plus the recursive disk walk.

use super::util::{split_into_atoms, walk_cl_dir, WalkedFile};
use super::*;
use crate::storage::{ClIndexEntry, Project};
use std::collections::HashSet;

impl SignalingBridge {
    // ---- Context Library (CL) index ------------------------------------

    /// Resolve the on-disk root for a project's CL files. `_globals` maps to
    /// the CL dir (`<data_dir>/library/`); named projects honor
    /// `projects.cl_path` when set, otherwise fall back to
    /// `<data_dir>/library/projects/<name>`. Returns None only when the bridge
    /// has no `data_dir` configured (test bridges built via `new()`).
    ///
    /// Clones the Storage handle out of the mutex before awaiting so callers
    /// holding the bridge mutex (e.g. `cl_rescan`) don't deadlock when this
    /// method tries to re-lock for its own lookup.
    pub(crate) async fn cl_project_root(&self, project: &str) -> Option<PathBuf> {
        let data_dir = self.data_dir.as_ref()?.clone();
        let paths = crate::paths::Paths::for_data_dir(data_dir.clone());
        if project == Project::GLOBALS {
            return Some(paths.cl_dir);
        }
        let storage = self.storage.lock().await.clone();
        match storage {
            Some(storage) => storage.cl_path_for_project(&data_dir, project).await.ok(),
            None => Some(paths.project_dir(project)),
        }
    }

    /// Read-side discovery for agents. Wraps storage.cl_index_search.
    pub async fn cl_index_search(
        &self,
        project: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ClIndexEntry>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(Vec::new());
        };
        storage.cl_index_search(project, query).await
    }

    /// Read-side discovery for FOLDER descriptions. Parallel to
    /// [`cl_index_search`]. Returns lightweight rows; empty list when storage
    /// isn't configured (test bridges).
    pub async fn cl_folder_search(
        &self,
        project: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<crate::storage::ClFolder>> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(Vec::new());
        };
        storage.cl_folder_search(project, query).await
    }

    /// Write-side for agents: upsert a folder description. The jsonrpc layer
    /// gates this to HANDS (brian); Rain is denied.
    pub async fn cl_register_folder_description(
        &self,
        project: &str,
        folder_path: &str,
        description: &str,
        tags: Option<&str>,
    ) -> Result<()> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(());
        };
        // Ensure the project row exists. _globals is bootstrapped by the
        // initial migration; named projects might not have an upsert yet.
        if project != Project::GLOBALS {
            storage
                .upsert_project(project, project, None, None, None)
                .await?;
        }
        storage
            .upsert_folder_description(project, folder_path, description, tags)
            .await?;
        Ok(())
    }

    /// Record an audit row: this agent (in this session) read this file.
    /// Looks up the cl_index row by (project, file_path); silently no-ops
    /// if the index doesn't know about the file yet.
    pub async fn cl_register_read(
        &self,
        agent: &str,
        session_id: Option<&str>,
        project: &str,
        file_path: &str,
    ) -> Result<()> {
        let storage_guard = self.storage.lock().await;
        let Some(storage) = storage_guard.as_ref() else {
            return Ok(());
        };
        let Some(entry) = storage.get_cl_index(project, file_path).await? else {
            tracing::debug!(
                project,
                file_path,
                "cl_register_read: no index row for path; skipping audit insert"
            );
            return Ok(());
        };
        storage.record_cl_read(entry.id, session_id, agent).await
    }

    /// Diff a project's on-disk CL directory against the index. Three outcomes:
    ///   - added:    file on disk, no index row → insert with auto-extracted description
    ///   - touched:  index row exists, mtime newer than stored updated_at → bump
    ///   - orphaned: index row exists, file gone → auto-purge the dangling row
    ///     (it points at a file the agent can no longer read) and report it
    pub async fn cl_rescan(&self, project: &str) -> Result<ClRescanReport> {
        let mut report = ClRescanReport::default();
        // Clone the Storage handle out of the bridge mutex BEFORE calling
        // cl_project_root, which also acquires the same mutex. tokio's Mutex
        // is not reentrant — holding the guard across that call deadlocks.
        let storage = match self.storage.lock().await.clone() {
            Some(s) => s,
            None => return Ok(report),
        };
        let Some(root) = self.cl_project_root(project).await else {
            return Ok(report);
        };
        if !root.is_dir() {
            return Ok(report);
        }

        // Walk disk; collect relative_path -> WalkedFile { mtime, snippet, body }.
        let mut on_disk: HashMap<String, WalkedFile> = HashMap::new();
        walk_cl_dir(&root, &root, project, &mut on_disk);

        // Existing rows.
        let existing = storage.cl_index_search(Some(project), None).await?;
        let existing_paths: HashSet<String> =
            existing.iter().map(|e| e.file_path.clone()).collect();

        // Adds + touches. Index existing rows by path for O(1) lookup — this was
        // an O(disk × index) linear scan (`existing.iter().find`) per on-disk file.
        let by_path: HashMap<&str, &_> =
            existing.iter().map(|e| (e.file_path.as_str(), e)).collect();
        for (rel, walked) in &on_disk {
            let WalkedFile { mtime, snippet, body } = walked;
            match by_path.get(rel.as_str()) {
                None => {
                    storage.upsert_cl_index(project, rel, snippet, None).await?;
                    storage
                        .replace_atoms_for_file(project, rel, &split_into_atoms(body), mtime)
                        .await?;
                    report.added.push(rel.clone());
                }
                Some(row) if row.updated_at.as_str() < mtime.as_str() => {
                    // Body changed on disk → re-derive the description from the
                    // fresh snippet AND re-split the body into fresh atoms (don't
                    // just bump the timestamp), so neither the index TOC nor the
                    // FTS atoms can drift from the file. User-set tags are preserved.
                    storage
                        .refresh_cl_index_description(project, rel, snippet, mtime)
                        .await?;
                    storage
                        .replace_atoms_for_file(project, rel, &split_into_atoms(body), mtime)
                        .await?;
                    report.touched.push(rel.clone());
                }
                _ => {}
            }
        }

        // Orphans (index has it, disk doesn't): the file was deleted on disk
        // OUTSIDE bot-hq (via `rm`, an editor, or a user cleanup). The index
        // pointer is now pointing at /dev/null; auto-purge so cl_index_search
        // never returns a row the agent can't actually read.
        for path in &existing_paths {
            if !on_disk.contains_key(path) {
                // Purge atoms FIRST, and only drop the index row if that succeeds.
                // Order matters: stranding atoms whose index row is already gone
                // would surface them in cl_retrieve as phantom hits for a file
                // cl_index_search no longer lists. If atom deletion fails we leave
                // the index row too, so the next rescan retries the whole purge. The
                // reverse (an index row with no atoms) is harmless — cl_retrieve
                // just returns nothing for it.
                match storage.delete_atoms_for_file(project, path).await {
                    Ok(_) => {
                        if let Err(e) = storage.delete_cl_index(project, path).await {
                            tracing::warn!(?e, ?path, "failed to purge orphaned CL index row");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            ?e,
                            ?path,
                            "failed to purge orphaned CL atoms; leaving index row for next rescan"
                        );
                    }
                }
                report.orphaned.push(path.clone());
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use crate::signaling::bridge::SignalingBridge;
    use crate::storage::Storage;

    /// End-to-end: `cl_rescan` splits a CL file into atoms on add, and purges them
    /// on orphan. The touch/update path is covered at the storage layer by
    /// `replace_atoms_for_file`'s idempotent-replace test (mtime resolution makes a
    /// same-process rewrite flaky to trigger here). `_globals` resolves its CL root
    /// to `<data_dir>/library/`, so no project-path setup is needed.
    #[tokio::test]
    async fn cl_rescan_populates_then_orphans_atoms() {
        let data_dir = std::env::temp_dir().join(format!("bot-hq-rescan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data_dir);
        let lib = data_dir.join("library");
        std::fs::create_dir_all(&lib).unwrap();
        let file = lib.join("notes.md");
        std::fs::write(
            &file,
            "# Notes\n## Gotchas\nthe migration is immutable\n## Setup\nrun cargo build\n",
        )
        .unwrap();

        // Storage is Clone (shared pool) — keep a handle to assert on cl_atoms.
        let storage = Storage::memory().await.unwrap();
        let bridge = SignalingBridge::new_with(None, Some(data_dir.clone()));
        bridge.set_storage(storage.clone()).await;

        // ADD → the file's two non-empty heading sections become two atoms.
        let report = bridge.cl_rescan("_globals").await.unwrap();
        assert!(report.added.iter().any(|p| p == "notes.md"), "notes.md added");
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cl_atoms WHERE file_path = 'notes.md'")
            .fetch_one(storage.pool())
            .await
            .unwrap();
        assert_eq!(n.0, 2, "two non-empty heading sections → two atoms");

        // ORPHAN → removing the file purges its atoms alongside the index row.
        std::fs::remove_file(&file).unwrap();
        let report = bridge.cl_rescan("_globals").await.unwrap();
        assert!(report.orphaned.iter().any(|p| p == "notes.md"), "notes.md orphaned");
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cl_atoms WHERE file_path = 'notes.md'")
            .fetch_one(storage.pool())
            .await
            .unwrap();
        assert_eq!(n.0, 0, "orphaned file's atoms purged");

        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
