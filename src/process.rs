//! The one constructor for every child process bot-hq starts.
//!
//! A Windows release build is a GUI-subsystem program since 1.0.9 (no console
//! window beside the app, and none to close by accident). A console program it
//! starts — claude.exe, git, Git for Windows' `sh.exe`, gh, reg — would then
//! get a console window of its own on every call, so every spawn goes through
//! here and carries `CREATE_NO_WINDOW`. Its pipes work as before. Off Windows
//! the constructors are plain `Command::new`.
//!
//! A raw `Command::new(` anywhere else in `src/` fails
//! `tests/no_raw_spawn_test.rs`: converting sites by hand misses one, and a
//! missed site flashes a console window on every call. The terminal tab's
//! ConPTY shell (`portable_pty::CommandBuilder`) is not a `Command` and stays
//! as it is — that one is meant to have a console.

use std::ffi::OsStr;

/// `CREATE_NO_WINDOW`: a console program runs without a console window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A `std::process::Command` for `program` that opens no console window.
pub fn command(program: impl AsRef<OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Every live descendant of `pid` (children, grandchildren, …), deepest
/// first, from one `ps` listing. Empty when `ps` cannot be read or `pid` has
/// no children. Unix only; Windows has no cheap tree walk here and its
/// single-PID `kill_child` stands in (the job-object reaping follow-up).
#[cfg(unix)]
pub fn descendants(pid: u32) -> Vec<u32> {
    let Ok(out) = command("ps").args(["-axo", "pid=,ppid="]).output() else {
        return Vec::new();
    };
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut words = line.split_whitespace();
        if let (Some(p), Some(pp)) = (
            words.next().and_then(|w| w.parse::<u32>().ok()),
            words.next().and_then(|w| w.parse::<u32>().ok()),
        ) {
            children.entry(pp).or_default().push(p);
        }
    }
    // Breadth-first, then reversed: a parent is listed after its children.
    let mut order = Vec::new();
    let mut frontier = vec![pid];
    while let Some(next) = frontier.pop() {
        if let Some(kids) = children.get(&next) {
            for &kid in kids {
                order.push(kid);
                frontier.push(kid);
            }
        }
    }
    order.reverse();
    order
}

/// SIGKILL `pid` and every descendant it has, children first — a running
/// tool command's whole tree (`zsh -lc cargo build` → `cargo` → `rustc`),
/// which killing the shell alone would orphan (EYES P7). Best-effort: a pid
/// that is already gone is skipped. Unix only; see [`descendants`].
#[cfg(unix)]
pub fn kill_process_tree(pid: u32) {
    for p in descendants(pid).into_iter().chain(std::iter::once(pid)) {
        // SAFETY: kill(2) is thread-safe; a stale pid is refused by the kernel.
        unsafe {
            libc::kill(p as i32, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
pub fn kill_process_tree(pid: u32) {
    crate::agents::spawn::kill_child(pid);
}

/// A `tokio::process::Command` for `program` that opens no console window.
pub fn tokio_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both constructors run the program they were given, with its pipes —
    /// the flag changes the window, never the I/O.
    #[test]
    fn the_constructors_run_the_program_with_its_pipes() {
        let out = command("git").arg("--version").output().expect("git runs");
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).starts_with("git version"));
    }

    #[tokio::test]
    async fn the_tokio_constructor_runs_the_program_with_its_pipes() {
        let out = tokio_command("git").arg("--version").output().await.expect("git runs");
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).starts_with("git version"));
    }

    /// The tree, not the leader: a shell with two sleeping children is listed
    /// children-first and all three die on the kill (EYES P7).
    #[cfg(unix)]
    #[test]
    fn the_process_tree_is_walked_and_killed_children_first() {
        let mut shell = command("sh")
            .args(["-c", "sleep 300 & sleep 300 & wait"])
            .spawn()
            .expect("sh runs");
        let pid = shell.id();
        // Give the shell a moment to fork its children.
        let mut kids = Vec::new();
        for _ in 0..50 {
            kids = descendants(pid);
            if kids.len() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(kids.len() >= 2, "two sleeping children expected, got {kids:?}");
        kill_process_tree(pid);
        let _ = shell.wait();
        for _ in 0..50 {
            if kids.iter().all(|k| unsafe { libc::kill(*k as i32, 0) } != 0) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("a child survived the tree kill: {kids:?}");
    }

    /// `Command` has no getter for its creation flags, so the flag itself can
    /// only be pinned as the constant the docs give.
    #[cfg(windows)]
    #[test]
    fn the_flag_is_create_no_window() {
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }
}
