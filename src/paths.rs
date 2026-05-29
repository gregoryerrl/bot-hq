//! Data-dir resolution + first-run init.
//!
//! Layout under `<data_dir>` (default `~/.bot-hq/`, overridable via
//! `BOT_HQ_DATA_DIR`):
//!
//! ```text
//! <data_dir>/
//!   cl-version.txt                 ("1" for v1)
//!   custom-general-rules.md        (optional user additions; hardcoded core
//!                                   lives in agents::general_rules)
//!   agents/<name>/custom-instruction.md  (emma, brian, rain — user tweaks)
//!   projects/<p>/conventions.md
//!   projects/<p>/notes.md
//!   mcp-token                      (external MCP bearer token, 0600)
//!   violations.jsonl               (policy audit trail)
//!   .local/
//!     bot-hq.db
//!     lock                         (single-instance PID lock)
//!     session-permissions/<sid>.json  (per-session grant mirrors)
//! ```

use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Outcome of [`Paths::init`]. Used by the UI layer to surface one-time toasts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitOutcome {
    /// Data dir didn't exist; we created it from baked-in defaults.
    FirstRun,
    /// Data dir exists and was complete. No CL writes performed.
    Existing,
    /// Data dir existed but was missing the version file and/or one of the
    /// required CL slots. We re-initialized the missing pieces. The list of
    /// slot names that were filled is included so the UI can name them.
    Repaired { repaired_slots: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub local_dir: PathBuf,
    pub db_path: PathBuf,
    pub lock_path: PathBuf,
    pub cl_version_path: PathBuf,
    /// Bearer token for the external MCP server. Generated on first run as a
    /// UUIDv4 with 0o600 perms. Persists across restarts; user can rotate by
    /// editing the file and restarting bot-hq.
    pub mcp_token_path: PathBuf,
}

impl Paths {
    /// Compute paths from environment. Respects `BOT_HQ_DATA_DIR`. Falls back
    /// to `~/.bot-hq/`. Expands a leading `~` segment.
    pub fn from_env() -> Result<Self> {
        let data_dir = match std::env::var("BOT_HQ_DATA_DIR") {
            Ok(v) if !v.trim().is_empty() => expand_tilde(v.trim())?,
            _ => default_data_dir()?,
        };
        Ok(Self::for_data_dir(data_dir))
    }

    pub fn for_data_dir(data_dir: PathBuf) -> Self {
        let local_dir = data_dir.join(".local");
        let db_path = local_dir.join("bot-hq.db");
        let lock_path = local_dir.join("lock");
        let cl_version_path = data_dir.join("cl-version.txt");
        let mcp_token_path = data_dir.join("mcp-token");
        Self {
            data_dir,
            local_dir,
            db_path,
            lock_path,
            cl_version_path,
            mcp_token_path,
        }
    }

    /// Read the persisted external-MCP bearer token. Trims trailing whitespace
    /// so a UUID written with a trailing newline still matches incoming
    /// `Authorization: Bearer <token>` headers. Returns an error if the file
    /// is missing — call `init()` first.
    pub fn read_mcp_token(&self) -> Result<String> {
        let raw = fs::read_to_string(&self.mcp_token_path)
            .with_context(|| format!("reading mcp-token at {}", self.mcp_token_path.display()))?;
        Ok(raw.trim().to_string())
    }

    /// Idempotent. Creates the data dir + CL skeleton on first run, repairs
    /// missing required CL slots on subsequent runs, leaves user content
    /// untouched otherwise. Returns the outcome for UI toasts.
    ///
    /// First-run signal: `cl-version.txt` doesn't exist yet. A brand-new
    /// installation, or one where the user wiped the data dir clean, gets
    /// `FirstRun` and a silent init. A user who deleted just one slot gets
    /// `Repaired { slots }` so the UI can name it.
    pub fn init(&self) -> Result<InitOutcome> {
        let first_run = !self.cl_version_path.exists();

        fs::create_dir_all(&self.data_dir)
            .with_context(|| format!("creating data dir at {}", self.data_dir.display()))?;
        fs::create_dir_all(&self.local_dir)
            .with_context(|| format!("creating local dir at {}", self.local_dir.display()))?;

        let mut repaired_slots = Vec::new();

        if !self.cl_version_path.exists() {
            fs::write(&self.cl_version_path, "1\n")
                .with_context(|| format!("writing {}", self.cl_version_path.display()))?;
        }

        for (path, body) in default_cl_files(&self.data_dir) {
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                fs::write(&path, body)
                    .with_context(|| format!("writing default CL slot at {}", path.display()))?;
                if !first_run {
                    if let Ok(rel) = path.strip_prefix(&self.data_dir) {
                        repaired_slots.push(rel.display().to_string());
                    }
                }
            }
        }

        // mcp-token: generate UUIDv4 on first run / if missing. 0o600 perms on
        // Unix so other users on the machine can't read it. Idempotent — if the
        // file exists, leave it alone (user might have rotated).
        if !self.mcp_token_path.exists() {
            let token = uuid::Uuid::new_v4().to_string();
            fs::write(&self.mcp_token_path, format!("{token}\n"))
                .with_context(|| format!("writing mcp-token at {}", self.mcp_token_path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = fs::Permissions::from_mode(0o600);
                fs::set_permissions(&self.mcp_token_path, perms).with_context(|| {
                    format!("0o600 perms on {}", self.mcp_token_path.display())
                })?;
            }
            if !first_run {
                repaired_slots.push("mcp-token".to_string());
            }
        }

        if first_run {
            info!(data_dir = %self.data_dir.display(), "first-run init complete");
            Ok(InitOutcome::FirstRun)
        } else if repaired_slots.is_empty() {
            Ok(InitOutcome::Existing)
        } else {
            warn!(slots = ?repaired_slots, "repaired missing CL slots");
            Ok(InitOutcome::Repaired { repaired_slots })
        }
    }
}

/// Resolve the user's home directory via `directories::BaseDirs`. Single
/// source of truth — every path helper below routes through this so the
/// `.context()` message stays identical.
fn home_dir() -> Result<PathBuf> {
    Ok(directories::BaseDirs::new()
        .context("locating user home dir")?
        .home_dir()
        .to_path_buf())
}

/// Default data dir = `~/.bot-hq/`.
fn default_data_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".bot-hq"))
}

