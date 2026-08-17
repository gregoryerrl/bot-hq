//! **Every storage method the phase-advance vote depends on must have a
//! production caller.**
//!
//! Round 5, finding E1. Migration 0062 shipped the vote with an epoch to close
//! its TIME axis and justified the design by counting call sites — *"exactly ONE
//! production call site, `AppState`'s phase writer"*. That call site was never
//! written. `Storage::bump_phase_epoch` had a definition, seven tests calling it
//! directly, and nothing else, so `sessions.phase_epoch` stayed at 0 across 114
//! live phase changes: no vote was ever invalidated by a transition, and no vote
//! row was ever cleared. A two-participant tally could be completed by one live
//! vote plus one left over from a previous visit to the same phase.
//!
//! **Three layers of verification could not see it, and the reason is
//! mechanical.** `bump_phase_epoch` is `pub` on a `pub struct` in a lib crate, so
//! it is reachable from outside and rustc's `dead_code` can never fire on it.
//! Clippy had nothing to say. And every test referencing it called it directly —
//! *a component test never pins its own mount*.
//!
//! So this guard asks the one question none of them could: **is anything outside
//! the defining file using this at all?** No knowledge of the feature, no
//! reasoning about correctness — just the wire.
//!
//! Scope is deliberately the vote's storage surface rather than every `pub fn` in
//! the crate. A crate-wide version would flag the many methods that are legitimately
//! reachable only from a Tauri command or an MCP dispatcher, and the noise would
//! retire it. Widen it when a second feature earns the same treatment.

use std::fs;
use std::path::{Path, PathBuf};

/// The file that DEFINES all seven. A reference here proves nothing — that is
/// exactly the shape E1 hid in.
const DEFINING_FILE: &str = "src/storage/participants.rs";

