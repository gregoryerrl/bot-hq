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
/// **This function has been wrong three times. Read the history before changing
/// it, because two of the three fixes look like each other's bug.**
///
/// Each attempt matched a PROXY for "this is test code" instead of the thing
/// itself, and each proxy missed a different shape:
///
/// 1. **`split("mod tests {")`** (pre-round-5) missed the five modules in `src/`
///    named something else — `phase_vote_tests`, `plugin_kv_tests`,
///    `plugin_tests`, `ensure_claude_runnable_tests`, `session_doc_tests`.
///    `bridge/mod.rs` holds `phase_vote_tests` *and* a later `mod tests {`, so
///    the split cut at the wrong one and handed 112 lines of the phase vote's
///    own tests to the guard as production — in the file carrying five wires.
/// 2. **`split("#[cfg(test)]")`** (round 5, E2) matched the attribute on ANY
///    item. `core/pump.rs:13` is `#[cfg(test)] use std::time::Duration;`, so the
///    production half was **12 lines of 1046**; `core/sequencer.rs:3539` is a
///    test-only `fn`, hiding 263 more. 1297 lines invisible (round 6, F3).
/// 3. **Truncating at `#[cfg(test)]` + `mod`** (round 6, first fix) still read
///    the literal `#[cfg(test)]`, so it could not see
///    `#[cfg(all(test, not(windows)))]` at `agents/spawn.rs:1273` — the crate's
///    only compound predicate, and one of the five modules attempt 1 named. It
///    also scanned the test-only `debug_env`/`debug_command` at `:1509`/`:1525`,
///    which sit BEFORE that file's `mod tests`. ~263 lines of test code read as
///    production (round 6, EYES `cc3f369f`).
///
/// **The tension that makes this oscillate, stated so it stops.** Fixing 3 by
/// truncating at any cfg-test item recreates 2. Fixing 2 by truncating only at a
/// module recreates 3. Truncation itself is the bug: a file's test code is not
/// one contiguous tail. So this **excises each cfg-test-marked ITEM** and keeps
/// everything else, which is correct for all three shapes and has no ordering
/// assumption to violate.
///
/// The predicate is matched as `#[cfg(` … the word `test` … `)]`, so
/// `all(test, not(windows))` is caught. The item is then brace-matched to its
/// close, or to its `;` for `use` / `mod name;`. Brace matching **skips strings,
/// char literals and comments** — a lone `'{'` in a literal would desynchronize
/// the scan, and a desynchronized scan swallows or exposes code silently.
///
/// **Do not restore the "truncating early is safe" reassurance.** It said early
/// truncation can only report a live wire as DEAD, which fails loudly. True, and
/// it is why F3 was latent — but it holds for EXISTENCE guards only. An ABSENCE
/// guard ("nothing may still call X", the shape `retired_identifier_test.rs`
/// has) inherits the blindness in the SILENT direction, because unscanned code
/// cannot violate a prohibition. Reading test code AS production is silent in
/// BOTH polarities, which is why shape 3 mattered more than shape 2.
///
/// `core/state.rs`'s own in-file guards keep `split("mod tests {")` — that file
/// has exactly one test module, so it is correct there. It is the walker over
/// OTHER files that inherited the assumption.
/// Scanning is done on BYTES, never on `&str` slices at a computed index.
/// The first version walked `&body[i..]`, and `i` lands mid-character the moment
/// a doc comment contains a `…` — 220561 bytes into `core/session.rs`, as it
/// happens. Every delimiter this scan cares about is ASCII, and a UTF-8
/// continuation byte can never equal one, so byte comparisons are both safe and
/// exact. Excision removes whole ASCII-delimited items, so what is left is still
/// valid UTF-8.
fn production_half(body: &str) -> String {
    let b = body.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if let Some(j) = opaque_end(b, i) {
            out.extend_from_slice(&b[i..j]);
            i = j;
            continue;
        }
        if b[i..].starts_with(b"#[cfg(") {
            if let Some((attr_end, names_test)) = cfg_predicate(b, i) {
                if names_test {
                    i = item_end(b, attr_end);
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).expect("excising ASCII-delimited items preserves UTF-8")
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// End of a comment / string / char literal starting at `i`, or `None` if one
/// does not start there. Brace matching must not see delimiters inside these.
///
/// `'a` (a lifetime) is deliberately NOT treated as opaque — only a real char
/// literal is, which is why the two-and-three-byte forms are checked explicitly.
fn opaque_end(b: &[u8], i: usize) -> Option<usize> {
    let s = &b[i..];
    if s.starts_with(b"//") {
        return Some(s.iter().position(|c| *c == b'\n').map_or(b.len(), |n| i + n + 1));
    }
    if s.starts_with(b"/*") {
        let mut depth = 0usize;
        let mut k = 0;
        while k + 1 < s.len() {
            if s[k] == b'/' && s[k + 1] == b'*' {
                depth += 1;
                k += 2;
            } else if s[k] == b'*' && s[k + 1] == b'/' {
                depth -= 1;
                k += 2;
                if depth == 0 {
                    return Some(i + k);
                }
            } else {
                k += 1;
            }
        }
        return Some(b.len());
    }
    if s.first() == Some(&b'r') {
        let hashes = s[1..].iter().take_while(|c| **c == b'#').count();
        if s.get(1 + hashes) == Some(&b'"') {
            let mut close = vec![b'"'];
            close.extend(std::iter::repeat_n(b'#', hashes));
            let start = 1 + hashes + 1;
            return Some(
                find_bytes(&s[start..], &close).map_or(b.len(), |n| i + start + n + close.len()),
            );
        }
    }
    if s.first() == Some(&b'"') {
        let mut k = 1;
        while k < s.len() {
            match s[k] {
                b'\\' => k += 2,
                b'"' => return Some(i + k + 1),
                _ => k += 1,
            }
        }
        return Some(b.len());
    }
    if s.first() == Some(&b'\'') {
        if s.get(1) == Some(&b'\\') {
            return s[1..].iter().position(|c| *c == b'\'').map(|n| i + 1 + n + 1);
        }
        // A char literal holds exactly one char, which may be several bytes
        // (`'…'`). A lifetime (`'a` in `&'a str`) has no closing quote and must
        // NOT be treated as opaque.
        for len in 1..=4usize {
            if s.get(1 + len) == Some(&b'\'')
                && std::str::from_utf8(&s[1..1 + len]).is_ok_and(|t| t.chars().count() == 1)
            {
                return Some(i + 1 + len + 1);
            }
        }
        return None;
    }
    None
}

/// Parse `#[cfg(...)]` at `i`. Returns the byte index just past the attribute
/// and whether its predicate names `test` as a whole word — so bare `test` and
/// `all(test, not(windows))` both answer true, and `windows` alone does not.
fn cfg_predicate(b: &[u8], i: usize) -> Option<(usize, bool)> {
    let open = i + "#[cfg".len();
    let mut k = open;
    let mut depth = 0usize;
    while k < b.len() {
        match b[k] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        k += 1;
    }
    if k >= b.len() {
        return None;
    }
    // The predicate is ASCII by construction (identifiers, parens, commas), so
    // this slice is always a valid boundary pair.
    let names_test = std::str::from_utf8(&b[open..=k])
        .is_ok_and(|p| p.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "test"));
    let end = b[k..].iter().position(|c| *c == b']').map(|n| k + n + 1)?;
    Some((end, names_test))
}

/// End of the item an attribute decorates: brace-matched for a block item, or
/// the `;` for `use` / `mod name;`. Further attributes between the two are
/// skipped for free — they carry no braces at depth 0.
fn item_end(b: &[u8], from: usize) -> usize {
    let mut i = from;
    let mut depth = 0usize;
    while i < b.len() {
        if let Some(j) = opaque_end(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            b';' if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    b.len()
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

/// **Test code is excised item by item; production code around it survives.**
///
/// Pinned apart from [`test_only_files`] so a regression in either mechanism
/// names itself, and asserted by CONTENT rather than line count so it does not
/// rot as the files grow. Every case here is a shape that shipped: see
/// [`production_half`]'s doc for which round each one broke in.
#[test]
fn cfg_test_items_are_excised_without_truncating_what_follows() {
    // Shape 2 (round 5): an import must not take the rest of the file with it.
    let p = production_half("fn a() {}\n#[cfg(test)]\nuse std::time::Duration;\nfn b() {}\n");
    assert!(p.contains("fn a()") && p.contains("fn b()"), "got: {p:?}");
    assert!(!p.contains("use std::time::Duration"), "got: {p:?}");

    // Shape 2 again, on a fn rather than a use.
    let p = production_half("fn a() {}\n#[cfg(test)]\nfn helper() { let x = 1; }\nfn b() {}\n");
    assert!(p.contains("fn a()") && p.contains("fn b()"), "got: {p:?}");
    assert!(!p.contains("fn helper"), "got: {p:?}");

    // A module still goes, and so does anything after it inside its braces.
    let p = production_half("fn a() {}\n#[cfg(test)]\nmod t {\n fn x() {} \n}\nfn c() {}\n");
    assert!(p.contains("fn a()") && p.contains("fn c()"), "got: {p:?}");
    assert!(!p.contains("fn x()"), "got: {p:?}");
    assert!(!production_half("#[cfg(test)]\npub mod t {}\nfn a() {}\n").contains("mod t"));

    // Shape 3 (round 6, EYES): a COMPOUND predicate is still a test predicate.
    let p = production_half(
        "fn a() {}\n#[cfg(all(test, not(windows)))]\nmod m {\n fn x() {} \n}\nfn b() {}\n",
    );
    assert!(p.contains("fn a()") && p.contains("fn b()"), "got: {p:?}");
    assert!(
        !p.contains("fn x()"),
        "`all(test, not(windows))` names `test` and must be excised — this is \
         `agents/spawn.rs:1273`, the crate's only compound form. got: {p:?}"
    );
    // ...but a cfg that does NOT name test must survive untouched.
    let p = production_half("#[cfg(unix)]\nfn only_on_unix() {}\n");
    assert!(
        p.contains("fn only_on_unix"),
        "a non-test cfg must not be excised, or this guard starts hiding real \
         production code from itself. got: {p:?}"
    );

    // A brace inside a literal must not desynchronize the match. The assertion
    // has to be that test code AFTER the literal is still excised: an early
    // termination excises LESS, so checking only that the opening line is gone
    // passes either way. That vacuous form was caught by its own kill test —
    // the string-skipping could be disabled with the suite still green.
    let p = production_half(
        "#[cfg(test)]\nmod t {\n fn a() { let s = \"}\"; }\n \
         fn b() { x.cast_phase_vote(); }\n}\nfn after() {}\n",
    );
    assert!(p.contains("fn after()"), "got: {p:?}");
    assert!(
        !p.contains("cast_phase_vote"),
        "a `}}` inside a string literal ended the item early, so the REST of the \
         test module survived into the production half — a call site in it would \
         read as a live wire. This is the silent direction. got: {p:?}"
    );
    // Same for a line comment and a char literal.
    let p = production_half(
        "#[cfg(test)]\nmod t {\n // }\n fn b() { x.cast_phase_vote(); }\n}\nfn after() {}\n",
    );
    assert!(p.contains("fn after()") && !p.contains("cast_phase_vote"), "got: {p:?}");
    let p = production_half(
        "#[cfg(test)]\nmod t {\n fn a() -> char { '}' }\n \
         fn b() { x.cast_phase_vote(); }\n}\nfn after() {}\n",
    );
    assert!(p.contains("fn after()") && !p.contains("cast_phase_vote"), "got: {p:?}");

    let root = repo_root();

    // The three real files, by content. `pump.rs:13` is a test-only import whose
    // real test module is ~1000 lines later; `sequencer.rs:3539` a test-only fn;
    // `spawn.rs` carries the compound module AND two test-only fns before its
    // own `mod tests`.
    for (path, must_keep, must_drop) in [
        ("src/core/pump.rs", "struct ToolUseRow", "async fn buffers_until_the_phase"),
        ("src/core/sequencer.rs", "fn jaccard_from_sets", "fn jaccard_similarity"),
        ("src/agents/spawn.rs", "pub struct SpawnConfig", "mod ensure_claude_runnable_tests"),
    ] {
        let body = fs::read_to_string(root.join(path)).unwrap_or_else(|_| panic!("{path}"));
        let half = production_half(&body);
        if body.contains(must_keep) {
            assert!(
                half.contains(must_keep),
                "{path}: production item `{must_keep}` was excised — the scan \
                 desynchronized and this guard is now blind to real code"
            );
        }
        assert!(
            !half.contains(must_drop),
            "{path}: test item `{must_drop}` survived into the production half, \
             so a dead wire in it would read as LIVE — the silent direction"
        );
    }

    // spawn.rs's test-only helpers sit BEFORE its `mod tests`, which is what
    // made truncation wrong there specifically.
    let spawn = fs::read_to_string(root.join("src/agents/spawn.rs")).expect("agents/spawn.rs");
    let half = production_half(&spawn);
    for helper in ["fn debug_env", "fn debug_command"] {
        assert!(
            !half.contains(helper),
            "`{helper}` is `#[cfg(test)]` and must be excised even though it \
             precedes the file's test module"
        );
    }
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