/// Expand a leading `~` (and optionally `~/`) in a path string.
fn expand_tilde(s: &str) -> Result<PathBuf> {
    if let Some(stripped) = s.strip_prefix("~/") {
        Ok(home_dir()?.join(stripped))
    } else if s == "~" {
        home_dir()
    } else {
        Ok(PathBuf::from(s))
    }
}

/// The required CL slots and their baked-in default contents.
///
/// Universal rules are no longer here — they moved into the binary as the
/// `agents::general_rules::GENERAL_RULES` constant. What lives in CL now is:
///
/// - `custom-general-rules.md` — optional user additions appended to the
///   hardcoded universal rules at session spawn
/// - `agents/<name>/custom-instruction.md` — empty placeholders the user can
///   fill in to tweak each agent without touching role identity
fn default_cl_files(root: &Path) -> Vec<(PathBuf, &'static str)> {
    vec![
        (
            root.join("custom-general-rules.md"),
            include_str!("../templates/cl/custom-general-rules.md"),
        ),
        (
            root.join("agents/emma/custom-instruction.md"),
            include_str!("../templates/cl/agents/emma/custom-instruction.md"),
        ),
        (
            root.join("agents/brian/custom-instruction.md"),
            include_str!("../templates/cl/agents/brian/custom-instruction.md"),
        ),
        (
            root.join("agents/rain/custom-instruction.md"),
            include_str!("../templates/cl/agents/rain/custom-instruction.md"),
        ),
    ]
}

