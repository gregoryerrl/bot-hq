//! No source IDENTIFIER may name a retired agent as a current thing.
//!
//! # Why this exists
//!
//! Rounds 1, 2 and 3 of the rc3 audit each swept for the retired agent names
//! and each reported the production surface clean. Round 4 found two live
//! identifiers — `build_rain_disallowed_tools` and a `has_rain` parameter — that
//! all three sweeps had been structurally unable to see.
//!
//! The reason is worth stating precisely, because it is the whole design of this
//! test. The recorded sweep command was
//!
//! ```text
//! rg -n --no-heading -w -e brian -e rain -e Brian -e Rain src -g '*.rs'
//! ```
//!
//! and `-w` requires a NON-WORD character on both sides of the match. `_` is a
//! word character. So a word-boundary search can match `rain` in prose or in a
//! string literal, and can **never** match it inside `has_rain` or
//! `build_rain_disallowed_tools` — which is exactly where a live identifier
//! lives. Three rounds measured the harmless half of the problem with confidence.
//!
//! # The rule
//!
//! Split each identifier on `_` and compare SEGMENTS, case-insensitively. That
//! is what `-w` should have been:
//!
//! | identifier | segments | verdict |
//! |---|---|---|
//! | `has_rain` | `has`, `rain` | **flagged** |
//! | `build_rain_disallowed_tools` | …, `rain`, … | **flagged** |
//! | `the_drain_rather_than_finishing` | …, `drain`, … | clean — `drain` ≠ `rain` |
//! | `constraint` | `constraint` | clean |
//!
//! Substring matching would flag all four; `-w` would flag none. Segment
//! matching flags exactly the two that name a retired agent.
//!
//! # What this does NOT guard, deliberately
//!
//! **Comments are exempt**, the same call `frontend/src/lib/framing.ts` makes and
//! for the same reason: a comment describing a 2026-05-28 incident in that day's
//! vocabulary is a RECORD, and rewriting history to match current vocabulary
//! destroys the record. A false present-tense claim in a comment is a real
//! defect (round-4 F2) but it is a claim about behaviour, which no lexical rule
//! can judge.
//!
//! **Bare words are exempt** — a `"brian"` string literal is legacy DATA (the
//! author of a pre-rc3 message row, a legacy path segment, a fixture's arbitrary
//! agent name). Round 3 hand-classified 272 of those and found one defect among
//! them; the class is not where live names hide. This guard is aimed at the class
//! that IS: a compound identifier the compiler carries.
//!
//! # Two bounds, so neither reads as an oversight
//!
//! **It walks `src/` only, and that is load-bearing, not laziness.** This file
//! spells `has_rain` and `build_rain_disallowed_tools` as string literals in
//! `rule_tests` — they are the specimens the rule is proved against. Widening the
//! walk to `tests/` would make the guard fail on its own evidence, and the obvious
//! repair (delete the specimens) removes the only proof the rule works. `tests/`
//! was swept by hand at round 4 and holds no retired identifier outside comments.
//!
//! **`code_of` truncates at a non-`://` `//` inside a string literal**, so an
//! identifier written after one in the same string is not scanned. That is a false
//! NEGATIVE, never a false positive: the guard can miss, it cannot cry wolf, and a
//! guard that cries wolf is one people learn to skip. Proper string-literal
//! parsing is the fix if a real case ever appears.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Names retired by rc3 D10, lowercase. A segment equal to one of these is a hit.
const RETIRED: &[&str] = &["brian", "rain"];

/// Files allowed to carry a retired name in an identifier, each because the
/// identifier's whole job is to name a retired thing. **Adding to this list is a
/// decision, not a fix** — the question to answer first is whether the thing
/// being named is genuinely frozen.
const EXEMPT: &[(&str, &str)] = &[
    (
        "src/signaling/external_jsonrpc.rs",
        "The published driver wire. `brian_model_id` / `rain_model_at_spawn` are \
         argument and field NAMES an external client sends; renaming them breaks \
         every installed driver. D10's recorded exemption, documented at :38-62 \
         and pinned by its own test at :851.",
    ),
    (
        "src/paths.rs",
        "`LEGACY_BRIAN_CUSTOM_INSTRUCTION` / `LEGACY_RAIN_CUSTOM_INSTRUCTION` are \
         byte-frozen seed templates: `migrate_agent_custom_instructions` compares \
         `body.trim() != legacy_template.trim()`, so ANY change makes an \
         untouched seed read as real user content. Same class as an applied \
         migration.",
    ),
];

/// One flagged occurrence.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Hit {
    file: String,
    line: usize,
    identifier: String,
}

/// True when `identifier` has a `_`-separated segment equal to a retired name.
///
/// Single-segment tokens are never hits: a bare `rain` is a string literal or a
/// word in a `&str`, which this guard exempts by design.
fn names_retired(identifier: &str) -> Option<&'static str> {
    let segments: Vec<&str> = identifier.split('_').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return None;
    }
    RETIRED
        .iter()
        .find(|r| segments.iter().any(|s| s.eq_ignore_ascii_case(r)))
        .copied()
}

