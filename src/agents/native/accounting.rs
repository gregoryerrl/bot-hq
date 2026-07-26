//! Durable per-turn context accounting for native agents.
//!
//! ## Why a file
//!
//! `init_logging` (`main.rs`) is `tracing_subscriber::fmt()` to stderr with no
//! file sink, so a `native turn accounting` log line survives exactly as long as
//! the launching terminal's scrollback. The first live native run's numbers were
//! lost to an app restart before they could be read. A measurement with no
//! durable home is not a measurement.
//!
//! Modelled on `policy/violations.rs`: append-only JSONL under `.local/`, a
//! `std::sync::Mutex` around a small blocking append with no await inside, and
//! poison recovery so a writer that panicked mid-append can't wedge the log.
//!
//! ## Why it matters more than the UI meter
//!
//! No provider declares a context window (see `profile.rs` — a window is a
//! per-model fact this repo cannot know), so `ContextUsage` is usually `None` and
//! the meter shows a gap. `used_tokens` needs no denominator, which makes this
//! file the only record of how fast a real session fills — the input to the
//! compaction design.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// One turn's accounting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRecord {
    /// UTC, RFC3339 with millis — matches the rest of bot-hq's timestamps.
    pub ts: String,
    pub session_id: String,
    pub agent: String,
    pub model: String,
    /// `input + cache_read_input + cache_creation_input`. The occupancy figure.
    pub used_tokens: u64,
    /// Conversation length at this turn, so growth in tokens can be read against
    /// growth in messages.
    pub history_messages: usize,
    /// The window, when known. Usually `None` — see the module doc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

pub struct AccountingLog {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl AccountingLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Mutex::new(()),
        }
    }

    /// `<data_dir>/.local/native-accounting.jsonl`, alongside `violations.jsonl`.
    pub fn for_data_dir(data_dir: &Path) -> Self {
        Self::new(data_dir.join(".local").join("native-accounting.jsonl"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, rec: &TurnRecord) -> Result<()> {
        let line = serde_json::to_string(rec).context("serializing turn record")?;
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
            .with_context(|| format!("opening accounting log at {}", self.path.display()))?;
        f.write_all(line.as_bytes())
            .with_context(|| format!("writing record to {}", self.path.display()))?;
        f.write_all(b"\n")
            .with_context(|| format!("writing newline to {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn rec(used: u64) -> TurnRecord {
        TurnRecord {
            ts: "2026-07-26T10:00:00.000Z".into(),
            session_id: "s-1".into(),
            agent: "rain".into(),
            model: "deepseek-v4-pro".into(),
            used_tokens: used,
            history_messages: 3,
            context_window: None,
            stop_reason: Some("end_turn".into()),
        }
    }

    #[test]
    fn appends_one_json_object_per_line() {
        let dir = TempDir::new().unwrap();
        let log = AccountingLog::new(dir.path().join("acct.jsonl"));
        log.append(&rec(100)).unwrap();
        log.append(&rec(250)).unwrap();

        let body = std::fs::read_to_string(log.path()).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: TurnRecord = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first.used_tokens, 100);
        assert_eq!(
            serde_json::from_str::<TurnRecord>(lines[1]).unwrap().used_tokens,
            250
        );
    }

    #[test]
    fn creates_missing_parent_directories() {
        let dir = TempDir::new().unwrap();
        let log = AccountingLog::new(dir.path().join("a").join("b").join("acct.jsonl"));
        log.append(&rec(1)).unwrap();
        assert!(log.path().exists());
    }

    #[test]
    fn an_absent_window_is_omitted_rather_than_written_as_null() {
        let dir = TempDir::new().unwrap();
        let log = AccountingLog::new(dir.path().join("acct.jsonl"));
        log.append(&rec(1)).unwrap();
        let body = std::fs::read_to_string(log.path()).unwrap();
        assert!(!body.contains("context_window"));
        assert!(body.contains("\"used_tokens\":1"));
    }

    #[test]
    fn for_data_dir_sits_beside_the_violations_log() {
        let log = AccountingLog::for_data_dir(Path::new("/tmp/bh"));
        assert_eq!(
            log.path(),
            Path::new("/tmp/bh/.local/native-accounting.jsonl")
        );
    }
}
