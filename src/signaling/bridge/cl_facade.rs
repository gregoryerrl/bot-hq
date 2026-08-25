//! Context Library read surface for agents: index/folder search, read-audit,
//! folder-description writes, and the disk↔index reconciliation pass
//! (`cl_rescan`). Thin wrappers over storage plus the recursive disk walk.

use super::util::{normalize_cl_path_input, split_into_atoms, walk_cl_dir, WalkedFile};
use super::*;
use crate::storage::{ClIndexEntry, Project};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Age threshold for the repo-less staleness fallback (issues.md #23). With no
/// repo to hash-check against, an atom this old gets an age-worded ⚠ flag —
/// the CL's own drift record (handoffs going wrong within days, a followups
/// doc inverting reality inside a week) says a month-old unverifiable claim
/// deserves a date-check before it's trusted.
const AGE_STALE_FALLBACK_DAYS: i64 = 30;

/// Days since an atom's indexed mtime (RFC3339). None on missing/unparseable
/// mtime — the fallback then simply doesn't flag that atom.
fn atom_age_days(mtime: Option<&str>) -> Option<i64> {
    let raw = mtime?;
    let ts = chrono::DateTime::parse_from_rfc3339(raw).ok()?;
    Some((chrono::Utc::now() - ts.with_timezone(&chrono::Utc)).num_days().max(0))
}

/// The repo-less staleness fallback: flag atoms whose file hasn't been touched
/// in [`AGE_STALE_FALLBACK_DAYS`], carrying the age so the ⚠ wording says
/// "old and unverifiable", not "drift detected". Never touches atoms the
/// code-hash path owns (callers pick one branch per project).
/// Repo-less age fallback, per-atom (issues.md #23). Per-atom since the
/// `_globals` union: one result set can mix repo-backed atoms (hash-checked)
/// with cross-scope `_globals` atoms (age-checked), so staleness is decided
/// atom by atom in `cl_retrieve`'s loop.
fn age_stale_one(atom: &mut crate::storage::RetrievedAtom) {
    if let Some(days) = atom_age_days(atom.mtime.as_deref()) {
        if days >= AGE_STALE_FALLBACK_DAYS {
            atom.stale = true;
            atom.stale_age_days = Some(days);
        }
    }
}

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

    /// Read-side discovery, unfiltered — the Library UI and rescan sync (which
    /// must see user-hidden rows). Wraps storage.cl_index_search.
    pub async fn cl_index_search(
        &self,
        project: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ClIndexEntry>> {
        let Some(storage) = self.storage.lock().await.clone() else {
            return Ok(Vec::new());
        };
        storage.cl_index_search(project, query).await
    }

    /// Agent/plugin-facing discovery: excludes rows the user marked
    /// `agent_visible = 0`. Wraps storage.cl_index_search_agent.
    pub async fn cl_index_search_agent(
        &self,
        project: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ClIndexEntry>> {
        let Some(storage) = self.storage.lock().await.clone() else {
            return Ok(Vec::new());
        };
        storage.cl_index_search_agent(project, query).await
    }

    /// Report CL claims naming code that is gone (rc3 **P4**).
    ///
    /// Needs both halves — the project's CL directory and the repo it
    /// documents — and says which one it could not resolve rather than
    /// reporting a clean library it never checked. A project with no registered
    /// repo has nothing to check against, and that is a configuration answer,
    /// not "no drift".
    pub async fn cl_stale_refs(&self, project: &str) -> Result<String> {
        let Some(cl_dir) = self.cl_project_root(project).await else {
            anyhow::bail!("bot-hq's data dir is not configured; cannot locate the library");
        };
        let repo_root = {
            let storage = self.storage.lock().await.clone();
            match storage {
                Some(s) => s
                    .get_project(project)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|p| p.working_repo_path),
                None => None,
            }
        };
        let Some(repo_root) = repo_root.map(std::path::PathBuf::from) else {
            anyhow::bail!(
                "project '{project}' has no registered repo, so there is nothing to check its \
                 CL claims against. Register one in the Context Library tab first."
            );
        };
        let claims = tokio::task::spawn_blocking({
            let (cl_dir, repo_root) = (cl_dir.clone(), repo_root.clone());
            move || super::cl_staleness::stale_claims(&cl_dir, &repo_root)
        })
        .await
        .map_err(|e| anyhow::anyhow!("cl_stale_refs task panicked: {e}"))?
        .map_err(|e| anyhow::anyhow!("scanning the repo failed: {e}"))?;
        // The drift sweep (Batch 10; the W6 verdict's gap, narrowed by review:
        // the retrieve path already hash-flags drifted atoms per query — the
        // REPORT was the blind surface). Same comparison, whole project: an
        // atom whose cited code's hash moved since indexing is "code moved
        // since written" — the symbol may well still exist, which is exactly
        // the case the identifier scan above cannot see.
        let drifted: Vec<String> = {
            let storage = self.storage.lock().await.clone();
            match storage {
                Some(s) => {
                    let atoms = s.cl_atoms_with_code_hash(project).await.unwrap_or_default();
                    atoms
                        .iter()
                        .filter(|a| {
                            let current =
                                super::cl_refs::compute_code_hash(&a.body, &repo_root);
                            current.as_deref() != a.code_hash.as_deref()
                        })
                        .map(|a| format!("{} > {}", a.file_path, a.heading_path))
                        .collect()
                }
                None => Vec::new(),
            }
        };
        let mut report = super::cl_staleness::render_report(project, &repo_root, &claims);
        if !drifted.is_empty() {
            report.push_str(&format!(
                "
## Code moved since written ({} atom{})
                 The cited identifiers still exist, but the code they describe has                  changed since the atom was indexed — behavior claims may be stale                  even though nothing above flags them:
{}
",
                drifted.len(),
                if drifted.len() == 1 { "" } else { "s" },
                drifted
                    .iter()
                    .map(|d| format!("- {d}"))
                    .collect::<Vec<_>>()
                    .join("
")
            ));
        }
        Ok(report)
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
        include_globals: bool,
    ) -> Result<Vec<crate::storage::RetrievedAtom>> {
        let Some(storage) = self.storage.lock().await.clone() else {
            return Ok(Vec::new());
        };
        // Caller-supplied paths → stored `/` key form. These land in an exact
        // `file_path IN (…)` predicate (`cl_atoms::cl_retrieve`), so a `\`
        // spelling silently narrows the result to nothing instead of erroring.
        let normalized: Option<Vec<String>> = paths.map(|p| {
            p.iter()
                .map(|s| normalize_cl_path_input(s))
                .collect::<Vec<_>>()
        });
        let mut atoms = storage
            .cl_retrieve(
                project,
                query,
                normalized.as_deref(),
                budget_tokens,
                include_globals,
            )
            .await?;
        // Flag atoms whose cited code drifted since indexing. Repo coupling lives
        // in the bridge (storage stays pure): recompute each atom's code_hash from
        // the current repo and compare to the stored baseline. Staleness is
        // per-atom: a `_globals`-union row inside a project query takes the
        // repo-less AGE fallback (issues.md #23) keyed off the ATOM's project,
        // never the queried project's repo.
        let repo_root = storage
            .get_project(project)
            .await
            .ok()
            .flatten()
            .and_then(|p| p.working_repo_path)
            .map(PathBuf::from);
        for atom in &mut atoms {
            if atom.project_id != project {
                age_stale_one(atom);
            } else if let Some(root) = &repo_root {
                if let Some(stored) = atom.code_hash.as_deref() {
                    let current = cl_refs::compute_code_hash(&atom.body, root);
                    atom.stale = current.as_deref() != Some(stored);
                }
            } else {
                age_stale_one(atom);
            }
        }
        Ok(atoms)
    }

    /// Best-effort Stage-4b telemetry: log one `cl_retrieve` call to
    /// `retrieval_events`. Derives atom/token/stale counts from the returned
    /// atoms (token estimate matches the retrieval budget's own estimator). A
    /// logging failure is swallowed (warn only) — measurement must never break a
    /// retrieval, mirroring the fail-open DB-hook pattern.
    pub async fn log_retrieval_event(
        &self,
        session_id: String,
        agent: String,
        project: &str,
        query: &str,
        atoms: &[crate::storage::RetrievedAtom],
        budget: i64,
    ) {
        let Some(storage) = self.storage.lock().await.clone() else {
            return;
        };
        let atom_count = atoms.len() as i64;
        let tokens_returned: i64 = atoms
            .iter()
            .map(|a| crate::storage::estimate_tokens(&a.body))
            .sum();
        let stale_count = atoms.iter().filter(|a| a.stale).count() as i64;
        let returned: Vec<serde_json::Value> = atoms
            .iter()
            .map(|a| {
                serde_json::json!({
                    "file_path": a.file_path,
                    "heading_path": a.heading_path,
                    "tokens": crate::storage::estimate_tokens(&a.body),
                    "stale": a.stale,
                })
            })
            .collect();
        let returned_atoms = serde_json::to_string(&returned).unwrap_or_else(|_| "[]".to_string());
        if let Err(e) = storage
            .log_retrieval_event(
                Some(&session_id),
                Some(&agent),
                project,
                query,
                atom_count,
                tokens_returned,
                budget,
                stale_count,
                &returned_atoms,
            )
            .await
        {
            tracing::warn!("failed to log retrieval_event for {project}: {e:#}");
        }
    }

    /// Read-side discovery for FOLDER descriptions. Parallel to
    /// [`cl_index_search`]. Returns lightweight rows; empty list when storage
    /// isn't configured (test bridges).
    pub async fn cl_folder_search(
        &self,
        project: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<crate::storage::ClFolder>> {
        let Some(storage) = self.storage.lock().await.clone() else {
            return Ok(Vec::new());
        };
        storage.cl_folder_search(project, query).await
    }

    /// Write-side for agents: upsert a folder description. The jsonrpc layer
    /// gates this on the `write_context_library` capability.
    pub async fn cl_register_folder_description(
        &self,
        project: &str,
        folder_path: &str,
        description: &str,
        tags: Option<&str>,
    ) -> Result<()> {
        let Some(storage) = self.storage.lock().await.clone() else {
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
        let Some(storage) = self.storage.lock().await.clone() else {
            return Ok(());
        };
        // Caller-supplied path → stored `/` key form (see
        // `normalize_cl_path_input`). Without it a `\` spelling silently misses
        // and the audit row is dropped on the `debug!` below.
        let file_path = &normalize_cl_path_input(file_path);
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
        // Sync `read_to_string` per file, off the reactor: the library is a
        // few hundred files today and grows, and this runs on the fs-watcher
        // path as well as the MCP one.
        let on_disk: HashMap<String, WalkedFile> = {
            let root = root.clone();
            let project = project.to_string();
            tokio::task::spawn_blocking(move || {
                let mut out = HashMap::new();
                walk_cl_dir(&root, &root, &project, &mut out);
                out
            })
            .await
            .map_err(|e| anyhow::anyhow!("CL walk task failed: {e}"))?
        };

        // Existing rows.
        let existing = storage.cl_index_search(Some(project), None).await?;
        let existing_paths: HashSet<String> =
            existing.iter().map(|e| e.file_path.clone()).collect();

        // Adds + touches. Index existing rows by path for O(1) lookup — this was
        // an O(disk × index) linear scan (`existing.iter().find`) per on-disk file.
        let by_path: HashMap<&str, &_> =
            existing.iter().map(|e| (e.file_path.as_str(), e)).collect();
        // Which indexed files already have atoms — one grouped query, so the
        // unchanged-file arm below is a set lookup instead of a COUNT per file
        // (round 10; this loop also runs from the fs-watcher).
        let atomised = storage.files_with_atoms(project).await?;
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
                    if !atomised.contains(rel.as_str()) {
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
            .cl_retrieve("_globals", "queryable", None, 1000, false)
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
        let hits = bridge.cl_retrieve("proj", "helper", None, 1000, false).await.unwrap();
        assert!(!hits.is_empty(), "atom should be retrievable");
        assert!(hits.iter().all(|h| !h.stale), "fresh atom is not stale");

        // The cited code changes (no re-rescan) → the citing atom is flagged stale...
        std::fs::write(repo_s.join("src/foo.rs"), "fn foo() { changed() }").unwrap();
        let foo_hits = bridge.cl_retrieve("proj", "helper", None, 1000, false).await.unwrap();
        assert!(foo_hits.iter().any(|h| h.stale), "atom citing changed code is stale");

        // ...but an atom that cites no code is never flagged.
        let bare_hits = bridge.cl_retrieve("proj", "prose", None, 1000, false).await.unwrap();
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

    /// issues.md #23: repo-less projects get the age fallback — old atoms are
    /// flagged with their age; fresh or mtime-less atoms are left alone.
    #[test]
    fn age_fallback_flags_old_atoms_in_repoless_projects() {
        use super::{age_stale_one, AGE_STALE_FALLBACK_DAYS};
        let atom = |mtime: Option<String>| crate::storage::RetrievedAtom {
            project_id: "_globals".into(),
            file_path: "notes.md".into(),
            heading_path: "Notes".into(),
            body: "b".into(),
            code_hash: None,
            stale: false,
            mtime,
            stale_age_days: None,
        };
        let old = (chrono::Utc::now()
            - chrono::Duration::days(AGE_STALE_FALLBACK_DAYS + 15))
        .to_rfc3339();
        let fresh = chrono::Utc::now().to_rfc3339();
        let mut atoms = vec![
            atom(Some(old)),
            atom(Some(fresh)),
            atom(None),
            atom(Some("garbage".into())),
        ];
        for a in &mut atoms {
            age_stale_one(a);
        }

        assert!(atoms[0].stale, "old atom flagged");
        assert_eq!(atoms[0].stale_age_days, Some(AGE_STALE_FALLBACK_DAYS + 15));
        assert!(!atoms[1].stale, "fresh atom untouched");
        assert!(!atoms[2].stale, "mtime-less atom untouched");
        assert!(!atoms[3].stale, "unparseable mtime untouched");
        assert!(
            atoms[1..].iter().all(|a| a.stale_age_days.is_none()),
            "age recorded only on flagged atoms"
        );
    }
}
