//! CL claims that name code which is gone (rc3 **P4**).
//!
//! bot-hq's advantage over plain claude-code is the Context Library, and the
//! library is maintained by bot-hq sessions — a loop that closes only if
//! somebody notices when it has drifted. On 2026-08-12 an audit found ~57 stale
//! agent-name references and a whole learning describing the native connector —
//! **deleted that day** — written in confident present tense. An outsider
//! caught it, which is the part that does not scale.
//!
//! That class is mechanically detectable: `strip_claude_code_tool_inventory`,
//! `may_run_native`, `models.native`, `AgentRole` were all named in CL prose and
//! none exists in the tree. This module finds them.
//!
//! # It REPORTS. It never edits.
//!
//! Two reasons, and the second is the load-bearing one:
//!
//! 1. A hit is not proof of a defect. `decisions.md` and `issues.md` are
//!    append-only historical logs, and a learning that says "X was deleted"
//!    *has* to name X.
//! 2. D15's constraint: automated CL work must permit silence. An agent handed a
//!    list and told to fix it produces plausible filler, and this layer exists
//!    so future sessions orient from it INSTEAD of re-reading the code, so
//!    invented content is not noise to prune later — it is fabricated knowledge
//!    the next session builds on.
//!
//! # Precision over recall, deliberately
//!
//! Only **backticked** spans are candidates: the backtick is the author's own
//! mark that a span is code. Only two shapes are checked — repo-relative paths,
//! and identifiers carrying `_` or CamelCase — because a bare lowercase word in
//! backticks is as often prose. And any line carrying a retirement marker
//! ("deleted", "no longer", "used to", …) is skipped outright: that sentence is
//! doing its job by naming the dead symbol.
//!
//! Complements [`super::cl_refs`], which hashes the code an atom CITES and flags
//! the atom when that code drifts. It prunes references that do not resolve —
//! which is exactly this case — so the two are disjoint: drift there, absence
//! here.

use std::collections::BTreeSet;
use std::path::Path;

/// Why a claim is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleKind {
    /// A repo-relative path that does not exist.
    MissingFile,
    /// An identifier with no occurrence anywhere in the tracked source.
    MissingSymbol,
}

impl StaleKind {
    fn describe(&self) -> &'static str {
        match self {
            Self::MissingFile => "no such file in the repo",
            Self::MissingSymbol => "no occurrence in the tracked source",
        }
    }
}

/// One CL claim naming code that is not there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleClaim {
    /// CL file, relative to the project's CL directory.
    pub file: String,
    /// 1-indexed line, so the reader can go straight to it.
    pub line: usize,
    /// The backticked span, verbatim.
    pub reference: String,
    pub kind: StaleKind,
}

/// Phrases that mark a line as *about* a retirement. A line carrying one is
/// skipped: naming a deleted symbol is what such a line is for, and flagging it
/// would bury the real hits under the library's own history.
const RETIREMENT_MARKERS: [&str; 14] = [
    "deleted",
    "removed",
    "retired",
    "obsolete",
    "no longer",
    "used to",
    "does not exist",
    "doesn't exist",
    "don't exist",
    "gone",
    "superseded",
    "pre-rc3",
    "legacy",
    "was renamed",
];

/// CL files whose whole job is to record history, and which therefore name
/// dead code correctly. Never reported.
const HISTORY_FILES: [&str; 2] = ["decisions.md", "issues.md"];

/// Source extensions that count as "the code". A symbol living only in a
/// changelog or a plan document is not a symbol that exists.
const CODE_EXTENSIONS: [&str; 10] = [
    "rs", "ts", "tsx", "js", "jsx", "sql", "toml", "json", "yaml", "yml",
];

/// Backticked spans on one line that are shaped like code, with retirement
/// lines already excluded by the caller.
fn candidates_on_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        let span = &after[..close];
        rest = &after[close + 1..];
        let span = span.trim();
        // Multi-word spans are commands or prose (`git push`, `cargo test`).
        if span.is_empty() || span.contains(char::is_whitespace) {
            continue;
        }
        if path_candidate(span).is_some() || is_symbol_shaped(span) {
            out.push(span.to_string());
        }
    }
    out
}

