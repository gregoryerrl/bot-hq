//! Direct Context Library writes for agents. `cl_write_file` (create, replace
//! or append) and `cl_edit_file` (an in-place old→new correction) are the two
//! agent write paths, and they share ONE guarded body: traversal guard,
//! size cap, shrink guard, advisory lints, atomic write, git snapshot, index
//! rescan and the close-out gate lift, so no separate `cl_rescan` call is
//! needed and no guard can reach one entry point and not the other.

use super::util::{normalize_cl_path_input, rel_key};
use super::*;
use crate::storage::Project;
use anyhow::Context;

/// Hard cap on a single CL write. The CL is study notes, not a data store —
/// anything larger than the UI editor's 1 MB read cap (tauri_cmd/cl.rs)
/// would be unreadable there anyway.
const MAX_WRITE_BYTES: usize = 1_048_576; // 1 MiB

/// What a guarded CL write does to its target.
#[derive(Debug, Clone)]
pub(crate) enum WriteOp {
    /// Replace the whole body; creates the file when it does not exist.
    Replace(String),
    /// Add after a blank line; creates the file when it does not exist.
    Append(String),
    /// Replace exactly `expect` non-overlapping occurrences of `old` with
    /// `new` in a file that must already exist (feedback #30, week 35: a
    /// three-line correction cost a 38 KB re-emit, and the cheap alternative —
    /// an append — put the correction 296 lines below the claim it replaced).
    Edit {
        old: String,
        new: String,
        expect: usize,
    },
}

