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
        let trimmed = body.join("\n").trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        let heading_path = path.clone().unwrap_or_else(|| "(intro)".to_string());
        // Fast path: a section within the token bound stays ONE atom with its
        // original text intact (preserves prior behavior). Only an over-long
        // section — e.g. an ever-growing `## Learnings` list — is sub-split into
        // token-bounded atoms at block boundaries so it can't become a single
        // unbounded atom that crowds the retrieval budget. Sub-atoms share the
        // section heading_path; retrieval's rowid tie-break keeps them ordered.
        if crate::storage::estimate_tokens(&trimmed) <= MAX_ATOM_TOKENS {
            atoms.push(Atom { heading_path, body: trimmed, code_hash: None });
            return;
        }
        for chunk in pack_blocks(split_into_blocks(&trimmed)) {
            atoms.push(Atom { heading_path: heading_path.clone(), body: chunk, code_hash: None });
        }
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

/// Token ceiling for a single atom. A heading-delimited section larger than this
/// is sub-split at block boundaries so one ever-growing section can't become a
/// single unbounded atom that crowds the retrieval budget. ~200 tokens is a few
/// bullet entries; re-atomization is free (boot rescan re-splits).
const MAX_ATOM_TOKENS: i64 = 200;

/// Break a section body into blocks: each top-level (column-0) markdown list item
/// starts a new block, and blank lines separate paragraphs. Fence-aware — lines
/// inside a fenced code block (delimited by triple backticks) never start a block,
/// so fenced code is not split mid-block. Indented continuation / sub-bullets stay
/// with their parent block.
fn split_into_blocks(text: &str) -> Vec<String> {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        let is_fence = line.trim_start().starts_with("```");
        if !in_fence && !is_fence {
            if line.trim().is_empty() {
                if !cur.is_empty() {
                    blocks.push(std::mem::take(&mut cur));
                }
                continue; // drop the blank separator
            }
            if is_top_level_list_item(line) && !cur.is_empty() {
                blocks.push(std::mem::take(&mut cur));
            }
        }
        if is_fence {
            in_fence = !in_fence;
        }
        cur.push(line);
    }
    if !cur.is_empty() {
        blocks.push(cur);
    }
    blocks.into_iter().map(|b| b.join("\n")).collect()
}

/// True if `line` begins (at column 0) with a markdown list marker: `- `, `* `,
/// `+ `, or an ordered `N.` / `N)` followed by a space. Indented markers are not
/// top-level — they belong to the enclosing block.
fn is_top_level_list_item(line: &str) -> bool {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        return true;
    }
    let digits = line.bytes().take_while(|b| b.is_ascii_digit()).count();
    digits > 0 && (line[digits..].starts_with(". ") || line[digits..].starts_with(") "))
}

