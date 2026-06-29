//! Context Library read surface for agents: index/folder search, read-audit,
//! folder-description writes, and the disk↔index reconciliation pass
//! (`cl_rescan`). Thin wrappers over storage plus the recursive disk walk.

use super::util::{split_into_atoms, walk_cl_dir, WalkedFile};
use super::*;
use crate::storage::{ClIndexEntry, Project};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Build the atoms for a CL file, stamping each with a hash of the repo source it
/// cites (`None` when there's no repo or no valid ref) so retrieval can flag
/// drift. Storage stays pure — the repo coupling is computed here in the bridge.
fn atoms_with_code_hash(body: &str, repo_root: Option<&Path>) -> Vec<crate::storage::Atom> {
    let mut atoms = split_into_atoms(body);
    if let Some(root) = repo_root {
        for a in &mut atoms {
            a.code_hash = cl_refs::compute_code_hash(&a.body, root);
        }
    }
    atoms
}

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

    /// Read-side ranked retrieval for agents: returns the CL atom bodies best
    /// matching `query` under a token budget. Wraps `storage.cl_retrieve`; empty
    /// when storage isn't configured (test bridges).
    pub async fn cl_retrieve(
        &self,
        project: &str,
        query: &str,
        paths: Option<&[String]>,
        budget_tokens: i64,
    ) -> Result<Vec<crate::storage::RetrievedAtom>> {
        let Some(storage) = self.storage.lock().await.clone() else {
            return Ok(Vec::new());
        };
        let mut atoms = storage.cl_retrieve(project, query, paths, budget_tokens).await?;
        // Flag atoms whose cited code drifted since indexing. Repo coupling lives
        // in the bridge (storage stays pure): recompute each atom's code_hash from
        // the current repo and compare to the stored baseline.
        let repo_root = storage
            .get_project(project)
            .await
            .ok()
            .flatten()
            .and_then(|p| p.working_repo_path);
        if let Some(repo_root) = repo_root {
            let repo_root = PathBuf::from(repo_root);
            for atom in &mut atoms {
                if let Some(stored) = atom.code_hash.as_deref() {
                    let current = cl_refs::compute_code_hash(&atom.body, &repo_root);
                    atom.stale = current.as_deref() != Some(stored);
                }
            }
        }
        Ok(atoms)
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

        // The project's CODE repo (for atom->code stale-flagging): atoms cite
        // source paths relative to this root. None when unregistered / _globals,
        // so code_hash stays None and nothing is stale-flagged.
        let repo_root = storage
            .get_project(project)
            .await
            .ok()
            .flatten()
            .and_then(|p| p.working_repo_path)
            .map(PathBuf::from);

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
                        .replace_atoms_for_file(project, rel, &atoms_with_code_hash(body, repo_root.as_deref()), mtime)
                        .await?;
                    report.added.push(rel.clone());
                }
                Some(row) if disk_is_newer(&row.updated_at, mtime) => {
                    // Body changed on disk → re-derive the description from the
                    // fresh snippet AND re-split the body into fresh atoms (don't
                    // just bump the timestamp), so neither the index TOC nor the
                    // FTS atoms can drift from the file. User-set tags are preserved.
                    storage
                        .refresh_cl_index_description(project, rel, snippet, mtime)
                        .await?;
                    storage
                        .replace_atoms_for_file(project, rel, &atoms_with_code_hash(body, repo_root.as_deref()), mtime)
                        .await?;
                    report.touched.push(rel.clone());
                }
                _ => {
                    if storage.count_atoms_for_file(project, rel).await? == 0 {
                        storage
                            .replace_atoms_for_file(project, rel, &atoms_with_code_hash(body, repo_root.as_deref()), mtime)
                            .await?;
                        report.touched.push(rel.clone());
                    }
                }
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

/// True if the on-disk file mtime is strictly newer than the stored index
/// timestamp — the rescan "did this file change?" check. Parses both as RFC3339
/// instants so a FORMAT difference can't make the comparison misfire:
/// `now_utc()` writes `Z`/millisecond (`2026-..T12:00:00.500Z`) while a disk
/// mtime is `+00:00`/nanosecond (`2026-..T12:00:00.500999999+00:00`), and a
/// lexicographic string compare mis-orders those (`Z` sorts AFTER the nanosecond
/// digits). Unparseable timestamps are treated as "changed" so a malformed value
/// re-derives rather than silently skipping the file forever.
fn disk_is_newer(stored_updated_at: &str, disk_mtime: &str) -> bool {
    use chrono::DateTime;
    match (
        DateTime::parse_from_rfc3339(stored_updated_at),
        DateTime::parse_from_rfc3339(disk_mtime),
    ) {
        (Ok(stored), Ok(disk)) => disk > stored,
        _ => true, // unparseable → assume changed; re-derive rather than skip
    }
}

#[cfg(test)]
mod tests {
    use crate::signaling::bridge::SignalingBridge;
    use crate::storage::Storage;

