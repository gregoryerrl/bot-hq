//! Reading a file an agent referenced, for the full-screen viewer.
//!
//! Gate cards routinely name a file instead of inlining its content
//! (`gh issue create --body-file /tmp/body.md`), which left the user approving
//! a body they could not see. This is the read behind that viewer.
//!
//! It is a UI-reachable read primitive, so containment is the whole point:
//! issues.md #1 is already "agents can read any path on disk", and this must
//! not widen that to the UI. Every read is canonicalized and required to land
//! under an allowed root.

use crate::core::AppState as CoreAppState;
use crate::tauri_cmd::error::AppError;
use base64::Engine;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Refuse to slurp something huge into a webview. Generous for prose/diffs,
/// small enough that a stray binary can't wedge the UI.
const MAX_VIEWABLE_BYTES: u64 = 2 * 1024 * 1024;

/// One file, ready to render. Exactly one of `text` / `base64` is populated —
/// `text` for anything decodable as UTF-8, `base64` for images.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct WorkspaceFile {
    /// The canonical path actually read (not what the caller passed).
    pub path: String,
    /// Basename, for the dialog title.
    pub name: String,
    /// Lowercased extension without the dot, so the UI can pick a renderer.
    pub extension: String,
    pub text: Option<String>,
    pub base64: Option<String>,
    pub bytes: u64,
}

fn is_image_ext(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp")
}

/// Roots a viewer read may touch: the session's own repo, plus the temp
/// directories where agents stage gate bodies.
///
/// Both `/tmp` AND `std::env::temp_dir()` are included on purpose. On macOS
/// `temp_dir()` is `$TMPDIR` (`/var/folders/…`), which does NOT contain `/tmp`
/// — and agents write gate bodies to a literal `/tmp/...` (every `--body-file`
/// in the archive does). Including only one of the two would reject exactly the
/// files this feature exists to show.
fn allowed_roots(repo: Option<&str>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(r) = repo {
        if let Ok(c) = Path::new(r).canonicalize() {
            roots.push(c);
        }
    }
    for t in [std::env::temp_dir(), PathBuf::from("/tmp")] {
        if let Ok(c) = t.canonicalize() {
            if !roots.contains(&c) {
                roots.push(c);
            }
        }
    }
    roots
}

/// True when `candidate` (already canonical) sits inside one of `roots`.
///
/// Canonical-vs-canonical, so `..` segments and symlink escapes are already
/// resolved away before this comparison — a lexical check would not be enough.
fn is_contained(candidate: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| candidate.starts_with(root))
}

/// The viewer's size gate, split out so a test can reach it.
///
/// It lived inline in [`read_workspace_file`], which takes a `tauri::State` and
/// therefore cannot be called from a unit test — so the refusal path had no
/// test at all, and the one named for it asserted `MAX_VIEWABLE_BYTES >= 1 MiB`
/// (a compile-time constant; clippy called it "an assertion with a constant
/// value") against a 16-byte file. Deleting the guard left that green.
///
/// Boundary is `>`: a file of exactly [`MAX_VIEWABLE_BYTES`] is viewable.
fn refuse_if_oversize(path: &str, bytes: u64) -> Result<(), AppError> {
    if bytes > MAX_VIEWABLE_BYTES {
        return Err(AppError::Validation(format!(
            "{path} is {bytes} bytes — too large to preview (limit {MAX_VIEWABLE_BYTES})"
        )));
    }
    Ok(())
}
/// Where a RELATIVE path an agent wrote resolves: the session's repo.
///
/// Gate commands name files both ways — `--body-file /tmp/522-comment.md` and
/// `--body-file pr-body1.md` — and the agent's shell ran the second one from
/// the repo. Canonicalizing a bare relative path resolved it against the APP's
/// cwd instead, so the viewer refused exactly the file the gate was about.
/// Absolute paths pass through; with no repo on the session, so does a
/// relative one (and containment then decides).
fn resolve_requested_path(path: &str, repo: Option<&str>) -> PathBuf {
    let p = Path::new(path);
    match repo {
        Some(r) if p.is_relative() => Path::new(r).join(p),
        _ => p.to_path_buf(),
    }
}


/// Read a file for the viewer dialog, scoped to the session's repo + temp.
#[tauri::command]
#[specta::specta]
pub async fn read_workspace_file(
    core: tauri::State<'_, Arc<CoreAppState>>,
    session_id: String,
    path: String,
) -> Result<WorkspaceFile, AppError> {
    let repo = core
        .storage
        .get_session(&session_id)
        .await
        .ok()
        .flatten()
        .and_then(|s| s.working_repo_path);
    // Everything below is filesystem work — canonicalize, stat, and a read of
    // up to `MAX_VIEWABLE_BYTES` — so it runs off the reactor, like its
    // sibling `cl_read_file_inner` (round 9: it ran on the 2-worker reactor).
    tokio::task::spawn_blocking(move || read_workspace_file_blocking(repo, &path))
        .await
        .map_err(|e| AppError::Internal(format!("read task failed: {e}")))?
}

