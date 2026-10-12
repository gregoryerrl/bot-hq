//! CL claims that name code which is gone (rc3 **P4**).
//!
//! bot-hq's advantage over plain claude-code is the Context Library, and the
//! library is maintained by bot-hq sessions — a loop that closes only if
//! somebody notices when it has drifted. On 2026-08-12 an audit found ~57 stale
//! agent-name references and a whole learning describing the native connector —
//! **deleted that day** — written in confident present tense. An outsider
//! caught it, which is the part that does not scale.
//!
//! That class is mechanically detectable: the deleted `strip_claude_code_tool_inventory`,
//! the deleted `may_run_native`, `models.native`, `AgentRole` were all named in
//! CL prose and none exists in the tree. This module finds them.
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
use std::path::{Path, PathBuf};

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
pub const RETIREMENT_MARKERS: [&str; 14] = [
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
/// dead code correctly. Never reported. The dated round files
/// (`learnings-<date>-…`, `notes-<date>-…`) are history by construction — a
/// learning that explains a deletion has to name what was deleted — and are
/// the same set the close-out vocabulary sweep skips (`cl_write::sweep_skips`);
/// until 1.0.5 they were reported here, and were 16 of a real run's 25 hits.
pub(super) fn is_history_file(name: &str) -> bool {
    name == "decisions.md"
        || name == "issues.md"
        || name.starts_with("learnings-")
        || name.starts_with("notes-20")
}

/// Prose extensions, the one kind of tracked file whose BODY is not the code:
/// a symbol living only in a changelog or a plan document is not a symbol that
/// exists. Everything else tracked is read (feedback #100): the old allow-list
/// of ten code extensions read 2 of STRYKA's 121 tracked files and none of a
/// Python, shell, PHP or Makefile project, so every symbol in those libraries
/// reported as missing.
const PROSE_EXTENSIONS: [&str; 6] = ["md", "markdown", "mdx", "rst", "txt", "adoc"];

/// A tracked file bigger than this is not read (a vendored bundle, a fixture
/// dump, a lockfile the size of a novel); its path still counts.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// What the scan read, so the report can say what "missing" was measured
/// against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CorpusScope {
    /// Tracked paths, every one of which counts as an occurrence of itself.
    pub tracked: usize,
    /// Files whose text went into the corpus.
    pub read: usize,
    /// Prose files, skipped by extension.
    pub prose: usize,
    /// Files over [`MAX_FILE_BYTES`], skipped.
    pub oversized: usize,
    /// Files that are not UTF-8 (binaries, mostly), skipped.
    pub non_utf8: usize,
}

/// The text a symbol is searched in, every tracked path, and what was read.
#[derive(Debug)]
pub struct Corpus {
    pub text: String,
    pub tracked: Vec<String>,
    pub scope: CorpusScope,
}

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

