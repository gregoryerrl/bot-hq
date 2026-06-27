//! Free helper functions shared across the bridge submodules. Pure functions
//! (no `&self`); `pub(super)` so sibling submodules can call them. `walk_cl_dir`
//! reads each CL file once into a [`WalkedFile`] (snippet for the index + full
//! body for atom splitting); [`split_into_atoms`] turns a body into FTS atoms.

use super::*;
use crate::paths::IGNORED_BUILD_DIRS;
use crate::policy::ViolationOutcome;
use crate::storage::{Atom, Project};

/// One indexed CL file as seen on disk by [`walk_cl_dir`]: its mtime (RFC3339),
/// the short `description` snippet (first H1 / first 80 chars), and the FULL body
/// (for atom splitting). The file is read exactly once to fill all three.
pub(super) struct WalkedFile {
    pub(super) mtime: String,
    pub(super) snippet: String,
    pub(super) body: String,
}

/// Walk `dir` recursively; for each text-ish file (.md, .yaml, .txt) populate
/// `out` with `relative_path -> WalkedFile { mtime, snippet, body }`. Skips
/// hidden files/dirs (anything starting with '.') and a few well-known noise
/// directories (`projects` at the CL-dir (`library/`) level is handled by
/// per-project rescans, not here).
pub(super) fn walk_cl_dir(
    dir: &Path,
    root: &Path,
    project: &str,
    out: &mut HashMap<String, WalkedFile>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        // At the _globals root (the CL dir, `<data_dir>/library/`), the
        // per-project subdirectories show up under `projects/` — skip them;
        // they'll be rescanned with their own project name.
        if project == Project::GLOBALS && dir == root && name == "projects" {
            continue;
        }
        if path.is_dir() {
            // Skip build/dependency dirs — a repo-rooted cl_path otherwise pulls
            // every node_modules/target text file into the index.
            if IGNORED_BUILD_DIRS.contains(&name) {
                continue;
            }
            walk_cl_dir(&path, root, project, out);
            continue;
        }
        // Only index human-readable text-ish files. Binary / large data files
        // don't belong in the agent's discovery surface.
        let is_text = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md" | "yaml" | "yml" | "txt" | "toml" | "json")
        );
        if !is_text {
            continue;
        }
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        let mtime = match entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(chrono::DateTime::<chrono::Utc>::from)
        {
            Some(t) => t.to_rfc3339(),
            None => continue,
        };
        // Read the file ONCE: derive the index snippet and keep the full body so
        // cl_rescan can split it into atoms without a second read.
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let snippet = extract_description(&content);
        out.insert(rel, WalkedFile { mtime, snippet, body: content });
    }
}

/// First H1 (`# ...`) line; failing that, the first non-empty line trimmed
/// to 80 chars. Used to seed `cl_index.description` when an entry is auto-
/// added during a rescan. Takes the already-read file `content` so
/// [`walk_cl_dir`] reads each file only once. User can edit later via the UI.
fn extract_description(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return rest.trim().to_string();
        }
    }
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() <= 80 {
            return trimmed.to_string();
        }
        return trimmed.chars().take(80).collect::<String>() + "…";
    }
    "(empty file)".to_string()
}

/// A line-start ATX heading (`#`/`##`/`###` then a space/tab then text). Returns
/// the level (1–3) and trimmed heading text. NOT a heading: indented `#`, `#tag`
/// (no space), `####`+ (h4+ falls through to body), or a `#` mid-line.
fn heading_level(line: &str) -> Option<(usize, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if (1..=3).contains(&hashes) {
        let rest = &line[hashes..];
        if rest.starts_with(' ') || rest.starts_with('\t') {
            return Some((hashes, rest.trim()));
        }
    }
    None
}

/// Split markdown `content` into heading-delimited [`Atom`]s for the FTS index.
/// Each `#`/`##`/`###` heading opens a section whose `heading_path` is the
/// "H1 > H2" breadcrumb of the enclosing headings; content before the first
/// heading becomes an `(intro)` atom. Empty sections (a heading with no body of
/// its own — e.g. a parent that only holds sub-headings) are dropped; the heading
/// still appears in its children's paths. h4+ and non-line-start `#` are body.
pub(super) fn split_into_atoms(content: &str) -> Vec<Atom> {
    fn flush(path: &Option<String>, body: &[&str], atoms: &mut Vec<Atom>) {
        let text = body.join("\n").trim().to_string();
        if text.is_empty() {
            return;
        }
        atoms.push(Atom {
            heading_path: path.clone().unwrap_or_else(|| "(intro)".to_string()),
            body: text,
        });
    }

    let mut atoms = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut path: Option<String> = None; // None until the first heading → "(intro)"
    let mut body: Vec<&str> = Vec::new();

    for line in content.lines() {
        if let Some((level, text)) = heading_level(line) {
            flush(&path, &body, &mut atoms);
            body.clear();
            while stack.last().is_some_and(|(l, _)| *l >= level) {
                stack.pop();
            }
            stack.push((level, text.to_string()));
            path = Some(stack.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" > "));
        } else {
            body.push(line);
        }
    }
    flush(&path, &body, &mut atoms);
    atoms
}

