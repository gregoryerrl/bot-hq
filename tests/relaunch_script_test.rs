//! `scripts/relaunch.sh` — the decision path of the detached run, against a
//! real on-disk database and a stand-in launcher that only records that it
//! was reached. A relaunch kills every live agent, so each refusal here is a
//! working session saved: another session mid-turn, a busy check that could
//! not be made (EYES ac3448b8), and a caller whose temporary halt is gone —
//! the boot brings back only sessions holding a wake instant, so stopping
//! then would strand it (EYES 731ce8e3). Unix only, like the script.
#![cfg(unix)]

use bot_hq::storage::Storage;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Bench {
    dir: tempfile::TempDir,
    storage: Storage,
}

impl Bench {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".local")).unwrap();
        let storage = Storage::open(&dir.path().join(".local/bot-hq.db")).await.unwrap();
        // The launcher stand-in: touching the marker IS "the app was relaunched".
        let start = dir.path().join("start");
        std::fs::write(&start, "#!/bin/sh\ntouch \"$(dirname \"$0\")/launched\"\nexit 0\n").unwrap();
        chmod_x(&start);
        Self { dir, storage }
    }

    fn launched(&self) -> bool {
        self.dir.path().join("launched").exists()
    }

    /// Run the detached branch with no delay; the (exit status, everything it printed).
    fn run(&self, from: &str, extra_path: Option<&Path>) -> (bool, String) {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut path = std::env::var("PATH").unwrap_or_default();
        if let Some(p) = extra_path {
            path = format!("{}:{path}", p.display());
        }
        let out = Command::new("bash")
            .arg(repo.join("scripts/relaunch.sh"))
            .args(["--detached", "--from", from, "--delay", "0"])
            .env("BOT_HQ_DATA_DIR", self.dir.path())
            .env("BOT_HQ_RELAUNCH_START", self.dir.path().join("start"))
            .env("PATH", path)
            .output()
            .expect("bash runs the script");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        (out.status.success(), text)
    }
}

fn chmod_x(p: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn tools_present() -> bool {
    let has = |t: &str| Command::new("sh").args(["-c", &format!("command -v {t}")]).output().map(|o| o.status.success()).unwrap_or(false);
    if !(has("sqlite3") && has("cargo") && has("npm") && has("bash")) {
        eprintln!("skipped: the script needs sqlite3, cargo, npm and bash on PATH");
        return false;
    }
    true
}

#[tokio::test]
async fn the_caller_with_a_wake_instant_and_nobody_busy_is_relaunched() {
    if !tools_present() {
        return;
    }
    let b = Bench::new().await;
    b.storage.create_session("me", "t", None).await.unwrap();
    b.storage.create_session("idle", "t", None).await.unwrap();
    b.storage.declare_temporary_session_halt("me", "hands", "relaunching", "2026-10-21T09:00:00Z").await.unwrap();
    // The caller's OWN turn is the one being cut, by design.
    b.storage.set_current_turn("me", Some(7)).await;
    let (ok, text) = b.run("me", None);
    assert!(ok, "{text}");
    assert!(text.contains("me wakes at 2026-10-21T09:00:00Z"), "{text}");
    assert!(b.launched(), "the launcher was reached: {text}");
}

#[tokio::test]
async fn another_session_mid_turn_refuses_the_relaunch() {
    if !tools_present() {
        return;
    }
    let b = Bench::new().await;
    b.storage.create_session("me", "t", None).await.unwrap();
    b.storage.create_session("working", "t", None).await.unwrap();
    b.storage.declare_temporary_session_halt("me", "hands", "relaunching", "2026-10-21T09:00:00Z").await.unwrap();
    b.storage.set_current_turn("working", Some(3)).await;
    let (ok, text) = b.run("me", None);
    assert!(!ok);
    assert!(text.contains("another session holds a turn — working"), "{text}");
    assert!(!b.launched(), "nothing was launched: {text}");
}

#[tokio::test]
async fn a_busy_check_that_cannot_be_made_refuses_the_relaunch() {
    if !tools_present() {
        return;
    }
    let b = Bench::new().await;
    b.storage.create_session("me", "t", None).await.unwrap();
    b.storage.declare_temporary_session_halt("me", "hands", "relaunching", "2026-10-21T09:00:00Z").await.unwrap();
    // A sqlite3 that cannot answer, first on PATH.
    let bin = b.dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("sqlite3"), "#!/bin/sh\necho 'Error: database is locked' >&2\nexit 1\n").unwrap();
    chmod_x(&bin.join("sqlite3"));
    let (ok, text) = b.run("me", Some(&bin));
    assert!(!ok);
    assert!(text.contains("the busy-session check could not be made") && text.contains("database is locked"), "{text}");
    assert!(!b.launched(), "an unknown answer is a no: {text}");
}

#[tokio::test]
async fn a_caller_whose_halt_was_released_is_not_stranded() {
    if !tools_present() {
        return;
    }
    let b = Bench::new().await;
    b.storage.create_session("me", "t", None).await.unwrap();
    // Declared, then released by a user message during the delay.
    b.storage.declare_temporary_session_halt("me", "hands", "relaunching", "2026-10-21T09:00:00Z").await.unwrap();
    b.storage.clear_session_halt("me").await.unwrap();
    let (ok, text) = b.run("me", None);
    assert!(!ok);
    assert!(text.contains("me holds no temporary halt"), "{text}");
    assert!(!b.launched(), "stopping now would strand the caller: {text}");
    // A plain halt (no wake instant) is no better: the boot does not respawn it.
    b.storage.declare_session_halt("me", "hands", "waiting on you").await.unwrap();
    let (ok, text) = b.run("me", None);
    assert!(!ok && text.contains("me holds no temporary halt"), "{text}");
    assert!(!b.launched());
}