/// Every storage method `signaling/bridge`, `core/sequencer` or `core/state`
/// needs for the vote to function. Losing any one of them silently disables a
/// half of D37 while the rest keeps reporting success.
const REQUIRED_WIRES: &[(&str, &str)] = &[
    (
        "bump_phase_epoch",
        "the transition that invalidates every vote cast about the phase being \
         left — E1 itself",
    ),
    ("cast_phase_vote", "recording a participant's vote"),
    (
        "retract_phase_votes",
        "a pass takes back the passer's own vote",
    ),
    (
        "phase_vote_tally",
        "the (voted, of) operands the tool reports back",
    ),
    (
        "all_active_voted_to_advance",
        "the consensus test that performs the transition",
    ),
    (
        "phase_artifact_fingerprint",
        "the content axis — changing the work invalidates the votes",
    ),
    (
        "phase_epoch",
        "reading the current epoch to key a vote against",
    ),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

/// A line's code half. Comments are excluded for the reason `framing.ts` and
/// `retired_identifier_test.rs` both give: prose ABOUT a symbol is a record, and
/// a guard that counted it would be satisfied by its own explanation. A `//`
/// inside a string literal would truncate early — that only ever makes this
/// guard stricter, never laxer, so it is left simple on purpose.
fn code_of(line: &str) -> &str {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
        return "";
    }
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// **A file's PRODUCTION half — everything before its `#[cfg(test)]` module.**
///
/// This is not tidiness, it is the whole guard. Written without it, and caught by
/// its own mutation check: deleting the epoch bump from `core/state.rs` left this
/// guard GREEN, because the sibling count guard's assertion contains the literal
/// `".bump_phase_epoch("` as a string, and a whole-file scan reads a test fixture
/// as a live call site.
///
/// That is round 4's F7 exactly — `cl_stale_refs` counted a symbol present if it
/// appeared anywhere in tracked code, so the guard's own fixture kept a retired
/// name alive and *"the remedy suppressed the metric"*. Reproducing it here, in a
/// guard whose only job is to refuse false reassurance, is the reason this
/// function exists and is documented rather than inlined.
///
/// **The split is on `#[cfg(test)]`, NOT on `mod tests {`** — and that
/// distinction is itself a round-5 finding (E2), caught in review one turn after
/// the bug above. Five modules in `src/` are not called `tests`:
/// `phase_vote_tests`, `plugin_kv_tests`, `plugin_tests`,
/// `ensure_claude_runnable_tests`, `session_doc_tests`. `bridge/mod.rs` holds
/// `phase_vote_tests` at 1727 *and* a later `mod tests {` at 1838, so a
/// `mod tests {` split cut at the wrong one and handed 112 lines of the phase
/// vote's own tests to the guard as production — in the very file carrying five
/// of the seven wires. `sequencer.rs` leaked 264 lines the same way, and two
/// files with no `mod tests {` at all leaked their whole test module.
///
/// Verified safe against this tree: every production call site precedes its
/// file's first `#[cfg(test)]` (state.rs 275 vs 1949, bridge/mod.rs 1354-1375 vs
/// 1726, sequencer.rs 2856 vs 3539), so all seven wires stay visible. The
/// failure directions are not symmetric, which is why the stricter form wins:
/// truncating too early can only report a live wire as DEAD, which fails loudly
/// and gets read; truncating too late reports a dead wire as LIVE, which is
/// silent and is the entire failure this file exists to prevent.
///
/// `core/state.rs`'s own in-file guards keep `split("mod tests {")` — that file
/// has exactly one test module, so it is correct there. It is the walker over
/// OTHER files that inherited the assumption.
fn production_half(body: &str) -> &str {
    body.split("#[cfg(test)]").next().unwrap_or(body)
}

#[test]
fn every_phase_vote_storage_method_has_a_production_caller() {
    let root = repo_root();
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    assert!(
        files.len() > 40,
        "the walker found only {} rust files — it is not reaching src/",
        files.len()
    );

    let defining = root.join(DEFINING_FILE);
    let mut sources = Vec::new();
    for f in &files {
        if *f == defining {
            continue;
        }
        let Ok(body) = fs::read_to_string(f) else {
            continue;
        };
        sources.push((f.clone(), body));
    }
    assert!(
        sources.len() + 1 == files.len(),
        "{DEFINING_FILE} must exist and be excluded exactly once — it is the \
         file whose own references prove nothing"
    );

    let mut dead = Vec::new();
    for (name, why) in REQUIRED_WIRES {
        // The CALL form. A bare-name search would match a `use` line, a doc
        // reference or a string literal, and every one of those is exactly the
        // false reassurance this guard exists to refuse.
        let needle = format!(".{name}(");
        let found = sources.iter().any(|(_, body)| {
            production_half(body)
                .lines()
                .any(|l| code_of(l).contains(needle.as_str()))
        });
        if !found {
            dead.push(format!("  `{name}` — {why}"));
        }
    }

    assert!(
        dead.is_empty(),
        "these phase-vote storage methods are defined and tested but nothing \
         outside {DEFINING_FILE} calls them, so the behaviour they implement is \
         inert in production:\n{}\n\nThis is round 5's E1. A `pub` method on a \
         `pub struct` in a lib crate is never `dead_code` to rustc, and a test \
         that calls it directly does not pin its mount — so this assertion is \
         the only thing standing between a carefully-reviewed storage layer and \
         a feature that does nothing.",
        dead.join("\n")
    );
}

/// **The guard must be able to see the failure it claims to prevent.**
///
/// A guard that passes on first run proves nothing (conventions.md, from the
/// 2026-08-06 migration-guard work). The live assertion above cannot be proved by
/// its own green run, so this pins the DISCRIMINATION separately: a name nothing
/// calls must be reported, and the call-form needle must not be satisfied by
/// prose or by a definition.
#[test]
fn the_guard_reports_a_wire_that_is_not_there() {
    let root = repo_root();
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    let defining = root.join(DEFINING_FILE);

    let sees = |needle: &str| {
        files.iter().filter(|f| **f != defining).any(|f| {
            fs::read_to_string(f)
                .map(|b| {
                    production_half(&b)
                        .lines()
                        .any(|l| code_of(l).contains(needle))
                })
                .unwrap_or(false)
        })
    };

    assert!(
        !sees(".bump_phase_epoch_that_does_not_exist("),
        "a method nobody calls must read as absent, or the guard above is \
         vacuous"
    );
    assert!(
        sees(".bump_phase_epoch("),
        "the real wire must read as present — if this fails, the fix for E1 has \
         been reverted"
    );
    assert_eq!(
        code_of("        // storage.bump_phase_epoch(id) — how it used to work"),
        "",
        "a comment naming the call form must not satisfy the guard: prose about \
         a symbol is a record, not a wire"
    );
    assert_eq!(
        code_of("    let x = 1; // .bump_phase_epoch(").trim(),
        "let x = 1;",
        "a trailing comment is stripped, so an explanation beside real code \
         cannot stand in for the call it describes"
    );

    // E2: a test module is stripped whatever it is CALLED. `mod tests {` was
    // the first split and it was wrong for five modules in `src/` — including
    // `phase_vote_tests`, in the file carrying five of the seven wires.
    let sample = "fn real() { x.cast_phase_vote(); }\n\
                  #[cfg(test)]\n\
                  mod phase_vote_tests {\n\
                      fn t() { s.bump_phase_epoch(); }\n\
                  }\n";
    let prod = production_half(sample);
    assert!(
        prod.contains(".cast_phase_vote("),
        "production code before the test module must survive the split"
    );
    assert!(
        !prod.contains(".bump_phase_epoch("),
        "a test module named anything other than `tests` must still be \
         stripped — otherwise a future test in `phase_vote_tests` keeps a \
         deleted production call reading as live, which is this file's own \
         failure mode wearing the guard's clothes"
    );

    // And the real file it was found in, not just a synthetic sample.
    let bridge = fs::read_to_string(root.join("src/signaling/bridge/mod.rs"))
        .expect("the bridge carries five of the seven wires");
    assert!(
        !production_half(&bridge).contains("mod phase_vote_tests"),
        "`phase_vote_tests` must fall outside the production half of the file \
         that defines the vote's call sites"
    );
}
