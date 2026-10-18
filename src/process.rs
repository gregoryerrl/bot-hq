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

    /// `Command` has no getter for its creation flags, so the flag itself can
    /// only be pinned as the constant the docs give.
    #[cfg(windows)]
    #[test]
    fn the_flag_is_create_no_window() {
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }
}
