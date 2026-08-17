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
        self: &Arc<Self>,
        session_id: String,
        agent: String,
        project: String,
        file_path: String,
        content: String,
        append: bool,
        confirm_shrink: bool,
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
        // User-hidden files (agent_visible = 0) refuse AGENT writes: an agent
        // that can't see a diary in search must not be able to overwrite it by
        // guessing its path. The Library UI edits bypass this (different path).
        if let Some(storage) = self.storage.lock().await.clone() {
            if let Ok(Some(row)) = storage.get_cl_index(&project, &file_path).await {
                if !row.agent_visible {
                    anyhow::bail!(
                        "'{file_path}' is marked user-only (hidden from agents) — \
                         ask the user to edit it or unhide it in the Library tab"
                    );
                }
            }
        }
        let project_root = self
            .cl_project_root(&project)
            .await
            .ok_or_else(|| anyhow::anyhow!("bridge data_dir is not configured"))?;
        // The whole library is one local git repo; every agent write snapshots
        // it (see `git_version_library`).
        let library_root = self
            .data_dir
            .as_ref()
            .map(|d| crate::paths::Paths::for_data_dir(d.clone()).cl_dir);
        let fp = file_path.clone();
        let proj = project.clone();
        let commit_summary = format!("cl: {project}/{file_path} ({agent})");
        let replaced =
            tokio::task::spawn_blocking(move || -> Result<(bool, Option<String>, Vec<String>)> {
            let root_real = project_root.canonicalize().with_context(|| {
                format!("canonicalizing CL project root {}", project_root.display())
            })?;
            let (target, replaced) = if root_real.join(&fp).exists() {
                (resolve_existing_file(&root_real, &fp)?, true)
            } else {
                (resolve_new_path(&root_real, &fp)?, false)
            };
            assert_not_protected_globals_write(&proj, &root_real, &target)?;
            let final_content = if append && replaced {
                let existing = std::fs::read_to_string(&target)
                    .with_context(|| format!("reading '{fp}' for append"))?;
                let joined = if existing.trim_end().is_empty() {
                    content
                } else {
                    format!("{}\n\n{content}", existing.trim_end())
                };
                if joined.len() > MAX_WRITE_BYTES {
                    anyhow::bail!(
                        "appending would grow '{fp}' to {} bytes — the CL write cap \
                         is 1 MiB. CL files are high-signal study notes; prune first",
                        joined.len()
                    );
                }
                joined
            } else {
                if replaced && !confirm_shrink {
                    assert_not_suspicious_shrink(&target, &fp, &content)?;
                }
                content
            };
            // Both advisories read the pre-write body once; neither ever blocks
            // the write, and both fail open on a read error.
            let old_body = replaced.then(|| std::fs::read_to_string(&target).ok()).flatten();
            // Advisory status-flip lint (issues.md #19).
            let lint = old_body
                .as_deref()
                .and_then(|old| status_flip_warning(old, &final_content));
            // Retired concepts, for the close-out staleness sweep (issues.md
            // #31). An append can only add, so it retires nothing.
            let retired = match (&old_body, append) {
                (Some(old), false) => retired_terms(old, &final_content),
                _ => Vec::new(),
            };
            atomic_write(&target, &final_content)?;
            if let Some(lib) = library_root {
                git_version_library(&lib, &commit_summary);
            }
            Ok((replaced, lint, retired))
        })
        .await
        .context("cl_write_file task panicked")??;
        let (replaced, lint, retired) = replaced;
        self.record_retired_terms(&session_id, &project, retired).await;
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
        // …and back the library up (rc3 P6). The remote existed and nothing
        // ever pushed to it, so the library drifted from the first session
        // onward — a snapshot, not a backup. Detached, because a network round
        // trip must not sit inside the tool call an agent is waiting on, and
        // fail-open, because a library that cannot push is merely un-backed-up.
        // (Round 9: this line said "detached" while `.await`ing the push —
        // off the reactor thread, but still inside the call. It spawns now.)
        self.push_library_after_write(&session_id);
        let mut msg = format!(
            "{} '{file_path}' in project '{project}'",
            if replaced {
                if append { "appended to" } else { "replaced" }
            } else {
                "created"
            }
        );
        if let Some(lint) = lint {
            msg.push_str(&lint);
        }
        Ok(msg)
    }

    /// Scan-then-push the library, off the caller's thread (rc3 **P6**).
    ///
    /// **A refusal is posted as a row, not just logged.** P2's lesson applied
    /// to the user's own data: a scan that quietly declines to push is
    /// indistinguishable from one that never ran, and the failure mode it
    /// guards — a credential file tracked in the library — is one the user has
    /// to act on. Everything else is log-level: an offline machine, or a
    /// non-fast-forward from a concurrent session, is not the user's problem to
    /// fix mid-turn. Nothing here is auto-merged; a rejected push stays
    /// rejected rather than pulling someone else's library on top of this one.
    fn push_library_after_write(self: &Arc<Self>, session_id: &str) {
        let Some(dir) = self.data_dir.clone() else {
            return;
        };
        let bridge = Arc::clone(self);
        let session_id = session_id.to_string();
        let handle = tokio::spawn(async move {
            let root = crate::paths::Paths::for_data_dir(dir).cl_dir;
            let outcome = match tokio::task::spawn_blocking(move || {
                crate::signaling::bridge::scan_then_push(&root)
            })
            .await
            {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!(%e, "library push task panicked");
                    return;
                }
            };
            match &outcome {
                crate::signaling::bridge::PushOutcome::RefusedSecrets(_) => {
                    let body = format!("[System: {}]", outcome.summary());
                    tracing::warn!(%body, "library push refused by the secret scan");
                    if let Some(storage) = bridge.storage.lock().await.clone() {
                        crate::core::post_system_notice(
                            &storage,
                            Some(&bridge),
                            &session_id,
                            crate::storage::MessageKind::SystemNotice,
                            body,
                            None,
                        )
                        .await;
                    }
                }
                crate::signaling::bridge::PushOutcome::Failed(_) => {
                    tracing::warn!(summary = %outcome.summary(), "library push")
                }
                _ => tracing::info!(summary = %outcome.summary(), "library push"),
            }
        });
        // Keep the newest handle for the tests; a superseded one is simply
        // dropped — the task it names keeps running to completion (detached).
        *self.library_push.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
    }

    /// Await the most recent detached library push, if one was spawned. Tests
    /// that assert on the remote call this after `cl_write_file`; production
    /// never does — the point of the detachment is that the call returns first.
    #[cfg(test)]
    pub(crate) async fn await_library_push(&self) {
        let handle = self.library_push.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(h) = handle {
            let _ = h.await;
        }
    }
}

