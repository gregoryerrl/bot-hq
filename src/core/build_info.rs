//! What build is running, and whether its program file has changed since
//! launch — the footer's version stamp and restart chip (the user's
//! 2026-09-25 ask).
//!
//! The restart chip matters here more than in most apps: every registered
//! project's git hooks exec the program file the app was LAUNCHED from, so a
//! rebuild without a relaunch leaves the hooks on the new code and the app on
//! the old — the two halves of the policy layer disagreeing, silently.

use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

/// The program file as it stood at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExeStamp {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub len: u64,
    /// The inode, where the platform has one (macOS, Linux). Windows has no
    /// portable equivalent in std, so there the stamp is mtime and size.
    pub ino: Option<u64>,
}

impl ExeStamp {
    /// `None` when the file cannot be read (gone, or unreadable).
    pub fn read(path: &Path) -> Option<ExeStamp> {
        let md = std::fs::metadata(path).ok()?;
        Some(ExeStamp {
            path: path.to_path_buf(),
            modified: md.modified().ok(),
            len: md.len(),
            ino: ino_of(&md),
        })
    }
}

#[cfg(unix)]
fn ino_of(md: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(md.ino())
}

#[cfg(not(unix))]
fn ino_of(_: &std::fs::Metadata) -> Option<u64> {
    None
}

static LAUNCH: OnceLock<Option<ExeStamp>> = OnceLock::new();

/// Record the program file as it stands now. Called once, at the start of
/// the GUI launch; later calls change nothing.
pub fn record_launch_stamp() {
    LAUNCH.get_or_init(|| std::env::current_exe().ok().and_then(|p| ExeStamp::read(&p)));
}

/// The program file as it was at launch; `None` if it could not be read then.
pub fn launch_stamp() -> Option<&'static ExeStamp> {
    LAUNCH.get().and_then(|s| s.as_ref())
}

/// Where the program file stands against the one that was launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExeState {
    /// Unchanged since launch — or unknowable (no launch stamp, e.g. inside a
    /// read-only AppImage mount), which shows nothing rather than a guess.
    Current,
    /// Rebuilt or replaced since launch: git hooks already run the new file,
    /// so restart the app to match it.
    Changed,
    /// Gone from disk (a `cargo clean`): nothing can run it, so every commit
    /// fails until a rebuild — restarting alone does not help.
    Missing,
}

/// **Any** difference counts, not just a newer mtime (EYES F3): a file copied
/// in with its old timestamp (`cp -p`) is still a different program.
pub fn exe_state(launch: Option<&ExeStamp>) -> ExeState {
    let Some(launch) = launch else {
        return ExeState::Current;
    };
    match ExeStamp::read(&launch.path) {
        None => ExeState::Missing,
        Some(now) if now == *launch => ExeState::Current,
        Some(_) => ExeState::Changed,
    }
}

/// The commit this binary was built from, when its build said so:
/// `BOTHQ_BUILD_COMMIT` at compile time. `./start` sets it on the release
/// cargo command only, and the release workflow sets it to the workflow's
/// commit. Debug and test builds never see it, so a commit never forces
/// `cargo test` to recompile.
pub fn build_commit() -> Option<String> {
    short_commit(option_env!("BOTHQ_BUILD_COMMIT"))
}

/// Seven hex characters of a full SHA, keeping a `-dirty` suffix; anything
/// else passes through trimmed, and an empty value is `None`.
pub fn short_commit(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let (sha, dirty) = match raw.strip_suffix("-dirty") {
        Some(sha) => (sha, true),
        None => (raw, false),
    };
    let short = if sha.len() > 7 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        &sha[..7]
    } else {
        sha
    };
    Some(if dirty { format!("{short}-dirty") } else { short.to_string() })
}

/// `release` or `debug` — which of the two the running binary is.
pub fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_commit_keeps_seven_hex_and_the_dirty_mark() {
        assert_eq!(
            short_commit(Some("9bb7e53c0ffee0123456789abcdef0123456789a")).as_deref(),
            Some("9bb7e53"),
            "the release workflow passes the full sha"
        );
        assert_eq!(short_commit(Some("9bb7e53-dirty")).as_deref(), Some("9bb7e53-dirty"));
        assert_eq!(short_commit(Some(" 9bb7e53\n")).as_deref(), Some("9bb7e53"));
        assert_eq!(short_commit(Some("")), None);
        assert_eq!(short_commit(None), None);
    }

    #[test]
    fn exe_state_flags_any_change_and_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("bot-hq");
        std::fs::write(&exe, b"build one").unwrap();
        let launch = ExeStamp::read(&exe).unwrap();
        assert_eq!(exe_state(Some(&launch)), ExeState::Current);
        assert_eq!(exe_state(None), ExeState::Current, "no launch stamp: nothing to report");

        // A different program with the SAME timestamp — `cp -p` — still counts.
        std::fs::write(&exe, b"build two, longer").unwrap();
        let f = std::fs::File::options().write(true).open(&exe).unwrap();
        f.set_modified(launch.modified.unwrap()).unwrap();
        drop(f);
        assert_eq!(exe_state(Some(&launch)), ExeState::Changed);

        std::fs::remove_file(&exe).unwrap();
        assert_eq!(exe_state(Some(&launch)), ExeState::Missing, "a cargo clean: rebuild");
    }
}
