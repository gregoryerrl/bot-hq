//! Append-only JSONL log of every enforcement event.
//!
//! Located at `<data_dir>/violations.jsonl`. Replaces the old CL's
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
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    /// `git push` intercepted by push_gate policy.
    PushGate,
    /// `git commit` proposed message contained a forbidden word.
    CommitGrep,
    /// `git push --force` / `--force-with-lease`.
    ForcePush,
    /// Bash command matched `tool_blocklist`.
    ToolBlocklist,
    /// Bash command matched `per_action_approval`.
    PerAction,
    /// Generic agent-initiated approval request (free-form).
    GenericApproval,
    /// A policy.yaml file was modified outside the Settings UI flow.
    /// Audit-only in v1 — we log but don't block (yet). Catches the
    /// "agent edits policy.yaml to remove forbidden words" attack.
    PolicyMutation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
            path: data_dir.join("violations.jsonl"),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record. Serializes to a single line; tolerates concurrent
    /// callers via an internal mutex so two writes can't interleave bytes.
    pub async fn append(&self, rec: ViolationRecord) -> Result<()> {
        let line =
            serde_json::to_string(&rec).context("serializing violation record to JSON")?;
        let _g = self.write_lock.lock().await;
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
        let rec = ViolationRecord {
            ts: Utc::now().to_rfc3339(),
            session_id: session_id.into(),
            agent: agent.into(),
            kind,
            action: action.into(),
            outcome,
            detail,
        };
        self.append(rec).await
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
        // Pre-populate with a mix of valid + junk.
        std::fs::write(
            log.path(),
            "not json\n{\"ts\":\"2026-01-01T00:00:00Z\",\"session_id\":\"s\",\"agent\":\"a\",\"kind\":\"push_gate\",\"action\":\"x\",\"outcome\":\"approved\"}\n",
        )
        .unwrap();
        let recs = log.read_all().unwrap();
        assert_eq!(recs.len(), 1);
    }
}