/// Uppercase status vocabulary for the flip lint (issues.md #19). Uppercase
/// only: that is how tracked statuses are written in practice, and lowercase
/// prose ("pending review", "work is done") would drown the lint in noise.
const PENDING_WORDS: [&str; 7] =
    ["PENDING", "STILL OWED", "OWED", "BLOCKED", "WAITING", "UNCONFIRMED", "IN PROGRESS"];
const RESOLVED_WORDS: [&str; 8] =
    ["RESOLVED", "DONE", "SHIPPED", "MERGED", "CLOSED", "FIXED", "COMPLETED", "COMPLETE"];

/// Advisory status-flip lint (issues.md #19, warn-not-block): a CL rewrite once
/// upgraded a PENDING-stakeholder question to "RESOLVED (keep both)" by
/// inference — the OPPOSITE of the stakeholder's actual decision (2026-07-24).
/// Detects lines whose pending-family status word disappeared while a matching
/// line gained a resolved-family word, and warns when no evidence marker (commit
/// sha, URL, or date) sits beside the upgrade. Returns None when clean.
fn status_flip_warning(old_body: &str, new_body: &str) -> Option<String> {
    // A line's identity anchor: its first `#123` issue-ref when present, else
    // its normalized leading text (markdown decoration stripped, lowercased).
    fn anchor(line: &str) -> Option<String> {
        if let Some(pos) = line.find('#') {
            let digits: String =
                line[pos + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return Some(format!("#{digits}"));
            }
        }
        let norm: String = line
            .trim_start_matches(['-', '*', '>', ' ', '\t', '.', ')'])
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start_matches(['.', ')', ' '])
            .trim()
            .trim_start_matches("**")
            .to_lowercase();
        // The anchor is the item's identity, so cut BEFORE the first status
        // word — otherwise "…refresh: BLOCKED" and "…refresh: DONE" normalize
        // to different anchors and the flip is never matched.
        let cut = PENDING_WORDS
            .iter()
            .chain(RESOLVED_WORDS.iter())
            .filter_map(|w| norm.find(&w.to_lowercase()))
            .min()
            .unwrap_or(norm.len());
        let ident = norm[..cut].trim_end();
        let head: String = if ident.len() >= 8 {
            ident.chars().take(24).collect()
        } else {
            norm.chars().take(24).collect()
        };
        (head.len() >= 8).then_some(head)
    }
    fn has_evidence(line: &str) -> bool {
        // Commit sha: 7+ hex chars containing at least one digit (filters
        // ordinary words like "deadbee"-free prose). URL or ISO date also count.
        let sha = line.split(|c: char| !c.is_ascii_hexdigit()).any(|tok| {
            tok.len() >= 7 && tok.chars().any(|c| c.is_ascii_digit())
        });
        sha || line.contains("http")
            || regex_lite_date(line)
    }
    // `20YY-MM-DD` without pulling a regex crate.
    fn regex_lite_date(line: &str) -> bool {
        line.as_bytes().windows(10).any(|w| {
            w[0] == b'2'
                && w[1] == b'0'
                && w[2].is_ascii_digit()
                && w[3].is_ascii_digit()
                && w[4] == b'-'
                && w[5].is_ascii_digit()
                && w[6].is_ascii_digit()
                && w[7] == b'-'
                && w[8].is_ascii_digit()
                && w[9].is_ascii_digit()
        })
    }

    let new_lines: Vec<&str> = new_body.lines().collect();
    let mut flags = Vec::new();
    for old_line in old_body.lines() {
        if !PENDING_WORDS.iter().any(|w| old_line.contains(w)) {
            continue;
        }
        // Still present verbatim (append path, untouched section): no flip.
        if new_body.contains(old_line.trim()) {
            continue;
        }
        let Some(key) = anchor(old_line) else { continue };
        for (i, new_line) in new_lines.iter().enumerate() {
            let same_item = anchor(new_line).is_some_and(|k| k == key);
            let resolved = RESOLVED_WORDS.iter().any(|w| new_line.contains(w));
            if same_item && resolved {
                let next = new_lines.get(i + 1).copied().unwrap_or("");
                if !has_evidence(new_line) && !has_evidence(next) {
                    flags.push(key.clone());
                }
                break;
            }
        }
    }
    if flags.is_empty() {
        return None;
    }
    flags.truncate(3);
    Some(format!(
        "\n⚠ status-lint: pending→resolved upgrade on {} with no evidence marker \
         (commit sha / URL / date) beside it. Status words need same-turn evidence \
         — cite the merge/message/query output next to the new status, or revert \
         the flip if it was inferred.",
        flags.join(", ")
    ))
}

