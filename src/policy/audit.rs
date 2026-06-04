//! Policy-mutation detection.
//!
//! Closes the "agent edits policy.yaml to remove forbidden words, then commits"
//! attack vector. We hash every policy.yaml file each time it's read; a hash
//! mismatch against the previous read writes a `PolicyMutation` violation.
//!
//! v1 is audit-only — we don't block on mutation, we just log it. The user
//! reviews violations.jsonl periodically. v2 (later) can distinguish
//! authorized (via Settings UI) vs unauthorized mutations and block the
//! latter from taking effect on the next git operation.
//!
//! ## Hash cache location
//!
//! `<data_dir>/.policy-hashes.json` — single JSON file mapping the policy
//! file's absolute path → its last-known hex hash. Cheap to read/write; we
//! reload on every audit (no in-memory cache means the hook handler subprocess
//! sees the same state as the GUI process without IPC).

use crate::policy::violations::{ViolationKind, ViolationOutcome, ViolationsLog};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const HASH_CACHE_FILE: &str = ".policy-hashes.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct HashCache {
    /// Map: absolute policy file path → hex hash of last-seen content.
    /// Sorted for stable serialization (helps reviewers eyeball changes).
    entries: BTreeMap<String, String>,
}

impl HashCache {
    fn load(data_dir: &Path) -> Result<Self> {
        let p = data_dir.join(HASH_CACHE_FILE);
        match std::fs::read_to_string(&p) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(cache) => Ok(cache),
                // Stay resilient (a corrupt cache shouldn't brick the audit), but
                // make the reset LOUD: with an empty cache every policy file
                // re-registers as `FirstSeen` instead of `Changed`, so this
                // cycle's policy-mutation tamper detection is disarmed. A silent
                // `unwrap_or_default()` hid exactly that.
                Err(e) => {
                    tracing::warn!(
                        ?e,
                        path = %p.display(),
                        "policy hash cache is corrupt — resetting to empty; \
                         policy-mutation detection is disarmed for this cycle"
                    );
                    Ok(Self::default())
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading hash cache at {}", p.display())),
        }
    }

