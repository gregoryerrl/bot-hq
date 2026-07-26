//! Conversation persistence for native agents.
//!
//! ## Why this is a v1 requirement, not a nicety
//!
//! `spawn_supervised_agent`'s `supervise` respawns a dead incarnation on a
//! transient API error and expects it to pick up where it left off — on the
//! claude-code path that works because `--resume <uuid>` restores the
//! conversation from the CLI's own store. A native loop holds its `messages` in
//! the task, so a respawn would silently restart the agent **amnesiac
//! mid-session**: it keeps talking, with no signal that it forgot everything.
//! That is worse than failing loudly.
//!
//! ## Why a file rather than the DB
//!
//! `SpawnConfig` carries `data_dir` but not `Storage`, and
//! `.local/session-policies/<sid>.yaml` is existing precedent for per-session
//! state under `.local/`. A file also survives an app restart, which is the
//! closer parity with `--resume`.
//!
//! ## Failure posture
//!
//! Every operation here is best-effort. A load failure starts a fresh
//! conversation; a save failure is logged and the turn proceeds. Degrading to
//! today's behaviour is acceptable — failing a turn because a cache write missed
//! is a regression.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Where one agent's conversation lives.
pub fn history_path(data_dir: &Path, session_id: &str, agent: &str) -> PathBuf {
    data_dir
        .join(".local")
        .join("native-history")
        .join(format!("{}-{}.json", sanitize(session_id), sanitize(agent)))
}

/// Keep ids to a filename-safe alphabet. Session ids are `s-<hex>` and agents
/// are `brian`/`rain` today, so this never fires in practice — it exists so a
/// future id containing a separator cannot escape the directory.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Read a persisted conversation. `Ok(None)` when there is no file yet (a first
/// spawn); `Err` only when a file exists and could not be parsed.
pub fn load(path: &Path) -> Result<Option<Vec<Value>>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let history: Vec<Value> =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok((!history.is_empty()).then_some(history))
}

/// Write the conversation, replacing any previous content.
///
/// Writes to a sibling temp file and renames, so a crash mid-write leaves the
/// previous good history rather than a truncated file that would then fail to
/// parse and silently reset the conversation.
pub fn save(path: &Path, history: &[Value]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent for {}", path.display()))?;
    }
    let body = serde_json::to_vec(history).context("serializing conversation history")?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Remove a persisted conversation (session closed, or a deliberate reset).
pub fn clear(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn sample() -> Vec<Value> {
        vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": "hi" }] }),
            json!({ "role": "assistant", "content": [
                { "type": "thinking", "thinking": "hmm", "signature": "sig-abc" },
                { "type": "text", "text": "hello" }
            ]}),
        ]
    }

    #[test]
    fn round_trips_a_conversation() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("h.json");
        save(&p, &sample()).unwrap();
        assert_eq!(load(&p).unwrap().unwrap(), sample());
    }

    #[test]
    fn thinking_signatures_survive_the_round_trip() {
        // Trap 1 again: a signature lost in persistence 400s the request AFTER
        // the respawn, which would look like a native-loop bug rather than a
        // serialization one.
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("h.json");
        save(&p, &sample()).unwrap();
        let back = load(&p).unwrap().unwrap();
        assert_eq!(back[1]["content"][0]["signature"], "sig-abc");
    }

    #[test]
    fn a_missing_file_is_a_fresh_conversation_not_an_error() {
        let dir = TempDir::new().unwrap();
        assert!(load(&dir.path().join("nope.json")).unwrap().is_none());
    }

    #[test]
    fn an_empty_file_is_a_fresh_conversation() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("h.json");
        std::fs::write(&p, "   ").unwrap();
        assert!(load(&p).unwrap().is_none());
    }

    #[test]
    fn an_empty_array_is_a_fresh_conversation() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("h.json");
        save(&p, &[]).unwrap();
        assert!(load(&p).unwrap().is_none());
    }

    #[test]
    fn a_corrupt_file_errors_rather_than_silently_resetting() {
        // The caller decides what to do; silently treating garbage as "fresh"
        // would hide a real problem.
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("h.json");
        std::fs::write(&p, "{not json").unwrap();
        assert!(load(&p).is_err());
    }

    #[test]
    fn save_creates_missing_parents_and_leaves_no_temp_file() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("a").join("b").join("h.json");
        save(&p, &sample()).unwrap();
        assert!(p.exists());
        assert!(!p.with_extension("json.tmp").exists());
    }

    #[test]
    fn save_overwrites_rather_than_appending() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("h.json");
        save(&p, &sample()).unwrap();
        save(&p, &sample()[..1]).unwrap();
        assert_eq!(load(&p).unwrap().unwrap().len(), 1);
    }

    #[test]
    fn path_is_per_session_and_per_agent() {
        let d = Path::new("/tmp/bh");
        assert_eq!(
            history_path(d, "s-abc123", "rain"),
            Path::new("/tmp/bh/.local/native-history/s-abc123-rain.json")
        );
        assert_ne!(
            history_path(d, "s-abc123", "rain"),
            history_path(d, "s-abc123", "brian")
        );
    }

    #[test]
    fn path_separators_in_an_id_cannot_escape_the_directory() {
        let p = history_path(Path::new("/tmp/bh"), "../../etc/x", "rain");
        assert!(p.starts_with("/tmp/bh/.local/native-history"));
        assert!(!p.to_string_lossy().contains(".."));
    }

    #[test]
    fn clear_removes_the_file_and_tolerates_a_missing_one() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("h.json");
        save(&p, &sample()).unwrap();
        clear(&p);
        assert!(!p.exists());
        clear(&p); // idempotent
    }
}