/// Words too common to be a retired *concept*. The "absent from the whole new
/// body" filter already does most of the work; this stops an ordinary sentence
/// rewrite from seeding the close-out sweep with prose noise.
const SWEEP_STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "had", "her", "was",
    "one", "our", "out", "day", "get", "has", "him", "his", "how", "its", "new", "now", "old",
    "see", "two", "way", "who", "did", "put", "say", "she", "too", "use", "that", "with", "have",
    "this", "will", "your", "from", "they", "know", "want", "been", "good", "much", "some",
    "time", "very", "when", "come", "here", "just", "like", "long", "make", "many", "over",
    "such", "take", "than", "them", "well", "were", "what", "only", "then", "into", "also",
    "back", "even", "most", "still", "there", "would", "about", "which", "their", "could",
    "other", "after", "first", "these", "where", "before", "because", "should", "instead",
    "already", "always", "never", "every", "however", "though", "while", "since", "without",
    "within", "between", "another", "through", "against", "across", "under", "above", "below",
    "again", "once", "each", "both", "same", "more", "less", "must", "being", "does", "made",
    "need", "needs", "keep", "kept", "left", "right", "thing", "things",
];

/// Concepts this write RETIRED: tokens present in `old` that no longer appear
/// anywhere in `new`. Seeds the close-out staleness sweep (issues.md #31) —
/// a session that renames or drops a concept in one CL file should be told
/// which OTHER files still cite the old one, mechanically, instead of relying
/// on an agent remembering to grep. Ranked by how load-bearing the term looked
/// in the old body (occurrence count, then length) and capped, so a wholesale
/// rewrite yields the handful of real concepts rather than its whole vocabulary.
///
/// Deliberately token-level and case-insensitive: the live specimen was the
/// word "duo" surviving in `conventions.md:3` for hours after the harness
/// framing rule retired it (2026-08-05).
pub(super) fn retired_terms(old: &str, new: &str) -> Vec<String> {
    const MIN_LEN: usize = 3;
    const MAX_TERMS: usize = 12;
    // Above this many distinct candidates, the write is a BULK REWRITE and
    // frequency-ranking selects ordinary prose ("real", "pass", "empty" — the
    // s-761704e8 tasks.md refactor produced a 510-hit report of exactly such
    // words). A targeted edit retires a handful of terms and stays under it.
    const BULK_REWRITE_CANDIDATES: usize = 25;
    fn tokens(body: &str) -> impl Iterator<Item = String> + '_ {
        body.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .map(|t| t.trim_matches(['-', '_']).to_lowercase())
            .filter(|t| t.len() >= MIN_LEN && t.chars().any(|c| c.is_alphabetic()))
    }
    let surviving: std::collections::HashSet<String> = tokens(new).collect();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for tok in tokens(old) {
        if surviving.contains(&tok) || SWEEP_STOPWORDS.contains(&tok.as_str()) {
            continue;
        }
        *counts.entry(tok).or_default() += 1;
    }
    // Bulk rewrites keep only DISTINCTIVE candidates: term-shaped tokens
    // (hyphen/underscore/digit — code and file names) or tokens the old body
    // marked structurally (backticked, bolded, or in a heading). A plain word
    // like the live "duo" specimen still reports on a targeted edit, where the
    // pool is small and the old ranking already worked; a plain word in a
    // 1,500-line rewrite is vocabulary, not a concept.
    if counts.len() > BULK_REWRITE_CANDIDATES {
        let old_lower = old.to_lowercase();
        counts.retain(|tok, _| term_shaped(tok) || structurally_marked(&old_lower, tok));
    }
    let mut terms: Vec<(String, usize)> = counts.into_iter().collect();
    // Most-used first (a concept the old body leaned on), longest as tiebreak
    // (more distinctive to grep), then alphabetical so the output is stable.
    terms.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(b.0.len().cmp(&a.0.len()))
            .then(a.0.cmp(&b.0))
    });
    terms.truncate(MAX_TERMS);
    terms.into_iter().map(|(t, _)| t).collect()
}