// ---- single-instance lock ---------------------------------------------

/// PID-based lockfile guard. Drops the file when released.
///
/// Best-effort: if the recorded PID is still alive (`kill -0`) we refuse to
/// start. If it's stale, we steal the lock and continue. This avoids the
/// classic "crashed-with-lockfile" lockout without needing platform-specific
/// flock syscalls in v1.
#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
}

impl LockGuard {
    pub fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating lock parent at {}", parent.display()))?;
        }

        if path.exists() {
            let mut existing = String::new();
            if let Ok(mut f) = fs::File::open(path) {
                let _ = f.read_to_string(&mut existing);
            }
            if let Ok(pid) = existing.trim().parse::<i32>() {
                if pid_alive(pid) {
                    anyhow::bail!(
                        "bot-hq is already running (pid {pid}, lockfile {}). \
                         Quit the other instance first.",
                        path.display()
                    );
                }
                warn!(pid, "stale lockfile, taking over");
            }
        }

        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("opening lockfile {}", path.display()))?;
        let pid = std::process::id();
        writeln!(f, "{pid}").with_context(|| format!("writing pid to {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn pid_alive(pid: i32) -> bool {
    // signal 0 = no signal, just permission/existence check.
    // safety: bare libc-style FFI through std on unix.
    #[cfg(unix)]
    unsafe {
        extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        kill(pid, 0) == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn first_run_creates_skeleton() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        let outcome = paths.init().unwrap();
        assert_eq!(outcome, InitOutcome::FirstRun);
        assert!(paths.cl_version_path.exists());
        assert!(paths.local_dir.exists());
        assert!(
            tmp.path().join("custom-general-rules.md").exists(),
            "first run should seed custom-general-rules.md stub"
        );
        assert!(
            !tmp.path().join("general-rules.md").exists(),
            "general-rules.md is hardcoded now — should not be seeded"
        );
        assert!(tmp
            .path()
            .join("agents/brian/custom-instruction.md")
            .exists());
    }

    #[test]
    fn second_init_is_no_op() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        let outcome = paths.init().unwrap();
        assert_eq!(outcome, InitOutcome::Existing);
    }

    #[test]
    fn missing_slot_gets_repaired() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        fs::remove_file(tmp.path().join("agents/rain/custom-instruction.md")).unwrap();
        let outcome = paths.init().unwrap();
        match outcome {
            InitOutcome::Repaired { repaired_slots } => {
                assert!(repaired_slots.iter().any(|s| s.contains("rain")));
            }
            other => panic!("expected Repaired, got {other:?}"),
        }
    }

    #[test]
    fn lock_acquire_and_release() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("lock");
        let guard = LockGuard::acquire(&lock_path).unwrap();
        assert!(lock_path.exists());
        drop(guard);
        assert!(!lock_path.exists());
    }

    #[test]
    fn lock_blocks_double_acquire_same_process() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("lock");
        let _guard = LockGuard::acquire(&lock_path).unwrap();
        let err = LockGuard::acquire(&lock_path).unwrap_err();
        assert!(err.to_string().contains("already running"));
    }

    #[test]
    fn lock_steals_stale_pid() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("lock");
        // PID 0 isn't a real running process — kill(0, 0) returns ESRCH.
        // Use a clearly-impossible PID instead: i32::MAX.
        fs::write(&lock_path, format!("{}\n", i32::MAX)).unwrap();
        let guard = LockGuard::acquire(&lock_path).unwrap();
        let contents = fs::read_to_string(&lock_path).unwrap();
        assert!(contents.contains(&std::process::id().to_string()));
        drop(guard);
    }

    #[test]
    fn tilde_expansion() {
        let expanded = expand_tilde("~/foo").unwrap();
        let home = directories::BaseDirs::new().unwrap().home_dir().to_path_buf();
        assert_eq!(expanded, home.join("foo"));
    }
}