/// Every tracked non-prose file's text, concatenated — the corpus a symbol is
/// searched in — plus every tracked path, one per line, and the scope of what
/// was read. `git ls-files` so the walk inherits `.gitignore` (no `target/`,
/// no `node_modules/`).
///
/// The stems are load-bearing: a CL claim naming a test FILE by its stem
/// (`codebase_map_test`, for `tests/codebase_map_test.rs`) is a symbol-shaped
/// span, and integration tests are never named inside any other file — so a
/// contents-only corpus reported three real files as "no occurrence in the
/// tracked source" (round 7: 3 of the 30 hits). A file's own path is the
/// evidence it exists (and its stem is a substring of it).
fn source_corpus(repo_root: &Path) -> Result<Corpus, CorpusError> {
    let out = crate::core::git::hardened(repo_root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| CorpusError::Git(e.to_string()))?;
    if !out.status.success() {
        return Err(CorpusError::Git(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    let listing = String::from_utf8_lossy(&out.stdout).to_string();
    // An EMPTY tracked set is a refusal, not a corpus (feedback #99): with
    // nothing to search, every symbol reads as missing and the report lists
    // the whole library. Live case: a project folder registered inside a
    // bigger repository that tracks nothing under it ("Minecraft Mods", whose
    // toplevel is `~/Projects`). A TRACKED subfolder keeps working — its
    // `ls-files` lists its files — so the test is the empty set, not a
    // toplevel that differs.
    if listing.split('\0').all(|p| p.is_empty()) {
        return Err(CorpusError::NoTrackedFiles {
            folder: repo_root.to_path_buf(),
            toplevel: git_toplevel(repo_root).filter(|top| !same_dir(top, repo_root)),
        });
    }
    let mut corpus = String::new();
    let mut tracked = Vec::new();
    let mut scope = CorpusScope::default();
    for rel in listing.split('\0').filter(|p| !p.is_empty()) {
        tracked.push(rel.to_string());
        scope.tracked += 1;
        // The path counts as an occurrence of itself — and, by substring, of
        // its stem, which is what a CL claim usually names.
        corpus.push_str(rel);
        corpus.push('\n');
        let is_prose = Path::new(rel)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| PROSE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        if is_prose {
            scope.prose += 1;
            continue;
        }
        let path = repo_root.join(rel);
        // A deleted-but-still-indexed file, or one git lists that is a
        // directory (a submodule): nothing to read, and the path still counts.
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if !meta.is_file() {
            continue;
        }
        if meta.len() > MAX_FILE_BYTES {
            scope.oversized += 1;
            continue;
        }
        match std::fs::read(&path).map(String::from_utf8) {
            Ok(Ok(body)) => {
                scope.read += 1;
                corpus.push_str(&body);
                corpus.push('\n');
            }
            Ok(Err(_)) => scope.non_utf8 += 1,
            Err(_) => {}
        }
    }
    Ok(Corpus { text: corpus, tracked, scope })
}

/// Why a folder yielded no corpus to check CL claims against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorpusError {
    /// `git ls-files` itself failed (no repository, no git): its stderr.
    Git(String),
    /// The repository tracks no file under `folder`: a project folder
    /// registered inside a bigger repository that tracks nothing there
    /// (`toplevel` is that repository's root), or a repository with nothing
    /// committed yet (`None`). A configuration problem to name, not a scan to
    /// run.
    NoTrackedFiles { folder: PathBuf, toplevel: Option<PathBuf> },
}

impl std::fmt::Display for CorpusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CorpusError::Git(e) => write!(f, "{e}"),
            CorpusError::NoTrackedFiles { folder, toplevel: Some(top) } => write!(
                f,
                "the folder {} has no tracked files to check CL claims against: it sits inside the \
                 git repository at {}, which tracks nothing under it. Register that repository's \
                 own folder as the project's repo, or commit the project's files, and run the \
                 report again.",
                folder.display(),
                top.display()
            ),
            CorpusError::NoTrackedFiles { folder, toplevel: None } => write!(
                f,
                "the repository at {} tracks no files yet, so there is nothing to check CL claims \
                 against. Commit the project's files and run the report again.",
                folder.display()
            ),
        }
    }
}

/// The root of the repository `dir` is inside, per git; `None` when git cannot say.
fn git_toplevel(dir: &Path) -> Option<PathBuf> {
    let out = crate::core::git::hardened(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!top.is_empty()).then(|| PathBuf::from(top))
}

/// The same directory once symlinks and relative segments are resolved
/// (`/tmp` is `/private/tmp` on macOS; git reports the resolved form).
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
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
/// actually has. The deleted `core/router.rs` passes (there is a `core/`, the
/// file is gone — a real hit); `Casks/bot-hq.rb` does not.
fn names_this_repo(tracked: &[String], rel: &str) -> bool {
    let Some(first) = rel.split('/').next() else {
        return false;
    };
    let needle = format!("{first}/");
    tracked.iter().any(|t| t.starts_with(&needle) || t.contains(&format!("/{needle}")))
}

