//! Append-only JSONL log of every enforcement event.
//!
//! Located at `<data_dir>/.local/violations.jsonl`. Replaces the old CL's
//! `discipline-log.md` + `voice-mirror-log.md` with a single structured file.
//!
//! Every approval round writes ONE record, regardless of outcome (approved
//! or denied). That gives us a complete audit trail — not just blocked
//! attempts but also "user approved this push to main on 2026-05-15".

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    /// `git push` intercepted by push_gate policy.
    PushGate,
    /// `git commit` proposed message contained a forbidden word.
    CommitGrep,
    /// `git push --force` / `--force-with-lease`.
    ForcePush,
    /// Tool Gate / `action_gate` approval (legacy wire name `tool_blocklist`,
    /// kept for back-compat; the per-project `tool_blocklist` it was named for
    /// is retired — see the Tool Gate).
    ToolBlocklist,
    /// Bash command matched `per_action_approval`.
    PerAction,
    /// Generic agent-initiated approval request (free-form).
    GenericApproval,
    /// A policy.yaml file was modified outside the Settings UI flow.
    /// Audit-only in v1 — we log but don't block (yet). Catches the
    /// "agent edits policy.yaml to remove forbidden words" attack.
    PolicyMutation,
    /// A `git commit` / `git push` blocked by the EYES-sign-off gate — HANDS
    /// tried to ship with unresolved EYES `blocking` findings. Logged Denied by
    /// the pre-commit / pre-push hook on a block.
    Findings,
}

impl ViolationKind {
    /// The kinds an AGENT may request approval for through `request_approval`
    /// — the tool's `kind` enum is derived from this list (`serde` names), so
    /// the descriptor cannot drift from the parser. `PolicyMutation` (audit
    /// only) and `Findings` (a hook verdict) are not requestable.
    pub const REQUESTABLE: [ViolationKind; 6] = [
        ViolationKind::PushGate,
        ViolationKind::CommitGrep,
        ViolationKind::ForcePush,
        ViolationKind::ToolBlocklist,
        ViolationKind::PerAction,
        ViolationKind::GenericApproval,
    ];

