//! `git` the way bot-hq runs it ON ITS OWN inside a registered repository.
//!
//! A repository's local config is code: `core.fsmonitor` names a program
//! that `status` / `ls-files` / `diff` run while refreshing the index,
//! `diff.external` and a `.gitattributes` driver's `diff.<d>.command` /
//! `textconv` are programs `diff` runs per file, and `.git/hooks/post-checkout`
//! runs on `worktree add`. None of that is cloned from a remote — but a
//! repository delivered as a directory (an archive, a shared drive, a
//! template) carries its `.git/config` and hooks with it, and bot-hq runs
//! these commands the moment a session opens the Apply tab or a worktree is
//! created, before the user has run anything from the repo themselves
//! (2026-09-06 sweep, S6).
//!
//! So every invocation bot-hq makes without the user asking for THAT command
//! goes through [`hardened`], which disables the config-driven program hooks
//! for that one process. The agent's own `git commit` / `git push` in the
//! session shell are the user's business (and where bot-hq's own hooks must
//! run) — those are untouched.

use std::path::Path;
use std::process::Command;

/// `-c key=value` pairs applied to every hardened invocation. Config only,
/// never behaviour the command's output depends on.
pub const HARDENING_CONFIG: &[&str] = &[
    // The index-refresh hook. `false` is the documented "off" (a path names a
    // hook program, `true` the built-in daemon).
    "core.fsmonitor=false",
    // A hooks dir that does not exist disables every repo hook for this
    // process — `worktree add` would otherwise run `post-checkout`. (Not the
    // empty string: git resolves that relative to the repo root.)
    "core.hooksPath=/nonexistent-bot-hq-hooks-disabled",
];

/// Extra args for `git diff` specifically: refuse external diff drivers
/// (`diff.external`, `.gitattributes` `diff=<d>` + `diff.<d>.command`) and
/// textconv filters, both of which are programs named in local config.
/// Flags rather than `-c diff.external=` because an EMPTY `diff.external` is
/// not "unset" to git — it would try to run `""`.
pub const DIFF_HARDENING_ARGS: &[&str] = &["--no-ext-diff", "--no-textconv"];

/// `git -C <repo> -c … -c …`, ready for `.args([...])`. Std `Command`; the
/// async call sites wrap it in `spawn_blocking` as they already do.
pub fn hardened(repo: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo);
    for kv in HARDENING_CONFIG {
        cmd.arg("-c").arg(kv);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire: the helper actually emits every hardening pair, in `-c`
    /// form, before the subcommand.
    #[test]
    fn hardened_emits_every_config_pair() {
        let cmd = hardened(Path::new("/tmp/x"));
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..2], ["-C", "/tmp/x"]);
        for kv in HARDENING_CONFIG {
            let i = args.iter().position(|a| a == kv).expect(kv);
            assert_eq!(args[i - 1], "-c", "{kv} must ride a -c");
        }
        assert!(DIFF_HARDENING_ARGS.contains(&"--no-ext-diff"));
        assert!(DIFF_HARDENING_ARGS.contains(&"--no-textconv"));
    }

    /// Functional proof on a real repo: plant a `core.fsmonitor` hook and a
    /// `diff.external` driver that each write a marker file. Plain `git`
    /// runs them (so the test discriminates — if it did not, the hardening
    /// would prove nothing); the hardened form does not.
    #[cfg(unix)]
    #[test]
    fn hardened_git_does_not_run_config_named_programs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
                .args(args)
                .output()
                .expect("git runs");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q"]);
        std::fs::write(repo.join("f.txt"), "one\n").unwrap();
        git(&["add", "f.txt"]);
        git(&["commit", "-q", "-m", "one"]);
        std::fs::write(repo.join("f.txt"), "two\n").unwrap();

        // Two marker-writing programs, wired through LOCAL config exactly as
        // a delivered repository would carry them.
        let fs_marker = dir.path().join("fsmonitor-ran");
        let diff_marker = dir.path().join("external-diff-ran");
        let script = |marker: &Path| -> std::path::PathBuf {
            let p = dir.path().join(format!("{}.sh", marker.file_name().unwrap().to_string_lossy()));
            std::fs::write(&p, format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display())).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let fs_hook = script(&fs_marker);
        let diff_hook = script(&diff_marker);
        git(&["config", "core.fsmonitor", &fs_hook.display().to_string()]);
        git(&["config", "diff.external", &diff_hook.display().to_string()]);

        // Discriminate: the plain invocation DOES run them.
        let _ = Command::new("git").arg("-C").arg(&repo).args(["status", "--porcelain"]).output().unwrap();
        let _ = Command::new("git").arg("-C").arg(&repo).args(["diff", "HEAD"]).output().unwrap();
        assert!(fs_marker.exists(), "plain `git status` must fire core.fsmonitor for this test to mean anything");
        assert!(diff_marker.exists(), "plain `git diff` must fire diff.external for this test to mean anything");
        std::fs::remove_file(&fs_marker).unwrap();
        std::fs::remove_file(&diff_marker).unwrap();

        // The hardened form does not.
        let out = hardened(&repo).args(["status", "--porcelain"]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("f.txt"), "status still reports the change");
        let out = hardened(&repo).arg("diff").args(DIFF_HARDENING_ARGS).arg("HEAD").output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("+two"), "diff still shows the change");
        let out = hardened(&repo).args(["ls-files", "--others", "--exclude-standard", "-z"]).output().unwrap();
        assert!(out.status.success());
        assert!(!fs_marker.exists(), "hardened git ran the core.fsmonitor program");
        assert!(!diff_marker.exists(), "hardened git ran the diff.external program");
    }
}
