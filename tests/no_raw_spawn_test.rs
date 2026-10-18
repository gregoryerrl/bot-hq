//! Every child process bot-hq starts goes through `bot_hq::process` — the one
//! constructor that keeps a Windows build, a GUI-subsystem program since
//! 1.0.9, from opening a console window for each child (`CREATE_NO_WINDOW`).
//! A raw `Command::new(` in production code under `src/` fails this test.
//! Converting the call sites by hand would miss one, and a missed site
//! flashes a console window on every call.
//!
//! Production code is a file's text up to its first `#[cfg(test)]` test
//! module — test modules sit at the end of their file (clippy's
//! `items_after_test_module`, cleared in `283e3f8`), and a test that spawns
//! git is a test, not a window on the user's screen. Comment lines are skipped.
//! `portable_pty::CommandBuilder` (the terminal tab's ConPTY shell) is not
//! a `Command` and does not match.

use std::fs;
use std::path::{Path, PathBuf};

const NEEDLE: &str = "Command::new(";
const CONSTRUCTOR: &str = "src/process.rs";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The text before the file's first test module: a `#[cfg(test)]` line whose
/// next non-attribute line opens a `mod`.
fn production_part(src: &str) -> &str {
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut offset = 0;
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == "#[cfg(test)]" {
            let next = lines[i + 1..]
                .iter()
                .map(|l| l.trim())
                .find(|l| !l.starts_with("#["));
            if next.is_some_and(|l| l.starts_with("mod ") || l.starts_with("pub mod ")) {
                return &src[..offset];
            }
        }
        offset += line.len();
    }
    src
}

/// `(line number, line)` of every raw spawn in `src`'s production part.
fn raw_spawns(src: &str) -> Vec<(usize, String)> {
    production_part(src)
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//") && l.contains(NEEDLE))
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .collect()
}

#[test]
fn every_production_spawn_goes_through_the_process_constructor() {
    let root = repo_root();
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    files.sort();
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel == CONSTRUCTOR {
            continue;
        }
        let src = fs::read_to_string(file).unwrap();
        scanned += 1;
        for (n, line) in raw_spawns(&src) {
            offenders.push(format!("{rel}:{n}: {line}"));
        }
    }
    assert!(scanned > 100, "the walk found only {scanned} files under src/ — it is not reading the tree");
    assert!(
        offenders.is_empty(),
        "raw spawns outside {CONSTRUCTOR} — use bot_hq::process::command / tokio_command \
         (CREATE_NO_WINDOW on Windows), or a console window opens per call:\n{}",
        offenders.join("\n")
    );
    // The constructor itself is where the needle lives — if it is gone, the
    // test above proves nothing about what replaced it.
    let constructor = fs::read_to_string(root.join(CONSTRUCTOR)).unwrap();
    assert!(constructor.contains("std::process::Command::new(") && constructor.contains("tokio::process::Command::new("));
    assert!(constructor.contains("CREATE_NO_WINDOW"));
}

#[test]
fn the_scan_sees_production_spawns_and_skips_tests_and_comments() {
    let src = "use x;\n\
               // Command::new(\"in a comment\")\n\
               fn a() { let c = std::process::Command::new(\"git\"); }\n\
               fn b() { let c = StdCommand::new(\"sh\"); }\n\
               fn c() { let c = portable_pty::CommandBuilder::new(\"cmd.exe\"); }\n\
               #[cfg(test)]\n\
               #[allow(dead_code)]\n\
               mod tests {\n\
                   fn t() { let c = std::process::Command::new(\"git\"); }\n\
               }\n";
    let hits = raw_spawns(src);
    assert_eq!(hits.iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![3, 4], "{hits:?}");
    // A `#[cfg(test)]` on an item that is not a module does not end the scan.
    let item = "#[cfg(test)]\nfn helper() {}\nfn d() { tokio::process::Command::new(\"x\"); }\n";
    assert_eq!(raw_spawns(item).len(), 1);
}