/// Code- or artifact-shaped: a hyphen, an underscore, or a digit — the shapes
/// prose words don't have and branch names, file stems, columns and issue
/// numbers do.
fn term_shaped(tok: &str) -> bool {
    tok.chars().any(|c| c == '-' || c == '_' || c.is_ascii_digit())
}

/// Did the (lowercased) old body mark this token structurally — backticks,
/// bold, or a heading line? Structure is how a CL file says "this word is a
/// TERM here"; prose vocabulary never gets it.
fn structurally_marked(old_lower: &str, tok: &str) -> bool {
    if old_lower.contains(&format!("`{tok}`")) || old_lower.contains(&format!("**{tok}**")) {
        return true;
    }
    old_lower.lines().filter(|l| l.trim_start().starts_with('#')).any(|l| {
        l.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .any(|t| t.trim_matches(['-', '_']).eq_ignore_ascii_case(tok))
    })
}

/// Cap on the close-out sweep's reported hits — the point is to surface the
/// contradiction, not to paste the library back at the agent.
pub(super) const SWEEP_MAX_HITS: usize = 20;

/// Files the sweep never flags, because keeping an old term is their JOB:
/// `decisions.md` is append-only history (rewriting it is forbidden), and the
/// dated `learnings-*` / `notes-<date>-*` files are session records of what was
/// true when written. Flagging them would make every sweep noisy and train the
/// reader to skip it.
fn sweep_skips(file_name: &str) -> bool {
    file_name == "decisions.md"
        || file_name.starts_with("learnings-")
        || file_name.starts_with("notes-20")
}

/// Grep one project's CL root for surviving uses of `terms`. Returns
/// `"<file>:<line> — <term>"` strings, at most one per (file, term) so a term
/// repeated through a file reports once. Case-insensitive, whole-token match:
/// substring matching would flag "duo" inside "duologue".
pub(super) fn sweep_project(root: &Path, project: &str, terms: &[String]) -> Vec<String> {
    fn md_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
        if depth > 4 || out.len() > 500 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                md_files(&path, out, depth + 1);
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    md_files(root, &mut files, 0);
    files.sort();
    let mut hits = Vec::new();
    for path in files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if sweep_skips(name) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let rel = path.strip_prefix(root).unwrap_or(&path).display().to_string();
        for term in terms {
            if let Some((lineno, _)) = body.lines().enumerate().find(|(_, line)| {
                line.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
                    .any(|tok| tok.trim_matches(['-', '_']).eq_ignore_ascii_case(term))
            }) {
                hits.push(format!("  {project}/{rel}:{} — \"{term}\"", lineno + 1));
            }
        }
    }
    hits
}

