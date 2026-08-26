//! Pushing the Context Library to its private remote — scan first (rc3 **P6**).
//!
//! The library is a git repo with a remote (`bot-hq-dev-data`) that **nothing
//! pushed to**. Agents commit as they work, so it drifts from the first session
//! onward: a snapshot, not a backup.
//!
//! **The order in this module is the point.** The scan runs before the push,
//! always, and a hit refuses the push naming the file — because the push is the
//! step that makes a mistake irreversible, and the library already carried a
//! production credential file for 153 commits. See [`crate::policy::secret_scan`]
//! for what counts as credential-shaped and why the patterns are narrow.
//!
//! Everything here is **fail-open with respect to CL writes**: a library that
//! cannot push is a library that is merely un-backed-up, and refusing to save an
//! agent's knowledge because a network call failed would be the worse trade. The
//! caller logs; it never propagates.

use crate::policy::secret_scan::{self, SecretHit};
use std::path::Path;
use std::process::Command;

/// What a push attempt did. Every arm is a fact the caller can log or show;
/// none of them is an error the CL write path should propagate.
#[derive(Debug, Clone, PartialEq)]
pub enum PushOutcome {
    /// Pushed `n` commits.
    Pushed { commits: usize },
    /// Local is level with the remote — nothing to send.
    UpToDate,
    /// No `origin` remote configured; the library is local-only.
    NoRemote,
    /// **Refused by the scan.** Carries the hits so the message can name files.
    RefusedSecrets(Vec<SecretHit>),
    /// git itself failed (offline, auth, non-fast-forward). The string is
    /// git's own stderr, trimmed — a rejected push in a concurrent-session race
    /// lands here rather than being papered over with an automatic pull, which
    /// would merge a knowledge base behind the user's back.
    Failed(String),
}

impl PushOutcome {
    /// One line for the log / the UI.
    pub fn summary(&self) -> String {
        match self {
            Self::Pushed { commits } => format!("pushed {commits} commit(s) to the library remote"),
            Self::UpToDate => "library remote is already up to date".to_string(),
            Self::NoRemote => "library has no remote configured; nothing to push".to_string(),
            Self::RefusedSecrets(hits) => secret_scan::refusal_message(hits),
            Self::Failed(err) => format!("library push failed: {err}"),
        }
    }
}

/// Run `git` in the library, returning trimmed stdout on success.
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    // This git reaches the NETWORK: under an AppImage launch the payload's
    // libcurl chain makes `git-remote-https` abort with a symbol lookup error
    // before the transport opens, so the library push fails for a reason that
    // has nothing to do with git. A no-op off a payload. See `appimage_env`.
    crate::appimage_env::scrub_std(&mut cmd);
    let out = cmd
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Every TRACKED file that is credential-shaped, by name or by content.
///
/// **Tracked, not staged, and not just the outgoing diff.** A secret committed
/// three commits ago is still a secret this push would carry, and scanning only
/// what changed would wave it through on the second attempt. The library is
/// markdown and a few KB of config, so reading it all is cheap; a file that
/// does not decode as UTF-8 is skipped by the name check alone (its bytes
/// cannot match a text pattern anyway).
pub fn scan_tracked(root: &Path) -> Result<Vec<SecretHit>, String> {
    let listing = git(root, &["ls-files", "-z"])?;
    let mut hits = Vec::new();
    for rel in listing.split('\0').filter(|p| !p.is_empty()) {
        let body = std::fs::read_to_string(root.join(rel)).unwrap_or_default();
        if let Some(hit) = secret_scan::scan_file(rel, &body) {
            hits.push(hit);
        }
    }
    Ok(hits)
}