/// What the write did, for the reply.
#[derive(Debug)]
enum Done {
    Created,
    Replaced,
    Appended,
    Edited {
        occurrences: usize,
        old_len: usize,
        new_len: usize,
    },
}

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
        // **F10: what an AGENT writes to the CL is redacted** (the user's pick
        // `c2ca371d`), so a secret an agent writes never reaches the file, the
        // library's git history or its remote. On an append only the new text
        // is redacted — never what the file already holds. The Context Library
        // tab saves through `tauri_cmd::cl::cl_write_file`, not here, and stays
        // as the user typed it.
        let (content, redacted) = crate::policy::secret_scan::redact_counting(content);
        let op = if append {
            WriteOp::Append(content)
        } else {
            WriteOp::Replace(content)
        };
        let mut msg = self
            .write_cl(session_id, agent, project, file_path, op, confirm_shrink)
            .await?;
        if redacted > 0 {
            msg.push_str(&crate::policy::secret_scan::redaction_note(redacted));
        }
        Ok(msg)
    }

    /// Replace `expect_occurrences` occurrences of `old_string` with
    /// `new_string` in an EXISTING `file_path` under `project`'s CL root —
    /// the same contract as an ordinary file-edit tool, behind every guard
    /// `cl_write_file` has. A count that differs from the expectation refuses
    /// and names the count, so a non-unique anchor is widened rather than
    /// guessed at; a result that loses more than half the file is refused
    /// without `confirm_shrink`, exactly like a replace.
    pub async fn cl_edit_file(
        self: &Arc<Self>,
        session_id: String,
        agent: String,
        project: String,
        file_path: String,
        old_string: String,
        new_string: String,
        expect_occurrences: usize,
        confirm_shrink: bool,
    ) -> Result<String> {
        if old_string.is_empty() {
            anyhow::bail!("old_string is empty — give the exact text to replace");
        }
        if old_string == new_string {
            anyhow::bail!("old_string and new_string are identical — nothing to change");
        }
        if expect_occurrences == 0 {
            anyhow::bail!("expect_occurrences must be at least 1");
        }
        // F10: only the REPLACEMENT is redacted — `old_string` must match the
        // file as it is, secrets included. So an edit that merely carries a
        // user-typed secret through (raw in `old_string`, repeated in
        // `new_string`) rewrites it to its `[redacted: …]` marker.
        let (new_string, redacted) = crate::policy::secret_scan::redact_counting(new_string);
        let op = WriteOp::Edit {
            old: old_string,
            new: new_string,
            expect: expect_occurrences,
        };
        let mut msg = self
            .write_cl(session_id, agent, project, file_path, op, confirm_shrink)
            .await?;
        if redacted > 0 {
            msg.push_str(&crate::policy::secret_scan::redaction_note(redacted));
        }
        Ok(msg)
    }

    /// The one guarded write path both tools share.
    async fn write_cl(
        self: &Arc<Self>,
        session_id: String,
        agent: String,
        project: String,
        file_path: String,
        op: WriteOp,
        confirm_shrink: bool,
    ) -> Result<String> {
        if project.trim().is_empty() {
            anyhow::bail!("project is required");
        }
        if file_path.trim().is_empty() || file_path.starts_with('/') || file_path.contains("..") {
            anyhow::bail!("file_path must be a relative CL path within the project");
        }
        // The cap is re-checked on the RESULT inside the closure (an append or
        // an edit grows a file it never re-emits); this is the cheap early
        // refusal for a body that is over it on its own.
        let incoming = match &op {
            WriteOp::Replace(c) | WriteOp::Append(c) => c.len(),
            WriteOp::Edit { new, .. } => new.len(),
        };
        if incoming > MAX_WRITE_BYTES {
            anyhow::bail!(
                "content is {incoming} bytes — the CL write cap is 1 MiB. CL files are \
                 high-signal study notes; trim or split instead"
            );
        }
        // Normalize the caller's separators to the stored `/` key form BEFORE
        // the guard below. Without this, an agent writing the natural
        // `agents/rain/notes.md` missed a row keyed `agents\rain\notes.md`,
        // `get_cl_index` returned Ok(None), the `if let Some(row)` never fired,
        // and the agent_visible check was SKIPPED — while Windows resolved both
        // spellings to the SAME FILE, so the write landed on a user-hidden
        // file. Runs after the `..` / leading-`/` validation above so those
        // rejections still see the raw input.
        let file_path = normalize_cl_path_input(&file_path);
        // User-hidden files (agent_visible = 0) refuse AGENT writes: an agent
        // that can't see a diary in search must not be able to overwrite it by
        // guessing its path. The Library UI edits bypass this (different path).
        let storage = self.storage.lock().await.clone();
        if let Some(storage) = storage {
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
        let outcome =
            tokio::task::spawn_blocking(move || -> Result<(Done, Option<String>, Vec<String>, Option<Snapshot>, Snapshot)> {
            let root_real = project_root.canonicalize().with_context(|| {
                format!("canonicalizing CL project root {}", project_root.display())
            })?;
            let exists = root_real.join(&fp).exists();
            if !exists && matches!(op, WriteOp::Edit { .. }) {
                anyhow::bail!(
                    "no such file '{fp}' in project '{proj}' — cl_edit_file edits an \
                     existing file; create it with cl_write_file"
                );
            }
            let target = if exists {
                resolve_existing_file(&root_real, &fp)?
            } else {
                resolve_new_path(&root_real, &fp)?
            };
            assert_not_protected_globals_write(&proj, &root_real, &target)?;
            // The pre-write body, read ONCE: the append join, the edit and both
            // advisories all work from it. A create has none. An append or an
            // edit NEEDS it and fails on an unreadable target; a replace does
            // not — it keeps writing with the advisories skipped, as it always
            // did (the shrink guard reads metadata, not the body).
            let needs_body = matches!(op, WriteOp::Append(_) | WriteOp::Edit { .. });
            let old_body = match (exists, needs_body) {
                (false, _) => None,
                (true, true) => Some(
                    std::fs::read_to_string(&target)
                        .with_context(|| format!("reading '{fp}' before editing it"))?,
                ),
                (true, false) => std::fs::read_to_string(&target).ok(),
            };
            // Create-vs-existing is keyed on `exists`, never on whether the
            // body could be read: a replace over an unreadable existing file
            // must still run the shrink guard and report "replaced".
            let (done, final_content) = match (op, exists, old_body.as_deref()) {
                (WriteOp::Append(content), true, Some(existing)) => {
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
                    (Done::Appended, joined)
                }
                (WriteOp::Append(content), false, _) | (WriteOp::Replace(content), false, _) => {
                    (Done::Created, content)
                }
                (WriteOp::Replace(content), true, _) => {
                    if !confirm_shrink {
                        assert_not_suspicious_shrink(&target, &fp, &content)?;
                    }
                    (Done::Replaced, content)
                }
                (WriteOp::Edit { old, new, expect }, true, Some(existing)) => {
                    let found = existing.matches(old.as_str()).count();
                    if found != expect {
                        anyhow::bail!(
                            "found {found} occurrence(s) of old_string in '{fp}', expected \
                             {expect} — {}",
                            if found == 0 {
                                // F10: an agent's earlier write stored a secret
                                // as its marker, so an old_string quoting the
                                // secret can never match — say so, rather than
                                // "check the exact text", which the agent did.
                                match crate::policy::secret_scan::redact(old.as_str()) {
                                    std::borrow::Cow::Owned(marked) if existing.contains(marked.as_str()) => {
                                        "old_string quotes a secret, but the file holds its \
                                         `[redacted: …]` marker there (bot-hq redacts secrets in \
                                         what agents write) — match the marker text instead; \
                                         nothing was changed"
                                            .to_string()
                                    }
                                    _ => "check the exact text (whitespace and punctuation \
                                          included); nothing was changed"
                                        .to_string(),
                                }
                            } else {
                                format!(
                                    "widen old_string until it is unique, or pass \
                                     expect_occurrences: {found} to replace every one; \
                                     nothing was changed"
                                )
                            }
                        );
                    }
                    let edited = existing.replacen(old.as_str(), new.as_str(), expect);
                    if edited.len() > MAX_WRITE_BYTES {
                        anyhow::bail!(
                            "the edit would grow '{fp}' to {} bytes — the CL write cap \
                             is 1 MiB. CL files are high-signal study notes; prune first",
                            edited.len()
                        );
                    }
                    // An edit that deletes more than half the file is the same
                    // accident shape as a partial replace, and takes the same
                    // flag to confirm.
                    if !confirm_shrink {
                        assert_not_suspicious_shrink(&target, &fp, &edited)?;
                    }
                    (
                        Done::Edited {
                            occurrences: expect,
                            old_len: existing.len(),
                            new_len: edited.len(),
                        },
                        edited,
                    )
                }
                // An edit on a missing file was refused above, and an existing
                // file's body was read with `?` for both ops that need it.
                (WriteOp::Edit { .. }, _, _) | (WriteOp::Append(_), true, None) => {
                    unreachable!("an edit or append reaches here only with a body")
                }
            };
            // Both advisories compare against the pre-write body; neither ever
            // blocks the write.
            // Advisory status-flip lint (issues.md #19).
            let lint = old_body
                .as_deref()
                .and_then(|old| status_flip_warning(old, &final_content));
            // Retired concepts, for the close-out staleness sweep (issues.md
            // #31). An append can only add, so it retires nothing; a replace
            // or an edit can delete a term.
            let retired = match (&old_body, &done) {
                (Some(old), Done::Replaced | Done::Edited { .. }) => {
                    retired_terms(old, &final_content)
                }
                _ => Vec::new(),
            };
            // One library git operation at a time, process-wide: the library
            // is shared by every project's sessions, and two interleaved
            // add/commit pairs can each sweep the other's file or collide on
            // the index lock (feedback #27/#28).
            let _library_git = LIBRARY_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            // Content on disk that never reached git (a bare Write/Bash) is
            // committed ALONE before this write replaces it — otherwise the
            // "rollback point" an agent records is a generation stale.
            let pre = match (library_root.as_deref(), exists) {
                (Some(lib), true) => snapshot_unversioned(lib, &target),
                _ => None,
            };
            // A failed pre-snapshot REFUSES the write (EYES b80187af): writing
            // anyway would destroy the only copy of content git never saw —
            // the very loss #27/#28 is about.
            if let Some(Snapshot::Failed(why)) = &pre {
                anyhow::bail!(
                    "'{fp}' holds content that was never versioned, and snapshotting it failed \
                     ({why}) — nothing was written, so that content is intact on disk. Tell the \
                     user; retry once the library's git works again."
                );
            }
            atomic_write(&target, &final_content)?;
            let snapshot = match library_root.as_deref() {
                Some(lib) => git_version_library(lib, &commit_summary),
                None => Snapshot::NotARepo,
            };
            Ok((done, lint, retired, pre, snapshot))
        })
        .await
        .context("CL write task panicked")??;
        let (done, lint, retired, pre, snapshot) = outcome;
        self.record_retired_terms(&session_id, &project, retired, Some(&file_path)).await;
        self.record_cl_write(&session_id, &project, &file_path).await;
        let library = self
            .data_dir
            .as_ref()
            .map(|d| crate::paths::Paths::for_data_dir(d.clone()).cl_dir.display().to_string())
            .unwrap_or_default();
        let concurrent = note_cl_writer(&library, &project, &file_path, &session_id);
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
        let mut msg = match done {
            Done::Created => format!("created '{file_path}' in project '{project}'"),
            Done::Replaced => format!("replaced '{file_path}' in project '{project}'"),
            Done::Appended => format!("appended to '{file_path}' in project '{project}'"),
            Done::Edited {
                occurrences,
                old_len,
                new_len,
            } => format!(
                "edited '{file_path}' in project '{project}' — {occurrences} occurrence(s) \
                 replaced, {old_len} → {new_len} bytes"
            ),
        };
        // The snapshot's OUTCOME rides the reply (feedback #27/#28, week 35): an
        // agent recorded `git log -1 -- <path>` as its rollback point before a
        // full-file replace, and that commit was a generation stale because a
        // write had never been snapshotted at all. The sha here is the baseline
        // to record; "no snapshot" is a fact to act on, not a silent gap.
        match &pre {
            Some(Snapshot::Committed(sha)) => msg.push_str(&format!(
                " — the file held content that was never versioned (written outside \
                 cl_write_file); committed it first as {sha}, which holds what was there before \
                 this write"
            )),
            _ => {} // a failed pre-snapshot refused the write above
        }
        msg.push_str(&format!(" — {}", snapshot.describe()));
        if let Some(warning) = concurrent {
            msg.push_str(&warning);
        }
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
        let lock = Arc::clone(&self.library_push_lock);
        let handle = tokio::spawn(async move {
            // One push at a time (F7b): a second write's push must start after
            // the first has finished, so it never races the remote ref.
            let _serial = lock.lock().await;
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
                    let storage = bridge.storage.lock().await.clone();
                    if let Some(storage) = storage {
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

/// Concepts this write RETIRED: terms present in `old` that no longer appear
/// anywhere in `new`. Seeds the close-out staleness sweep (issues.md #31) —
/// a session that renames or drops a concept in one CL file should be told
/// which OTHER files still cite the old one, mechanically, instead of relying
/// on an agent remembering to grep. Ranked by how load-bearing the term looked
/// in the old body (occurrence count, then length) and capped, so a wholesale
/// rewrite yields the handful of real concepts rather than its whole vocabulary.
///
/// **Distinctive-only** (the user's pick a9f8c705, 2026-08-24, narrowed by
/// their pick 4136edff, 2026-09-24): a word reports only when it is
/// code-shaped (dash/underscore/digit) or BACKTICKED in the old body. Words in
/// headings and bold no longer count — every EOD heading ("Tom", "Down",
/// "Report") seeded the sweep, 609 false hits in one close (feedback #38).
/// **Filenames and paths** (`eod.md`, `projects/x/notes.md`) always count:
/// the word tokenizer splits them at the dot, which is how a renamed
/// `eod.md` slipped past the sweep while it reported ordinary words.
pub(super) fn retired_terms(old: &str, new: &str) -> Vec<String> {
    const MIN_LEN: usize = 3;
    const MAX_TERMS: usize = 12;
    fn tokens(body: &str) -> impl Iterator<Item = String> + '_ {
        body.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .map(|t| t.trim_matches(['-', '_']).to_lowercase())
            .filter(|t| t.len() >= MIN_LEN && t.chars().any(|c| c.is_alphabetic()))
    }
    let surviving: std::collections::HashSet<String> = tokens(new).collect();
    let surviving_artifacts: std::collections::HashSet<String> = artifact_tokens(new).collect();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for tok in tokens(old) {
        if surviving.contains(&tok) || SWEEP_STOPWORDS.contains(&tok.as_str()) {
            continue;
        }
        *counts.entry(tok).or_default() += 1;
    }
    let old_lower = old.to_lowercase();
    counts.retain(|tok, _| term_shaped(tok) || backticked(&old_lower, tok));
    let mut artifacts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for a in artifact_tokens(old) {
        if !surviving_artifacts.contains(&a) {
            *artifacts.entry(a).or_default() += 1;
        }
    }
    // A word that is only a PIECE of a retired filename/path (`tool-gate` of
    // `tool-gate.json`) would report the same reference twice.
    counts.retain(|tok, _| {
        !artifacts.keys().any(|a| {
            a.split(['/', '.']).any(|piece| piece == tok)
        })
    });
    counts.extend(artifacts);
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

/// Did the (lowercased) old body BACKTICK this token? Code spans are how a CL
/// file says "this word is a TERM here"; headings and bold also carry plain
/// prose, and counting them flooded the sweep (feedback #38).
fn backticked(old_lower: &str, tok: &str) -> bool {
    old_lower.contains(&format!("`{tok}`"))
}

/// Filenames (`eod.md`, `tool-gate.json`) and paths (`projects/x/notes.md`,
/// `~/.bot-hq/library`) in `body`, lowercased, trimmed of surrounding
/// punctuation. See [`is_artifact`] for what qualifies.
fn artifact_tokens(body: &str) -> impl Iterator<Item = String> + '_ {
    body.split(|c: char| {
        c.is_whitespace()
            || matches!(c, '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | '|' | '*' | '{' | '}')
    })
    .map(|t| t.trim_matches(|c: char| matches!(c, '.' | ':' | '!' | '?' | '#')).to_lowercase())
    .filter(|t| is_artifact(t))
}

/// Extensions a CL file names when it cites a FILE. An allow-list: `and/or`,
/// `I/P/A/V` and `github.com` are prose, and counting them as filename terms
/// brought back the very noise the sweep was narrowed to remove (EYES
/// 8179f7cc).
const FILE_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "json", "jsonl", "yaml", "yml", "toml", "ini", "cfg", "conf", "env",
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "rb", "go", "java", "kt", "swift", "c",
    "h", "cpp", "hpp", "cs", "php", "sh", "zsh", "bash", "fish", "ps1", "sql", "csv", "tsv",
    "html", "htm", "css", "scss", "xml", "svg", "png", "jpg", "jpeg", "gif", "pdf", "lock",
    "log", "bak", "patch", "diff", "proto", "graphql", "vue", "svelte", "dart", "lua",
    "ipynb", "sqlite", "db", "plist", "dmg", "zip", "gz", "tgz",
];

/// A filename is a stem of 2+ characters with a KNOWN extension (so `e.g`,
/// `1.0.6`, `github.com` are not files); a path without a filename counts only
/// when it is anchored like one (`/…`, `./…`, `~/…`, `projects/…`) — so prose
/// slashes (`and/or`, `read/write`, `Added/Fixed/Changed`) never do. URLs are
/// not artifacts.
fn is_artifact(t: &str) -> bool {
    if t.len() < 4 || t.contains("://") || !t.chars().any(char::is_alphabetic) {
        return false;
    }
    let last = t.rsplit('/').next().unwrap_or(t);
    let is_file = last.rsplit_once('.').is_some_and(|(stem, ext)| {
        stem.chars().count() >= 2 && FILE_EXTENSIONS.contains(&ext)
    });
    let anchored = ["/", "./", "~/", "projects/"].iter().any(|p| t.starts_with(p));
    let is_path = anchored && t.split('/').filter(|seg| !seg.is_empty()).count() >= 2;
    is_file || is_path
}

/// Is `term` a filename/path term (vs a word)? The two match differently.
pub(super) fn is_artifact_term(term: &str) -> bool {
    term.contains('.') || term.contains('/')
}

/// Does `line` cite `term`? Words match a whole token, case-insensitively;
/// filenames/paths match as a case-insensitive substring bounded by
/// non-name characters (a `/` before is fine: `x/eod.md` cites `eod.md`).
pub(super) fn line_cites(line: &str, term: &str) -> bool {
    if !is_artifact_term(term) {
        return line
            .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .any(|tok| tok.trim_matches(['-', '_']).eq_ignore_ascii_case(term));
    }
    let lower = line.to_lowercase();
    let name_char = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.');
    let mut from = 0;
    while let Some(pos) = lower[from..].find(term) {
        let start = from + pos;
        let end = start + term.len();
        let before_ok = lower[..start].chars().next_back().is_none_or(|c| !name_char(c) || c == '/');
        let after_ok = lower[end..].chars().next().is_none_or(|c| !name_char(c) || c == '.' && lower[end + 1..].chars().next().is_none_or(|n| !name_char(n)));
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
        while !lower.is_char_boundary(from) {
            from += 1;
        }
    }
    false
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
pub(super) fn sweep_project(
    root: &Path,
    project: &str,
    terms: &[String],
    own_files: &[String],
) -> Vec<String> {
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
        // `/`-form so the reported hit path matches the CL key the reader will
        // search for, rather than a native `agents\rain\x.md` spelling.
        let rel = rel_key(&path, root).unwrap_or_else(|| path.display().to_string());
        let own = own_files.iter().any(|f| f == &rel);
        for term in terms {
            // A file this session itself wrote is skipped for WORD terms (it
            // was just brought up to date by hand) — never for a filename or
            // path: a partly-updated file is exactly where a stale `eod.md`
            // survives (plan review M4).
            if own && !is_artifact_term(term) {
                continue;
            }
            if let Some((lineno, _)) = body.lines().enumerate().find(|(_, line)| line_cites(line, term)) {
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
             (>50% loss) — this shape is usually a partial rewrite that would destroy \
             the rest. Correct a passage in place with cl_edit_file, add with mode: \
             \"append\", write the full replacement, or pass confirm_shrink: true if \
             the prune is intentional"
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
///
/// Returns what the versioning did, so the tool reply can carry the commit
/// (the agent's real rollback point) or say plainly that there is none.
/// The library's HEAD sha, when the library is a git repo with a commit.
pub(super) fn library_head(library_root: &Path) -> Option<String> {
    if !library_root.join(".git").exists() {
        return None;
    }
    let out = library_git(library_root, &["rev-parse", "HEAD"]).ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `(project, term)` for every CL file deleted or renamed since `head` —
/// committed or still only in the working tree — as the sweep's filename
/// terms: the basename, and the project-relative path when it is nested.
/// `projects/<p>/…` belongs to project `p`; anything else to `_globals`.
/// The library is shared, so another session's removals in the same window
/// are included — the sweep is advisory.
pub(super) fn library_files_removed_since(library_root: &Path, head: &str) -> Vec<(String, String)> {
    let Ok(out) = library_git(
        library_root,
        &["diff", "--no-ext-diff", "--name-status", "-M", "--diff-filter=DR", head],
    ) else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut gone = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut cols = line.split('\t');
        let status = cols.next().unwrap_or("");
        let Some(old) = cols.next() else { continue };
        if !(status.starts_with('D') || status.starts_with('R')) {
            continue;
        }
        let (project, rel) = match old.strip_prefix("projects/").and_then(|r| r.split_once('/')) {
            Some((p, rest)) => (p.to_string(), rest.to_lowercase()),
            None => (crate::storage::Project::GLOBALS.to_string(), old.to_lowercase()),
        };
        let base = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        if is_artifact(&base) {
            gone.push((project.clone(), base.clone()));
        }
        if rel != base && is_artifact(&rel) {
            gone.push((project, rel));
        }
    }
    gone
}

/// How recently another session's write to the same CL file counts as
/// concurrent (feedback #44(3): two sessions edited the shared EOD files the
/// same afternoon, and one chased a "change" that was the other's write).
const CONCURRENT_WRITE_WINDOW: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// The last session to write each CL file, process-wide (every session runs in
/// this one app).
/// Keyed by (library root, project, file): one app has one library, and the
/// root keeps two libraries (tests, or a dev build beside a release) apart.
static CL_LAST_WRITER: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<(String, String, String), (String, std::time::Instant)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Record this write and, when a DIFFERENT session wrote the same file within
/// [`CONCURRENT_WRITE_WINDOW`], return the warning the reply carries.
fn note_cl_writer(library: &str, project: &str, file_path: &str, session_id: &str) -> Option<String> {
    let mut map = CL_LAST_WRITER.lock().unwrap_or_else(|p| p.into_inner());
    let key = (library.to_string(), project.to_string(), file_path.to_string());
    let warning = match map.get(&key) {
        Some((other, at)) if other != session_id && at.elapsed() < CONCURRENT_WRITE_WINDOW => Some(format!(
            " — ⚠ another session ({other}) wrote this file {} min ago: re-read it before \
             relying on your copy, and check you did not replace its change (both versions are \
             in the library's git history)",
            at.elapsed().as_secs() / 60
        )),
        _ => None,
    };
    map.insert(key, (session_id.to_string(), std::time::Instant::now()));
    warning
}

/// Serialises every library git operation in this process (see the write
/// path): a sync mutex, held on the blocking thread for add + commit.
static LIBRARY_GIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `git` in the library the way bot-hq runs it on its own: the hardened
/// invocation (repo hooks and fsmonitor off — `core::git`) plus signing off,
/// so a global `commit.gpgsign` or a library hook cannot fail every CL
/// snapshot (EYES b80187af). Identity is passed per commit.
fn library_git(library_root: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    crate::core::git::hardened(library_root)
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
}

/// Commit `target` ALONE when its on-disk content never reached git — written
/// with a bare `Write`/`Bash`, or left uncommitted by an interrupted write —
/// before a CL write replaces it (feedback #27/#28: a session recorded a
/// rollback point that was a generation stale because the latest content had
/// never been snapshotted). Only this path is committed (`git commit -- <p>`):
/// the library is shared, and another session's pending edits must not ride
/// along under this message. `None` when there is nothing to do.
fn snapshot_unversioned(library_root: &Path, target: &Path) -> Option<Snapshot> {
    if !library_root.join(".git").exists() {
        return None; // the first write's `git init` sweeps everything in
    }
    let root = library_root.canonicalize().unwrap_or_else(|_| library_root.to_path_buf());
    let rel = target
        .strip_prefix(&root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| target.to_string_lossy().to_string());
    let git = |args: &[&str]| library_git(library_root, args);
    let status = git(&["status", "--porcelain", "--", &rel]).ok()?;
    if !status.status.success() || status.stdout.is_empty() {
        return None; // clean: the current content is already a commit
    }
    match git(&["add", "--", &rel]) {
        Ok(out) if out.status.success() => {}
        Ok(out) => return Some(Snapshot::Failed(format!(
            "git add: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        Err(e) => return Some(Snapshot::Failed(e.to_string())),
    }
    let message = format!("cl: {rel} (unversioned content found before a write)");
    match git(&[
        "-c", "user.name=bot-hq", "-c", "user.email=bot-hq@local",
        "commit", "-q", "-m", &message, "--", &rel,
    ]) {
        Ok(out) if out.status.success() => Some(match git(&["rev-parse", "--short=7", "HEAD"]) {
            Ok(rev) if rev.status.success() => {
                Snapshot::Committed(String::from_utf8_lossy(&rev.stdout).trim().to_string())
            }
            _ => Snapshot::Committed("(sha unreadable)".into()),
        }),
        Ok(out) => Some(Snapshot::Failed(format!(
            "git commit: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        Err(e) => Some(Snapshot::Failed(e.to_string())),
    }
}

fn git_version_library(library_root: &Path, summary: &str) -> Snapshot {
    let git = |args: &[&str]| library_git(library_root, args);
    if !library_root.join(".git").exists() {
        match git(&["init", "-q"]) {
            Ok(out) if out.status.success() => {}
            other => {
                tracing::warn!(?other, root = %library_root.display(), "CL git init failed; library writes are unversioned");
                return Snapshot::Failed("git init failed".into());
            }
        }
    }
    match git(&["add", "-A"]) {
        Ok(out) if out.status.success() => {}
        other => {
            tracing::warn!(?other, "CL git add failed; skipping version commit");
            return Snapshot::Failed("git add failed".into());
        }
    }
    // "Nothing to commit" is decided HERE, by git itself — not inferred from a
    // failed commit, which would report a lock clash or a hook refusal as "the
    // library already held this content" (feedback #27/#28).
    match git(&["diff", "--cached", "--quiet"]) {
        Ok(out) if out.status.success() => return Snapshot::NothingToCommit,
        Ok(out) if out.status.code() == Some(1) => {} // staged changes — commit them
        other => {
            tracing::warn!(?other, "CL git diff --cached failed; skipping version commit");
            return Snapshot::Failed("git diff --cached failed".into());
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
        Ok(out) if out.status.success() => match git(&["rev-parse", "--short=7", "HEAD"]) {
            Ok(rev) if rev.status.success() => {
                Snapshot::Committed(String::from_utf8_lossy(&rev.stdout).trim().to_string())
            }
            _ => Snapshot::Committed("(sha unreadable)".into()),
        },
        // Staged changes and a failed commit: a real failure, said as one.
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            tracing::warn!(%stderr, "CL git commit failed");
            Snapshot::Failed(format!("git commit: {stderr}"))
        }
        Err(err) => {
            tracing::warn!(%err, "CL git commit failed");
            Snapshot::Failed(err.to_string())
        }
    }
}

/// What versioning a CL write did — the reply's rollback line (feedback #27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Snapshot {
    /// A commit landed; the short sha is the baseline to record.
    Committed(String),
    /// The library was already at this content — no new commit, the previous
    /// HEAD is the baseline.
    NothingToCommit,
    /// Versioning failed (init / add / commit) — the write is on disk and
    /// UNVERSIONED until a later write sweeps it up.
    Failed(String),
    /// No library root resolved — the write is on disk and unversioned.
    NotARepo,
}

impl Snapshot {
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Committed(sha) => format!("snapshot {sha} (the rollback point to record)"),
            Self::NothingToCommit => "no new snapshot: the library already held this content \
                 (the previous commit is the rollback point)"
                .to_string(),
            Self::Failed(why) => format!(
                "NO SNAPSHOT TAKEN ({why}): the write is on disk but unversioned — do not \
                 record a rollback point from git log"
            ),
            Self::NotARepo => "NO SNAPSHOT TAKEN (no library root): the write is on disk but \
                 unversioned"
                .to_string(),
        }
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
        // F7a (feedback #27/#28): the reply names the snapshot it took — a
        // real short sha the agent can record as its rollback point — and it
        // is the library's HEAD.
        let sha = msg
            .split("snapshot ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .expect("the reply carries `snapshot <sha>`");
        assert_eq!(sha.len(), 7, "short sha: {msg}");
        let head = std::process::Command::new("git")
            .args(["-C", tmp.path().join("library").to_str().unwrap(), "rev-parse", "--short=7", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), sha);
        // Writing the identical body again takes no new snapshot and says so,
        // instead of leaving the agent to infer a baseline from `git log`.
        let again = bridge
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
        assert!(again.contains("no new snapshot"), "got: {again}");
    }

    /// Feedback #27/#28: content written OUTSIDE cl_write_file (a bare Write
    /// the library's git never saw) is committed ALONE before a write replaces
    /// it — so the parent of the write's snapshot really holds what was there —
    /// and the reply says so. A clean file takes no pre-write snapshot.
    #[tokio::test]
    async fn unversioned_content_is_snapshotted_before_a_write_replaces_it() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let lib = tmp.path().join("library");
        let path = lib.join("projects/bot-hq/notes.md");
        let write = |body: &str| {
            let bridge = bridge.clone();
            let body = body.to_string();
            async move {
                bridge
                    .cl_write_file(
                        "s1".to_string(),
                        "hands".to_string(),
                        "bot-hq".to_string(),
                        "notes.md".to_string(),
                        body,
                        false,
                        false,
                    )
                    .await
                    .unwrap()
            }
        };
        write("versioned body, the first generation\n").await;
        std::fs::write(&path, "OUT-OF-BAND body, the second generation\n").unwrap();
        let msg = write("the third generation, written by the tool\n").await;
        assert!(msg.contains("never versioned"), "the reply names the pre-write snapshot: {msg}");
        let show = |rev: &str| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&lib)
                .arg("show")
                .arg(format!("{rev}:projects/bot-hq/notes.md"))
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        assert_eq!(show("HEAD~1"), "OUT-OF-BAND body, the second generation\n");
        assert_eq!(show("HEAD"), "the third generation, written by the tool\n");
        let again = write("the fourth generation, written by the tool\n").await;
        assert!(!again.contains("never versioned"), "a clean file takes no pre-snapshot: {again}");
    }

    /// EYES b80187af: a library whose git would fail a plain commit (global
    /// or local `commit.gpgsign` with no usable signer, a repo hook) still
    /// versions CL writes — bot-hq's own invocation turns both off.
    #[tokio::test]
    async fn library_snapshots_ignore_signing_config_and_repo_hooks() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let lib = tmp.path().join("library");
        let write = |body: &str| {
            let bridge = bridge.clone();
            let body = body.to_string();
            async move {
                bridge
                    .cl_write_file("s1".into(), "hands".into(), "bot-hq".into(), "notes.md".into(), body, false, false)
                    .await
                    .unwrap()
            }
        };
        write("first body, which initialises the library\n").await;
        let git = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&lib).args(args).output().unwrap()
        };
        git(&["config", "commit.gpgsign", "true"]);
        git(&["config", "gpg.program", "/nonexistent-signer"]);
        let hook = lib.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let msg = write("second body, committed despite the config\n").await;
        assert!(msg.contains("snapshot ") && !msg.contains("NO SNAPSHOT"), "got: {msg}");
    }

    /// EYES b80187af: when snapshotting never-versioned content FAILS, the
    /// write is refused — the only copy of that content stays on disk.
    #[tokio::test]
    async fn a_failed_pre_write_snapshot_refuses_the_write() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let lib = tmp.path().join("library");
        let path = lib.join("projects/bot-hq/notes.md");
        bridge
            .cl_write_file("s1".into(), "hands".into(), "bot-hq".into(), "notes.md".into(), "versioned first body\n".into(), false, false)
            .await
            .unwrap();
        std::fs::write(&path, "OUT-OF-BAND content, never committed\n").unwrap();
        // A stale index lock makes every `git add` fail.
        std::fs::write(lib.join(".git/index.lock"), "").unwrap();
        let err = bridge
            .cl_write_file("s1".into(), "hands".into(), "bot-hq".into(), "notes.md".into(), "the replacement body text\n".into(), false, false)
            .await
            .expect_err("a failed pre-snapshot must refuse the write");
        assert!(err.to_string().contains("nothing was written"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "OUT-OF-BAND content, never committed\n",
            "the unversioned content is intact"
        );
    }

    /// Feedback #44(3): a write to a CL file another session wrote minutes
    /// ago says so; the same session writing again does not.
    #[tokio::test]
    async fn a_write_after_another_sessions_recent_write_warns() {
        let (bridge, _storage, _tmp) = bridge_with_data_dir().await;
        let write = |session: &str, body: &str| {
            let bridge = bridge.clone();
            let (session, body) = (session.to_string(), body.to_string());
            async move {
                bridge
                    .cl_write_file(session, "hands".into(), "bot-hq".into(), "shared-c21.md".into(), body, false, false)
                    .await
                    .unwrap()
            }
        };
        let first = write("s-a", "first version of the shared file\n").await;
        assert!(!first.contains("another session"), "{first}");
        let same = write("s-a", "second version, same session\n").await;
        assert!(!same.contains("another session"), "{same}");
        let other = write("s-b", "third version, another session\n").await;
        assert!(other.contains("another session (s-a)"), "{other}");
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

    /// The nested case the top-level test above could never catch.
    ///
    /// `diary.md` has no separator, so its key matched under every spelling and
    /// the guard always fired. A NESTED key does not: the guard is an exact SQL
    /// match (`get_cl_index`), so before `normalize_cl_path_input` an agent
    /// writing the other separator spelling got `Ok(None)`, the
    /// `if let Some(row)` never fired, the `agent_visible` check was SKIPPED —
    /// and Windows resolved both spellings to the SAME FILE, so the write
    /// landed on a user-hidden file. Silent, because `Ok(None)` is
    /// indistinguishable from the ordinary "new file" case.
    #[tokio::test]
    async fn cl_write_file_refuses_a_hidden_nested_file_under_either_separator() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let dir = tmp.path().join("library/projects/bot-hq/agents/rain");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("diary.md"), "mine").unwrap();
        // Keys are stored `/`-form on every platform — see `util::rel_key`.
        storage
            .upsert_cl_index("bot-hq", "agents/rain/diary.md", "d", None)
            .await
            .unwrap();
        storage
            .set_cl_agent_visibility("bot-hq", "agents/rain/diary.md", false)
            .await
            .unwrap();

        // The canonical spelling must be refused on every platform.
        let mut spellings = vec!["agents/rain/diary.md"];
        // The `\` spelling is only an alias for the same file on WINDOWS. On
        // Unix a backslash is a legal filename character, so `agents\rain\…`
        // names a genuinely different file and must NOT be conflated — which is
        // exactly why `normalize_cl_path_input` is one-directional and
        // `#[cfg(windows)]`-gated.
        if cfg!(windows) {
            spellings.push("agents\\rain\\diary.md");
        }

        for spelling in spellings {
            let err = bridge
                .cl_write_file(
                    "s1".to_string(),
                    "hands".to_string(),
                    "bot-hq".to_string(),
                    spelling.to_string(),
                    "overwrite attempt".to_string(),
                    false,
                    false,
                )
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("user-only"),
                "spelling {spelling:?} must be refused, got: {err}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("diary.md")).unwrap(),
            "mine",
            "the hidden file must be untouched"
        );
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

    /// The shorthand every edit test below uses: one occurrence expected,
    /// no shrink confirmation.
    async fn edit(
        bridge: &Arc<SignalingBridge>,
        file: &str,
        old: &str,
        new: &str,
    ) -> Result<String> {
        bridge
            .cl_edit_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                file.to_string(),
                old.to_string(),
                new.to_string(),
                1,
                false,
            )
            .await
    }

    /// Feedback #30's contract: the correction lands IN PLACE, the file is
    /// not re-emitted, and the reply carries the snapshot like a replace does.
    #[tokio::test]
    async fn cl_edit_file_corrects_a_passage_in_place_and_snapshots() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "## Gotchas\n- the share is unreadable\n- other fact\n").unwrap();

        let msg = edit(&bridge, "notes.md", "the share is unreadable", "the share reads fine since 09-01")
            .await
            .unwrap();
        assert!(msg.starts_with("edited 'notes.md' in project 'bot-hq'"), "got: {msg}");
        assert!(msg.contains("1 occurrence(s) replaced"), "got: {msg}");
        assert!(msg.contains("snapshot "), "the reply names the git snapshot: {msg}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "## Gotchas\n- the share reads fine since 09-01\n- other fact\n"
        );
    }

    /// A count that differs from the expectation changes NOTHING and names
    /// the count, both ways: an anchor that is missing, and one that is not
    /// unique. `expect_occurrences` is the opt-in to replace every one.
    #[tokio::test]
    async fn cl_edit_file_refuses_a_mismatched_count_and_names_it() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        let body = "- staging is the word\n- staging again\n";
        std::fs::write(&path, body).unwrap();

        let err = edit(&bridge, "notes.md", "sandbox", "staging").await.unwrap_err();
        assert!(err.to_string().contains("found 0 occurrence(s)"), "got: {err}");
        assert!(err.to_string().contains("expected 1"), "got: {err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body, "nothing changed");

        let err = edit(&bridge, "notes.md", "staging", "prod").await.unwrap_err();
        assert!(err.to_string().contains("found 2 occurrence(s)"), "got: {err}");
        assert!(err.to_string().contains("expect_occurrences: 2"), "got: {err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body, "nothing changed");

        let msg = bridge
            .cl_edit_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "staging".to_string(),
                "prod".to_string(),
                2,
                false,
            )
            .await
            .unwrap();
        assert!(msg.contains("2 occurrence(s) replaced"), "got: {msg}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "- prod is the word\n- prod again\n"
        );
    }

    /// The same accident shape as a partial replace takes the same flag: an
    /// edit whose result loses more than half the file is refused without
    /// `confirm_shrink` and lands with it.
    #[tokio::test]
    async fn cl_edit_file_shrink_guard_matches_replace() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        let big = "x".repeat(600);
        let original = format!("keep\n{big}\nkeep\n");
        std::fs::write(&path, &original).unwrap();

        let err = edit(&bridge, "notes.md", &big, "").await.unwrap_err();
        assert!(err.to_string().contains(">50% loss"), "got: {err}");
        assert!(err.to_string().contains("cl_edit_file"), "the guard names the in-place tool: {err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

        let msg = bridge
            .cl_edit_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                big.clone(),
                String::new(),
                1,
                true,
            )
            .await
            .unwrap();
        assert!(msg.starts_with("edited"), "got: {msg}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep\n\nkeep\n");
    }

    /// Every refusal `cl_write_file` has reaches the edit path too, plus the
    /// two that are the edit's own: a file that does not exist, and an anchor
    /// that is empty or identical to its replacement.
    #[tokio::test]
    async fn cl_edit_file_refuses_missing_hidden_protected_and_degenerate_edits() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let proj = tmp.path().join("library/projects/bot-hq");

        let err = edit(&bridge, "absent.md", "a", "b").await.unwrap_err();
        assert!(err.to_string().contains("no such file"), "got: {err}");
        assert!(err.to_string().contains("cl_write_file"), "points at the create path: {err}");
        assert!(!proj.join("absent.md").exists(), "an edit never creates");

        std::fs::write(proj.join("diary.md"), "private a\n").unwrap();
        storage.upsert_cl_index("bot-hq", "diary.md", "d", None).await.unwrap();
        storage.set_cl_agent_visibility("bot-hq", "diary.md", false).await.unwrap();
        let err = edit(&bridge, "diary.md", "private", "public").await.unwrap_err();
        assert!(err.to_string().contains("user-only"), "got: {err}");
        assert_eq!(std::fs::read_to_string(proj.join("diary.md")).unwrap(), "private a\n");

        std::fs::write(proj.join("notes.md"), "a body\n").unwrap();
        let err = edit(&bridge, "notes.md", "", "x").await.unwrap_err();
        assert!(err.to_string().contains("old_string is empty"), "got: {err}");
        let err = edit(&bridge, "notes.md", "a body", "a body").await.unwrap_err();
        assert!(err.to_string().contains("identical"), "got: {err}");
        let err = edit(&bridge, "../notes.md", "a", "b").await.unwrap_err();
        assert!(err.to_string().contains("relative CL path"), "got: {err}");

        // A protected `_globals` system file is refused for an edit exactly as
        // for a replace — an agent must not correct its own standing rules.
        let lib = tmp.path().join("library");
        std::fs::write(lib.join("custom-instructions.md"), "rule one\n").unwrap();
        let err = bridge
            .cl_edit_file(
                "s1".to_string(),
                "hands".to_string(),
                "_globals".to_string(),
                "custom-instructions.md".to_string(),
                "rule one".to_string(),
                "rule none".to_string(),
                1,
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("protected"), "got: {err}");
        assert_eq!(std::fs::read_to_string(lib.join("custom-instructions.md")).unwrap(), "rule one\n");
    }

    /// A replace over an existing file whose body cannot be read as text
    /// still writes (advisories skipped, shrink guard on metadata), as it did
    /// before the shared path — while an append or an edit, which need the
    /// body, refuse. Keyed on `exists`, not on the read succeeding: a
    /// replace here must report "replaced", never "created".
    #[tokio::test]
    async fn replace_stays_fail_open_on_an_unreadable_body_and_edit_does_not() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        let mut bytes = vec![0xffu8, 0xfe];
        bytes.extend_from_slice(&[b'x'; 200]);
        std::fs::write(&path, &bytes).unwrap();

        let err = edit(&bridge, "notes.md", "x", "y").await.unwrap_err();
        assert!(err.to_string().contains("before editing"), "got: {err}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "nothing changed");

        // The shrink guard still stands between a tiny body and the file.
        let err = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "tiny".to_string(),
                false,
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains(">50% loss"), "got: {err}");

        let msg = bridge
            .cl_write_file(
                "s1".to_string(),
                "hands".to_string(),
                "bot-hq".to_string(),
                "notes.md".to_string(),
                "z".repeat(150),
                false,
                false,
            )
            .await
            .unwrap();
        assert!(msg.starts_with("replaced"), "an existing file is replaced, not created: {msg}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "z".repeat(150));
    }

    /// The rescan wire: an edit that changes the H1 changes the index
    /// description, because `write_cl` rescans after every write. Deleting
    /// the `cl_rescan` call turns this red.
    #[tokio::test]
    async fn cl_edit_file_rescans_so_the_index_follows_an_edited_heading() {
        let (bridge, storage, tmp) = bridge_with_data_dir().await;
        let path = tmp.path().join("library/projects/bot-hq/notes.md");
        std::fs::write(&path, "# old heading\nbody\n").unwrap();
        bridge.cl_rescan("bot-hq").await.unwrap();
        assert_eq!(
            storage.get_cl_index("bot-hq", "notes.md").await.unwrap().unwrap().description,
            "old heading"
        );

        edit(&bridge, "notes.md", "# old heading", "# new heading").await.unwrap();
        assert_eq!(
            storage.get_cl_index("bot-hq", "notes.md").await.unwrap().unwrap().description,
            "new heading"
        );
    }

    /// The three guards a Replace-only test suite would let an edit skip
    /// (EYES R1, week 35): the 1 MiB cap on the RESULT, the status-flip lint,
    /// and the retired-term diff feeding the close-out sweep.
    #[tokio::test]
    async fn cl_edit_file_result_cap_lint_and_retired_terms_apply() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let proj = tmp.path().join("library/projects/bot-hq");

        // Cap on the result: a small anchor swapped for a body that pushes the
        // file over 1 MiB is refused, and nothing changes.
        let path = proj.join("big.md");
        let body = format!("# big\n{}\nANCHOR\n", "y".repeat(MAX_WRITE_BYTES - 100));
        std::fs::write(&path, &body).unwrap();
        let err = edit(&bridge, "big.md", "ANCHOR", &"z".repeat(200)).await.unwrap_err();
        assert!(err.to_string().contains("1 MiB"), "got: {err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);

        // Status-flip lint: an evidence-free PENDING → RESOLVED edit lands,
        // and the reply carries the advisory.
        std::fs::write(
            proj.join("followups.md"),
            "## Items\n- **#94 rollout** — STILL OWED to the team\n",
        )
        .unwrap();
        let msg = edit(&bridge, "followups.md", "STILL OWED to the team", "DONE, everything landed")
            .await
            .unwrap();
        assert!(msg.starts_with("edited"), "advisory, not a block: {msg}");
        assert!(msg.contains("status-lint") && msg.contains("#94"), "got: {msg}");

        // Retired terms: an edit that deletes a marked term records it, so the
        // close-out sweep reports the file still citing it.
        std::fs::write(proj.join("conventions.md"), "line one\nThe duo maintains this.\n").unwrap();
        std::fs::write(proj.join("vision.md"), "The `duo` (Brian + Rain) is the core.\n").unwrap();
        edit(&bridge, "vision.md", "The `duo` (Brian + Rain)", "The harness").await.unwrap();
        let report = bridge.staleness_sweep("s1").await.expect("the edit retired `duo`");
        assert!(report.contains("conventions.md:2"), "got:\n{report}");
        assert!(report.contains("\"duo\""), "got:\n{report}");
    }

    #[test]
    fn status_flip_lint_flags_evidence_free_upgrades_only() {
        use super::status_flip_warning;
        // The 2026-07-24 shape: #435 flips PENDING→RESOLVED with no evidence.
        let old = "- **#435 delta loop** — PENDING the stakeholder's reply\n- other note\n";
        let new = "- **#435 delta loop** — RESOLVED (keep both)\n- other note\n";
        let warn = status_flip_warning(old, new).expect("evidence-free flip warns");
        assert!(warn.contains("#435"), "got: {warn}");
        assert!(warn.contains("status-lint"), "got: {warn}");

        // Same flip WITH a commit sha beside it: clean.
        let cited = "- **#435 delta loop** — RESOLVED via 43fa153a (register fixed)\n";
        assert!(status_flip_warning(old, cited).is_none());

        // Evidence on the FOLLOWING line also counts.
        let next_line = "- **#435 delta loop** — RESOLVED\n  per the stakeholder, 2026-07-23\n";
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
        // Distinctive-only everywhere (the user's pick a9f8c705, 2026-08-24):
        // a MARKED term reports; a plain unmarked word does not, whatever the
        // edit size. The historic "duo" catch survives through the marking a
        // well-kept CL file gives a real term (backticks here).
        let old = "The `duo` (Brian + Rain) maintains bot-hq. The `duo` is the core. \
                   However, the trio was retired. See tmp-scratch too.";
        let new = "The harness maintains bot-hq. The harness is the core.";
        let terms = retired_terms(old, new);
        // Backticked in the old body → a TERM, and first on occurrence count.
        assert_eq!(terms.first().map(String::as_str), Some("duo"));
        // Code-shaped (hyphen) reports without any marking.
        assert!(terms.contains(&"tmp-scratch".to_string()), "got: {terms:?}");
        // A plain unmarked word is vocabulary now, not a concept — the five
        // generic-English flags at s-a73699ec's close were exactly this class.
        assert!(!terms.contains(&"trio".to_string()), "got: {terms:?}");
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
        // Since the user's pick 4136edff (2026-09-24) a HEADING no longer
        // marks a term: EOD headings ("Tom", "Down", "Report") seeded 609
        // false hits in one close (feedback #38). Backticks still do.
        assert!(
            !terms.iter().any(|t| t == "sandbox"),
            "a heading word is vocabulary, not a term: {terms:?}"
        );
        assert!(terms.iter().any(|t| t == "meta_reconcile_runs"), "got: {terms:?}");
        assert!(terms.iter().any(|t| t == "tmp-prod-logs"), "got: {terms:?}");
        for noise in ["real", "pass", "empty", "wrong", "running", "tests", "php", "walk"] {
            assert!(
                !terms.iter().any(|t| t == noise),
                "prose vocabulary must not report in a bulk rewrite: {noise} in {terms:?}"
            );
        }
        // Since a9f8c705 (2026-08-24) the distinctive filter is unconditional
        // — no under-threshold allowance for plain words remains; the sibling
        // test pins the marked-term path that replaced it.
    }

    /// Feedback #38: filenames and paths are terms; bold and heading words
    /// are not; `e.g.` and version numbers are not files.
    #[test]
    fn filenames_are_terms_and_headings_are_not() {
        let old = "## Tom\n**Down** the line, see `eod.md` and projects/bcc/notes.md. \
                   Also eod-cc.md, e.g. v1.0.6 and 1.2.3.\n";
        let new = "## Tom\nsee eod-cc.md.\n";
        let terms = retired_terms(old, new);
        assert!(terms.contains(&"eod.md".to_string()), "got: {terms:?}");
        assert!(terms.contains(&"projects/bcc/notes.md".to_string()), "got: {terms:?}");
        for noise in ["down", "e.g", "v1.0.6", "1.2.3", "eod"] {
            assert!(!terms.iter().any(|t| t == noise), "{noise} in {terms:?}");
        }
        // EYES 8179f7cc: prose slashes and bare domains are not artifacts.
        for prose in ["and/or", "read/write", "i/p/a/v", "added/fixed/changed", "client/server", "github.com", "claude.ai"] {
            assert!(!is_artifact(prose), "{prose} must not be a filename/path term");
        }
        for real in ["eod.md", "src/core/pump.rs", "~/.bot-hq/library", "projects/bcc/audit", "/tmp/x-dir", "tool-gate.json"] {
            assert!(is_artifact(real), "{real} is a filename/path term");
        }
        assert!(!terms.contains(&"eod-cc.md".to_string()), "still cited in the new body");
    }

    #[test]
    fn a_filename_term_is_cited_only_as_that_file() {
        assert!(line_cites("cp eod.md /tmp/x", "eod.md"));
        assert!(line_cites("see projects/x/eod.md.", "eod.md"));
        assert!(line_cites("(`eod.md`)", "eod.md"));
        assert!(!line_cites("the old-eod.md file", "eod.md"));
        assert!(!line_cites("eod.mdx", "eod.md"));
        assert!(!line_cites("the eod is done", "eod.md"));
        // Words still match whole tokens only.
        assert!(line_cites("The duo maintains it", "duo"));
        assert!(!line_cites("duotone", "duo"));
    }

    /// Feedback #38's real miss: a CL file RENAMED during the session (here
    /// with a bare `mv`, as the reporting session did) left other files citing
    /// its old name. The sweep reports them from the library's git diff since
    /// the session registered.
    #[tokio::test]
    async fn the_sweep_names_files_still_citing_a_renamed_cl_file() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let lib = tmp.path().join("library");
        let proj = lib.join("projects/bot-hq");
        std::fs::write(proj.join("runbook.md"), "Run `grep -n x eod.md` first.\n").unwrap();
        std::fs::write(lib.join("eod.md"), "# EOD\n").unwrap();
        // A first write initialises the library repo; the session registers
        // after it (its HEAD is the baseline).
        bridge
            .cl_write_file("s0".into(), "hands".into(), "bot-hq".into(), "seed.md".into(), "seed body\n".into(), false, false)
            .await
            .unwrap();
        bridge.register_session("s1".into(), Some("bot-hq".into())).await;
        std::fs::rename(lib.join("eod.md"), lib.join("eod-cc.md")).unwrap();
        let report = bridge.staleness_sweep("s1").await.expect("the rename is reported");
        assert!(report.contains("runbook.md:1") && report.contains("\"eod.md\""), "got:\n{report}");
    }

    /// A file this session wrote is skipped for WORD terms (it was just
    /// updated by hand) but never for a filename term (plan review M4); a
    /// term a later write put back into its source file is dropped.
    #[tokio::test]
    async fn own_files_skip_words_but_not_filenames_and_restored_terms_drop() {
        let (bridge, _storage, tmp) = bridge_with_data_dir().await;
        let proj = tmp.path().join("library/projects/bot-hq");
        std::fs::write(proj.join("a.md"), "The `widget-x` is here; see old-plan.md.\n").unwrap();
        std::fs::write(proj.join("b.md"), "Mentions widget-x and old-plan.md.\n").unwrap();
        let write = |file: &str, body: &str| {
            let bridge = bridge.clone();
            let (file, body) = (file.to_string(), body.to_string());
            async move {
                bridge
                    .cl_write_file("s1".into(), "hands".into(), "bot-hq".into(), file, body, false, false)
                    .await
                    .unwrap();
            }
        };
        // a.md drops both terms; the session also rewrites b.md (still citing both).
        write("a.md", "The replacement text for this file, long enough.\n").await;
        write("b.md", "Mentions widget-x and old-plan.md, rewritten.\n").await;
        let report = bridge.staleness_sweep("s1").await.expect("the filename still reports");
        assert!(report.contains("\"old-plan.md\""), "a filename is never skipped:\n{report}");
        assert!(!report.contains("\"widget-x\""), "a word in an own-written file is:\n{report}");

        // A term put back into its own source file is no longer retired.
        let (bridge2, _s2, tmp2) = bridge_with_data_dir().await;
        let proj2 = tmp2.path().join("library/projects/bot-hq");
        std::fs::write(proj2.join("a.md"), "The `gizmo-9` rules.\n").unwrap();
        std::fs::write(proj2.join("c.md"), "gizmo-9 is cited here.\n").unwrap();
        for body in ["Other words entirely, a while.\n", "The `gizmo-9` rules, restored.\n"] {
            bridge2
                .cl_write_file("s2".into(), "hands".into(), "bot-hq".into(), "a.md".into(), body.into(), false, false)
                .await
                .unwrap();
        }
        assert!(bridge2.staleness_sweep("s2").await.is_none(), "a restored term is not stale");
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
        // `duo` is backticked — a marked TERM (distinctive-only everywhere,
        // a9f8c705): an unmarked plain word would no longer record at all.
        std::fs::write(proj.join("vision.md"), "The `duo` (Brian + Rain) is the core.\n").unwrap();

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