/// The repo-relative path a span names, or `None` if it does not name one.
///
/// Written as a normalizer rather than a predicate because CL prose writes
/// paths several ways and the differences are not meaningful: a `file.rs:12`
/// line anchor, a `file.rs::symbol` pointer, a leading `src/` that the author
/// dropped. What it must NOT do is accept things that only look like paths —
/// a URL, a glob, a `<data_dir>` template, a path in the data dir rather than
/// the repo — because each of those is unresolvable by construction and would
/// be reported forever.
fn path_candidate(span: &str) -> Option<String> {
    // `file.rs::symbol` → `file.rs`; `file.rs:12` → `file.rs`.
    let path = span.split("::").next().unwrap_or(span);
    let path = path.split(':').next().unwrap_or(path);
    if !path.contains('/') || path.starts_with('/') || path.contains("..") {
        return None;
    }
    // Globs, brace-expansions and templates name a SET, not a file.
    if path.contains(['*', '{', '}', '<', '>', '~', '?']) {
        return None;
    }
    // URLs and data-dir paths are not repo paths.
    if path.starts_with("http") || path.contains(".com/") || path.contains(".bot-hq") {
        return None;
    }
    Path::new(path).extension().is_some().then(|| path.to_string())
}

/// An identifier distinctive enough to search for: `snake_case` or CamelCase,
/// at least 8 characters. The floor matters — short generic identifiers
/// (`is_ok`, `Some`) match everywhere and prove nothing.
fn is_symbol_shaped(span: &str) -> bool {
    if span.len() < 8 || !span.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    let snake = span.contains('_') && span.chars().any(|c| c.is_ascii_lowercase());
    let camel = span.chars().filter(|c| c.is_ascii_uppercase()).count() >= 2
        && span.chars().any(|c| c.is_ascii_lowercase());
    snake || camel
}

/// Every tracked source file's text, concatenated — the corpus a symbol is
/// searched in — plus every tracked path, one per line.
/// `git ls-files` so the walk inherits `.gitignore` (no `target/`, no
/// `node_modules/`).
///
/// The stems are load-bearing: a CL claim naming a test FILE by its stem
/// (`codebase_map_test`, for `tests/codebase_map_test.rs`) is a symbol-shaped
/// span, and integration tests are never named inside any other file — so a
/// contents-only corpus reported three real files as "no occurrence in the
/// tracked source" (round 7: 3 of the 30 hits). A file's own path is the
/// evidence it exists (and its stem is a substring of it).
fn source_corpus(repo_root: &Path) -> Result<(String, Vec<String>), String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let listing = String::from_utf8_lossy(&out.stdout).to_string();
    let mut corpus = String::new();
    let mut tracked = Vec::new();
    for rel in listing.split('\0').filter(|p| !p.is_empty()) {
        tracked.push(rel.to_string());
        // The path counts as an occurrence of itself — and, by substring, of
        // its stem, which is what a CL claim usually names.
        corpus.push_str(rel);
        corpus.push('\n');
        let is_code = Path::new(rel)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| CODE_EXTENSIONS.contains(&e));
        if is_code {
            if let Ok(body) = std::fs::read_to_string(repo_root.join(rel)) {
                corpus.push_str(&body);
                corpus.push('\n');
            }
        }
    }
    Ok((corpus, tracked))
}

/// Does the repo hold this path? Either literally, or as the tail of a tracked
/// path — CL prose routinely drops the leading `src/` (`signaling/jsonrpc.rs`),
/// and treating that as a missing file would bury the real hits under a naming
/// habit.
fn repo_has_path(repo_root: &Path, tracked: &[String], rel: &str) -> bool {
    repo_root.join(rel).exists()
        || tracked
            .iter()
            .any(|t| t == rel || t.ends_with(&format!("/{rel}")))
}

/// Is this path even ABOUT this repo?
///
/// A CL note cites paths in several trees — the data dir (`.local/violations.
/// jsonl`), the library itself (`projects/bot-hq/policy.yaml`), a packaging tap
/// (`Casks/bot-hq.rb`). None of those can resolve here, so reporting them is
/// noise that never clears, and noise that never clears is what teaches a
/// reader to skim the report.
///
/// The test is whether the path's FIRST segment names a directory the repo
/// actually has. `core/router.rs` passes (there is a `core/`, the file is
/// gone — a real hit); `Casks/bot-hq.rb` does not.
fn names_this_repo(tracked: &[String], rel: &str) -> bool {
    let Some(first) = rel.split('/').next() else {
        return false;
    };
    let needle = format!("{first}/");
    tracked.iter().any(|t| t.starts_with(&needle) || t.contains(&format!("/{needle}")))
}

