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
//! One divergence from that model, learned the hard way: `ViolationsLog` is
//! constructed ONCE in `main.rs` and shared, so its mutex genuinely serializes.
//! This log used to be constructed per spawned agent — a private mutex each,
//! over the same file — which is no protection at all across agents. Two layers
//! now hold instead: [`AccountingLog::shared_for_data_dir`] hands every spawn
//! the same instance (so the mutex means what it says), and [`append`] writes
//! the record and its newline as ONE `write_all` (so even an unshared instance,
//! as tests create, cannot interleave records under `O_APPEND`).
//!
//! [`append`]: AccountingLog::append
//!
//! The file is unbounded ON PURPOSE: it is the measurement input the B6
//! compaction design depends on, and rotating it would discard exactly the
//! long-horizon growth curve being measured. Delete it freely; every line is
//! self-contained.
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
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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

    /// The shared instance for a data dir — what spawn paths must use.
    ///
    /// `spawn_native_agent` runs once per agent and carries no app-wide state
    /// (it takes only a `SpawnConfig`), so without this registry every agent
    /// constructed its own instance: a private mutex each, all appending to the
    /// same file. The mutex looked like protection and provided none across
    /// agents. Keyed by the resolved log path so distinct data dirs (tests)
    /// stay isolated.
    pub fn shared_for_data_dir(data_dir: &Path) -> Arc<Self> {
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<AccountingLog>>>> = OnceLock::new();
        let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
        let mut map = registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let log = Self::for_data_dir(data_dir);
        Arc::clone(map.entry(log.path.clone()).or_insert_with(|| Arc::new(log)))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, rec: &TurnRecord) -> Result<()> {
        // Record + newline in ONE buffer, written by ONE call. Two `write_all`s
        // left a window where another instance's append landed between a record
        // and its newline, producing two objects on one line — unparseable
        // JSONL. Under `O_APPEND` a single small write lands contiguously, so
        // this holds even across instances the mutex below cannot see.
        let mut line = serde_json::to_string(rec).context("serializing turn record")?;
        line.push('\n');
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

    #[test]
    fn shared_for_data_dir_hands_every_spawn_the_same_instance() {
        // A per-spawn instance means a private mutex each over the same file —
        // which serializes nothing. Same data dir must mean same instance.
        let dir = TempDir::new().unwrap();
        let a = AccountingLog::shared_for_data_dir(dir.path());
        let b = AccountingLog::shared_for_data_dir(dir.path());
        assert!(Arc::ptr_eq(&a, &b), "two spawns got two instances");

        let other = TempDir::new().unwrap();
        let c = AccountingLog::shared_for_data_dir(other.path());
        assert!(!Arc::ptr_eq(&a, &c), "distinct data dirs must stay isolated");
    }

    #[test]
    fn concurrent_appends_from_unshared_instances_stay_line_atomic() {
        // The second safety layer, for writers the registry cannot see (or a
        // future second process): record + newline go down in ONE write, so
        // O_APPEND keeps each line contiguous. Two UNshared instances hammering
        // the same file must still produce parseable JSONL.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("acct.jsonl");
        let a = AccountingLog::new(&path);
        let b = AccountingLog::new(&path);

        std::thread::scope(|s| {
            for log in [&a, &b] {
                s.spawn(move || {
                    for i in 0..100 {
                        log.append(&rec(i)).unwrap();
                    }
                });
            }
        });

        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 200);
        for line in lines {
            serde_json::from_str::<TurnRecord>(line)
                .unwrap_or_else(|e| panic!("interleaved record: {e}: {line}"));
        }
    }
}