/// Scan, then push the library's current branch to `origin`.
///
/// Blocking (it shells out to git and talks to the network) — call it from a
/// blocking task, never on an async executor thread.
pub fn scan_then_push(root: &Path) -> PushOutcome {
    if !root.join(".git").exists() {
        return PushOutcome::NoRemote;
    }
    match git(root, &["remote", "get-url", "origin"]) {
        Ok(url) if !url.is_empty() => url,
        _ => return PushOutcome::NoRemote,
    };

    // THE GATE. Before any bytes leave: a hit refuses, and the refusal names
    // the files rather than saying "a secret was found" and leaving the user to
    // grep for it.
    match scan_tracked(root) {
        Ok(hits) if !hits.is_empty() => return PushOutcome::RefusedSecrets(hits),
        Ok(_) => {}
        // A scan that cannot run is not a scan that passed.
        Err(e) => return PushOutcome::Failed(format!("secret scan failed, not pushing: {e}")),
    }

    let branch = match git(root, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(b) if !b.is_empty() && b != "HEAD" => b,
        Ok(_) => return PushOutcome::Failed("library is in a detached HEAD".into()),
        Err(e) => return PushOutcome::Failed(e),
    };
    // How many commits this push would carry. Best-effort: a branch with no
    // upstream yet has no `@{u}`, and that is a first push, not a failure.
    let ahead = git(root, &["rev-list", "--count", "@{u}..HEAD"])
        .ok()
        .and_then(|n| n.parse::<usize>().ok());
    if ahead == Some(0) {
        return PushOutcome::UpToDate;
    }

    match git(root, &["push", "origin", &branch]) {
        Ok(_) => PushOutcome::Pushed {
            commits: ahead.unwrap_or(0),
        },
        Err(e) => PushOutcome::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A library repo with one commit and a bare `origin` it is level with.
    fn library_with_remote() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let remote = dir.path().join("remote.git");
        let root = dir.path().join("library");
        std::fs::create_dir_all(&root).unwrap();
        Command::new("git")
            .args(["init", "--bare", "-q"])
            .arg(&remote)
            .output()
            .unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.name", "bot-hq"],
            vec!["config", "user.email", "bot-hq@local"],
        ] {
            git(&root, &args).unwrap();
        }
        std::fs::write(root.join("notes.md"), "# notes\n").unwrap();
        git(&root, &["add", "-A"]).unwrap();
        git(&root, &["commit", "-q", "-m", "seed"]).unwrap();
        git(&root, &["remote", "add", "origin", remote.to_str().unwrap()]).unwrap();
        git(&root, &["push", "-q", "-u", "origin", "main"]).unwrap();
        (dir, root)
    }

    /// rc3 **P6**: the refusal is the half worth testing.
    ///
    /// A push path that works is easy; the reason this one is gated is that a
    /// production credential file lived in this repo for 153 commits and was
    /// caught by a human looking. The test therefore asserts that a
    /// credential-shaped file BLOCKS the push and that the remote did not move
    /// — a refusal that still pushed would be worse than no scan at all,
    /// because it would read as protection.
    #[test]
    fn a_push_carrying_a_credential_is_refused_and_names_the_file() {
        let (_dir, root) = library_with_remote();
        let before = git(&root, &["rev-parse", "origin/main"]).unwrap();

        // `git add -f`: exactly the case .gitignore cannot stop.
        std::fs::write(root.join("prod.env"), "DB_PASSWORD=hunter2\n").unwrap();
        git(&root, &["add", "-f", "prod.env"]).unwrap();
        git(&root, &["commit", "-q", "-m", "oops"]).unwrap();

        match scan_then_push(&root) {
            PushOutcome::RefusedSecrets(hits) => {
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].path, "prod.env");
                let msg = PushOutcome::RefusedSecrets(hits).summary();
                assert!(msg.contains("prod.env"), "the refusal must name the file: {msg}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(
            git(&root, &["rev-parse", "origin/main"]).unwrap(),
            before,
            "a refused push must not have moved the remote"
        );
    }

    /// A secret committed EARLIER still blocks: the scan reads tracked files,
    /// not the outgoing diff, so a second attempt cannot walk it through.
    #[test]
    fn an_older_committed_secret_still_blocks_a_later_push() {
        let (_dir, root) = library_with_remote();
        std::fs::write(root.join("id_rsa"), "ssh-key-bytes\n").unwrap();
        git(&root, &["add", "-f", "id_rsa"]).unwrap();
        git(&root, &["commit", "-q", "-m", "key"]).unwrap();
        assert!(matches!(
            scan_then_push(&root),
            PushOutcome::RefusedSecrets(_)
        ));

        // An unrelated commit on top does not clear it.
        std::fs::write(root.join("notes.md"), "# notes\nmore\n").unwrap();
        git(&root, &["add", "-A"]).unwrap();
        git(&root, &["commit", "-q", "-m", "notes"]).unwrap();
        assert!(
            matches!(scan_then_push(&root), PushOutcome::RefusedSecrets(_)),
            "the scan must not be scoped to what changed since the last attempt"
        );
    }

    #[test]
    fn a_clean_library_pushes_and_then_reports_up_to_date() {
        let (_dir, root) = library_with_remote();
        assert_eq!(scan_then_push(&root), PushOutcome::UpToDate);

        std::fs::write(root.join("notes.md"), "# notes\nlearned a thing\n").unwrap();
        git(&root, &["add", "-A"]).unwrap();
        git(&root, &["commit", "-q", "-m", "cl: a learning"]).unwrap();
        assert_eq!(scan_then_push(&root), PushOutcome::Pushed { commits: 1 });
        assert_eq!(
            git(&root, &["rev-parse", "origin/main"]).unwrap(),
            git(&root, &["rev-parse", "HEAD"]).unwrap()
        );
        assert_eq!(scan_then_push(&root), PushOutcome::UpToDate);
    }

    #[test]
    fn a_library_without_a_remote_is_not_a_failure() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        git(&root, &["init", "-q"]).unwrap();
        assert_eq!(scan_then_push(&root), PushOutcome::NoRemote);
    }
}
