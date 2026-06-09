//! Data-dir resolution + first-run init.
//!
//! Layout under `<data_dir>` (default `~/.bot-hq/`, overridable via
//! `BOT_HQ_DATA_DIR`). The Context Library lives in its own `library/` subtree
//! so it can be backed up / cloud-synced independently of host-local state;
//! secrets, logs, and runtime state live under `.local/` (never synced):
//!
//! ```text
//! <data_dir>/
//!   version.txt                    (whole-home schema marker, "1" for v1)
//!   library/                       (Context Library — its own folder)
//!     custom-general-rules.md      (optional user additions; hardcoded core
//!                                   lives in agents::general_rules)
//!     eod.md, tasks.md             (cross-project _globals files)
//!     agents/<name>/custom-instruction.md  (brian, rain — user tweaks)
//!     projects/<p>/conventions.md
//!     projects/<p>/notes.md
//!     projects/<p>/policy.yaml     (CL-coupled: policy resolver reads here)
//!   plugins/                       (installed plugins)
//!   general-policy.yaml            (machine policy — stays at root in v1)
//!   tool-gate.json
//!   claude-overrides.json
//!   .local/                        (host-only; never synced)
//!     bot-hq.db
//!     lock                         (single-instance PID lock)
//!     mcp-token                    (external MCP bearer token, 0600 unix)
//!     violations.jsonl             (policy audit trail)
//!     .policy-hashes.json          (policy-file hash cache)
//!     screenshots/<ts>.png
//!     session-permissions/<sid>.json  (per-session grant mirrors)
//! ```
//!
//! Pre-`library/` installs (root-level CL, no `version.txt`) are migrated once
//! into this shape by [`Paths::init`] via [`Paths::migrate_legacy_layout`].

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
    /// Context Library root: `<data_dir>/library/`. Holds agent custom
    /// instructions, `custom-general-rules.md`, cross-project `_globals` files
    /// (`eod.md`, `tasks.md`), and `projects/<p>/` (conventions, notes,
    /// decisions, and `policy.yaml`). Its own folder so it can be backed up /
    /// cloud-synced independently of host-local state.
    pub cl_dir: PathBuf,
    /// Installed-plugin root: `<data_dir>/plugins/`.
    pub plugins_dir: PathBuf,
    /// Host-only runtime state, secrets, and logs: `<data_dir>/.local/`.
    /// Never synced.
    pub local_dir: PathBuf,
    pub db_path: PathBuf,
    pub lock_path: PathBuf,
    /// Whole-data-home schema marker: `<data_dir>/version.txt`. Its absence is
    /// the first-run signal; an old install missing it but carrying root-level
    /// CL triggers the one-time `library/` migration in [`Paths::init`].
    pub version_path: PathBuf,
    /// Bearer token for the external MCP server. Generated on first run as a
    /// UUIDv4 with 0o600 perms (unix). Lives under `.local/` (host-only secret,
    /// never synced). User can rotate by editing the file and restarting.
    pub mcp_token_path: PathBuf,
    /// Policy audit trail: `<data_dir>/.local/violations.jsonl`.
    pub violations_path: PathBuf,
    /// Policy-file hash cache: `<data_dir>/.local/.policy-hashes.json`.
    pub policy_hashes_path: PathBuf,
    /// Webview screenshot output dir: `<data_dir>/.local/screenshots/`.
    pub screenshots_dir: PathBuf,
    /// The internal signaling server's bound address (e.g. `127.0.0.1:54321`),
    /// written at startup so the git pre-push hook — a separate subprocess that
    /// can't reach the running app's bridge directly — can POST `/hooks/pre-push`
    /// to surface a per-push approval prompt under `push_gate=ask`. Lives under
    /// `.local/` (runtime state, not user content); removed on clean shutdown.
    pub signaling_addr_path: PathBuf,
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
        let cl_dir = data_dir.join("library");
        let plugins_dir = data_dir.join("plugins");
        let local_dir = data_dir.join(".local");
        let db_path = local_dir.join("bot-hq.db");
        let lock_path = local_dir.join("lock");
        let version_path = data_dir.join("version.txt");
        let mcp_token_path = local_dir.join("mcp-token");
        let violations_path = local_dir.join("violations.jsonl");
        let policy_hashes_path = local_dir.join(".policy-hashes.json");
        let screenshots_dir = local_dir.join("screenshots");
        let signaling_addr_path = local_dir.join("signaling-addr");
        Self {
            data_dir,
            cl_dir,
            plugins_dir,
            local_dir,
            db_path,
            lock_path,
            version_path,
            mcp_token_path,
            violations_path,
            policy_hashes_path,
            screenshots_dir,
            signaling_addr_path,
        }
    }

    /// Single source of truth for the per-project CL convention path:
    /// `<cl_dir>/projects/<name>/`. All convention callers (storage
    /// `cl_path_for_project`, policy resolver, policy audit) route through this
    /// so a layout change can't desync them.
    pub fn project_dir(&self, name: &str) -> PathBuf {
        self.cl_dir.join("projects").join(name)
    }

    /// The `projects/` root under the CL dir, walked by the startup backfill.
    pub fn cl_projects_dir(&self) -> PathBuf {
        self.cl_dir.join("projects")
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

    /// Persist the internal signaling server's bound address so the git
    /// pre-push hook subprocess can reach the running app (see
    /// [`read_signaling_addr`]). Overwritten on every startup so it always
    /// reflects the live ephemeral port. Best-effort cleanup on clean shutdown
    /// lives in `SignalingServer::Drop`.
    pub fn write_signaling_addr(&self, addr: std::net::SocketAddr) -> Result<()> {
        if let Some(parent) = self.signaling_addr_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("creating parent for {}", self.signaling_addr_path.display())
            })?;
        }
        fs::write(&self.signaling_addr_path, format!("{addr}\n")).with_context(|| {
            format!(
                "writing signaling addr at {}",
                self.signaling_addr_path.display()
            )
        })?;
        Ok(())
    }

    /// Idempotent. Creates the data dir + CL skeleton on first run, repairs
    /// missing required CL slots on subsequent runs, migrates a pre-`library/`
    /// install once, and leaves user content untouched otherwise. Returns the
    /// outcome for UI toasts.
    ///
    /// First-run signal: `version.txt` doesn't exist AND there's no legacy
    /// root-level CL. A brand-new install (or a wiped data dir) gets `FirstRun`
    /// and a silent init. A pre-`library/` install (no `version.txt` but
    /// root-level CL present) is migrated into the new layout and reported as
    /// `Repaired`. A user who deleted just one slot also gets `Repaired`.
    pub fn init(&self) -> Result<InitOutcome> {
        // Detect a pre-`library/` install BEFORE creating anything: the old
        // marker `cl-version.txt` or any root-level CL means we migrate rather
        // than treat this as a fresh first run.
        let has_legacy = self.data_dir.join("cl-version.txt").exists()
            || self.data_dir.join("projects").is_dir()
            || self.data_dir.join("agents").is_dir()
            || self.data_dir.join("custom-general-rules.md").exists();
        let first_run = !self.version_path.exists() && !has_legacy;

        fs::create_dir_all(&self.data_dir)
            .with_context(|| format!("creating data dir at {}", self.data_dir.display()))?;

        // One-time migration of a legacy root-level CL into library/ + .local/.
        let migrated = self.migrate_legacy_layout()?;

        fs::create_dir_all(&self.cl_dir)
            .with_context(|| format!("creating library dir at {}", self.cl_dir.display()))?;
        fs::create_dir_all(&self.plugins_dir)
            .with_context(|| format!("creating plugins dir at {}", self.plugins_dir.display()))?;
        fs::create_dir_all(&self.local_dir)
            .with_context(|| format!("creating local dir at {}", self.local_dir.display()))?;

        let mut repaired_slots = Vec::new();

        if !self.version_path.exists() {
            fs::write(&self.version_path, "1\n")
                .with_context(|| format!("writing {}", self.version_path.display()))?;
        }

        for (path, body) in default_cl_files(&self.cl_dir) {
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
            if let Some(parent) = self.mcp_token_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            let token = uuid::Uuid::new_v4().to_string();
            fs::write(&self.mcp_token_path, format!("{token}\n")).with_context(|| {
                format!("writing mcp-token at {}", self.mcp_token_path.display())
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = fs::Permissions::from_mode(0o600);
                fs::set_permissions(&self.mcp_token_path, perms)
                    .with_context(|| format!("0o600 perms on {}", self.mcp_token_path.display()))?;
            }
            if !first_run {
                repaired_slots.push("mcp-token".to_string());
            }
        }

        if migrated {
            repaired_slots.push("(migrated to library/ layout)".to_string());
        }

        if first_run {
            info!(data_dir = %self.data_dir.display(), "first-run init complete");
            Ok(InitOutcome::FirstRun)
        } else if repaired_slots.is_empty() {
            Ok(InitOutcome::Existing)
        } else {
            warn!(slots = ?repaired_slots, "repaired/migrated CL slots");
            Ok(InitOutcome::Repaired { repaired_slots })
        }
    }

    /// Move a pre-`library/` install into the new layout exactly once. Returns
    /// `true` if anything was moved. Gated on `version.txt` being absent (the
    /// post-migration marker), so it's a no-op on every later run and on a
    /// genuinely fresh data dir. Idempotent per-entry: only moves a source that
    /// exists when the destination doesn't. Projects whose CL lives at an
    /// explicit absolute `cl_path` (stored in the db) are untouched — only the
    /// convention layout under the data dir moves.
    fn migrate_legacy_layout(&self) -> Result<bool> {
        if self.version_path.exists() {
            return Ok(false);
        }
        fs::create_dir_all(&self.cl_dir)
            .with_context(|| format!("creating library dir at {}", self.cl_dir.display()))?;
        fs::create_dir_all(&self.local_dir)
            .with_context(|| format!("creating local dir at {}", self.local_dir.display()))?;

        let mut moved = false;

        // CL content → library/
        for name in [
            "projects",
            "agents",
            "custom-general-rules.md",
            "eod.md",
            "tasks.md",
        ] {
            let from = self.data_dir.join(name);
            let to = self.cl_dir.join(name);
            if from.exists() && !to.exists() {
                move_path(&from, &to)?;
                moved = true;
            }
        }

        // host-only state → .local/
        for name in [
            "mcp-token",
            "violations.jsonl",
            ".policy-hashes.json",
            "screenshots",
        ] {
            let from = self.data_dir.join(name);
            let to = self.local_dir.join(name);
            if from.exists() && !to.exists() {
                move_path(&from, &to)?;
                moved = true;
            }
        }

        // Old schema marker → new whole-home marker.
        let old_marker = self.data_dir.join("cl-version.txt");
        if old_marker.exists() && !self.version_path.exists() {
            move_path(&old_marker, &self.version_path)?;
            moved = true;
        }

        if moved {
            warn!(
                data_dir = %self.data_dir.display(),
                "migrated legacy root-level CL into library/ + .local/"
            );
        }
        Ok(moved)
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

/// Move a file or directory, preferring an atomic `fs::rename` and falling back
/// to recursive copy + remove when rename fails — e.g. a cross-filesystem
/// `EXDEV` on unix, or a locked/open file on Windows. Used by the one-time
/// legacy-layout migration.
fn move_path(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent for {}", to.display()))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_recursive(from, to)
        .with_context(|| format!("copying {} -> {}", from.display(), to.display()))?;
    if from.is_dir() {
        fs::remove_dir_all(from)
    } else {
        fs::remove_file(from)
    }
    .with_context(|| format!("removing source {} after copy", from.display()))?;
    Ok(())
}

/// Recursively copy a file or directory tree.
fn copy_recursive(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to).with_context(|| format!("creating dir {}", to.display()))?;
        for entry in fs::read_dir(from).with_context(|| format!("reading dir {}", from.display()))? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating parent for {}", to.display()))?;
        }
        fs::copy(from, to)
            .with_context(|| format!("copying file {} -> {}", from.display(), to.display()))?;
    }
    Ok(())
}