    fn save(&self, data_dir: &Path) -> Result<()> {
        let p = data_dir.join(HASH_CACHE_FILE);
        let body = serde_json::to_string_pretty(self).context("serializing hash cache")?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, body)
            .with_context(|| format!("writing temp hash cache at {}", tmp.display()))?;
        std::fs::rename(&tmp, &p)
            .with_context(|| format!("renaming temp into {}", p.display()))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationOutcome {
    /// File doesn't exist (no policy.yaml for this project). Nothing to audit.
    FileAbsent,
    /// First time we've seen this file. Hash recorded; not a mutation.
    FirstSeen,
    /// Hash matches last-seen value. No mutation.
    Unchanged,
    /// Hash changed since last read. Mutation logged.
    Changed { from: String, to: String },
}

/// Audit every policy.yaml that could contribute to the resolved policy for
/// `project`. Logs `PolicyMutation` for any file whose content changed since
/// the previous audit.
///
/// `data_dir` is the CL root. We audit:
/// - `<data_dir>/general-policy.yaml`
/// - `<data_dir>/projects/<project>/policy.yaml` (if `project` is set)
///
/// Returns the per-file outcomes (mainly for tests + UI surfacing).
pub fn audit_policy_files(
    data_dir: &Path,
    project: Option<&str>,
    log: Option<&ViolationsLog>,
    caller_session: &str,
    caller_agent: &str,
) -> Result<Vec<(PathBuf, MutationOutcome)>> {
    audit_policy_files_at_root(
        data_dir,
        project,
        None,
        log,
        caller_session,
        caller_agent,
    )
}

/// Like [`audit_policy_files`] but accepts an explicit `project_root` so
/// in-process callers that have already resolved `cl_path` don't duplicate the
/// DB lookup. `None` falls back to the default convention.
pub fn audit_policy_files_at_root(
    data_dir: &Path,
    project: Option<&str>,
    project_root: Option<&Path>,
    log: Option<&ViolationsLog>,
    caller_session: &str,
    caller_agent: &str,
) -> Result<Vec<(PathBuf, MutationOutcome)>> {
    let mut targets: Vec<PathBuf> = vec![data_dir.join("general-policy.yaml")];
    if let Some(p) = project {
        let proj_dir = match project_root {
            Some(root) => root.to_path_buf(),
            None => data_dir.join("projects").join(p),
        };
        targets.push(proj_dir.join("policy.yaml"));
    }
    let mut cache = HashCache::load(data_dir)?;
    let mut cache_dirty = false;
    let mut outcomes = Vec::new();
    for path in targets {
        let outcome = audit_one(&path, &mut cache, &mut cache_dirty)?;
        if let MutationOutcome::Changed { from, to } = &outcome {
            if let Some(log) = log {
                log_sync(log, caller_session, caller_agent, &path, from, to)?;
            }
        }
        outcomes.push((path, outcome));
    }
    if cache_dirty {
        cache.save(data_dir)?;
    }
    Ok(outcomes)
}

fn audit_one(
    path: &Path,
    cache: &mut HashCache,
    dirty: &mut bool,
) -> Result<MutationOutcome> {
    let body = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(MutationOutcome::FileAbsent),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let hash = content_hash(&body);
    let key = path.display().to_string();
    let outcome = match cache.entries.get(&key).cloned() {
        None => {
            cache.entries.insert(key, hash);
            *dirty = true;
            MutationOutcome::FirstSeen
        }
        Some(prev) if prev == hash => MutationOutcome::Unchanged,
        Some(prev) => {
            cache.entries.insert(key, hash.clone());
            *dirty = true;
            MutationOutcome::Changed { from: prev, to: hash }
        }
    };
    Ok(outcome)
}

/// Record a policy file's current content hash in the cache WITHOUT logging a
/// mutation. Called by the user-only Tauri policy editors right after they
/// write `general-policy.yaml` / `projects/<p>/policy.yaml`, so the authorized
/// edit doesn't read back as an unauthorized `PolicyMutation` on the next
/// hook/audit pass (the v2 "authorized via Settings UI" distinction anticipated
/// in this module's header). Absent file → cache entry removed (a deleted
/// policy resets to FirstSeen on next read). Best-effort; errors propagate so
/// the command can surface them.
pub fn record_policy_write(data_dir: &Path, path: &Path) -> Result<()> {
    let mut cache = HashCache::load(data_dir)?;
    let key = path.display().to_string();
    match std::fs::read_to_string(path) {
        Ok(body) => {
            cache.entries.insert(key, content_hash(&body));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            cache.entries.remove(&key);
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    }
    cache.save(data_dir)
}

fn content_hash(content: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Synchronously append a PolicyMutation entry to the log. This runs from both
/// the in-process async call sites (`spawn_session_handle`, the signaling
/// bridge) AND the hookless `policy-check` subprocess, so it must NOT build a
/// tokio runtime — a nested `block_on` panics inside the app's live runtime.
/// `ViolationsLog::record_blocking` does a plain blocking append, valid in
/// every context.
fn log_sync(
    log: &ViolationsLog,
    session: &str,
    agent: &str,
    path: &Path,
    from: &str,
    to: &str,
) -> Result<()> {
    log.record_blocking(
        session.to_string(),
        agent.to_string(),
        ViolationKind::PolicyMutation,
        path.display().to_string(),
        ViolationOutcome::Detected,
        Some(format!(
            "hash {} → {} (audit-only; review and approve via Settings)",
            &from[..from.len().min(8)],
            &to[..to.len().min(8)]
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn first_read_records_baseline_no_mutation() {
        let data = tempdir().unwrap();
        std::fs::write(data.path().join("general-policy.yaml"), "forbidden_in_commits:\n  - Claude\n").unwrap();
        let log = ViolationsLog::new(data.path());
        let outcomes = audit_policy_files(data.path(), None, Some(&log), "s1", "test").unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].1, MutationOutcome::FirstSeen);
        // No mutation → no log entry.
        let recs = log.read_all().unwrap();
        assert!(recs.is_empty(), "first-seen should not produce a violation");
    }

    #[test]
    fn second_read_unchanged_no_mutation() {
        let data = tempdir().unwrap();
        std::fs::write(data.path().join("general-policy.yaml"), "forbidden_in_commits:\n  - Claude\n").unwrap();
        let log = ViolationsLog::new(data.path());
        audit_policy_files(data.path(), None, Some(&log), "s1", "test").unwrap();
        let outcomes = audit_policy_files(data.path(), None, Some(&log), "s1", "test").unwrap();
        assert_eq!(outcomes[0].1, MutationOutcome::Unchanged);
        assert!(log.read_all().unwrap().is_empty());
    }

    #[test]
    fn mutation_detected_and_logged() {
        let data = tempdir().unwrap();
        let pol = data.path().join("general-policy.yaml");
        std::fs::write(&pol, "forbidden_in_commits:\n  - Claude\n  - bot-hq\n").unwrap();
        let log = ViolationsLog::new(data.path());
        // Baseline
        audit_policy_files(data.path(), None, Some(&log), "s1", "test").unwrap();
        // Mutate: agent quietly removes "bot-hq"
        std::fs::write(&pol, "forbidden_in_commits:\n  - Claude\n").unwrap();
        let outcomes = audit_policy_files(data.path(), None, Some(&log), "s1", "agent").unwrap();
        assert!(matches!(outcomes[0].1, MutationOutcome::Changed { .. }));
        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, ViolationKind::PolicyMutation);
        assert_eq!(recs[0].outcome, ViolationOutcome::Detected);
        assert!(recs[0].detail.as_ref().unwrap().contains("hash"));
        assert!(recs[0].action.contains("general-policy.yaml"));
    }

    #[tokio::test]
    async fn change_detected_inside_runtime_does_not_panic() {
        // Regression: audit_policy_files runs from async call sites
        // (spawn_session_handle, the signaling bridge). The Changed branch must
        // record the mutation WITHOUT building a nested tokio runtime — doing so
        // inside the live runtime panics with "Cannot start a runtime from
        // within a runtime", which wedged session start after policy files
        // changed (stale hash cache → Changed → log).
        let data = tempdir().unwrap();
        let policy = data.path().join("general-policy.yaml");
        std::fs::write(&policy, "forbidden_in_commits:\n  - Claude\n").unwrap();
        let log = ViolationsLog::new(data.path());
        // Baseline read records the first hash.
        audit_policy_files(data.path(), None, Some(&log), "s1", "test").unwrap();
        // Mutate so the next audit takes the Changed branch → records a mutation.
        std::fs::write(&policy, "forbidden_in_commits:\n  - Claude\n  - Anthropic\n").unwrap();
        let outcomes = audit_policy_files(data.path(), None, Some(&log), "s1", "test").unwrap();
        assert!(matches!(outcomes[0].1, MutationOutcome::Changed { .. }));
        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, ViolationKind::PolicyMutation);
    }

    #[test]
    fn missing_file_is_not_a_mutation() {
        let data = tempdir().unwrap();
        let log = ViolationsLog::new(data.path());
        let outcomes = audit_policy_files(data.path(), Some("nope"), Some(&log), "s1", "test").unwrap();
        // Both files absent → both FileAbsent.
        for (_, o) in &outcomes {
            assert_eq!(*o, MutationOutcome::FileAbsent);
        }
        assert!(log.read_all().unwrap().is_empty());
    }

    #[test]
    fn project_policy_audited_when_set() {
        let data = tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("projects/foo")).unwrap();
        std::fs::write(
            data.path().join("projects/foo/policy.yaml"),
            "forbidden_in_commits:\n  - bot-hq\n",
        )
        .unwrap();
        let log = ViolationsLog::new(data.path());
        let outcomes = audit_policy_files(data.path(), Some("foo"), Some(&log), "s1", "test").unwrap();
        assert_eq!(outcomes.len(), 2); // general (absent) + project
        let project_outcome = outcomes
            .iter()
            .find(|(p, _)| p.ends_with("projects/foo/policy.yaml"))
            .unwrap();
        assert_eq!(project_outcome.1, MutationOutcome::FirstSeen);
    }

    #[test]
    fn hash_cache_persists_across_calls() {
        let data = tempdir().unwrap();
        std::fs::write(data.path().join("general-policy.yaml"), "a: 1\n").unwrap();
        let log = ViolationsLog::new(data.path());
        audit_policy_files(data.path(), None, Some(&log), "s", "t").unwrap();
        // Cache file should exist
        let cache_file = data.path().join(HASH_CACHE_FILE);
        assert!(cache_file.exists());
        let body = std::fs::read_to_string(&cache_file).unwrap();
        assert!(body.contains("general-policy.yaml"));
    }

    #[test]
    fn record_policy_write_suppresses_spurious_mutation() {
        // Simulate the user editing policy via the Settings/CL editor: write the
        // file, then record_policy_write. The next audit must read Unchanged —
        // NOT Changed — so no spurious PolicyMutation violation is logged.
        let data = tempdir().unwrap();
        let pol = data.path().join("general-policy.yaml");
        std::fs::write(&pol, "forbidden_in_commits:\n  - Claude\n").unwrap();
        let log = ViolationsLog::new(data.path());
        // Baseline read records the first hash.
        audit_policy_files(data.path(), None, Some(&log), "s1", "test").unwrap();
        // User edits via the editor command → write + record.
        std::fs::write(&pol, "forbidden_in_commits:\n  - Claude\n  - bot-hq\n").unwrap();
        record_policy_write(data.path(), &pol).unwrap();
        // Next audit sees the recorded hash → Unchanged, no violation.
        let outcomes = audit_policy_files(data.path(), None, Some(&log), "s1", "agent").unwrap();
        assert_eq!(outcomes[0].1, MutationOutcome::Unchanged);
        assert!(
            log.read_all().unwrap().is_empty(),
            "authorized edit recorded via record_policy_write must not log a mutation"
        );
    }

    #[test]
    fn record_policy_write_on_absent_file_clears_entry() {
        let data = tempdir().unwrap();
        let pol = data.path().join("general-policy.yaml");
        std::fs::write(&pol, "a: 1\n").unwrap();
        audit_policy_files(data.path(), None, None, "s", "t").unwrap();
        // Delete the file, then record — entry should be dropped so a later
        // re-create reads FirstSeen (not Changed).
        std::fs::remove_file(&pol).unwrap();
        record_policy_write(data.path(), &pol).unwrap();
        std::fs::write(&pol, "a: 2\n").unwrap();
        let outcomes = audit_policy_files(data.path(), None, None, "s", "t").unwrap();
        assert_eq!(outcomes[0].1, MutationOutcome::FirstSeen);
    }

    #[test]
    fn audit_without_log_handle_still_updates_cache() {
        let data = tempdir().unwrap();
        std::fs::write(data.path().join("general-policy.yaml"), "a: 1\n").unwrap();
        // No log handle → mutations not logged but cache still tracks.
        let outcomes = audit_policy_files(data.path(), None, None, "s", "t").unwrap();
        assert_eq!(outcomes[0].1, MutationOutcome::FirstSeen);
        std::fs::write(data.path().join("general-policy.yaml"), "a: 2\n").unwrap();
        let outcomes = audit_policy_files(data.path(), None, None, "s", "t").unwrap();
        assert!(matches!(outcomes[0].1, MutationOutcome::Changed { .. }));
    }
}
