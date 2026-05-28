//! Free helper functions shared across the bridge submodules. Pure functions
//! (no `&self`); `pub(super)` so sibling submodules can call them, except
//! [`extract_description`] which is only used by [`walk_cl_dir`] here.

use super::*;
use crate::policy::ViolationOutcome;
use crate::storage::Project;

/// Walk `dir` recursively; for each text-ish file (.md, .yaml, .txt) populate
/// `out` with (relative_path, mtime_iso8601, description_snippet). Skips
/// hidden files/dirs (anything starting with '.') and a few well-known noise
/// directories (`projects` at the data_dir level is handled by per-project
/// rescans, not here).
pub(super) fn walk_cl_dir(
    dir: &Path,
    root: &Path,
    project: &str,
    out: &mut HashMap<String, (String, String)>,
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
        // At the _globals root, the per-project subdirectories show up under
        // `projects/` — skip them; they'll be rescanned with their own
        // project name. Same for `.local` (sqlite + per-project lock state).
        if project == Project::GLOBALS && dir == root && (name == "projects" || name == ".local") {
            continue;
        }
        if path.is_dir() {
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
        let snippet = extract_description(&path);
        out.insert(rel, (mtime, snippet));
    }
}

/// First H1 (`# ...`) line; failing that, the first non-empty line trimmed
/// to 80 chars. Used to seed `cl_index.description` when an entry is auto-
/// added during a rescan. User can edit later via the UI.
fn extract_description(path: &Path) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
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

/// Parse the branch from a push-gate `action` string. Accepts shapes like
/// `"git push origin <branch>"`, `"git push --force origin <branch>"`, and
/// `"git push origin <branch>:<remote-ref>"`. Returns None for unparseable
/// inputs (no false-positive remembered approvals).
pub(super) fn parse_push_branch(action: &str) -> Option<String> {
    let after_push = action.trim().strip_prefix("git push")?.trim_start();
    let tokens: Vec<&str> = after_push.split_whitespace().collect();
    // Refuse if the user is doing something other than "push this branch":
    // --delete/-d removes a remote ref, which we don't want to silently
    // remember as an approved push target.
    if tokens.iter().any(|t| *t == "--delete" || *t == "-d") {
        return None;
    }
    // Strip flag-shaped tokens. The first two remaining positional tokens are
    // <remote> <ref>. If only one is present (e.g. `git push -u origin`), bail
    // rather than guess.
    let positionals: Vec<&str> = tokens.iter().copied().filter(|t| !t.starts_with('-')).collect();
    let branch_arg = match positionals.as_slice() {
        [_remote, branch, ..] => *branch,
        _ => return None,
    };
    let branch = branch_arg.split(':').next().unwrap_or(branch_arg).trim();
    if branch.is_empty() {
        return None;
    }
    Some(branch.to_string())
}

#[cfg(test)]
#[test]
fn parse_push_branch_shapes() {
    assert_eq!(
        parse_push_branch("git push origin 346-streamline-onboarding-process"),
        Some("346-streamline-onboarding-process".into())
    );
    assert_eq!(
        parse_push_branch("git push origin main:release"),
        Some("main".into())
    );
    assert_eq!(parse_push_branch("git push --force origin main"), Some("main".into()));
    assert_eq!(parse_push_branch("not a push command"), None);
    assert_eq!(parse_push_branch("git push origin --delete x"), None); // safety: refuse flag-looking branches
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