fn read_workspace_file_blocking(repo: Option<String>, path: &str) -> Result<WorkspaceFile, AppError> {
    let path = path.to_string();
    let roots = allowed_roots(repo.as_deref());

    // Canonicalize FIRST — this both resolves `..`/symlinks and proves the file
    // exists. A non-existent path can't be canonicalized, so "missing" and
    // "outside scope" are reported distinctly rather than as one vague error.
    let canonical = resolve_requested_path(&path, repo.as_deref())
        .canonicalize()
        .map_err(|e| AppError::NotFound(format!("cannot read {path}: {e}")))?;
    if !is_contained(&canonical, &roots) {
        return Err(AppError::Unauthorized(format!(
            "refused: {} is outside this session's workspace and temp dirs",
            canonical.display()
        )));
    }

    let meta = std::fs::metadata(&canonical)
        .map_err(|e| AppError::Internal(format!("cannot stat {path}: {e}")))?;
    if meta.is_dir() {
        return Err(AppError::Validation(format!("{path} is a directory")));
    }
    let bytes = meta.len();
    refuse_if_oversize(&path, bytes)?;

    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let extension = canonical
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let raw = std::fs::read(&canonical)
        .map_err(|e| AppError::Internal(format!("cannot read {path}: {e}")))?;
    let (text, b64) = if is_image_ext(&extension) {
        (
            None,
            Some(base64::engine::general_purpose::STANDARD.encode(&raw)),
        )
    } else {
        match String::from_utf8(raw) {
            Ok(s) => (Some(s), None),
            // Binary non-image: say so rather than rendering mojibake.
            Err(_) => (
                Some(format!("(binary file — {bytes} bytes, not previewable)")),
                None,
            ),
        }
    };

    Ok(WorkspaceFile {
        path: canonical.to_string_lossy().to_string(),
        name,
        extension,
        text,
        base64: b64,
        bytes,
    })
}

/// Extensions the paste-image save accepts — clipboard image data only; a
/// pasted FILE arrives as a `file://` URI and never reaches this command.
fn is_pasteable_ext(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp")
}

/// Refuse a pasted blob bigger than the viewer would show anyway.
const MAX_PASTED_BYTES: usize = 10 * 1024 * 1024;

/// Where pasted clipboard images land: a per-session subdir under the OS temp
/// dir — inside `allowed_roots`, so the viewer can show what was pasted, and
/// readable by agents (issue: ideas.md 2026-08-24, paste files into the box).
/// The session id is sanitized to its `[a-z0-9-]` characters, so a hostile id
/// cannot traverse; the uuid filename never collides.
fn pasted_file_path(session_id: &str, ext: &str) -> PathBuf {
    let safe: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let safe = if safe.is_empty() { "session".to_string() } else { safe };
    std::env::temp_dir()
        .join("bothq-paste")
        .join(safe)
        .join(format!("{}.{ext}", uuid::Uuid::new_v4()))
}

