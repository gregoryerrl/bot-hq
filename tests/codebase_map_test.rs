//! `CODEBASE.md` is the area map of this repository — every source file is
//! assigned to exactly one area so a reader can study one area at a time. A map
//! that drifts from the tree is worse than none, so this test pins the two
//! directions mechanically:
//!
//! 1. every source file in the tree is NAMED in `CODEBASE.md` (by its
//!    repo-relative path — an unmapped file fails the suite until it is placed);
//! 2. every repo-relative path `CODEBASE.md` names EXISTS (a renamed or deleted
//!    file fails the suite until the map is updated).
//!
//! Coverage rules: `src/**/*.rs`, `tests/*.rs`, `examples/**` (files),
//! `frontend/src/**/*.{ts,tsx,css}` must be named. A frontend test file
//! `X.test.ts(x)` is covered by naming its subject `X.ts(x)`. `migrations/` is
//! covered as a directory (files there are immutable and numbered), so
//! individual migrations are not required — but any migration path the map
//! DOES name must exist. A `path:line` citation counts as naming `path`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const MAP: &str = "CODEBASE.md";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "node_modules" || name == "target" || name == "dist" {
                continue;
            }
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

/// The files the map must name, as repo-relative paths. Frontend test files are
/// folded onto their subject.
fn required_paths(root: &Path) -> BTreeSet<String> {
    let mut files = Vec::new();
    for sub in ["src", "tests", "examples", "frontend/src"] {
        walk(&root.join(sub), &mut files);
    }
    let mut out = BTreeSet::new();
    for f in files {
        let r = rel(root, &f);
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_source = if r.starts_with("frontend/src/") {
            matches!(ext, "ts" | "tsx" | "css")
        } else if r.starts_with("examples/") {
            true
        } else {
            ext == "rs"
        };
        if !is_source {
            continue;
        }
        // frontend `X.test.tsx` → `X.tsx`; a `.test.ts` whose subject is a
        // `.tsx` (`contextLibraryShared.test.ts` → `contextLibraryShared.tsx`)
        // folds onto the file that actually exists.
        let folded = if r.starts_with("frontend/src/") {
            let f = r.replace(".test.tsx", ".tsx").replace(".test.ts", ".ts");
            match f.strip_suffix(".ts") {
                Some(stem) if !root.join(&f).exists() && root.join(format!("{stem}.tsx")).exists() => {
                    format!("{stem}.tsx")
                }
                _ => f,
            }
        } else {
            r
        };
        out.insert(folded);
    }
    out
}

/// Every backtick-quoted token in the map that looks like a repo path.
fn named_paths(map: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for chunk in map.split('`').skip(1).step_by(2) {
        // `src/x.rs:123` and `src/x.rs:10-20` cite a line; the path is what
        // must exist.
        let token = chunk.trim().split(':').next().unwrap_or("").trim();
        let looks_like_path = ["src/", "tests/", "examples/", "frontend/", "migrations/", "docs/"]
            .iter()
            .any(|p| token.starts_with(p));
        if looks_like_path && !token.contains(' ') && !token.contains('*') {
            out.insert(token.to_string());
        }
    }
    out
}

#[test]
fn every_source_file_is_placed_in_the_codebase_map() {
    let root = repo_root();
    let map = fs::read_to_string(root.join(MAP))
        .unwrap_or_else(|e| panic!("{MAP} must exist at the repo root: {e}"));
    let named = named_paths(&map);
    let missing: Vec<String> = required_paths(&root)
        .into_iter()
        .filter(|p| !named.contains(p))
        .collect();
    assert!(
        missing.is_empty(),
        "{} source file(s) are not named in {MAP} — place each in its area:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn every_path_the_codebase_map_names_exists() {
    let root = repo_root();
    let map = fs::read_to_string(root.join(MAP))
        .unwrap_or_else(|e| panic!("{MAP} must exist at the repo root: {e}"));
    let stale: Vec<String> = named_paths(&map)
        .into_iter()
        .filter(|p| !root.join(p.trim_end_matches('/')).exists())
        .collect();
    assert!(
        stale.is_empty(),
        "{} path(s) named in {MAP} no longer exist — update the map:\n  {}",
        stale.len(),
        stale.join("\n  ")
    );
}

/// `scripts/test-windows.ps1` names every integration target explicitly, and
/// BOTH failure directions have already bitten this repo:
///
/// - a STALE name aborts the whole cargo invocation before anything compiles.
///   `external_mcp_test` was deleted 2026-08-17 and stayed in the list, so the
///   script's integration pass silently ran **nothing** for two months while
///   four newly-added targets were never executed once;
/// - a MISSING name means a test exists and never runs on the one platform it
///   was written for. `portable_home_test` had to be remembered into the list
///   by hand, and guards a bug that only manifests on Windows.
///
/// Neither direction is visible from the script alone or from `tests/` alone.
///
/// This lives inside an ALREADY-REGISTERED target on purpose: a standalone test
/// would have to be registered itself, and would inherit the exact bootstrap
/// problem it exists to prevent.
#[test]
fn the_windows_script_names_every_integration_target() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = fs::read_to_string(root.join("scripts/test-windows.ps1"))
        .expect("scripts/test-windows.ps1 must exist");

    // `--test <name>`, taking the first token so a trailing PowerShell
    // continuation backtick or `@args` does not become part of the name.
    let named: BTreeSet<String> = script
        .lines()
        .filter_map(|l| l.trim().strip_prefix("--test "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(str::to_string)
        .collect();

    let on_disk: BTreeSet<String> = fs::read_dir(root.join("tests"))
        .expect("tests/ must exist")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rs"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect();

    // Fail closed: a broken walk or a parse that matched nothing must report
    // failure, not "no differences found".
    assert!(!on_disk.is_empty(), "the tests/ walk found no .rs files");
    assert!(
        !named.is_empty(),
        "parsed no --test names out of scripts/test-windows.ps1 - the parser \
         broke, which would make this guard pass by comparing nothing"
    );

    let missing: Vec<_> = on_disk.difference(&named).collect();
    let stale: Vec<_> = named.difference(&on_disk).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "scripts/test-windows.ps1 is out of sync with tests/.\n  \
         missing from the script (exists but never runs on Windows): {missing:?}\n  \
         named but absent from tests/ (ABORTS the whole pass): {stale:?}"
    );
}
