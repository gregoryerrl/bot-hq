//! No source file may read the `HOME` environment variable directly.
//!
//! # Why this exists
//!
//! `HOME` is a POSIX convention. It is **unset** in the native Windows
//! environment, where the user's home lives in `USERPROFILE` — and a Git Bash
//! shell sets `HOME`, so the gap is invisible from the terminal a developer
//! tests in and only appears in the GUI-launched app.
//!
//! Two live sites had it on 2026-08-25, and both failed SILENTLY:
//!
//! | site | consequence on Windows |
//! |---|---|
//! | `signaling/server.rs` `default_user_settings_paths` | returned an empty `Vec`, so every agent spawned with **no user MCP servers forwarded** — no error anywhere |
//! | `core/telemetry.rs` `panic_event` | `redact_home` took its `None` arm and hashed the path UNREDACTED, so identical crashes never aggregated across machines (and `PRIVACY.md` promised otherwise) |
//!
//! Neither had any test. `paths.rs` documents this exact class as having been
//! consolidated in round 9 — these two were missed by that sweep, which is
//! precisely why a standing guard beats another sweep.
//!
//! # The rule
//!
//! Use `crate::paths::home_dir()`, which resolves `USERPROFILE` on Windows and
//! `HOME` elsewhere. `paths.rs` is the one file allowed to read the raw var,
//! because it is the thing doing the resolving.
//!
//! # What this does NOT guard
//!
//! Prose. The pattern matched is the CALL (`var("HOME")` / `var_os("HOME")`),
//! not the bare word, so a comment explaining the hazard does not trip it —
//! the same call `retired_identifier_test.rs` makes about comments.

use std::path::{Path, PathBuf};

/// The single file permitted to read the raw environment variable.
const ALLOWED: &str = "paths.rs";

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_source_reads_the_home_env_var_outside_paths_rs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    assert!(
        files.len() > 20,
        "the source walk found only {} files - the walk itself is broken, \
         which would make this guard pass by finding nothing",
        files.len()
    );

    let mut offenders = Vec::new();
    for file in &files {
        if file.file_name().and_then(|n| n.to_str()) == Some(ALLOWED) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(file) else {
            continue;
        };
        for (i, line) in body.lines().enumerate() {
            if line.contains(r#"var("HOME")"#) || line.contains(r#"var_os("HOME")"#) {
                offenders.push(format!("{}:{}: {}", file.display(), i + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "raw HOME reads outside {ALLOWED} - HOME is unset on Windows, so these \
         fail silently there. Use `crate::paths::home_dir()`:\n  {}",
        offenders.join("\n  ")
    );
}
