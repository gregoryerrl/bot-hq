//! **Every storage method the phase-advance vote depends on must have a
//! production caller.**
//!
//! Round 5, finding E1. Migration 0062 shipped the vote with an epoch to close
//! its TIME axis and justified the design by counting call sites — *"exactly ONE
//! production call site, `AppState`'s phase writer"*. That call site was never
//! written. `Storage::bump_phase_epoch` had a definition, five calls across three
//! `#[tokio::test]` functions in its own file, and nothing else, so
//! `sessions.phase_epoch` stayed at 0 across every phase change the database
//! recorded (125 by 2026-08-17, and the count only grows): no vote was
//! ever invalidated by a transition, and no vote row was ever cleared. A
//! two-participant tally could be completed by one live vote plus one left over
//! from a previous visit to the same phase.
//!
//! (The counts above were "seven tests" in the first version of this file and in
//! `6dd6ffd`'s message. Seven was the number of grep line HITS, two of which were
//! doc comments — a count of matches read as a count of tests. Corrected in
//! Verify by actually counting. The commit message keeps the wrong figure because
//! rewriting shipped history for a supporting detail costs more than it fixes;
//! this file is the artifact a future session reads.)
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

use std::collections::BTreeSet;
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
    (
        "set_persisted_ipav_phase",
        "the transition's other durable write (0063) — without it a restart \
         resumes at Investigate while the votes it cleared stay cleared",
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

/// **A file's PRODUCTION half — everything before its `#[cfg(test)]` MODULE.**
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
/// **Splitting on `mod tests {` was wrong (round 5, E2), and splitting on a bare
/// `#[cfg(test)]` was also wrong (round 6, F3).** Both failures are the same
/// mistake in different clothes: matching a proxy for "the test module starts
/// here" instead of the thing itself.
///
/// - `mod tests {` misses the five modules in `src/` that are named something
///   else — `phase_vote_tests`, `plugin_kv_tests`, `plugin_tests`,
///   `ensure_claude_runnable_tests`, `session_doc_tests`. `bridge/mod.rs` holds
///   `phase_vote_tests` at 1727 *and* a later `mod tests {`, so the split cut at
///   the wrong one and handed 112 lines of the phase vote's own tests to the
///   guard as production — in the very file carrying five of the seven wires.
/// - A bare `#[cfg(test)]` matches the attribute on ANY item, not just a module.
///   `core/pump.rs:13` is `#[cfg(test)] use std::time::Duration;` — an import —
///   so the production half was **12 lines of 1046**. `core/sequencer.rs:3539` is
///   a test-only `fn jaccard_similarity`, hiding 263 more. 1297 production lines
///   invisible, in the two largest files in the crate.
///
/// So the split is on `#[cfg(test)]` **followed by `mod`**, which is what was
/// meant both times.
///
/// **The safety argument for the old form was narrower than it read.** It said
/// early truncation can only report a live wire as DEAD, which fails loudly —
/// true, and it is why F3 was latent rather than a false green. But it holds for
/// EXISTENCE guards only. An ABSENCE guard — "nothing may still call X", the
/// shape `retired_identifier_test.rs` has — inherits the same blindness in the
/// SILENT direction: unscanned code cannot violate a prohibition. Do not carry
/// the reassurance across to a guard of the other polarity.
///
/// `core/state.rs`'s own in-file guards keep `split("mod tests {")` — that file
/// has exactly one test module, so it is correct there. It is the walker over
/// OTHER files that inherited the assumption.
fn production_half(body: &str) -> &str {
    const MARKER: &str = "#[cfg(test)]";
    let mut from = 0;
    while let Some(i) = body[from..].find(MARKER) {
        let at = from + i;
        let rest = body[at + MARKER.len()..].trim_start();
        let rest = rest.strip_prefix("pub ").unwrap_or(rest).trim_start();
        if rest.starts_with("mod ") {
            return &body[..at];
        }
        from = at + MARKER.len();
    }
    body
}

/// **Files that are test code in their entirety, declared as such by a PARENT.**
///
/// Round 6, E4 — and the reason `production_half` alone is not enough. A module
/// can be test-only without carrying a single `#[cfg(test)]` of its own, because
/// the attribute sits on the `mod` line in the parent:
///
/// ```text
/// src/signaling/mod.rs:28   #[cfg(test)]
/// src/signaling/mod.rs:29   mod parity;
/// ```
///
/// `src/signaling/parity.rs` is 643 lines of test-only code with no marker in it.
/// `walk()` visits it and `production_half` finds nothing to split on, so every
/// one of those lines reads as production. **No refinement of the split can fix
/// this** — the evidence is in a different file. It has to be resolved by
/// resolving the declaration.
///
/// Latent when found: all seven call-forms grep to zero hits inside `parity.rs`.
/// It is fixed anyway because the next symbol added to `REQUIRED_WIRES` gets no
/// such luck, and because the inverse was checked — 15 files in `src/` carry no
/// `#[cfg(test)]` at all, and `parity.rs` is the only one of them that is test
/// code.
///
/// Only the `mod name;` form (semicolon — a FILE module) is collected. An inline
/// `#[cfg(test)] mod name { … }` has no separate file to exclude and is already
/// handled by [`production_half`].
fn test_only_files(files: &[PathBuf]) -> BTreeSet<PathBuf> {
    const MARKER: &str = "#[cfg(test)]";
    let mut out = BTreeSet::new();
    for f in files {
        let Ok(body) = fs::read_to_string(f) else {
            continue;
        };
        let Some(dir) = f.parent() else { continue };
        let mut from = 0;
        while let Some(i) = body[from..].find(MARKER) {
            let at = from + i;
            from = at + MARKER.len();
            let rest = body[from..].trim_start();
            let rest = rest.strip_prefix("pub ").unwrap_or(rest).trim_start();
            let Some(rest) = rest.strip_prefix("mod ") else {
                continue;
            };
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                continue;
            }
            // `mod name;` only — an inline `mod name {` declares no file.
            if !rest[name.len()..].trim_start().starts_with(';') {
                continue;
            }
            out.insert(dir.join(format!("{name}.rs")));
            out.insert(dir.join(&name).join("mod.rs"));
        }
    }
    out
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
    let test_only = test_only_files(&files);
    let mut skipped_defining = 0;
    let mut sources = Vec::new();
    for f in &files {
        if *f == defining {
            skipped_defining += 1;
            continue;
        }
        // E4: a file the parent declares under `#[cfg(test)] mod name;` is test
        // code in full, with nothing in it to split on.
        if test_only.contains(f) {
            continue;
        }
        let Ok(body) = fs::read_to_string(f) else {
            continue;
        };
        sources.push((f.clone(), body));
    }
    assert_eq!(
        skipped_defining, 1,
        "{DEFINING_FILE} must exist and be excluded exactly once — it is the \
         file whose own references prove nothing"
    );
    assert!(
        test_only.iter().any(|p| p.ends_with("signaling/parity.rs")),
        "`src/signaling/parity.rs` is declared `#[cfg(test)] mod parity;` in \
         `signaling/mod.rs` and must be recognised as test-only — if this fails, \
         either it was renamed or the declaration scan stopped working, and 643 \
         lines of test code are being read as production again (round 6, E4)"
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

/// **`#[cfg(test)]` on an import or a function is not where the test module
/// starts.** Round 6, F3 — pinned apart from [`test_only_files`] so a regression
/// in either mechanism names itself.
///
/// The synthetic cases state the rule; the two real files state the cost, and
/// they are asserted by CONTENT rather than by line count so the test does not
/// rot as the files grow.
#[test]
fn a_test_only_import_or_fn_does_not_truncate_the_production_half() {
    assert_eq!(
        production_half("fn a() {}\n#[cfg(test)]\nuse std::time::Duration;\nfn b() {}\n"),
        "fn a() {}\n#[cfg(test)]\nuse std::time::Duration;\nfn b() {}\n",
        "an import carrying the attribute must not end the production half"
    );
    assert_eq!(
        production_half("fn a() {}\n#[cfg(test)]\nfn helper() {}\nfn b() {}\n"),
        "fn a() {}\n#[cfg(test)]\nfn helper() {}\nfn b() {}\n",
        "a test-only fn must not end it either"
    );
    assert_eq!(
        production_half("fn a() {}\n#[cfg(test)]\nmod t {\n fn x() {} \n}\n"),
        "fn a() {}\n",
        "a test MODULE must still end it"
    );
    assert_eq!(
        production_half("fn a() {}\n#[cfg(test)]\npub mod t {}\n"),
        "fn a() {}\n",
        "`pub mod` is still a module"
    );

    let root = repo_root();

    // `pump.rs:13` is `#[cfg(test)] use std::time::Duration;`, and its real test
    // module is ~1000 lines later. Under the old bare-literal split the guard
    // saw twelve lines of this file.
    let pump = fs::read_to_string(root.join("src/core/pump.rs")).expect("core/pump.rs");
    let half = production_half(&pump);
    assert!(
        half.contains("#[cfg(test)]\nuse std::time::Duration;"),
        "the test-only import must sit INSIDE the production half — if it does \
         not, the split truncated at the attribute again and ~1000 lines of \
         `pump.rs` are invisible to this guard"
    );
    assert!(
        !half.contains("#[cfg(test)]\nmod tests"),
        "`pump.rs`'s real test module must still be excluded"
    );

    // `sequencer.rs:3539` is a test-only `fn jaccard_similarity`.
    let seq = fs::read_to_string(root.join("src/core/sequencer.rs")).expect("core/sequencer.rs");
    assert!(
        production_half(&seq).contains("fn jaccard_similarity"),
        "the test-only fn must sit inside the production half of `sequencer.rs`"
    );
}

/// **A file can be test-only with no marker in it at all.** Round 6, E4.
///
/// The declaration lives in the parent, so this is the one hole no refinement of
/// [`production_half`] could close. Both directions are pinned: the real
/// test-only file must be FOUND, and an ordinary marker-free module must NOT be
/// swept up with it — without the second half, `test_only_files` returning every
/// path would pass.
#[test]
fn a_file_module_declared_under_cfg_test_is_found_without_a_marker_of_its_own() {
    let root = repo_root();
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    let test_only = test_only_files(&files);

    let parity = root.join("src/signaling/parity.rs");
    assert!(
        parity.is_file(),
        "src/signaling/parity.rs must exist for this test to mean anything"
    );
    assert!(
        !fs::read_to_string(&parity).unwrap().contains("#[cfg(test)]"),
        "`parity.rs` carries no marker of its own — that is the whole point. If \
         one was added, this test is no longer exercising E4 and needs a new \
         subject"
    );
    assert!(
        test_only.contains(&parity),
        "`#[cfg(test)] mod parity;` in signaling/mod.rs must make parity.rs \
         test-only"
    );

    // The other direction. `core/ipav.rs` is a plain `mod ipav;` — marker-free
    // production code — and must survive.
    let ipav = root.join("src/core/ipav.rs");
    assert!(
        !test_only.contains(&ipav),
        "a normally-declared module must not be swept up as test-only, or this \
         filter silently hides production code from the guard — the exact \
         failure direction the split's safety argument does NOT cover"
    );
    assert!(
        test_only.len() < files.len() / 4,
        "test_only_files matched {} of {} files — that is a scan gone wrong, \
         not a tree full of test-only modules",
        test_only.len(),
        files.len()
    );
}