/// Whether a source line is a comment. Line-level, matching the frontend guard:
/// a trailing `// …` is stripped by the caller, and a line that STARTS a comment
/// is skipped whole.
fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("/*") || t.starts_with('*')
}

/// The code half of a line — everything before a `//`, so a trailing comment
/// cannot fail the guard while an identifier on the same line still can.
///
/// `://` is left alone so a URL in a string does not truncate the line.
fn code_of(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes[i - 1] == b'/' && !(i >= 2 && bytes[i - 2] == b':') {
            return &line[..i - 1];
        }
        i += 1;
    }
    line
}

/// Every identifier-shaped token in `code`.
fn identifiers(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in code.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn scan() -> Vec<Hit> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_sources(&root.join("src"), &mut files);
    files.sort();
    assert!(
        files.len() > 20,
        "the walker found only {} source files — it is not reaching src/",
        files.len()
    );

    let exempt: BTreeSet<&str> = EXEMPT.iter().map(|(f, _)| *f).collect();
    let mut hits = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if exempt.contains(rel.as_str()) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (n, line) in body.lines().enumerate() {
            if is_comment(line) {
                continue;
            }
            for ident in identifiers(code_of(line)) {
                if names_retired(&ident).is_some() {
                    hits.push(Hit { file: rel.clone(), line: n + 1, identifier: ident });
                }
            }
        }
    }
    hits
}

#[test]
fn no_source_identifier_names_a_retired_agent() {
    let hits = scan();
    assert!(
        hits.is_empty(),
        "{} identifier(s) name an agent rc3 D10 retired:\n{}\n\n\
         Rename it. If the identifier's whole job is to name a retired thing — a \
         frozen wire, a byte-frozen template — add its FILE to `EXEMPT` in this \
         test with the reason, which is a decision to argue rather than a build \
         to unbreak.",
        hits.len(),
        hits.iter()
            .map(|h| format!("  {}:{}  {}", h.file, h.line, h.identifier))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The exemptions must stay HONEST: a file listed here that no longer carries a
/// retired identifier is a stale carve-out, and a stale carve-out is how the next
/// live name gets in unnoticed.
#[test]
fn every_exemption_is_still_earning_its_place() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (file, reason) in EXEMPT {
        let body = std::fs::read_to_string(root.join(file))
            .unwrap_or_else(|e| panic!("exempt file {file} is unreadable: {e}"));
        let found = body.lines().filter(|l| !is_comment(l)).any(|l| {
            identifiers(code_of(l)).iter().any(|i| names_retired(i).is_some())
        });
        assert!(
            found,
            "{file} is exempted but carries no retired identifier any more — \
             drop the exemption. Its stated reason was: {reason}"
        );
    }
}

#[cfg(test)]
mod rule_tests {
    use super::*;

    /// The defect this whole test exists for: `-w` cannot see these, and this
    /// rule must.
    #[test]
    fn the_two_identifiers_three_rounds_of_sweeps_missed_are_hits() {
        assert_eq!(names_retired("has_rain"), Some("rain"));
        assert_eq!(names_retired("build_rain_disallowed_tools"), Some("rain"));
        assert_eq!(names_retired("brian_model_id"), Some("brian"));
        assert_eq!(names_retired("reraise_only_after_brian_turn"), Some("brian"));
    }

    /// The false positives a substring rule produces. `drain` is not `rain`, and
    /// an audit tool that says otherwise gets ignored, which is worse than one
    /// that says nothing.
    #[test]
    fn a_substring_is_not_a_segment() {
        for clean in [
            "the_drain_rather_than_finishing_it",
            "a_parked_question_stops_the_drain",
            "constraint_check",
            "terrain_map",
            "brainstorm_notes",
        ] {
            assert_eq!(names_retired(clean), None, "{clean} is not a retired name");
        }
    }

    /// A bare word is data, not an identifier naming a live thing — the class
    /// round 3 hand-classified and this guard deliberately leaves alone.
    #[test]
    fn a_single_segment_is_never_a_hit() {
        assert_eq!(names_retired("brian"), None);
        assert_eq!(names_retired("rain"), None);
        assert_eq!(names_retired("Rain"), None);
    }

    /// Case follows Rust convention, so a const carries the same rule.
    #[test]
    fn the_comparison_ignores_case() {
        assert_eq!(names_retired("LEGACY_BRIAN_CUSTOM_INSTRUCTION"), Some("brian"));
        assert_eq!(names_retired("slot0_Rain_busy"), Some("rain"));
    }

    /// A trailing comment must not fail the guard, and must not hide an
    /// identifier sharing its line either.
    #[test]
    fn only_the_code_half_of_a_line_is_scanned() {
        assert_eq!(code_of("let has_rain = true; // rain was retired").trim(), "let has_rain = true;");
        assert!(is_comment("    /// `has_rain` used to gate this"));
        assert!(!is_comment("let x = 1;"));
        // A URL inside a string must not truncate the line at `//`.
        assert!(code_of(r#"let u = "https://x/brian_model_id";"#).contains("brian_model_id"));
    }
}
