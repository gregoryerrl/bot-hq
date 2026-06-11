//! Per-session git worktrees — parallel sessions on one project.
//!
//! When a session opts in (the default for repo-backed sessions), it runs in
//! an isolated `git worktree` carved from the project repo instead of the
//! repo itself: `sessions.working_repo_path` points at the WORKTREE (so every
//! downstream consumer — action_gate, hook install, diff anchoring, project
//! derivation — keeps working unchanged) and `sessions.base_repo_path` keeps
//! the main repo for `git worktree add/remove`.
//!
//! Layout: `<data_dir>/.local/worktrees/<session-id>/<repo-basename>/`.
//! The repo basename stays the FINAL path segment because
//! `spawn_session_handle` derives the session's project from
//! `working_repo_path.file_name()` — a session-id leaf would break the
//! project → policy/CL mapping.
//!
//! Branch: `bothq/<session-id>`, created at the base repo's HEAD on first
//! ensure. Two worktrees can't check out one branch, so per-session branches
//! are inherent to the feature; merging back is the user's flow.
//!
//! All functions shell out to `git` synchronously — call from a blocking
//! context (`spawn_blocking`) on the async paths.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a session's worktree lives. `None` when the base repo path has no
/// final component to mirror (e.g. `/`).
pub fn session_worktree_path(
    data_dir: &Path,
    session_id: &str,
    base_repo: &Path,
) -> Option<PathBuf> {
    let basename = base_repo.file_name()?;
    Some(
        data_dir
            .join(".local")
            .join("worktrees")
            .join(session_id)
            .join(basename),
    )
}

/// The session's dedicated branch. Session ids are short (`s-xxxxxxxx`), so
/// the ref stays readable: `bothq/s-xxxxxxxx`.
pub fn branch_for_session(session_id: &str) -> String {
    format!("bothq/{session_id}")
}

fn git(repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("running git -C {} {args:?}", repo.display()))
}

