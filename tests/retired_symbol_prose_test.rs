//! No COMMENT may name a deleted symbol as a current thing.
//!
//! # Why this exists
//!
//! `retired_identifier_test.rs` keeps retired AGENT NAMES out of compiler-carried
//! identifiers, and exempts comments on purpose: a comment describing a
//! 2026-05-28 incident in that day's vocabulary is a record. This guard covers
//! the class that sits beside it and that exemption leaves open — a comment
//! naming a SYMBOL the tree no longer has, in the present tense, as if it were
//! still the mechanism. Rounds 9, 11 and 12 each swept ~30 such sites by hand
//! (`HANDS_ONLY_TOOLS` as the live gate, `set_busy` as the busy setter, the
//! router's `break_volley`, the driver's `external_jsonrpc`), and the class
//! regenerates between rounds because nothing pins prose to code. EYES (round
//! 12, F8): spend the sweep on a test instead of re-editing thirty sites.
//!
//! # The rule — the Context Library's own
//!
//! A comment line that names a symbol on the retired list must carry a
//! RETIREMENT MARKER on that same line — one of
//! [`RETIREMENT_MARKERS`](bot_hq::signaling::RETIREMENT_MARKERS), the phrases
//! `cl_stale_refs` treats as "this line is ABOUT a deletion" ("deleted",
//! "removed", "retired", "gone", "no longer", "used to", "legacy", "pre-rc3",
//! …). Per LINE, exactly as the CL tool scans, and for the same reason the CL's
//! round-8 note records: a banner's marker does not reach its continuation
//! lines, so a symbol named two lines under "the router is gone" counts as a
//! fresh claim. Write "the deleted `break_volley`", not "`break_volley`" with the
//! deletion three lines up. A history sentence passes; a present-tense copy of
//! old prose fails.
//!
//! # What this does NOT guard, deliberately
//!
//! **Code lines are exempt.** A live reference to a deleted symbol does not
//! compile; the ones that survive in code are string literals — test specimens
//! (`cl_staleness.rs` proving it finds `may_run_native`), a guard test asserting
//! a prompt does NOT say `declare_working(`, a slug list. Those are the
//! mechanism working, not the defect.
//!
//! **`signaling/parity.rs` is exempt by design** — it is the oracle that
//! re-derives the deleted name gate (`HANDS_ONLY_TOOLS` and friends) to prove
//! the capability gate reproduces it; naming the replaced lists is its job. The
//! exemption is asserted to still earn its place below.
//!
//! **Migrations and docs are out of scope** (immutable history; audited by
//! `cl_stale_refs` for the CL and by hand for `docs/`).

use std::path::{Path, PathBuf};

use bot_hq::signaling::RETIREMENT_MARKERS;

/// Symbols the tree no longer has, each of which a round found named as
/// current in a comment. Add a symbol when a sweep finds it; remove one only if
/// the symbol comes BACK to the tree (the `lives_nowhere` check below refuses a
/// list entry that still exists in production code, so a comeback is loud).
const RETIRED_SYMBOLS: &[&str] = &[
    // the pre-rc3 name gate (rc3 D16)
    "HANDS_ONLY_TOOLS",
    "EYES_ONLY_TOOLS",
    "CL_MUTATE_TOOLS",
    // the bilateral router (deleted 2026-08-13)
    "core/router.rs",
    "core/duo.rs",
    "break_volley",
    "HEARTBEAT_LEADS",
    "FlushHeld",
    "RouterCommand",
    "route_forward",
    "user_silent_forwards",
    "consecutive_short",
    "last_forward",
    // the external driver server + its bench (deleted 2026-08-17)
    "external_jsonrpc",
    "external_server",
    "external_mcp_test",
    // the native connector (rc3 D9)
    "spawn_native_agent",
    "may_run_native",
    "strip_claude_code_tool_inventory",
    "NATIVE_TOOL_ADDENDUM",
    // retired names, flags and columns
    "declare_working",
    "brian_busy",
    "rain_busy",
    "tray_wake_step",
    "BOT_HQ_SEQUENCER",
    "maintainClPrompt",
    // the two-party busy setter (replaced by `set_busy_slug`)
    "set_busy(",
];

/// Files allowed to name a retired symbol in a comment without a marker, each
/// because naming the retired thing IS the file's job.
const EXEMPT: &[(&str, &str)] = &[
    (
        "src/signaling/parity.rs",
        "the parity oracle re-derives the deleted name gate to prove the capability \
         gate reproduces it; the replaced lists are its subject",
    ),
    (
        "tests/retired_symbol_prose_test.rs",
        "this file spells the list it scans for",
    ),
];

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Hit {
    file: String,
    line: usize,
    symbol: &'static str,
    text: String,
}

/// Whether a source line is a comment (Rust line/doc comments, TS block and
/// JSDoc continuation lines). Line-level, like the sibling guards.
fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("/*") || t.starts_with('*')
}

/// The trailing comment of a code line (`let x = 1; // the old set_busy`),
/// if any. A `://` (a URL) is not a comment start.
fn trailing_comment(line: &str) -> Option<&str> {
    let mut search = 0;
    while let Some(i) = line[search..].find("//") {
        let at = search + i;
        if at > 0 && line.as_bytes()[at - 1] == b':' {
            search = at + 2;
            continue;
        }
        return Some(&line[at..]);
    }
    None
}