/// Refuse a replace that looks like accidental data loss: writing an empty
/// body over a non-empty file, or shrinking it by more than half. Both real
/// incidents from the 2026-07-27 archive study — `temp.md` truncated to 0
/// bytes, and an approved draft replaced wholesale. `confirm_shrink: true`
/// overrides when the shrink is intentional (pruning is legitimate CL work).
fn assert_not_suspicious_shrink(target: &Path, rel_path: &str, new_content: &str) -> Result<()> {
    let old_len = std::fs::metadata(target).map(|m| m.len()).unwrap_or(0);
    if old_len == 0 {
        return Ok(());
    }
    let new_len = new_content.len() as u64;
    if new_len == 0 {
        anyhow::bail!(
            "refusing to replace non-empty '{rel_path}' ({old_len} bytes) with EMPTY \
             content. If intentional, pass confirm_shrink: true; to add content \
             instead, use mode: \"append\""
        );
    }
    if new_len * 2 < old_len {
        anyhow::bail!(
            "refusing to shrink '{rel_path}' from {old_len} to {new_len} bytes \
             (>50% loss) — cl_write_file replaces the WHOLE file, and this shape is \
             usually a partial rewrite that would destroy the rest. Read the current \
             body and write the full replacement, use mode: \"append\", or pass \
             confirm_shrink: true if the prune is intentional"
        );
    }
    Ok(())
}

