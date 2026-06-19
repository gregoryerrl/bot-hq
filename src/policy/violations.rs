//! Append-only JSONL log of every enforcement event.
//!
//! Located at `<data_dir>/.local/violations.jsonl`. Replaces the old CL's
//! `discipline-log.md` + `voice-mirror-log.md` with a single structured file.
//!
//! Every approval round writes ONE record, regardless of outcome (approved
//! or denied). That gives us a complete audit trail — not just blocked
//! attempts but also "user approved this push to main on 2026-05-15".

use anyhow::{Context, Result};
use chrono::Utc;
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
        let line =
            serde_json::to_string(&rec).context("serializing violation record to JSON")?;
        // std (not tokio) Mutex: the critical section is a small blocking write
        // with no await inside, so a sync lock is correct and lets sync and
        // async callers share one serialization point. Recover from poison so a
        // writer that panicked mid-append can't permanently wedge the log.
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
        f.write_all(b"\n")
            .with_context(|| format!("writing newline to {}", self.path.display()))?;
        Ok(())
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
            ts: Utc::now().to_rfc3339(),
            session_id: session_id.into(),
            agent: agent.into(),
            kind,
            action: action.into(),
            outcome,
            detail,
        }
    }

    /// Read back the entire log. Lines that fail to parse are skipped (logged
    /// at warn level); intended for the UI's "Recent enforcement events" panel.
    pub fn read_all(&self) -> Result<Vec<ViolationRecord>> {
        let body = match std::fs::read_to_string(&self.path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("reading violations log at {}", self.path.display()))
            }
        };
        let mut out = Vec::new();
        for (i, line) in body.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<ViolationRecord>(trimmed) {
                Ok(r) => out.push(r),
                Err(err) => {
                    tracing::warn!(
                        path = %self.path.display(),
                        line = i + 1,
                        %err,
                        "skipping malformed violations record"
                    );
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn append_then_read_round_trip() {
        let dir = tempdir().unwrap();
        let log = ViolationsLog::new(dir.path());
        log.record(
            "s1",
            "brian",
            ViolationKind::PushGate,
            "git push origin main",
            ViolationOutcome::Approved,
            Some("per_branch_approval".into()),
        )
        .await
        .unwrap();
        log.record(
            "s1",
            "brian",
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
}