    /// The serde wire name (`snake_case`), the same string `parse_violation_kind`
    /// accepts.
    pub fn wire_name(self) -> String {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::String(s)) => s,
            _ => format!("{self:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ViolationOutcome {
    Approved,
    Denied,
    /// User dismissed/canceled before deciding (e.g., closed dialog).
    Abandoned,
    /// Audit-only — we observed an event but didn't ask for approval.
    /// Used for PolicyMutation entries (no user prompt; just logged).
    Detected,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ViolationRecord {
    pub ts: String,
    pub session_id: String,
    pub agent: String,
    pub kind: ViolationKind,
    pub action: String,
    pub outcome: ViolationOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Append-only writer. Cheap to clone (Arc); safe to share across tasks.
/// Roll the violations log over at this size.
///
/// 4 MB is ~20k records at the shape these have — years of a normal install,
/// days of a pathological one — and is the point past which `read_all`'s
/// parse-the-whole-file cost is noticeable on a panel open.
const ROTATE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct ViolationsLog {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl ViolationsLog {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: crate::paths::Paths::for_data_dir(data_dir.to_path_buf()).violations_path,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record. Serializes to a single line; tolerates concurrent
    /// callers via an internal mutex so two writes can't interleave bytes.
    ///
    /// The body is a tiny blocking file append, so this async method just wraps
    /// the synchronous [`append_blocking`](Self::append_blocking). Sync callers
    /// — notably the policy-mutation audit, which runs both inside the app's
    /// runtime and in a hookless subprocess — call the blocking form directly
    /// rather than building a nested runtime (which panics inside a runtime).
    pub async fn append(&self, rec: ViolationRecord) -> Result<()> {
        self.append_blocking(rec)
    }

    /// Synchronous sibling of [`append`](Self::append). Safe in any context,
    /// with or without a tokio runtime present.
    pub fn append_blocking(&self, rec: ViolationRecord) -> Result<()> {
        // Record AND newline in ONE buffer, written with ONE `write_all` (G5).
        //
        // This used to be two `write_all` calls. `write_lock` below is a
        // `std::sync::Mutex`, which serializes appends **within one process** —
        // and this log has several writers that are separate PROCESSES: every
        // git hook is its own `bot-hq` subprocess (five `ViolationsLog::new`
        // sites in `policy::hooks`) alongside the app's own (`main.rs`). No
        // in-process lock reaches across that, so two hooks firing near each
        // other could interleave as `{j1}{j2}\n\n` — one unparseable line plus
        // an empty one, so `read_all` skipped it and **both** records vanished
        // from the trail that proves the gates fired.
        //
        // `O_APPEND` is what makes a single `write` atomic with respect to the
        // file offset; that is precisely why the bug was that a record took two
        // of them. One buffer, one call, and concurrent writers can only ever
        // produce whole lines.
        let mut line =
            serde_json::to_string(&rec).context("serializing violation record to JSON")?;
        line.push('\n');
        // std (not tokio) Mutex: the critical section is a small blocking write
        // with no await inside, so a sync lock is correct and lets sync and
        // async callers share one serialization point. Recover from poison so a
        // writer that panicked mid-append can't permanently wedge the log. It
        // still earns its keep for same-process concurrency (it serializes the
        // open/write/rotate sequence); it simply cannot be the whole answer.
        let _g = self
            .write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent for {}", self.path.display()))?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("opening violations log at {}", self.path.display()))?;
        f.write_all(line.as_bytes())
            .with_context(|| format!("writing violation to {}", self.path.display()))?;
        // Rotate AFTER the write, still under the lock, so the file the reader
        // opens is always a whole one.
        self.rotate_if_oversized(&f);
        Ok(())
    }

    /// One rollover, kept next to the live file (E4).
    ///
    /// The log had no rotation at all and `read_all` parses ALL of it on every
    /// Violations-panel open — an audit trail that only grows, read whole, on a
    /// UI click. Rolling at [`ROTATE_BYTES`] keeps one generation of history:
    /// enough that a rotation mid-incident does not lose the incident, bounded
    /// enough that the panel cannot be made slow by age alone.
    ///
    /// Best-effort by construction. A failed rotation must never fail the
    /// APPEND — the record is the point, and the append has already succeeded by
    /// the time this runs; the cost of a missed rollover is a large file, the
    /// cost of a propagated error is a lost violation.
    fn rotate_if_oversized(&self, f: &std::fs::File) {
        let too_big = f.metadata().map(|m| m.len() > ROTATE_BYTES).unwrap_or(false);
        if !too_big {
            return;
        }
        let rolled = self.rolled_path();
        if let Err(e) = std::fs::rename(&self.path, &rolled) {
            tracing::warn!(
                path = %self.path.display(),
                ?e,
                "violations log rotation failed; it keeps growing"
            );
            return;
        }
        tracing::info!(
            path = %self.path.display(),
            rolled = %rolled.display(),
            "violations log rotated"
        );
    }

    /// Convenience: build + append in one call.
    pub async fn record(
        &self,
        session_id: impl Into<String>,
        agent: impl Into<String>,
        kind: ViolationKind,
        action: impl Into<String>,
        outcome: ViolationOutcome,
        detail: Option<String>,
    ) -> Result<()> {
        self.append(Self::build_record(
            session_id, agent, kind, action, outcome, detail,
        ))
        .await
    }

    /// Synchronous sibling of [`record`](Self::record).
    pub fn record_blocking(
        &self,
        session_id: impl Into<String>,
        agent: impl Into<String>,
        kind: ViolationKind,
        action: impl Into<String>,
        outcome: ViolationOutcome,
        detail: Option<String>,
    ) -> Result<()> {
        self.append_blocking(Self::build_record(
            session_id, agent, kind, action, outcome, detail,
        ))
    }

    fn build_record(
        session_id: impl Into<String>,
        agent: impl Into<String>,
        kind: ViolationKind,
        action: impl Into<String>,
        outcome: ViolationOutcome,
        detail: Option<String>,
    ) -> ViolationRecord {
        ViolationRecord {
            ts: crate::storage::now_utc(),
            session_id: session_id.into(),
            agent: agent.into(),
            kind,
            action: action.into(),
            outcome,
            detail,
        }
    }

    /// Where [`Self::rotate_if_oversized`] moves the previous generation.
    fn rolled_path(&self) -> PathBuf {
        self.path.with_extension("jsonl.1")
    }

    /// Read back the entire log — **both generations**. Lines that fail to parse
    /// are skipped (logged at warn level); intended for the UI's "Recent
    /// enforcement events" panel.
    ///
    /// Round-2 audit G1: this used to read `self.path` alone, so the rollover
    /// that shipped to *preserve* one generation of history made that history
    /// unreachable instead. Nothing else in the tree ever opened `.jsonl.1`, and
    /// both consumers of this method at the time — the Violations panel
    /// (`tauri_cmd/policy.rs`) and the external driver (`external_jsonrpc.rs`, deleted 2026-08-17)
    /// — went through here, so a rotation emptied the audit
    /// trail from every surface a user or driver had. That was strictly worse
    /// than the no-rotation state it replaced. The panel is the only consumer
    /// now, and the argument holds for it alone.
    ///
    /// **Rolled generation first.** Rotation renames the live file aside *after*
    /// the append, so `.jsonl.1` holds the OLDER records and chronological order
    /// needs them ahead of the fresh file. Both consumers reverse before capping
    /// (`ViolationsPanel.tsx` reverses the whole list; the deleted `external_jsonrpc.rs`
    /// reversed then truncated to `limit`), so older records land
    /// where a truncate drops them rather than at the head of the panel.
    ///
    /// Cost: a panel open now parses up to `2 * ROTATE_BYTES` instead of
    /// `ROTATE_BYTES`. That is the price of the history being readable at all,
    /// and it is bounded by rotation — which is what `ROTATE_BYTES` is for.
    pub fn read_all(&self) -> Result<Vec<ViolationRecord>> {
        let mut out = Vec::new();
        self.read_generation(&self.rolled_path(), &mut out)?;
        self.read_generation(&self.path, &mut out)?;
        Ok(out)
    }

    /// Parse one generation onto `out`. A missing file is not an error — the
    /// rolled one does not exist until the first rotation, and the live one does
    /// not exist until the first violation.
    fn read_generation(&self, path: &Path, out: &mut Vec<ViolationRecord>) -> Result<()> {
        let body = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("reading violations log at {}", path.display()))
            }
        };
        for (i, line) in body.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<ViolationRecord>(trimmed) {
                Ok(r) => out.push(r),
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        line = i + 1,
                        %err,
                        "skipping malformed violations record"
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// **The audit trail rolls over** (E4): it had no rotation at all, and
    /// `read_all` parses the whole file on every Violations-panel open.
    #[tokio::test]
    async fn an_oversized_log_rolls_over_and_keeps_recording() {
        let dir = tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let path = dir.path().join(".local").join("violations.jsonl");

        // A file already past the threshold, then one ordinary append.
        //
        // The filler is newline-TERMINATED, which the original fixture was not.
        // Without it the appended record concatenates onto the filler's open
        // last line and the whole thing is one unparseable line — invisible
        // while this test only substring-matched the raw file, and the first
        // thing the `read_all` assertion below caught. Production always ends a
        // record with `\n`, so a terminated file is the realistic shape; the
        // filler line itself stays malformed on purpose and is skipped, which is
        // the documented tolerance (`malformed_line_is_skipped`).
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut filler = vec![b'x'; ROTATE_BYTES as usize];
        filler.push(b'\n');
        std::fs::write(&path, filler).unwrap();
        log.record(
            "s1",
            "hands",
            ViolationKind::PushGate,
            "git push origin main",
            ViolationOutcome::Approved,
            None,
        )
        .await
        .unwrap();

        assert!(
            path.with_extension("jsonl.1").exists(),
            "the oversized log was not rolled aside"
        );
        // The record that triggered the roll is IN the rolled file, not lost:
        // rotation happens after the append, so nothing is written to a file
        // that is about to be renamed away.
        //
        // Read through `read_all`, NOT `std::fs::read_to_string` — that is the
        // whole of round-2 audit G1. The first version of this assertion opened
        // the rolled file directly, which proves the bytes survived on disk
        // while saying nothing about whether any consumer can still see them.
        // They could not: `read_all` is the only path the Violations panel and
        // the external driver have, and it read the live file alone.
        let rolled = log.read_all().unwrap();
        assert!(
            rolled.iter().any(|r| r.action == "git push origin main"),
            "the rolled generation is unreachable through the only reader \
             consumers have: {rolled:?}"
        );

        // And the log keeps working: the next record starts the fresh file,
        // and BOTH generations come back, oldest first.
        log.record(
            "s1",
            "hands",
            ViolationKind::PushGate,
            "git push --force",
            ViolationOutcome::Denied,
            None,
        )
        .await
        .unwrap();
        let both = log.read_all().unwrap();
        assert_eq!(
            both.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(),
            vec!["git push origin main", "git push --force"],
            "both generations, chronological — the rolled one first"
        );
    }

    #[tokio::test]
    async fn append_then_read_round_trip() {
        let dir = tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        log.record(
            "s1",
            "hands",
            ViolationKind::PushGate,
            "git push origin main",
            ViolationOutcome::Approved,
            Some("per_branch_approval".into()),
        )
        .await
        .unwrap();
        log.record(
            "s1",
            "hands",
            ViolationKind::CommitGrep,
            "git commit",
            ViolationOutcome::Denied,
            Some("forbidden word 'bot-hq'".into()),
        )
        .await
        .unwrap();
        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].kind, ViolationKind::PushGate);
        assert_eq!(recs[0].outcome, ViolationOutcome::Approved);
        assert_eq!(recs[1].kind, ViolationKind::CommitGrep);
        assert_eq!(recs[1].outcome, ViolationOutcome::Denied);
    }

    #[tokio::test]
    async fn empty_file_reads_as_empty_vec() {
        let dir = tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        let recs = log.read_all().unwrap();
        assert!(recs.is_empty());
    }

    #[tokio::test]
    async fn malformed_line_is_skipped() {
        let dir = tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        // Pre-populate with a mix of valid + junk (log now lives under .local/).
        std::fs::create_dir_all(log.path().parent().unwrap()).unwrap();
        std::fs::write(
            log.path(),
            "not json\n{\"ts\":\"2026-01-01T00:00:00Z\",\"session_id\":\"s\",\"agent\":\"a\",\"kind\":\"push_gate\",\"action\":\"x\",\"outcome\":\"approved\"}\n",
        )
        .unwrap();
        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
    }

    /// **Concurrent writers cannot merge two records into one line** (G5).
    ///
    /// Every thread builds its OWN `ViolationsLog`, and that is the entire
    /// design of this test: `write_lock` is a per-instance `std::sync::Mutex`,
    /// so N instances over one path hold N independent locks — which is exactly
    /// the shape of the N separate `bot-hq` processes that really write this
    /// file (five `ViolationsLog::new` sites across the git hooks, plus the
    /// app's own). Sharing one instance would serialize the writers and prove
    /// nothing about the case that actually loses records.
    ///
    /// Before the fix a record was two `write_all` calls, so two writers could
    /// interleave to `{j1}{j2}\n\n`: one unparseable line, both records gone.
    /// The assertion is a COUNT rather than a shape check because that is the
    /// user-visible harm — the audit trail is what proves a gate fired, and a
    /// merge deletes two proofs at once.
    #[test]
    fn concurrent_writers_never_merge_records() {
        const THREADS: usize = 8;
        const PER_THREAD: usize = 50;
        let dir = tempdir().unwrap();
        let root = dir.path();

        std::thread::scope(|s| {
            for t in 0..THREADS {
                s.spawn(move || {
                    // Own instance == own lock == a separate process's posture.
                    let log = ViolationsLog::new(root);
                    for i in 0..PER_THREAD {
                        log.append_blocking(ViolationsLog::build_record(
                            "s1",
                            "hands",
                            ViolationKind::PushGate,
                            format!("cmd-{t}-{i}"),
                            ViolationOutcome::Denied,
                            None,
                        ))
                        .unwrap();
                    }
                });
            }
        });

        let recs = ViolationsLog::new(root).read_all().unwrap();
        assert_eq!(
            recs.len(),
            THREADS * PER_THREAD,
            "records were merged or lost by concurrent appends: got {} of {}",
            recs.len(),
            THREADS * PER_THREAD
        );
    }
}