/// Scan one CL body against a repo. Pure apart from the path existence checks,
/// so the extraction rules are testable without a corpus.
pub(crate) fn stale_in_body(
    cl_file: &str,
    body: &str,
    repo_root: &Path,
    corpus: &str,
    tracked: &[String],
) -> Vec<StaleClaim> {
    let mut seen: BTreeSet<(String, usize)> = BTreeSet::new();
    let mut out = Vec::new();
    for (idx, line) in body.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        if RETIREMENT_MARKERS.iter().any(|m| lower.contains(m)) {
            continue;
        }
        for span in candidates_on_line(line) {
            let kind = if let Some(path) = path_candidate(&span) {
                if repo_has_path(repo_root, tracked, &path)
                    || !names_this_repo(tracked, &path)
                {
                    continue;
                }
                StaleKind::MissingFile
            } else {
                if corpus.contains(&span) {
                    continue;
                }
                StaleKind::MissingSymbol
            };
            // One report per (reference, line): a line naming the same symbol
            // twice is one claim.
            if seen.insert((span.clone(), idx)) {
                out.push(StaleClaim {
                    file: cl_file.to_string(),
                    line: idx + 1,
                    reference: span,
                    kind,
                });
            }
        }
    }
    out
}

/// Walk a project's CL directory and report every claim naming absent code.
///
/// `cl_dir` is the project's own CL folder; `repo_root` the repo it documents.
pub fn stale_claims(cl_dir: &Path, repo_root: &Path) -> Result<Vec<StaleClaim>, String> {
    let (corpus, tracked) = source_corpus(repo_root)?;
    let mut out = Vec::new();
    let mut stack = vec![cl_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            // Append-only history is out of scope BY DESIGN, not by accident.
            // `decisions.md` and `issues.md` record what was decided and what
            // was broken at the time, so they name dead code on purpose and
            // must not be rewritten — reporting them is pure noise, and it is
            // the noise that trains a reader to skim the report.
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| HISTORY_FILES.contains(&n))
            {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            let rel = path
                .strip_prefix(cl_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            out.append(&mut stale_in_body(&rel, &body, repo_root, &corpus, &tracked));
        }
    }
    out.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    Ok(out)
}

/// Cap on what one report prints. The point is to surface the drift, not to
/// paste the library back at whoever asked.
pub const REPORT_MAX: usize = 40;