/// Version the library after a successful agent write: lazily `git init` the
/// library root, then stage-all + commit with a fixed synthetic identity (no
/// dependence on the user's git config). Fail-open at every step — versioning
/// must never break a CL write. This makes the history agents already assumed
/// existed (an agent replaced a user-approved draft while recording "archived
/// in git history"; the library was not a repo) actually exist.
fn git_version_library(library_root: &Path, summary: &str) {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(library_root)
            .args(args)
            .output()
    };
    if !library_root.join(".git").exists() {
        match git(&["init", "-q"]) {
            Ok(out) if out.status.success() => {}
            other => {
                tracing::warn!(?other, root = %library_root.display(), "CL git init failed; library writes are unversioned");
                return;
            }
        }
    }
    match git(&["add", "-A"]) {
        Ok(out) if out.status.success() => {}
        other => {
            tracing::warn!(?other, "CL git add failed; skipping version commit");
            return;
        }
    }
    match git(&[
        "-c",
        "user.name=bot-hq",
        "-c",
        "user.email=bot-hq@local",
        "commit",
        "-q",
        "-m",
        summary,
    ]) {
        Ok(out) if out.status.success() => {}
        // "nothing to commit" (identical content) is routine, not a failure.
        Ok(out) => tracing::debug!(
            stderr = %String::from_utf8_lossy(&out.stderr),
            "CL git commit made no commit"
        ),
        Err(err) => tracing::warn!(%err, "CL git commit failed"),
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

    /// Give the fixture's library a bare `origin` it is level with, and return
    /// a reader for the remote's head.
    fn with_remote(tmp: &tempfile::TempDir) -> impl Fn() -> String {
        let root = tmp.path().join("library");
        let remote = tmp.path().join("remote.git");
        let git = move |root: &std::path::Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        std::process::Command::new("git")
            .args(["init", "--bare", "-q"])
            .arg(&remote)
            .output()
            .unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.name", "bot-hq"],
            vec!["config", "user.email", "bot-hq@local"],
            vec!["commit", "-q", "--allow-empty", "-m", "seed"],
        ] {
            git(&root, &args);
        }
        git(&root, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&root, &["push", "-q", "-u", "origin", "main"]);
        move || git(&root, &["rev-parse", "origin/main"])
    }

    /// rc3 **P6**: an agent's CL write reaches the remote — and a
    /// credential-shaped file stops it there.
    ///
    /// The wire is what this pins. The scanner and the push are each covered in
    /// `cl_push`, and both could pass while `cl_write_file` never calls either:
    /// that is precisely the state P6 describes — a remote that exists and
    /// nothing that pushes to it — so a green scanner would be no evidence at
    /// all. Deleting the call in `cl_write_file` turns this red.
    #[tokio::test]
    async fn a_cl_write_reaches_the_remote_unless_it_carries_a_credential() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let remote_head = with_remote(&tmp);
        let before = remote_head();

        bridge
            .cl_write_file(
                "s1".into(),
                "hands".into(),
                "bot-hq".into(),
                "notes.md".into(),
                "a learning worth keeping".into(),
                false,
                false,
            )
            .await
            .unwrap();
        // Round 9: the push is DETACHED — the call returns before the network
        // round trip; the test drains the spawned task before it looks.
        bridge.await_library_push().await;
        let after_write = remote_head();
        assert_ne!(before, after_write, "the CL write never reached the remote");

        // Now the case the order exists for: a credential lands in the library
        // (the `git add -f` an agent can do), and the next write's push must be
        // refused rather than carrying it off the machine.
        std::fs::write(tmp.path().join("library/prod.env"), "DB_PASSWORD=hunter2\n").unwrap();
        bridge
            .cl_write_file(
                "s1".into(),
                "hands".into(),
                "bot-hq".into(),
                "notes.md".into(),
                "a learning worth keeping, plus one more line".into(),
                false,
                false,
            )
            .await
            .unwrap();
        bridge.await_library_push().await;
        assert_eq!(
            remote_head(),
            after_write,
            "a library carrying a credential must not push"
        );
        // …and the refusal is visible, naming the file, rather than a silent
        // no-op the user would read as a successful backup.
        let notice = _storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.content.contains("refusing to push"))
            .expect("the refusal must leave a row");
        assert!(notice.content.contains("prod.env"), "got: {}", notice.content);
    }

    #[tokio::test]
    async fn cl_write_file_creates_nested_file_and_indexes_it() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;

        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "plans/2026/handoff.md".to_string(),
                "nested body".to_string(),
                false,
                false,
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
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "new full body".to_string(),
                false,
                false,
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
                    "hands".to_string(),
                    "bot-hq".to_string(),
                    bad.to_string(),
                    "body".to_string(),
                    false,
                    false,
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
                "hands".to_string(),
                "bot-hq".to_string(),
                "big.md".to_string(),
                "x".repeat(MAX_WRITE_BYTES + 1),
                false,
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("1 MiB"), "got: {err}");
    }

    #[tokio::test]
    async fn cl_write_file_refuses_user_hidden_files() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        storage.upsert_project("bot-hq", "bot-hq", None, None, None).await.unwrap();
        std::fs::create_dir_all(tmp.path().join("library/projects/bot-hq")).unwrap();
        std::fs::write(tmp.path().join("library/projects/bot-hq/diary.md"), "mine").unwrap();
        storage.upsert_cl_index("bot-hq", "diary.md", "d", None).await.unwrap();
        storage.set_cl_agent_visibility("bot-hq", "diary.md", false).await.unwrap();

        let err = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "diary.md".to_string(),
                "overwrite attempt".to_string(),
                false,
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("user-only"), "got: {err}");
        // File untouched.
        let body = std::fs::read_to_string(tmp.path().join("library/projects/bot-hq/diary.md")).unwrap();
        assert_eq!(body, "mine");
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
                "hands".to_string(),
                "_globals".to_string(),
                "custom-instructions.md".to_string(),
                "agent-authored rules".to_string(),
                false,
                false,
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
                "hands".to_string(),
                "_globals".to_string(),
                "agents/sneaky.md".to_string(),
                "x".to_string(),
                false,
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("protected"), "got: {err}");

        // Loose cross-project files stay writable (eod.md, tasks.md live here).
        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "_globals".to_string(),
                "eod.md".to_string(),
                "today: shipped cl_write_file".to_string(),
                false,
                false,
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
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "a learning".to_string(),
                false,
                false,
            )
            .await
            .unwrap();
        assert!(
            !bridge.should_nudge_close("s1").await,
            "a cl_write_file should lift the close-out nudge"
        );
    }

    #[tokio::test]
    async fn append_mode_adds_to_the_end_without_full_rewrite() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "## Learnings\n- old fact\n").unwrap();

        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "- new delta".to_string(),
                true,
                false,
            )
            .await
            .unwrap();
        assert!(msg.starts_with("appended to"), "got: {msg}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "## Learnings\n- old fact\n\n- new delta"
        );
    }

    #[tokio::test]
    async fn empty_and_majority_shrink_replaces_are_refused_without_confirm() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        let original = "x".repeat(1000);
        std::fs::write(&path, &original).unwrap();

        // Empty replace: refused (the temp.md truncation incident).
        let err = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                String::new(),
                false,
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("EMPTY"), "got: {err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

        // >50% shrink: refused.
        let err = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "x".repeat(400),
                false,
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains(">50% loss"), "got: {err}");

        // Same shrink with confirm_shrink: allowed (intentional prune).
        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "x".repeat(400),
                false,
                true,
            )
            .await
            .unwrap();
        assert!(msg.starts_with("replaced"), "got: {msg}");

        // Mild shrink (<50%) never needs the flag.
        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "x".repeat(300),
                false,
                false,
            )
            .await
            .unwrap();
        assert!(msg.starts_with("replaced"), "got: {msg}");
    }

    #[test]
    fn status_flip_lint_flags_evidence_free_upgrades_only() {
        use super::status_flip_warning;
        // The 2026-07-24 shape: #435 flips PENDING→RESOLVED with no evidence.
        let old = "- **#435 delta loop** — PENDING Tom's reply\n- other note\n";
        let new = "- **#435 delta loop** — RESOLVED (keep both)\n- other note\n";
        let warn = status_flip_warning(old, new).expect("evidence-free flip warns");
        assert!(warn.contains("#435"), "got: {warn}");
        assert!(warn.contains("status-lint"), "got: {warn}");

        // Same flip WITH a commit sha beside it: clean.
        let cited = "- **#435 delta loop** — RESOLVED via 43fa153a (register fixed)\n";
        assert!(status_flip_warning(old, cited).is_none());

        // Evidence on the FOLLOWING line also counts.
        let next_line = "- **#435 delta loop** — RESOLVED\n  per Tom, 2026-07-23 Discord\n";
        assert!(status_flip_warning(old, next_line).is_none());

        // Anchor-by-prefix (no issue ref) flips are caught too.
        let old_p = "- Promote table refresh: BLOCKED on staging audit\n";
        let new_p = "- Promote table refresh: DONE\n";
        assert!(status_flip_warning(old_p, new_p).is_some());

        // Pending line untouched (the append shape): clean.
        let appended = format!("{old}- new unrelated learning\n");
        assert!(status_flip_warning(old, &appended).is_none());

        // Brand-new resolved line with no pending counterpart: clean (ordinary
        // note-taking must not trip the lint).
        assert!(status_flip_warning("- some note\n", "- some note\n- X SHIPPED 2x\n").is_none());

        // Lowercase prose is not status vocabulary.
        assert!(status_flip_warning("- work is pending review\n", "- work is done now\n").is_none());
    }

    #[tokio::test]
    async fn replace_with_status_flip_still_writes_but_warns_in_the_result() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/followups.md");
        std::fs::write(&path, "## Items\n- **#94 rollout** — STILL OWED to the team\n").unwrap();

        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "followups.md".to_string(),
                "## Items\n- **#94 rollout** — DONE, everything landed\n".to_string(),
                false,
                false,
            )
            .await
            .unwrap();
        assert!(msg.starts_with("replaced"), "write is advisory-linted, not blocked: {msg}");
        assert!(msg.contains("status-lint"), "got: {msg}");
        assert!(msg.contains("#94"), "got: {msg}");
        // The write itself landed.
        assert!(std::fs::read_to_string(&path).unwrap().contains("DONE"));
    }

    #[tokio::test]
    async fn library_writes_are_git_versioned_and_history_recovers_old_body() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let lib = tmp.path().join("library");

        bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "draft.md".to_string(),
                "the approved draft".to_string(),
                false,
                false,
            )
            .await
            .unwrap();
        bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "draft.md".to_string(),
                "a whole new direction".to_string(),
                false,
                true,
            )
            .await
            .unwrap();

        // The library became a repo lazily and each write committed.
        assert!(lib.join(".git").exists(), "library was git-initialized");
        let log = std::process::Command::new("git")
            .arg("-C")
            .arg(&lib)
            .args(["log", "--oneline"])
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&log.stdout).to_string();
        assert!(log.lines().count() >= 2, "one commit per write, got:\n{log}");
        assert!(log.contains("cl: bot-hq/draft.md (hands)"), "got:\n{log}");

        // The destroyed-draft scenario is now recoverable.
        let old = std::process::Command::new("git")
            .arg("-C")
            .arg(&lib)
            .args(["show", "HEAD~1:projects/bot-hq/draft.md"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&old.stdout), "the approved draft");
    }

    // --- issues.md #31: close-out staleness sweep -------------------------

    #[test]
    fn retired_terms_keeps_dropped_concepts_and_drops_noise() {
        let old = "The duo (Brian + Rain) maintains bot-hq. The duo is the core. \
                   However, the trio was retired.";
        let new = "The harness maintains bot-hq. The harness is the core.";
        let terms = retired_terms(old, new);
        // "duo" survives the length floor (3) — the live specimen was exactly
        // this word — and outranks the single-use "trio" on occurrence count.
        assert_eq!(terms.first().map(String::as_str), Some("duo"));
        assert!(terms.contains(&"trio".to_string()), "got: {terms:?}");
        // Still present in the new body → not retired.
        assert!(!terms.contains(&"maintains".to_string()), "got: {terms:?}");
        assert!(!terms.contains(&"core".to_string()), "got: {terms:?}");
        // Prose noise is filtered: stopwords and sub-3-char tokens.
        assert!(!terms.contains(&"however".to_string()), "got: {terms:?}");
        assert!(!terms.contains(&"was".to_string()), "got: {terms:?}");
        // An append can only add — a pure superset retires nothing.
        assert!(retired_terms("alpha beta", "alpha beta gamma").is_empty());
    }

    #[test]
    fn a_bulk_rewrite_reports_terms_not_vocabulary() {
        // s-761704e8: the tasks.md refactor (1,515 → 116 lines) made the
        // sweep flag "real", "pass", "empty" — 510 hits of ordinary prose,
        // dismissed in 18 seconds. Over the bulk threshold only DISTINCTIVE
        // candidates survive: term-shaped tokens and words the old body
        // marked structurally (backticks, bold, headings).
        let mut old = String::from(
            "## Sandbox rules\n\nUse `meta_reconcile_runs` and tmp-prod-logs for scratch.\n",
        );
        for i in 0..40 {
            old.push_str(&format!(
                "- item {i}: the real pass looks empty and wrong exactly here, running \
                 tests with php quickly against normal prose sentences forever\n\
                 - walk jump swim dance sing paint drive climb read write count speak\n"
            ));
        }
        let terms = retired_terms(&old, "lean.");
        assert!(
            terms.iter().any(|t| t == "sandbox"),
            "a heading-marked plain word is a TERM and survives: {terms:?}"
        );
        assert!(terms.iter().any(|t| t == "meta_reconcile_runs"), "got: {terms:?}");
        assert!(terms.iter().any(|t| t == "tmp-prod-logs"), "got: {terms:?}");
        for noise in ["real", "pass", "empty", "wrong", "running", "tests", "php", "walk"] {
            assert!(
                !terms.iter().any(|t| t == noise),
                "prose vocabulary must not report in a bulk rewrite: {noise} in {terms:?}"
            );
        }
        // Under the threshold nothing changes: the live "duo" specimen — a
        // plain unmarked word retired by a targeted edit — still reports
        // (pinned by `retired_terms_keeps_dropped_concepts_and_drops_noise`).
    }

    #[test]
    fn sweep_skips_history_files_only() {
        // Append-only + dated records legitimately keep old vocabulary.
        assert!(sweep_skips("decisions.md"));
        assert!(sweep_skips("learnings-2026-08-05-thing.md"));
        assert!(sweep_skips("notes-2026-07-02-cl-measurement.md"));
        // Living docs are exactly what the sweep is for.
        assert!(!sweep_skips("conventions.md"));
        assert!(!sweep_skips("notes.md"));
        assert!(!sweep_skips("vision.md"));
    }

    #[tokio::test]
    async fn close_out_sweep_reports_files_still_citing_a_retired_term() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let proj = tmp.path().join("library/projects/bot-hq");
        // conventions.md keeps the old word; decisions.md keeps it too but is
        // history, so only conventions.md may be reported.
        std::fs::write(proj.join("conventions.md"), "line one\nThe duo maintains this.\n").unwrap();
        std::fs::write(proj.join("decisions.md"), "2026-08-05: retired the duo framing.\n").unwrap();
        std::fs::write(proj.join("vision.md"), "The duo (Brian + Rain) is the core.\n").unwrap();

        // The session rewrites vision.md, retiring "duo".
        bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "vision.md".to_string(),
                "The harness is the core. The harness is the core, restated at length \
                 so the shrink guard stays out of this test's way."
                    .to_string(),
                false,
                false,
            )
            .await
            .unwrap();

        let report = bridge.staleness_sweep("s1").await.expect("sweep must report a hit");
        assert!(report.contains("conventions.md:2"), "got:\n{report}");
        assert!(report.contains("\"duo\""), "got:\n{report}");
        assert!(
            !report.contains("decisions.md"),
            "append-only history must not be flagged:\n{report}"
        );
        // Advisory: it fires at most once, so it can never hold a close shut.
        assert!(
            bridge.staleness_sweep("s1").await.is_none(),
            "the sweep must not repeat"
        );
    }

    #[tokio::test]
    async fn close_out_sweep_is_silent_when_nothing_survives() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let proj = tmp.path().join("library/projects/bot-hq");
        std::fs::write(proj.join("notes.md"), "nothing related here at all.\n").unwrap();
        std::fs::write(proj.join("vision.md"), "The duo is the core.\n").unwrap();
        bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "vision.md".to_string(),
                "The harness is the core, written out at a comparable length.".to_string(),
                false,
                false,
            )
            .await
            .unwrap();
        // "duo" was retired but no other file uses it → no report, no noise.
        assert!(bridge.staleness_sweep("s1").await.is_none());
    }

}