/// Which lines of `body` are retirement statements — the line carries a
/// marker itself, or a neighbouring line of the SAME paragraph does.
///
/// CL prose is hard-wrapped, so the symbol and the word that retires it are
/// routinely on different physical lines (`notes.md:292` names the file,
/// `:293` says "deleted"); a per-line test flagged exactly those. The window
/// is one line either side, bounded by the paragraph (a blank line, a
/// heading, a table row, or the start of a new list item ends one) — wide
/// enough for a wrapped sentence, narrow enough that a paragraph which
/// records one deletion and makes a live claim three lines later still
/// reports the live claim.
fn retirement_lines(body: &str) -> Vec<bool> {
    let lines: Vec<&str> = body.lines().collect();
    let starts_item = |l: &str| {
        let t = l.trim_start();
        t.starts_with("- ")
            || t.starts_with("* ")
            || t.starts_with("+ ")
            || t.starts_with('#')
            || t.starts_with('|')
            || (t.starts_with(|c: char| c.is_ascii_digit())
                && t.trim_start_matches(|c: char| c.is_ascii_digit()).starts_with(". "))
    };
    // Paragraph id per line: bumps on a blank line and on an item start.
    let mut para = vec![0usize; lines.len()];
    let mut id = 0usize;
    for (i, l) in lines.iter().enumerate() {
        if l.trim().is_empty() || (i > 0 && starts_item(l)) {
            id += 1;
        }
        para[i] = id;
    }
    let marked: Vec<bool> = lines
        .iter()
        .map(|l| {
            let lower = l.to_ascii_lowercase();
            RETIREMENT_MARKERS.iter().any(|m| lower.contains(m))
        })
        .collect();
    (0..lines.len())
        .map(|i| {
            let lo = i.saturating_sub(1);
            let hi = (i + 1).min(lines.len() - 1);
            (lo..=hi).any(|j| para[j] == para[i] && marked[j])
        })
        .collect()
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
    let retired = retirement_lines(body);
    for (idx, line) in body.lines().enumerate() {
        if retired[idx] {
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
                // A live bot-hq tool's name is not a claim about THIS repo: the
                // library of any project names `cl_index_search` and friends,
                // and only bot-hq's own source holds them (feedback #100).
                if corpus.contains(&span) || is_live_tool_name(&span) {
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

/// Is `span` the name of a tool bot-hq's MCP server registers right now?
fn is_live_tool_name(span: &str) -> bool {
    crate::signaling::protocol::tool_descriptors().iter().any(|t| t.name == span)
}

/// The claims of one scan, with the scope they were measured against.
pub struct StaleReport {
    pub claims: Vec<StaleClaim>,
    pub scope: CorpusScope,
}

/// Walk a project's CL directory and report every claim naming absent code.
///
/// `cl_dir` is the project's own CL folder; `repo_root` the repo it documents.
pub fn stale_claims(cl_dir: &Path, repo_root: &Path) -> Result<StaleReport, CorpusError> {
    let Corpus { text: corpus, tracked, scope } = source_corpus(repo_root)?;
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
                .is_some_and(is_history_file)
            {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            // `/`-form: these become the `file` field of every staleness hit,
            // which callers match against CL keys.
            let rel = super::util::rel_key(&path, cl_dir)
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            out.append(&mut stale_in_body(&rel, &body, repo_root, &corpus, &tracked));
        }
    }
    out.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    Ok(StaleReport { claims: out, scope })
}

/// Cap on what one report prints. The point is to surface the drift, not to
/// paste the library back at whoever asked.
pub const REPORT_MAX: usize = 40;

/// The sentence that says what "missing" was measured against (feedback
/// #100): a reader who does not know the scan read only ten extensions — or
/// skipped a 3 MB bundle — reads a false "missing" as a real one.
fn scope_sentence(scope: &CorpusScope) -> String {
    let mut skipped = Vec::new();
    if scope.prose > 0 {
        skipped.push(format!("{} prose file{}", scope.prose, if scope.prose == 1 { "" } else { "s" }));
    }
    if scope.oversized > 0 {
        skipped.push(format!("{} over 2 MiB", scope.oversized));
    }
    if scope.non_utf8 > 0 {
        skipped.push(format!("{} not UTF-8", scope.non_utf8));
    }
    let skipped = if skipped.is_empty() {
        String::new()
    } else {
        format!("; skipped: {}", skipped.join(", "))
    };
    format!(
        "Searched the text of {} of the {} tracked files (every tracked file except prose — \
         .md, .markdown, .mdx, .rst, .txt, .adoc — up to 2 MiB each{skipped}); every tracked \
         path counts as present, and so does a live bot-hq tool name.",
        scope.read, scope.tracked
    )
}

/// The report an agent or the UI reads.
pub fn render_report(project: &str, repo_root: &Path, claims: &[StaleClaim], scope: &CorpusScope) -> String {
    if claims.is_empty() {
        return format!(
            "No CL claims in `{project}` name code that is missing from {}. \
             (Checked backticked paths and identifiers; lines that mark something \
             as deleted or retired are skipped, since naming a dead symbol is what \
             they are for. {})",
            repo_root.display(),
            scope_sentence(scope)
        );
    }
    let mut out = format!(
        "{} CL claim(s) in `{project}` name code that is not in {}. {}\n\n",
        claims.len(),
        repo_root.display(),
        scope_sentence(scope)
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

    /// The two noise sources a real run showed (25 hits, 2026-09-06): the
    /// dated round files, and a retirement whose marker sits on the NEXT
    /// physical line of a hard-wrapped paragraph.
    #[test]
    fn dated_round_files_are_history_and_wrapped_retirements_are_not_claims() {
        assert!(is_history_file("learnings-2026-08-18-round9.md"));
        assert!(is_history_file("notes-2026-07-02-cl-measurement.md"));
        assert!(is_history_file("decisions.md"));
        assert!(is_history_file("issues.md"));
        assert!(!is_history_file("notes.md"));
        assert!(!is_history_file("conventions.md"));
        assert!(!is_history_file("session-study-method.md"));

        let repo = tempfile::tempdir().unwrap();
        let corpus = "fn live() {}";
        let tracked: [String; 0] = [];
        let body = "\
- The old `held_late` flag drove the wait; it went with the driver
  and was deleted on 2026-08-17.
- `PendingChoiceView` still renders the tray.

`SpectaFn` was renamed
in the specta 2 migration.
";
        let claims = stale_in_body("notes.md", body, repo.path(), corpus, &tracked);
        let refs: Vec<&str> = claims.iter().map(|c| c.reference.as_str()).collect();
        assert!(!refs.contains(&"held_late"), "marker on the continuation line: {refs:?}");
        assert!(!refs.contains(&"SpectaFn"), "marker in the next line of the paragraph: {refs:?}");
        assert!(refs.contains(&"PendingChoiceView"), "a live-tense claim in its own item still reports: {refs:?}");
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
        let Corpus { text: corpus, tracked, .. } = source_corpus(repo.path()).expect("corpus");
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

    /// Feedback #99: a folder whose tracked set is EMPTY is refused with a
    /// configuration message (naming the repository's root when the folder
    /// sits inside one), instead of a corpus in which every symbol is missing.
    /// A TRACKED subfolder of a bigger repository still gets its corpus.
    #[test]
    fn an_empty_tracked_set_is_refused_and_names_the_repository_it_sits_in() {
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
        // Nothing tracked anywhere yet: refused, and the folder IS the root.
        match source_corpus(repo.path()) {
            Err(CorpusError::NoTrackedFiles { folder, toplevel: None }) => assert_eq!(folder, repo.path()),
            other => panic!("expected a no-tracked-files refusal, got {other:?}"),
        }
        std::fs::create_dir_all(repo.path().join("app/src")).unwrap();
        std::fs::create_dir_all(repo.path().join("mods")).unwrap();
        std::fs::write(repo.path().join("app/src/main.rs"), "fn tracked_symbol() {}\n").unwrap();
        std::fs::write(repo.path().join("mods/notes.txt"), "untracked").unwrap();
        git(&["add", "app/src/main.rs"]);
        // The untracked subfolder: refused, naming the repository it sits in.
        let err = source_corpus(&repo.path().join("mods")).expect_err("nothing tracked under mods/");
        match &err {
            CorpusError::NoTrackedFiles { folder, toplevel: Some(top) } => {
                assert_eq!(folder, &repo.path().join("mods"));
                assert!(same_dir(top, repo.path()), "toplevel {top:?} is the repo root {:?}", repo.path());
            }
            other => panic!("got {other:?}"),
        }
        let msg = err.to_string();
        assert!(msg.contains("has no tracked files") && msg.contains("sits inside the git repository at"), "{msg}");
        assert!(msg.contains("Register that repository's own folder"), "{msg}");
        // The tracked subfolder keeps its corpus (its own files, relative to it).
        let Corpus { text: corpus, tracked, .. } = source_corpus(&repo.path().join("app")).expect("a tracked subfolder scans");
        assert_eq!(tracked, ["src/main.rs"]);
        assert!(corpus.contains("tracked_symbol"));
        // And `stale_claims` carries the refusal through to the facade's match.
        let cl = tempfile::tempdir().unwrap();
        std::fs::write(cl.path().join("notes.md"), "`tracked_symbol` lives here.").unwrap();
        assert!(matches!(
            stale_claims(cl.path(), &repo.path().join("mods")),
            Err(CorpusError::NoTrackedFiles { .. })
        ));
        assert!(stale_claims(cl.path(), &repo.path().join("app")).unwrap().claims.is_empty());
        // No repository at all is still a git error, not a configuration one.
        let bare = tempfile::tempdir().unwrap();
        assert!(matches!(source_corpus(bare.path()), Err(CorpusError::Git(_))));
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
        let scope = CorpusScope { tracked: 121, read: 100, prose: 19, oversized: 1, non_utf8: 1 };
        let out = render_report("bot-hq", Path::new("/repo"), &[], &scope);
        assert!(out.contains("No CL claims"), "got: {out}");
        // The scope is stated either way (feedback #100): what "missing" was
        // measured against, and what was skipped.
        assert!(
            out.contains("Searched the text of 100 of the 121 tracked files (every tracked file except prose"),
            "got: {out}"
        );
        assert!(out.contains("skipped: 19 prose files, 1 over 2 MiB, 1 not UTF-8"), "got: {out}");
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
            &CorpusScope { tracked: 2, read: 2, ..CorpusScope::default() },
        );
        assert!(out.contains("notes.md:3"), "got: {out}");
        assert!(out.contains("not a work order"), "got: {out}");
        assert!(out.contains("Searched the text of 2 of the 2 tracked files") && !out.contains("skipped:"), "got: {out}");
    }

    /// Feedback #100: every tracked file except prose is the corpus — a shell
    /// script, a Makefile and a PHP file count; a markdown file's body does not;
    /// an oversized file and a binary are skipped by count; and a live bot-hq
    /// tool name is never a missing symbol in another project's repo.
    #[test]
    fn the_corpus_is_every_tracked_non_prose_file_and_live_tool_names_are_present() {
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
        let write = |rel: &str, body: &[u8]| {
            let p = repo.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write("deploy.sh", b"deploy_everything() { :; }\n");
        write("Makefile", b"build_release_bundle:\n\t@true\n");
        write("web/index.php", b"<?php function render_invoice_page() {}\n");
        write("docs/PLAN.md", b"`only_in_the_plan` is coming.\n");
        write("big.min.js", &vec![b'x'; (MAX_FILE_BYTES + 1) as usize]);
        write("logo.png", &[0xff, 0xd8, 0xff, 0x00, 0x80, 0x81]);
        git(&["add", "-A"]);
        let Corpus { text, tracked, scope } = source_corpus(repo.path()).expect("corpus");
        assert_eq!(tracked.len(), 6);
        assert_eq!(scope, CorpusScope { tracked: 6, read: 3, prose: 1, oversized: 1, non_utf8: 1 }, "{scope:?}");
        for present in ["deploy_everything", "build_release_bundle", "render_invoice_page"] {
            assert!(text.contains(present), "{present} is in the corpus");
        }
        assert!(!text.contains("only_in_the_plan"), "a prose file's body is not the code");
        assert!(text.contains("docs/PLAN.md"), "but its path still counts");
        let body = "`deploy_everything` runs first; `only_in_the_plan` and `absent_everywhere` do not exist; agents call `cl_index_search` first.";
        let claims = stale_in_body("notes.md", body, repo.path(), &text, &tracked);
        let refs: Vec<&str> = claims.iter().map(|c| c.reference.as_str()).collect();
        assert_eq!(refs, ["only_in_the_plan", "absent_everywhere"], "{refs:?}");
        assert!(is_live_tool_name("cl_index_search") && !is_live_tool_name("absent_everywhere"));
    }
}