/// Read the persisted signaling-server address (`<data_dir>/.local/signaling-addr`)
/// for the git pre-push hook. Returns `None` when the file is missing or empty
/// (bot-hq not running) — the hook then fail-closes (blocks the push). Free fn
/// (not a `Paths` method) because the hook subprocess only has `--data-dir`.
pub fn read_signaling_addr(data_dir: &Path) -> Option<String> {
    let path = data_dir.join(".local").join("signaling-addr");
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Expand a leading `~` (and optionally `~/`) in a path string. Shared with
/// the policy-check hook subprocess (`policy::hooks`) so `~` resolves the same
/// way (via `directories::BaseDirs`) regardless of caller.
pub(crate) fn expand_tilde(s: &str) -> Result<PathBuf> {
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
        assert!(paths.version_path.exists());
        assert!(paths.cl_dir.exists());
        assert!(paths.plugins_dir.exists());
        assert!(paths.local_dir.exists());
        assert!(
            paths.cl_dir.join("custom-general-rules.md").exists(),
            "first run should seed custom-general-rules.md stub under library/"
        );
        assert!(
            !paths.cl_dir.join("general-rules.md").exists(),
            "general-rules.md is hardcoded now — should not be seeded"
        );
        assert!(paths
            .cl_dir
            .join("agents/brian/custom-instruction.md")
            .exists());
        // CL must NOT be seeded at the data-dir root anymore.
        assert!(!tmp.path().join("custom-general-rules.md").exists());
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
        fs::remove_file(paths.cl_dir.join("agents/rain/custom-instruction.md")).unwrap();
        let outcome = paths.init().unwrap();
        match outcome {
            InitOutcome::Repaired { repaired_slots } => {
                assert!(repaired_slots.iter().any(|s| s.contains("rain")));
            }
            other => panic!("expected Repaired, got {other:?}"),
        }
    }

    #[test]
    fn migrates_legacy_root_layout_into_library() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Simulate a pre-`library/` install: CL at the data-dir root + the old
        // marker + host-only state.
        fs::create_dir_all(root.join("projects/foo")).unwrap();
        fs::write(root.join("projects/foo/conventions.md"), "# foo\n").unwrap();
        fs::create_dir_all(root.join("agents/brian")).unwrap();
        fs::write(root.join("agents/brian/custom-instruction.md"), "hi\n").unwrap();
        fs::write(root.join("custom-general-rules.md"), "rules\n").unwrap();
        fs::write(root.join("eod.md"), "eod\n").unwrap();
        fs::write(root.join("cl-version.txt"), "1\n").unwrap();
        fs::write(root.join("mcp-token"), "tok\n").unwrap();
        fs::write(root.join("violations.jsonl"), "{}\n").unwrap();

        let paths = Paths::for_data_dir(root.to_path_buf());
        let outcome = paths.init().unwrap();

        // Not a first run — it's a migration, surfaced as Repaired.
        match outcome {
            InitOutcome::Repaired { repaired_slots } => {
                assert!(repaired_slots.iter().any(|s| s.contains("migrated")));
            }
            other => panic!("expected Repaired (migration), got {other:?}"),
        }

        // CL content moved under library/.
        assert!(paths.cl_dir.join("projects/foo/conventions.md").exists());
        assert!(paths.cl_dir.join("custom-general-rules.md").exists());
        assert!(paths.cl_dir.join("eod.md").exists());
        assert!(paths
            .cl_dir
            .join("agents/brian/custom-instruction.md")
            .exists());
        // host-only state moved under .local/.
        assert!(paths.mcp_token_path.exists());
        assert!(paths.violations_path.exists());
        // marker renamed; old root locations gone.
        assert!(paths.version_path.exists());
        assert!(!root.join("cl-version.txt").exists());
        assert!(!root.join("projects").exists());
        assert!(!root.join("custom-general-rules.md").exists());
        assert!(!root.join("mcp-token").exists());
    }

    #[test]
    fn migration_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("custom-general-rules.md"), "rules\n").unwrap();
        fs::write(root.join("cl-version.txt"), "1\n").unwrap();

        let paths = Paths::for_data_dir(root.to_path_buf());
        paths.init().unwrap();
        // Second init: version.txt now exists, nothing left to migrate → no-op.
        let outcome = paths.init().unwrap();
        assert_eq!(outcome, InitOutcome::Existing);
        assert!(paths.cl_dir.join("custom-general-rules.md").exists());
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
    fn signaling_addr_round_trip() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(tmp.path().to_path_buf());
        paths.init().unwrap();
        // Absent before the server writes it → None (hook fail-closes).
        assert!(read_signaling_addr(tmp.path()).is_none());
        let addr: std::net::SocketAddr = "127.0.0.1:54321".parse().unwrap();
        paths.write_signaling_addr(addr).unwrap();
        assert_eq!(
            read_signaling_addr(tmp.path()).as_deref(),
            Some("127.0.0.1:54321")
        );
    }

    #[test]
    fn tilde_expansion() {
        let expanded = expand_tilde("~/foo").unwrap();
        let home = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf();
        assert_eq!(expanded, home.join("foo"));
    }
}