fn git_ok(repo: &Path, args: &[&str]) -> Result<()> {
    let out = git(repo, args)?;
    if !out.status.success() {
        bail!(
            "git -C {} {:?} failed: {}",
            repo.display(),
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn branch_exists(base_repo: &Path, branch: &str) -> bool {
    git(
        base_repo,
        &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")],
    )
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// Idempotently materialize the session worktree at `worktree`, on `branch`.
///
/// - Valid worktree already at the path → no-op (respawn/restart path).
/// - Path missing → `git worktree prune` (clears a stale registration left by
///   a manual delete), then `git worktree add` — `-b <branch>` on first
///   ensure, plain `<branch>` checkout when the branch survived a prune.
/// - Path exists but is NOT a worktree → error (never adopt or clobber a
///   foreign directory).
///
/// Errors (missing git, unwritable parent, …) are the caller's signal to
/// fall back to running directly in the base repo. A commitless base repo is
/// git-version-dependent: modern git infers `--orphan` and succeeds (the
/// session works on an orphan branch); older git errors → fallback.
pub fn ensure_worktree(base_repo: &Path, worktree: &Path, branch: &str) -> Result<()> {
    if !base_repo.join(".git").exists() {
        bail!(
            "base repo {} has no .git — cannot create a session worktree",
            base_repo.display()
        );
    }
    if worktree.exists() {
        // `.git` in a linked worktree is a FILE pointing at the common dir.
        if worktree.join(".git").exists() {
            return Ok(());
        }
        bail!(
            "worktree path {} exists but is not a git worktree — refusing to touch it",
            worktree.display()
        );
    }
    if let Some(parent) = worktree.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating worktree parent {}", parent.display()))?;
    }
    // A worktree dir deleted out from under git leaves a stale registration
    // that blocks re-adding at the same path; prune is a no-op otherwise.
    let _ = git(base_repo, &["worktree", "prune"]);
    let path_str = worktree
        .to_str()
        .with_context(|| format!("non-UTF8 worktree path {}", worktree.display()))?;
    if branch_exists(base_repo, branch) {
        git_ok(base_repo, &["worktree", "add", path_str, branch])
    } else {
        git_ok(base_repo, &["worktree", "add", path_str, "-b", branch])
    }
}

/// Outcome of a close-time removal attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// Worktree was clean and is gone.
    Removed,
    /// `git worktree remove` refused (dirty / untracked work) — the worktree
    /// is KEPT so nothing uncommitted is ever discarded. Carries git's reason.
    Kept(String),
    /// The path was already gone; stale registration pruned.
    Gone,
}

/// Remove the session worktree if and only if it is clean. Never forces:
/// a dirty worktree outlives its session for manual recovery. The session
/// branch is always kept (the work lives there).
pub fn remove_worktree_if_clean(base_repo: &Path, worktree: &Path) -> RemoveOutcome {
    if !worktree.exists() {
        let _ = git(base_repo, &["worktree", "prune"]);
        return RemoveOutcome::Gone;
    }
    let path_str = match worktree.to_str() {
        Some(s) => s,
        None => return RemoveOutcome::Kept("non-UTF8 worktree path".into()),
    };
    match git(base_repo, &["worktree", "remove", path_str]) {
        Ok(out) if out.status.success() => {
            // Tidy the now-empty `<sid>/` holder dir (basename layout nests one
            // level under the session id). Best-effort.
            if let Some(parent) = worktree.parent() {
                let _ = std::fs::remove_dir(parent);
            }
            RemoveOutcome::Removed
        }
        Ok(out) => RemoveOutcome::Kept(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ),
        Err(e) => RemoveOutcome::Kept(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// `git init` + one commit so HEAD exists. Identity is passed per-command
    /// so the tests don't depend on (or mutate) global git config.
    fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args([
                    "-c",
                    "user.email=t@example.com",
                    "-c",
                    "user.name=t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("README"), "hi\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
    }

    fn setup() -> (TempDir, PathBuf, TempDir) {
        let repo_dir = TempDir::new().unwrap();
        let base = repo_dir.path().join("myproject");
        std::fs::create_dir(&base).unwrap();
        init_repo(&base);
        let data_dir = TempDir::new().unwrap();
        (repo_dir, base, data_dir)
    }

    #[test]
    fn worktree_path_keeps_repo_basename_leaf() {
        let p = session_worktree_path(Path::new("/data"), "s-ab12cd34", Path::new("/x/myproject"))
            .unwrap();
        assert_eq!(
            p,
            Path::new("/data/.local/worktrees/s-ab12cd34/myproject")
        );
        // The leaf drives project derivation at spawn — must be the basename.
        assert_eq!(p.file_name().unwrap(), "myproject");
    }

    #[test]
    fn ensure_creates_worktree_and_branch_then_is_idempotent() {
        let (_g, base, data) = setup();
        let wt = session_worktree_path(data.path(), "s-11111111", &base).unwrap();
        let branch = branch_for_session("s-11111111");
        ensure_worktree(&base, &wt, &branch).unwrap();
        assert!(wt.join(".git").is_file(), "worktree .git must be a file");
        assert!(branch_exists(&base, &branch));
        // Second ensure (respawn path): no-op.
        ensure_worktree(&base, &wt, &branch).unwrap();
        // The worktree is checked out on the session branch.
        let head = git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&head.stdout).trim(),
            branch.as_str()
        );
    }

    #[test]
    fn two_sessions_get_parallel_worktrees() {
        let (_g, base, data) = setup();
        for sid in ["s-aaaa0001", "s-aaaa0002"] {
            let wt = session_worktree_path(data.path(), sid, &base).unwrap();
            ensure_worktree(&base, &wt, &branch_for_session(sid)).unwrap();
            assert!(wt.join("README").exists());
        }
    }

    #[test]
    fn ensure_recovers_from_manual_delete() {
        let (_g, base, data) = setup();
        let wt = session_worktree_path(data.path(), "s-deleted1", &base).unwrap();
        let branch = branch_for_session("s-deleted1");
        ensure_worktree(&base, &wt, &branch).unwrap();
        std::fs::remove_dir_all(&wt).unwrap();
        // Stale registration: re-ensure must prune + re-add on the surviving
        // branch.
        ensure_worktree(&base, &wt, &branch).unwrap();
        assert!(wt.join("README").exists());
    }

    #[test]
    fn ensure_refuses_foreign_directory() {
        let (_g, base, data) = setup();
        let wt = session_worktree_path(data.path(), "s-foreign1", &base).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join("precious.txt"), "not ours").unwrap();
        let err = ensure_worktree(&base, &wt, "bothq/s-foreign1").unwrap_err();
        assert!(err.to_string().contains("not a git worktree"));
        assert!(wt.join("precious.txt").exists());
    }

    #[test]
    fn ensure_on_commitless_base_errors_or_yields_valid_worktree() {
        // Behavior is git-version-dependent: modern git (≥ ~2.48) infers
        // `--orphan` and succeeds; older git fails ("invalid reference").
        // Either is fine — the invariant is "Ok ⇒ a real worktree exists,
        // Err ⇒ nothing half-made was left at the path".
        let dir = TempDir::new().unwrap();
        let base = dir.path().join("empty");
        std::fs::create_dir(&base).unwrap();
        let out = Command::new("git")
            .arg("-C")
            .arg(&base)
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(out.status.success());
        let data = TempDir::new().unwrap();
        let wt = session_worktree_path(data.path(), "s-empty001", &base).unwrap();
        match ensure_worktree(&base, &wt, "bothq/s-empty001") {
            Ok(()) => assert!(wt.join(".git").exists(), "Ok must mean a valid worktree"),
            Err(_) => assert!(!wt.join(".git").exists(), "Err must not leave debris"),
        }
    }

    #[test]
    fn remove_clean_removes_and_tidies_holder() {
        let (_g, base, data) = setup();
        let wt = session_worktree_path(data.path(), "s-clean001", &base).unwrap();
        ensure_worktree(&base, &wt, &branch_for_session("s-clean001")).unwrap();
        assert_eq!(
            remove_worktree_if_clean(&base, &wt),
            RemoveOutcome::Removed
        );
        assert!(!wt.exists());
        // The <sid>/ holder dir is tidied too.
        assert!(!wt.parent().unwrap().exists());
        // Branch survives — the work lives there.
        assert!(branch_exists(&base, "bothq/s-clean001"));
    }

    #[test]
    fn remove_keeps_dirty_worktree() {
        let (_g, base, data) = setup();
        let wt = session_worktree_path(data.path(), "s-dirty001", &base).unwrap();
        ensure_worktree(&base, &wt, &branch_for_session("s-dirty001")).unwrap();
        std::fs::write(wt.join("uncommitted.rs"), "fn wip() {}\n").unwrap();
        match remove_worktree_if_clean(&base, &wt) {
            RemoveOutcome::Kept(_) => {}
            other => panic!("dirty worktree must be kept, got {other:?}"),
        }
        assert!(wt.join("uncommitted.rs").exists());
    }

    #[test]
    fn remove_on_missing_path_is_gone() {
        let (_g, base, data) = setup();
        let wt = session_worktree_path(data.path(), "s-gone0001", &base).unwrap();
        ensure_worktree(&base, &wt, &branch_for_session("s-gone0001")).unwrap();
        std::fs::remove_dir_all(&wt).unwrap();
        assert_eq!(remove_worktree_if_clean(&base, &wt), RemoveOutcome::Gone);
        // Registration pruned → a fresh ensure works again.
        ensure_worktree(&base, &wt, &branch_for_session("s-gone0001")).unwrap();
    }
}