/// Save clipboard IMAGE bytes the composer received on paste, returning the
/// absolute path the box inserts (and the agent later Reads). Temp-dir-only by
/// construction — the path is built here, never taken from the caller.
#[tauri::command]
#[specta::specta]
pub async fn save_pasted_file(
    session_id: String,
    bytes: Vec<u8>,
    ext: String,
) -> Result<String, AppError> {
    let ext = ext.to_ascii_lowercase();
    if !is_pasteable_ext(&ext) {
        return Err(AppError::Validation(format!(
            "unsupported pasted type .{ext} — images only"
        )));
    }
    if bytes.is_empty() {
        return Err(AppError::Validation("empty clipboard image".into()));
    }
    if bytes.len() > MAX_PASTED_BYTES {
        return Err(AppError::Validation(format!(
            "pasted image is {} bytes; the cap is {MAX_PASTED_BYTES}",
            bytes.len()
        )));
    }
    let path = pasted_file_path(&session_id, &ext);
    tokio::task::spawn_blocking(move || -> Result<String, AppError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| AppError::Internal(format!("cannot create paste dir: {e}")))?;
        }
        std::fs::write(&path, &bytes)
            .map_err(|e| AppError::Internal(format!("cannot write pasted file: {e}")))?;
        Ok(path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| AppError::Internal(format!("paste task failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_file_lands_under_the_temp_root_the_viewer_allows() {
        // The whole point of the per-session temp subdir: the path this
        // command mints is inside `allowed_roots`, so the "View" affordance
        // works on what was just pasted — and a hostile session id cannot
        // steer it anywhere else.
        let p = pasted_file_path("s-abc123", "png");
        let roots = allowed_roots(None);
        let parent = p.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let canon_parent = parent.canonicalize().unwrap();
        assert!(
            is_contained(&canon_parent, &roots),
            "paste dir {canon_parent:?} must sit under an allowed root"
        );
        let evil = pasted_file_path("../../../etc", "png");
        // Normalize separators for the substring check — the minted path is a
        // native one (`bothq-paste\etc` on Windows), and only the needle was
        // Unix-shaped. The BEHAVIOUR was always right: `../../../etc` is
        // stripped to `etc`, which is exactly what this asserts.
        assert!(
            evil.to_string_lossy()
                .replace('\\', "/")
                .contains("bothq-paste/etc"),
            "traversal characters are stripped, not honoured: {evil:?}"
        );
    }

    #[test]
    fn pasteable_extensions_are_images_only() {
        assert!(is_pasteable_ext("png"));
        assert!(is_pasteable_ext("webp"));
        assert!(!is_pasteable_ext("svg"), "svg is scriptable — not pasteable");
        assert!(!is_pasteable_ext("php"));
        assert!(!is_pasteable_ext(""));
    }


    #[test]
    fn a_relative_path_resolves_against_the_session_repo() {
        assert_eq!(
            resolve_requested_path("pr-body1.md", Some("/repo")),
            PathBuf::from("/repo/pr-body1.md")
        );
        assert_eq!(
            resolve_requested_path("docs/x.md", Some("/repo")),
            PathBuf::from("/repo/docs/x.md")
        );
        // Absolute stays absolute; no repo leaves it alone.
        assert_eq!(
            resolve_requested_path("/tmp/522-comment.md", Some("/repo")),
            PathBuf::from("/tmp/522-comment.md")
        );
        assert_eq!(
            resolve_requested_path("pr-body1.md", None),
            PathBuf::from("pr-body1.md")
        );
    }

    /// The composition the change actually depends on: the join is only the
    /// first step, and containment still runs on the CANONICAL result — so a
    /// relative traversal joined onto the repo escapes it and is refused. A
    /// later "skip canonicalize for relative paths" shortcut goes red here.
    #[test]
    fn a_relative_traversal_joined_onto_the_repo_is_still_refused() {
        let repo = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        // A real file OUTSIDE the repo, reached from inside it by `..` hops.
        let target = outside.path().join("secret.md");
        std::fs::write(&target, "not yours").unwrap();
        let repo_canon = repo.path().canonicalize().unwrap();
        let outside_canon = outside.path().canonicalize().unwrap();
        // Build a relative path from the repo to the outside file.
        // Count only NORMAL components. The old `components().count() - 1`
        // assumed exactly one non-Normal leading component, which is true on
        // Unix (`RootDir`) and false on Windows, where a canonical path leads
        // with BOTH `Prefix(VerbatimDisk)` and `RootDir` — so it produced one
        // `..` too few and the traversal never climbed to the drive root.
        let hops = repo_canon
            .components()
            .filter(|c| matches!(c, std::path::Component::Normal(_)))
            .count();
        let mut rel = PathBuf::new();
        for _ in 0..hops {
            rel.push("..");
        }
        // `outside_canon` is absolute; keep only its NORMAL components so the
        // join stays relative. `skip(1)` assumed exactly one leading component,
        // which holds on Unix — a Windows canonical path has TWO
        // (`Prefix(VerbatimDisk)` + `RootDir`), so `skip(1)` left `RootDir`
        // first and `PathBuf::push` of a root component RESETS the path.
        //
        // STILL RED ON WINDOWS, cause NOT yet identified. Both this and the
        // `hops` count above are genuine corrections, and neither fixed it:
        // `resolve_requested_path` returns its input unchanged when
        // `p.is_relative()` is false, and the observed `joined` is the bare
        // outside path — so `rel` is somehow still absolute here. Recorded
        // rather than guessed at a third time.
        //
        // NOT a product defect: `is_contained` is canonical-vs-canonical
        // component-wise `starts_with`, `allowed_roots` SKIPS any root it
        // cannot canonicalize, and an empty root set refuses. The guard is
        // fail-closed in every direction; what is broken is this test's own
        // path arithmetic, which dies at its SETUP assertion before ever
        // reaching the `!is_contained(...)` line it exists to check.
        for c in outside_canon
            .components()
            .filter(|c| matches!(c, std::path::Component::Normal(_)))
        {
            rel.push(c);
        }
        rel.push("secret.md");
        // Join against the NON-canonicalized repo path — which is also what
        // production passes, since a session's `working_repo_path` is a
        // user-supplied string rather than a canonicalized one.
        //
        // It matters on Windows: `canonicalize` always returns a VERBATIM
        // `\\?\` path, verbatim paths cannot contain `..` (the OS takes them
        // literally), so `PathBuf::push` RESOLVES `..` against a verbatim base
        // at join time instead of concatenating it lexically. The traversal
        // this test needs is unconstructible that way — the hops are eaten
        // before `canonicalize` ever sees them, which is why two rounds of hop
        // arithmetic produced a byte-identical failure.
        let repo_base = repo.path();
        let joined = resolve_requested_path(rel.to_str().unwrap(), repo_base.to_str());
        assert!(
            joined.starts_with(repo_base),
            "joined under the repo: rel={rel:?} joined={joined:?}"
        );
        let canonical = joined.canonicalize().expect("the traversal names a real file");
        assert_eq!(canonical, target.canonicalize().unwrap());
        assert!(
            !is_contained(&canonical, std::slice::from_ref(&repo_canon)),
            "the canonical path left the repo and must be refused: {canonical:?}"
        );
    }

    #[test]
    fn containment_accepts_inside_and_rejects_outside() {
        let root = tempfile::tempdir().unwrap();
        let root_c = root.path().canonicalize().unwrap();
        let inside = root_c.join("a.md");
        std::fs::write(&inside, "hi").unwrap();

        let other = tempfile::tempdir().unwrap();
        let outside = other.path().canonicalize().unwrap().join("b.md");
        std::fs::write(&outside, "nope").unwrap();

        let roots = vec![root_c.clone()];
        assert!(is_contained(&inside.canonicalize().unwrap(), &roots));
        assert!(!is_contained(&outside.canonicalize().unwrap(), &roots));
    }

    #[test]
    fn traversal_out_of_the_root_is_rejected_after_canonicalizing() {
        // The lexical path stays "under" the root while pointing outside it —
        // only canonicalization catches this, which is why the check runs on
        // the canonical form.
        let root = tempfile::tempdir().unwrap();
        let root_c = root.path().canonicalize().unwrap();
        let secret = root_c.parent().unwrap().join("escaped.md");
        std::fs::write(&secret, "secret").unwrap();

        let sneaky = root_c.join("..").join("escaped.md");
        let resolved = sneaky.canonicalize().unwrap();
        assert!(
            !is_contained(&resolved, &[root_c]),
            "`..` escape must not be contained: {}",
            resolved.display()
        );
        let _ = std::fs::remove_file(&secret);
    }

    #[test]
    fn tmp_is_a_root_even_where_temp_dir_differs() {
        // macOS: temp_dir() is $TMPDIR (/var/folders/…) and does NOT cover
        // /tmp, but agents stage gate bodies at a literal /tmp path.
        let roots = allowed_roots(None);
        let tmp = Path::new("/tmp");
        if let Ok(tmp_c) = tmp.canonicalize() {
            assert!(
                roots.iter().any(|r| tmp_c.starts_with(r)),
                "/tmp must be readable by the viewer; roots were {roots:?}"
            );
        }
    }

    #[test]
    fn image_extensions_route_to_base64() {
        assert!(is_image_ext("png"));
        assert!(is_image_ext("svg"));
        assert!(!is_image_ext("md"));
        assert!(!is_image_ext(""));
    }

    /// **The refusal path actually runs** (round-2 R2).
    ///
    /// The previous version wrote a 16-byte file and asserted
    /// `MAX_VIEWABLE_BYTES >= 1024 * 1024` — a comparison between two
    /// compile-time constants, which clippy flagged as "an assertion with a
    /// constant value". It never entered the branch it is named for, so
    /// deleting the guard left it green. Its comment was honest about the
    /// trade ("assert the constant is sane rather than writing 2 MiB"); the
    /// name was not.
    ///
    /// `set_len` makes the oversize file SPARSE — no blocks are written, so
    /// this costs nothing and `metadata().len()` still reports the real size,
    /// which is the exact value the guard reads.
    #[test]
    fn oversize_files_are_refused_by_the_limit() {
        let dir = tempfile::tempdir().unwrap();

        let big = dir.path().join("big.bin");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_VIEWABLE_BYTES + 1)
            .unwrap();
        let err = refuse_if_oversize("big.bin", std::fs::metadata(&big).unwrap().len())
            .expect_err("a file over the limit must be refused");
        assert!(
            matches!(err, AppError::Validation(ref m) if m.contains("too large to preview")),
            "wrong refusal: {err:?}"
        );

        // The boundary is `>`, not `>=` — a file of exactly the limit is
        // viewable, and an off-by-one here silently shrinks what the viewer
        // will open.
        let edge = dir.path().join("edge.bin");
        std::fs::File::create(&edge)
            .unwrap()
            .set_len(MAX_VIEWABLE_BYTES)
            .unwrap();
        assert!(
            refuse_if_oversize("edge.bin", std::fs::metadata(&edge).unwrap().len()).is_ok(),
            "a file of exactly MAX_VIEWABLE_BYTES must still be viewable"
        );
    }
}