/// Map a picked option string to an outcome enum. Anything that starts with
/// "approve" (case-insensitive) counts as Approved; everything else Denied.
/// Abandoned isn't reachable via resolve_choice (that path requires a pick).
pub(super) fn outcome_from_picked(picked: &str) -> ViolationOutcome {
    let lower = picked.to_lowercase();
    if lower.starts_with("approve") || lower == "ok" || lower == "yes" {
        ViolationOutcome::Approved
    } else {
        ViolationOutcome::Denied
    }
}

/// Build the out-of-band "your question resolved" message body fed back to an
/// agent that is no longer blocked on the original `ask_user_choice` tool
/// call — either because the MCP call timed out client-side, or because the
/// session was closed + reopened and the asking subprocess was replaced.
/// Shared by both resolve_choice fallbacks (dropped-receiver and the
/// reopened-session `None` path) so the wording stays identical.
pub(super) fn oob_resolution_body(agent_label: &str, question: &str, picked: &str) -> String {
    format!(
        "(out-of-band) Your earlier `ask_user_choice` for {agent_label} resolved while \
         you were no longer waiting on the tool call.\n\n\
         **Question:** {question}\n\
         **User picked:** {picked}\n\n\
         Treat this as the user's reply. Continue from here."
    )
}

#[cfg(test)]
mod tests {
    use super::{split_into_atoms, walk_cl_dir, WalkedFile};
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn walk_cl_dir_skips_build_and_dependency_dirs() {
        let base = std::env::temp_dir().join(format!("bot-hq-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(base.join("target")).unwrap();
        fs::create_dir_all(base.join("docs")).unwrap();
        fs::write(base.join("README.md"), "# readme").unwrap();
        fs::write(base.join("docs/guide.md"), "# guide").unwrap();
        fs::write(base.join("node_modules/pkg/package.json"), "{}").unwrap();
        fs::write(base.join("target/out.json"), "{}").unwrap();
        // macOS temp_dir is a /var -> /private/var symlink; canonicalize so the
        // strip_prefix in walk_cl_dir matches.
        let root = base.canonicalize().unwrap();

        let mut out: HashMap<String, WalkedFile> = HashMap::new();
        walk_cl_dir(&root, &root, "p", &mut out);

        let mut keys: Vec<_> = out.keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["README.md".to_string(), "docs/guide.md".to_string()]
        );
        // The full body is captured (not just the snippet) for atom splitting.
        assert_eq!(out["README.md"].body, "# readme");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn split_into_atoms_builds_heading_paths_and_intro() {
        let md = "preamble line\n# Title\nunder title\n## Section A\ncontent A\n### Deep\ndeep text\n## Section B\ncontent B\n";
        let atoms = split_into_atoms(md);
        let pairs: Vec<(&str, &str)> = atoms
            .iter()
            .map(|a| (a.heading_path.as_str(), a.body.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("(intro)", "preamble line"),
                ("Title", "under title"),
                ("Title > Section A", "content A"),
                ("Title > Section A > Deep", "deep text"),
                ("Title > Section B", "content B"),
            ]
        );
    }

    #[test]
    fn split_into_atoms_ignores_non_headings_and_drops_empty() {
        // mid-line '#', '#tag' (no space), and h4+ are body text, not splits; a
        // heading with no body of its own (Empty) is dropped — its path still
        // rides on the next child.
        let md = "# Real\nbody with # mid-line hash\n#nospace stays body\n#### h4 stays body\n## Empty\n## Has Body\nx\n";
        let atoms = split_into_atoms(md);
        let pairs: Vec<(&str, &str)> = atoms
            .iter()
            .map(|a| (a.heading_path.as_str(), a.body.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("Real", "body with # mid-line hash\n#nospace stays body\n#### h4 stays body"),
                ("Real > Has Body", "x"),
            ]
        );
    }

    #[test]
    fn split_into_atoms_empty_or_blank_is_no_atoms() {
        assert!(split_into_atoms("").is_empty());
        assert!(split_into_atoms("   \n\n  ").is_empty());
    }
}
