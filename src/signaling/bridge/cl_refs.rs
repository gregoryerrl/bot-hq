//! Atom -> code coupling for retrieval-time stale-flagging (CL v2, P1.2).
//!
//! A CL note often cites the code it documents (bot-hq's `file:line` convention,
//! e.g. `src/signaling/bridge/util.rs:133`). We hash the *referenced code* when an
//! atom is indexed ([`compute_code_hash`], called on the `cl_rescan` write path)
//! and recompute it at retrieval; a mismatch means the cited code drifted since
//! the note was written, so the atom is flagged possibly-stale. Distinct from
//! `body_hash`, which hashes the atom's own text — this hashes what it is ABOUT.
//!
//! Repo coupling lives here (and in the bridge wrapper), never in the pure
//! storage layer.

use std::collections::BTreeSet;
use std::path::Path;

/// Extract repo-relative source-file references from an atom `body`, keeping only
/// those that resolve to an existing file under `repo_root`. The candidate scan
/// is liberal (any token containing `/`), pruned by an on-disk existence check —
/// so prose paths, URLs, and typos don't anchor staleness. A trailing
/// `:line[:col]` (the clickable `file:line` form) is stripped. Returns a sorted,
/// de-duplicated list so the combined hash is deterministic.
pub(crate) fn extract_code_refs(body: &str, repo_root: &Path) -> Vec<String> {
    let mut refs: BTreeSet<String> = BTreeSet::new();
    for raw in body.split(|c: char| c.is_whitespace() || "`()[]{}<>\"'*,;|".contains(c)) {
        if let Some(rel) = normalize_candidate(raw) {
            if repo_root.join(&rel).is_file() {
                refs.insert(rel);
            }
        }
    }
    refs.into_iter().collect()
}

/// Normalize one whitespace/punctuation-delimited token into a candidate
/// repo-relative path, or `None` if it can't be one. Strips a `:line[:col]`
/// suffix and stray leading/trailing markdown punctuation; rejects absolute
/// paths, parent-escapes, and anything without a directory separator.
fn normalize_candidate(raw: &str) -> Option<String> {
    let path = raw.split(':').next().unwrap_or(raw);
    let path = path.trim_matches(|c: char| matches!(c, '.' | '`' | '#' | '!'));
    if path.is_empty() || path.starts_with('/') || path.contains("..") || !path.contains('/') {
        return None;
    }
    Some(path.to_string())
}

/// Combined SHA-256 over the current content of every source file the atom `body`
/// cites under `repo_root`, or `None` when it cites no existing source (then the
/// atom is never stale-flagged). Deterministic: refs are sorted and each
/// contributes `path \0 sha256(content)`.
pub(crate) fn compute_code_hash(body: &str, repo_root: &Path) -> Option<String> {
    let refs = extract_code_refs(body, repo_root);
    if refs.is_empty() {
        return None;
    }
    let mut combined = String::new();
    for rel in &refs {
        let bytes = std::fs::read(repo_root.join(rel)).unwrap_or_default();
        combined.push_str(rel);
        combined.push('\0');
        combined.push_str(&sha256_hex(&bytes));
        combined.push('\n');
    }
    Some(sha256_hex(combined.as_bytes()))
}

/// Lowercase-hex SHA-256, matching `cl_atoms::atom_body_hash` (stable across
/// processes and toolchains — not `DefaultHasher`).
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_repo(tag: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("bot-hq-clrefs-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("src")).unwrap();
        base.canonicalize().unwrap()
    }

    #[test]
    fn extract_keeps_existing_strips_line_and_drops_bogus() {
        let root = temp_repo("extract");
        fs::write(root.join("src/foo.rs"), "fn foo() {}").unwrap();
        let body = "See `src/foo.rs:133` for the helper. Not real: src/missing.rs. \
                    Also https://example.com/x and a bare word.";
        let refs = extract_code_refs(body, &root);
        assert_eq!(refs, vec!["src/foo.rs".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn compute_code_hash_tracks_file_and_is_none_without_refs() {
        let root = temp_repo("hash");
        fs::write(root.join("src/foo.rs"), "fn foo() {}").unwrap();
        let body = "documents `src/foo.rs`";

        let h1 = compute_code_hash(body, &root).expect("has an existing ref");
        // Stable for identical content...
        assert_eq!(Some(h1.clone()), compute_code_hash(body, &root));
        // ...changes when the cited code changes...
        fs::write(root.join("src/foo.rs"), "fn foo() { changed() }").unwrap();
        assert_ne!(Some(h1), compute_code_hash(body, &root));
        // ...and is None when the atom cites no existing source.
        assert_eq!(compute_code_hash("just prose, no refs", &root), None);
        let _ = fs::remove_dir_all(&root);
    }
}
