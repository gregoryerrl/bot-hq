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
    descendants_checked(pid).unwrap_or_default()
}

/// [`descendants`] that tells "no children" from "could not look": `None`
/// when `ps` cannot be run or exits non-zero. A caller deciding whether a
/// tree is CLEAN must use this one — an empty answer to a failed listing
/// read as clean is how a surviving command goes unnoticed (EYES, 2026-10-21).
#[cfg(unix)]
pub fn descendants_checked(pid: u32) -> Option<Vec<u32>> {
    Some(descendants_named(pid)?.into_iter().map(|d| d.pid).collect())
}

/// One process of a tree listing: its pid, its parent, and its program (the
/// first word of its command line).
#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descendant {
    pub pid: u32,
    pub ppid: u32,
    pub program: String,
}

/// [`descendants_checked`] with each process's parent and program, deepest
/// first.
#[cfg(unix)]
pub fn descendants_named(pid: u32) -> Option<Vec<Descendant>> {
    let out = command("ps").args(["-axo", "pid=,ppid=,command="]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    let mut info: std::collections::HashMap<u32, (u32, String)> = std::collections::HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut words = line.split_whitespace();
        if let (Some(p), Some(pp)) = (
            words.next().and_then(|w| w.parse::<u32>().ok()),
            words.next().and_then(|w| w.parse::<u32>().ok()),
        ) {
            children.entry(pp).or_default().push(p);
            info.insert(p, (pp, words.next().unwrap_or("").to_string()));
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
    Some(
        order
            .into_iter()
            .map(|p| {
                let (ppid, program) = info.remove(&p).unwrap_or_default();
                Descendant { pid: p, ppid, program }
            })
            .collect(),
    )
}

/// The program (first word of the command line) of one pid, when `ps` can
/// tell.
#[cfg(unix)]
pub fn program_of(pid: u32) -> Option<String> {
    let out = command("ps").args(["-o", "command=", "-p", &pid.to_string()]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().next().map(str::to_string)
}

/// Whether `pid` is finished: no such process, or a zombie its parent has
/// only to reap. Asked of the pid itself, so a child that was reparented
/// when its parent died first (and so left its old parent's tree) is still
/// seen alive until it is actually gone (EYES e3a49b5a).
#[cfg(unix)]
pub fn is_gone(pid: u32) -> bool {
    // SAFETY: kill(2) with signal 0 only checks; a stale pid answers ESRCH.
    let answered = unsafe { libc::kill(pid as i32, 0) };
    if answered != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        return true;
    }
    match command("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim_start().starts_with('Z'),
        _ => false,
    }
}

/// SIGKILL each pid, in the order given (callers pass children first).
/// Best-effort: a pid that is already gone is skipped.
#[cfg(unix)]
pub fn kill_each(pids: &[u32]) {
    for &p in pids {
        // SAFETY: kill(2) is thread-safe; a stale pid is refused by the kernel.
        unsafe {
            libc::kill(p as i32, libc::SIGKILL);
        }
    }
}

/// SIGKILL `pid` and every descendant it has, children first — a running
/// tool command's whole tree (`zsh -lc cargo build` → `cargo` → `rustc`),
/// which killing the shell alone would orphan (EYES P7). Best-effort: a pid
/// that is already gone is skipped. Unix only; see [`descendants`].
#[cfg(unix)]
pub fn kill_process_tree(pid: u32) {
    let mut all = descendants(pid);
    all.push(pid);
    kill_each(&all);
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

    /// `is_gone` answers for the pid itself: a running sleep is not gone, a
    /// killed-and-reaped one is, and a killed-but-unreaped one (a zombie) is
    /// finished too.
    #[cfg(unix)]
    #[test]
    fn is_gone_tells_a_finished_process_from_a_running_one() {
        let mut child = command("sleep").arg("300").spawn().expect("sleep runs");
        let pid = child.id();
        assert!(!is_gone(pid));
        kill_each(&[pid]);
        // Not yet reaped by this test: a zombie, which counts as gone.
        let mut gone = false;
        for _ in 0..40 {
            if is_gone(pid) {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(gone, "a SIGKILLed child is gone (zombie or reaped)");
        let _ = child.wait();
        assert!(is_gone(pid));
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