fn has_marker(text: &str) -> bool {
    let lower = text.to_lowercase();
    RETIREMENT_MARKERS.iter().any(|m| lower.contains(m))
}

fn names_symbol(text: &str) -> Option<&'static str> {
    RETIRED_SYMBOLS.iter().copied().find(|s| text.contains(s))
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "node_modules" || n == "dist") {
                continue;
            }
            sources(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "rs" | "ts" | "tsx"))
            && !path.file_name().is_some_and(|n| n == "bindings.ts")
        {
            out.push(path);
        }
    }
}

fn scan() -> Vec<Hit> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "tests", "frontend/src"] {
        sources(&root.join(dir), &mut files);
    }
    files.sort();
    let mut hits = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if EXEMPT.iter().any(|(f, _)| *f == rel) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (i, line) in body.lines().enumerate() {
            let prose = if is_comment(line) {
                Some(line)
            } else {
                trailing_comment(line)
            };
            let Some(prose) = prose else { continue };
            if let Some(symbol) = names_symbol(prose) {
                if !has_marker(prose) {
                    hits.push(Hit {
                        file: rel.clone(),
                        line: i + 1,
                        symbol,
                        text: prose.trim().chars().take(120).collect(),
                    });
                }
            }
        }
    }
    hits.sort();
    hits
}

#[test]
fn no_comment_names_a_retired_symbol_as_current() {
    let hits = scan();
    assert!(
        hits.is_empty(),
        "{} comment line(s) name a retired symbol with no retirement marker on the \
         same line. Either the claim is stale (say what replaced it), or it is \
         history — then put the marker ON that line (\"the deleted `X`\", \"`X` \
         used to …\"), the way cl_stale_refs reads the Context Library:\n{}",
        hits.len(),
        hits.iter()
            .map(|h| format!("  {}:{}  [{}]  {}", h.file, h.line, h.symbol, h.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Every listed symbol is genuinely gone from production code — a comeback
/// would make this guard flag correct comments, so it is refused here first.
#[test]
fn every_retired_symbol_lives_nowhere_in_production_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    let mut alive: Vec<String> = Vec::new();
    for path in files {
        let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().to_string();
        if rel.ends_with("parity.rs") {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else { continue };
        // Production code only: stop at the test module, skip comment lines and
        // string literals' test specimens by asking for the symbol as CODE.
        let prod = body.split("#[cfg(test)]").next().unwrap_or("");
        for (i, line) in prod.lines().enumerate() {
            if is_comment(line) {
                continue;
            }
            let code = trailing_comment(line).map_or(line, |c| &line[..line.len() - c.len()]);
            if code.contains('"') {
                continue; // a literal: data or a specimen, not a live reference
            }
            for s in RETIRED_SYMBOLS {
                // `set_busy(` must not match `set_busy_slug(`.
                let live = if *s == "set_busy(" {
                    code.contains("set_busy(") && !code.contains("fn set_busy(")
                } else {
                    code.contains(s)
                };
                if live {
                    alive.push(format!("{rel}:{}  [{s}]  {}", i + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        alive.is_empty(),
        "a RETIRED_SYMBOLS entry still appears in production code — either it came \
         back (remove it from the list) or the scan is wrong:\n{}",
        alive.join("\n")
    );
}

/// The exemptions earn their place: parity.rs must still name the replaced
/// gate (else the carve-out is a hole), and this file is on its own list.
#[test]
fn exemptions_still_earn_their_place() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let parity = std::fs::read_to_string(root.join("src/signaling/parity.rs")).unwrap();
    assert!(
        parity.contains("HANDS_ONLY_TOOLS"),
        "parity.rs no longer names the replaced gate; drop its exemption"
    );
    assert!(EXEMPT.iter().any(|(f, _)| *f == "tests/retired_symbol_prose_test.rs"));
    for (_, reason) in EXEMPT {
        assert!(reason.len() > 20, "an exemption carries its reason");
    }
}

#[cfg(test)]
mod rule_tests {
    use super::*;

    #[test]
    fn a_marker_on_the_same_line_passes_and_a_bare_claim_fails() {
        assert!(has_marker("the deleted `break_volley` suppressed the forward"));
        assert!(has_marker("`HANDS_ONLY_TOOLS` is gone — the capability set gates"));
        assert!(!has_marker("`HANDS_ONLY_TOOLS` decides which tools HANDS may call"));
        assert_eq!(names_symbol("see the old set_busy(x) call"), Some("set_busy("));
        assert_eq!(names_symbol("set_busy_slug(x, true) marks it"), None);
        assert_eq!(names_symbol("nothing retired here"), None);
    }

    #[test]
    fn trailing_comments_are_read_and_urls_are_not_comments() {
        assert_eq!(trailing_comment("let x = 1; // the old set_busy("), Some("// the old set_busy("));
        assert_eq!(trailing_comment("let u = \"https://example.com\";"), None);
        assert!(is_comment("    /// doc"));
        assert!(is_comment("  * jsdoc continuation"));
        assert!(!is_comment("let y = 2;"));
    }
}