/// Greedily pack blocks into atoms of <= [`MAX_ATOM_TOKENS`], breaking only at
/// block boundaries. A single block that alone exceeds the bound becomes its own
/// atom (we never split mid-block). Returns at least one chunk for non-empty input.
fn pack_blocks(blocks: Vec<String>) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_tokens = 0i64;
    for block in blocks {
        let bt = crate::storage::estimate_tokens(&block);
        if !cur.is_empty() && cur_tokens + bt > MAX_ATOM_TOKENS {
            chunks.push(std::mem::take(&mut cur));
            cur_tokens = 0;
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(&block);
        cur_tokens += bt;
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
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
/// A resolution older than this gets an explicit re-verify warning in the OOB
/// body: a mooted question answered hours later once read as CURRENT repo
/// state and produced three fabricated "not pushed yet" assertions
/// (2026-06-23, s-bb938f62 — issues.md #18).
const STALE_ANSWER_WARN_MINS: i64 = 10;

/// Parse a tray timestamp (`asked_at` / `answered_at`): RFC3339 (app-written
/// rows) or sqlite's `datetime('now')` format (schema default). Returns None on
/// anything else — the OOB body then simply omits the line that needed it.
pub(super) fn parse_tray_ts(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|n| n.and_utc())
}

/// "3m" / "2h 24m" / "3d 1h" — coarse, for the OOB age line.
fn render_age(mins: i64) -> String {
    match (mins / 1440, (mins % 1440) / 60, mins % 60) {
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

/// At most this many overtaking commands are listed in the OOB body — enough to
/// show the premise moved without burying the answer itself.
const MAX_MOOTING_LISTED: usize = 5;

/// Render the "approved since you asked" block (issues.md #18). `mooting` is
/// `(command, answered_at)` for gated commands APPROVED after this question was
/// parked, oldest-first; `asked` anchors the "N later" deltas.
///
/// Wording is load-bearing. A tray row proves the user APPROVED the command —
/// it does not prove the command SUCCEEDED: `maybe_run_gated` writes the
/// failure into the out-of-band message body, not back onto the row, so an
/// approved-but-failed gate is indistinguishable here from an approved-and-run
/// one. Claiming "ran" would assert an outcome this data cannot support, which
/// is the same class of error the block exists to prevent.
fn mooting_block(mooting: &[(String, String)], asked: Option<chrono::DateTime<chrono::Utc>>) -> String {
    if mooting.is_empty() {
        return String::new();
    }
    let lines = mooting
        .iter()
        .take(MAX_MOOTING_LISTED)
        .map(|(command, answered_at)| {
            let delta = asked
                .zip(parse_tray_ts(answered_at))
                .map(|(a, b)| format!(" ({} later)", render_age((b - a).num_minutes().max(0))))
                .unwrap_or_default();
            format!("- `{command}`{delta}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let more = mooting
        .len()
        .checked_sub(MAX_MOOTING_LISTED)
        .filter(|n| *n > 0)
        .map(|n| format!("\n- …and {n} more"))
        .unwrap_or_default();
    format!(
        "**Approved in this session after you asked:**\n{lines}{more}\n\
         bot-hq ran each at approval time; whether it succeeded is not recorded \
         on the tray row, so check the outcome rather than assuming either way. \
         This question's premise may already be settled.\n"
    )
}

pub(super) fn oob_resolution_body(
    agent_label: &str,
    question: &str,
    options: &[String],
    picked: &str,
    asked_at: Option<&str>,
    mooting: &[(String, String)],
) -> String {
    // Restate the full option list: every observed resolution arrives this way
    // (47/47 in the 2026-07-27 archive study), and without the menu the agent
    // loses its own decision frame — it can no longer tell whether the pick was
    // one of its options or the user reaching past the menu with free text.
    let options_block = if options.is_empty() {
        String::new()
    } else {
        let listed = options
            .iter()
            .enumerate()
            .map(|(i, o)| format!("{}. {o}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        format!("**Options were:**\n{listed}\n")
    };
    // Age-stamp the replay (issues.md #18): a resolution can arrive hours after
    // the ask, and an unstamped replay reads as CURRENT state — an agent once
    // adopted a 2.5h-dead premise and asserted un-pushed work three times
    // without a single verification command. Old answers carry an explicit
    // re-verify instruction.
    let asked_ts = asked_at.and_then(parse_tray_ts);
    let asked_block = match asked_ts {
        Some(ts) => {
            let mins = (chrono::Utc::now() - ts).num_minutes().max(0);
            let age = render_age(mins);
            if mins >= STALE_ANSWER_WARN_MINS {
                format!(
                    "**Asked:** {age} ago. State may have moved since — re-verify \
                     anything this question describes (pushes, merges, deploys, file \
                     state) before treating its premise as current.\n"
                )
            } else {
                format!("**Asked:** {age} ago.\n")
            }
        }
        None => String::new(),
    };
    format!(
        "(out-of-band) Your earlier `ask_user_choice` for {agent_label} resolved while \
         you were no longer waiting on the tool call.\n\n\
         **Question:** {question}\n\
         {options_block}\
         **User picked:** {picked}\n\
         {asked_block}\
         {mooting_block}\n\
         Treat this as the user's reply (a pick outside the listed options is the \
         user answering in their own words — honor the words, not the menu). \
         Continue from here.",
        mooting_block = mooting_block(mooting, asked_ts)
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
    fn split_into_atoms_splits_oversized_bulleted_section() {
        // A `## Learnings` list that outgrows the token bound is sub-split into
        // several atoms — instead of one ever-growing atom — all keeping the real
        // section heading_path (no synthetic "(entry N)" suffix).
        let mut md = String::from("## Learnings\n");
        for i in 0..14 {
            md.push_str(&format!(
                "- Learning {i}: a reasonably long one-line note about a specific gotcha somewhere in the codebase that we had to infer.\n"
            ));
        }
        let atoms = split_into_atoms(&md);
        assert!(atoms.len() >= 2, "oversized section should split, got {}", atoms.len());
        assert!(atoms.iter().all(|a| a.heading_path == "Learnings"));
        // No single atom holds the whole list, and every bullet survives somewhere.
        assert!(atoms
            .iter()
            .all(|a| !(a.body.contains("Learning 0:") && a.body.contains("Learning 13:"))));
        for i in 0..14 {
            assert!(
                atoms.iter().any(|a| a.body.contains(&format!("Learning {i}:"))),
                "bullet {i} missing after split"
            );
        }
    }

    #[test]
    fn split_into_atoms_keeps_small_section_verbatim() {
        // Under the token bound → one atom with the original text (incl. its blank
        // line) preserved exactly: the fast path does not reflow content.
        let md = "## Notes\nfirst paragraph\n\nsecond paragraph\n";
        let atoms = split_into_atoms(md);
        assert_eq!(atoms.len(), 1);
        assert_eq!(atoms[0].heading_path, "Notes");
        assert_eq!(atoms[0].body, "first paragraph\n\nsecond paragraph");
    }

    #[test]
    fn split_into_atoms_does_not_split_inside_code_fence() {
        // An over-long section whose bulk is a fenced code block (with blank lines
        // and bullet-like lines inside) stays a single atom — the fence is one
        // indivisible block.
        let mut md = String::from("## Example\n```\n");
        for i in 0..40 {
            md.push_str(&format!("- looks like a bullet but is code line {i} with padding text\n\n"));
        }
        md.push_str("```\n");
        let atoms = split_into_atoms(&md);
        assert_eq!(atoms.len(), 1, "fenced block must not be split, got {}", atoms.len());
        assert!(atoms[0].body.starts_with("```"));
        assert!(atoms[0].body.trim_end().ends_with("```"));
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

    #[test]
    fn oob_resolution_body_restates_the_full_menu() {
        let body = super::oob_resolution_body(
            "brian",
            "Push now?",
            &["Push 9a07930".to_string(), "Hold for review".to_string()],
            "Hold for review",
            None,
            &[],
        );
        assert!(body.contains("**Options were:**"));
        assert!(body.contains("1. Push 9a07930"));
        assert!(body.contains("2. Hold for review"));
        assert!(body.contains("**User picked:** Hold for review"));
        // No asked_at → no age line.
        assert!(!body.contains("**Asked:**"));

        // No options (free-text/halt shapes): no empty menu block.
        let bare = super::oob_resolution_body("brian", "Anything else?", &[], "done", None, &[]);
        assert!(!bare.contains("Options were"));
    }

    #[test]
    fn oob_resolution_body_age_stamps_late_answers_with_reverify_warning() {
        // 2.5h-old ask (the s-bb938f62 shape): age line + re-verify warning.
        let old = (chrono::Utc::now() - chrono::Duration::minutes(150)).to_rfc3339();
        let body = super::oob_resolution_body(
            "brian",
            "Re-push to staging?",
            &[],
            "discard",
            Some(&old),
            &[],
        );
        assert!(body.contains("**Asked:** 2h 30m ago"));
        assert!(body.contains("re-verify"));

        // Fresh ask: age line, no warning.
        let fresh = chrono::Utc::now().to_rfc3339();
        let quick = super::oob_resolution_body("brian", "Close?", &[], "yes", Some(&fresh), &[]);
        assert!(quick.contains("**Asked:** 0m ago"));
        assert!(!quick.contains("re-verify"));

        // Sqlite datetime('now') format parses too.
        let sqlite_ts = (chrono::Utc::now() - chrono::Duration::minutes(75))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let s = super::oob_resolution_body("rain", "Q?", &[], "ok", Some(&sqlite_ts), &[]);
        assert!(s.contains("**Asked:** 1h 15m ago"));

        // Garbage timestamp: line omitted, body still well-formed.
        let g = super::oob_resolution_body("brian", "Q?", &[], "ok", Some("not-a-time"), &[]);
        assert!(!g.contains("**Asked:**"));
        assert!(g.contains("**User picked:** ok"));
    }

    #[test]
    fn oob_resolution_body_lists_commands_approved_after_the_ask() {
        // The s-bb938f62 shape: question parked 2h30m ago, the command it was
        // about approved 2h1m later, answer arrives now.
        let asked = chrono::Utc::now() - chrono::Duration::minutes(150);
        let approved = asked + chrono::Duration::minutes(121);
        let body = super::oob_resolution_body(
            "brian",
            "Re-push to staging?",
            &[],
            "discard",
            Some(&asked.to_rfc3339()),
            &[(
                "git push origin staging".to_string(),
                approved.to_rfc3339(),
            )],
        );
        assert!(body.contains("**Approved in this session after you asked:**"));
        assert!(body.contains("`git push origin staging` (2h 1m later)"));
        // Must NOT claim the command succeeded — the tray row only proves the
        // user approved it (an approved-but-failed gate looks identical here).
        assert!(body.contains("whether it succeeded is not recorded"));
        assert!(!body.contains("Ran in this session"));
        // The age-stamp block still renders alongside it.
        assert!(body.contains("**Asked:** 2h 30m ago"));

        // No overtaking commands → no block at all.
        let clean = super::oob_resolution_body(
            "brian",
            "Re-push?",
            &[],
            "discard",
            Some(&asked.to_rfc3339()),
            &[],
        );
        assert!(!clean.contains("**Approved in this session"));
    }

    #[test]
    fn mooting_block_caps_the_list_and_survives_bad_timestamps() {
        let asked = chrono::Utc::now() - chrono::Duration::minutes(60);
        let many: Vec<(String, String)> = (0..7)
            .map(|i| {
                (
                    format!("cmd-{i}"),
                    (asked + chrono::Duration::minutes(i + 1)).to_rfc3339(),
                )
            })
            .collect();
        let body = super::oob_resolution_body(
            "brian",
            "Q?",
            &[],
            "ok",
            Some(&asked.to_rfc3339()),
            &many,
        );
        assert!(body.contains("`cmd-4` (5m later)"));
        assert!(!body.contains("`cmd-5`"));
        assert!(body.contains("…and 2 more"));

        // Unparseable answered_at: the command is still named, the delta is
        // simply omitted — never a wrong "(0m later)".
        let junk = super::oob_resolution_body(
            "brian",
            "Q?",
            &[],
            "ok",
            Some(&asked.to_rfc3339()),
            &[("git push".to_string(), "not-a-time".to_string())],
        );
        assert!(junk.contains("- `git push`\n"));
        assert!(!junk.contains("later)"));
    }
}