/// The report an agent or the UI reads.
pub fn render_report(project: &str, repo_root: &Path, claims: &[StaleClaim]) -> String {
    if claims.is_empty() {
        return format!(
            "No CL claims in `{project}` name code that is missing from {}. \
             (Checked backticked paths and identifiers; lines that mark something \
             as deleted or retired are skipped, since naming a dead symbol is what \
             they are for.)",
            repo_root.display()
        );
    }
    let mut out = format!(
        "{} CL claim(s) in `{project}` name code that is not in {}:\n\n",
        claims.len(),
        repo_root.display()
    );
    for c in claims.iter().take(REPORT_MAX) {
        out.push_str(&format!(
            "- {}:{} — `{}` ({})\n",
            c.file,
            c.line,
            c.reference,
            c.kind.describe()
        ));
    }
    if claims.len() > REPORT_MAX {
        out.push_str(&format!(
            "\n…and {} more (showing the first {REPORT_MAX}).\n",
            claims.len() - REPORT_MAX
        ));
    }
    out.push_str(
        "\n**This is a report, not a work order.** Verify each one against the \
         repo before touching the library: `decisions.md` and `issues.md` are \
         append-only history and legitimately name code that is gone, and a \
         learning that explains a deletion has to name what was deleted. \
         Writing nothing is a correct outcome.",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The specimens from the 2026-08-12 audit, and the prose that must NOT be
    /// flagged alongside them.
    #[test]
    fn it_finds_dead_symbols_and_leaves_the_history_that_names_them_alone() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join("src/core")).unwrap();
        std::fs::write(repo.path().join("src/core/session.rs"), "fn spawn_ring() {}").unwrap();
        let corpus = "fn spawn_ring() {}";
        let tracked = ["src/core/session.rs".to_string()];

        let body = "\
The connector reads `may_run_native` before spawning.
Spawn lives in `src/core/session.rs` and the ring in `src/core/router.rs`.
`spawn_ring` is the entry point.
`may_run_native` was deleted with the native loop (rc3 D9).
Run `cargo test` before every commit.
The audit log is at `.local/violations.jsonl`, outside this repo.
The tap formula is `Casks/bot-hq.rb`.
";
        let claims = stale_in_body("notes.md", body, repo.path(), corpus, &tracked);
        let refs: Vec<&str> = claims.iter().map(|c| c.reference.as_str()).collect();

        // Line 1: a live-tense claim about a symbol that no longer exists.
        assert!(refs.contains(&"may_run_native"), "got: {refs:?}");
        // Line 2: one path resolves, the other does not.
        assert!(refs.contains(&"src/core/router.rs"), "got: {refs:?}");
        assert!(!refs.contains(&"src/core/session.rs"), "a live file was flagged");
        // Line 3: the symbol is in the corpus.
        assert!(!refs.contains(&"spawn_ring"), "a live symbol was flagged");
        // Line 4 names the SAME dead symbol, but it is the sentence recording
        // the deletion — flagging it would bury the real hit under history.
        assert_eq!(
            claims.iter().filter(|c| c.reference == "may_run_native").count(),
            1,
            "the retirement line must not be reported: {claims:?}"
        );
        // Line 5 is a command, not a symbol.
        assert!(!refs.iter().any(|r| r.contains("cargo")), "got: {refs:?}");

        // Paths in trees this repo does not have are not claims about it. The
        // library cites the data dir and a packaging tap constantly, and those
        // can never resolve — noise that never clears is what teaches a reader
        // to skim the report. (Measured: dropping these took a real run from
        // 32 hits to 25, all of them genuine.)
        assert!(!refs.contains(&".local/violations.jsonl"), "got: {refs:?}");
        assert!(!refs.contains(&"Casks/bot-hq.rb"), "got: {refs:?}");

        assert_eq!(claims[0].line, 1, "claims carry the line to jump to");
    }

    /// A claim naming a FILE by its stem is a symbol-shaped span, and an
    /// integration test file is never named inside any other file — so a
    /// contents-only corpus called `codebase_map_test` absent while
    /// `tests/codebase_map_test.rs` sat in the tree (round 7: three of the
    /// thirty live hits). The corpus carries every tracked path (the stem is
    /// a substring of it).
    #[test]
    fn a_tracked_files_stem_counts_as_present() {
        let repo = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(args)
                .output()
                .expect("git runs");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q"]);
        std::fs::create_dir_all(repo.path().join("tests")).unwrap();
        // The file's body never says its own name.
        std::fs::write(
            repo.path().join("tests/codebase_map_test.rs"),
            "#[test] fn the_map_is_complete() {}\n",
        )
        .unwrap();
        git(&["add", "tests/codebase_map_test.rs"]);
        let (corpus, tracked) = source_corpus(repo.path()).expect("corpus");
        assert!(tracked.iter().any(|t| t == "tests/codebase_map_test.rs"));
        assert!(corpus.contains("codebase_map_test"), "the stem is in the corpus");
        assert!(corpus.contains("the_map_is_complete"), "and so is the body");
        let claims = stale_in_body(
            "learnings.md",
            "`codebase_map_test` folds `X.test.ts` onto `X.ts`; `an_absent_symbol` does not.",
            repo.path(),
            &corpus,
            &tracked,
        );
        let refs: Vec<&str> = claims.iter().map(|c| c.reference.as_str()).collect();
        assert!(!refs.contains(&"codebase_map_test"), "a tracked file's stem is present: {refs:?}");
        assert!(refs.contains(&"an_absent_symbol"), "a genuinely absent symbol still reports: {refs:?}");
    }

    /// The extraction floor. Without it the report is unreadable: every `Some`,
    /// `ok`, and prose word in backticks becomes a claim.
    #[test]
    fn short_or_prose_spans_are_not_claims() {
        for span in ["`Some`", "`ok`", "`the CL`", "`0049`", "`sessions`"] {
            assert!(
                candidates_on_line(span).is_empty(),
                "{span} should not be a candidate"
            );
        }
        // …and what does qualify.
        assert_eq!(
            candidates_on_line("see `resolve_spawn_roster` and `AgentRole`"),
            vec!["resolve_spawn_roster", "AgentRole"]
        );
    }

    #[test]
    fn an_empty_report_says_so_rather_than_printing_nothing() {
        let out = render_report("bot-hq", Path::new("/repo"), &[]);
        assert!(out.contains("No CL claims"), "got: {out}");
        // And a populated one carries the do-not-auto-edit warning, because the
        // list reads like a work order otherwise (D15).
        let out = render_report(
            "bot-hq",
            Path::new("/repo"),
            &[StaleClaim {
                file: "notes.md".into(),
                line: 3,
                reference: "may_run_native".into(),
                kind: StaleKind::MissingSymbol,
            }],
        );
        assert!(out.contains("notes.md:3"), "got: {out}");
        assert!(out.contains("not a work order"), "got: {out}");
    }
}