    #[test]
    fn disk_is_newer_parses_mixed_rfc3339_formats() {
        use super::disk_is_newer;
        // The discriminating case: stored is now_utc() (Z/millis); the disk mtime
        // is the SAME millisecond but later in nanoseconds (+00:00/nanos). It IS
        // newer, but a lexicographic compare missed it ('Z' > the nanos digits).
        assert!(
            disk_is_newer("2026-06-29T12:00:00.500Z", "2026-06-29T12:00:00.500999999+00:00"),
            "a disk mtime later within the same millisecond is newer"
        );
        // Exact same instant across the two formats → NOT newer (no spurious re-derive).
        assert!(!disk_is_newer("2026-06-29T12:00:00.000Z", "2026-06-29T12:00:00.000000000+00:00"));
        // Disk clearly earlier / clearly later.
        assert!(!disk_is_newer("2026-06-29T12:05:00.000Z", "2026-06-29T12:00:00.000000000+00:00"));
        assert!(disk_is_newer("2026-06-29T12:00:00.000Z", "2026-06-29T13:00:00.000000000+00:00"));
        // Unparseable on either side → treat as changed (safe re-derive).
        assert!(disk_is_newer("garbage", "2026-06-29T12:00:00.000Z"));
        assert!(disk_is_newer("2026-06-29T12:00:00.000Z", "not-a-date"));
    }

    #[tokio::test]
    async fn cl_rescan_backfills_atoms_for_existing_unchanged_index_rows() {
        let data_dir = std::env::temp_dir().join(format!(
            "bot-hq-rescan-backfill-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&data_dir);
        let lib = data_dir.join("library");
        std::fs::create_dir_all(&lib).unwrap();
        let file = lib.join("notes.md");
        std::fs::write(&file, "# Notes\n## Retrieval\nqueryable context works\n").unwrap();

        let storage = Storage::memory().await.unwrap();
        storage
            .upsert_cl_index("_globals", "notes.md", "old indexed row", None)
            .await
            .unwrap();
        let bridge = SignalingBridge::new_with(None, Some(data_dir.clone()));
        bridge.set_storage(storage.clone()).await;

        let before: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM cl_atoms WHERE project_id = '_globals' AND file_path = 'notes.md'",
        )
        .fetch_one(storage.pool())
        .await
        .unwrap();
        assert_eq!(before.0, 0, "test starts with an indexed file but no atoms");

        let report = bridge.cl_rescan("_globals").await.unwrap();
        assert!(
            report.touched.iter().any(|p| p == "notes.md"),
            "backfilled atomless files are reported as touched"
        );
        let after: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM cl_atoms WHERE project_id = '_globals' AND file_path = 'notes.md'",
        )
        .fetch_one(storage.pool())
        .await
        .unwrap();
        assert_eq!(after.0, 1, "existing unchanged file should be atomized");

        let retrieved = storage
            .cl_retrieve("_globals", "queryable", None, 1000)
            .await
            .unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].file_path, "notes.md");

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// End-to-end P1.2: an atom that cites repo code is flagged stale once that
    /// code changes after indexing, while a code-free atom never is.
    #[tokio::test]
    async fn cl_retrieve_flags_atom_when_cited_code_changes() {
        let base = std::env::temp_dir().join(format!("bot-hq-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // A code repo with a source file the note will cite.
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let repo_s = repo.canonicalize().unwrap();
        std::fs::write(repo_s.join("src/foo.rs"), "fn foo() {}").unwrap();
        // The project's CL dir (named-project fallback: <data_dir>/library/projects/<name>).
        let cl_dir = base.join("library/projects/proj");
        std::fs::create_dir_all(&cl_dir).unwrap();
        std::fs::write(
            cl_dir.join("notes.md"),
            "# Notes\n## Foo\nThe helper lives in src/foo.rs and does the thing.\n\
             ## Bare\nJust prose with no code reference here at all.\n",
        )
        .unwrap();

        let storage = Storage::memory().await.unwrap();
        storage
            .upsert_project("proj", "proj", repo_s.to_str(), None, None)
            .await
            .unwrap();
        let bridge = SignalingBridge::new_with(None, Some(base.clone()));
        bridge.set_storage(storage.clone()).await;
        bridge.cl_rescan("proj").await.unwrap();

        // Fresh index: the cited code is unchanged → not stale.
        let hits = bridge.cl_retrieve("proj", "helper", None, 1000).await.unwrap();
        assert!(!hits.is_empty(), "atom should be retrievable");
        assert!(hits.iter().all(|h| !h.stale), "fresh atom is not stale");

        // The cited code changes (no re-rescan) → the citing atom is flagged stale...
        std::fs::write(repo_s.join("src/foo.rs"), "fn foo() { changed() }").unwrap();
        let foo_hits = bridge.cl_retrieve("proj", "helper", None, 1000).await.unwrap();
        assert!(foo_hits.iter().any(|h| h.stale), "atom citing changed code is stale");

        // ...but an atom that cites no code is never flagged.
        let bare_hits = bridge.cl_retrieve("proj", "prose", None, 1000).await.unwrap();
        assert!(
            !bare_hits.is_empty() && bare_hits.iter().all(|h| !h.stale),
            "no-ref atom is never stale"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

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
