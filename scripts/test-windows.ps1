#!/usr/bin/env pwsh
# Run the full bot-hq backend test suite on Windows.
#
# Why this wrapper exists: cargo's test binaries receive no application
# manifest, so the loader activates the legacy v5.82 Common-Controls
# comctl32.dll, whose `CoTaskMemAlloc` import fails to bind -> the test binary
# crashes at load with STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) before a single
# test runs (tauri -> wry -> comctl32 links into every target). Declaring a
# Common-Controls v6 manifest fixes it, but cargo offers no link-arg scope that
# manifests the `--lib` harness without also hitting the [[bin]], where it
# collides with tauri-build's manifest (CVT1100/LNK1123). So the two test
# classes use two mechanisms, in two passes:
#
#   1. Integration tests (tests/*.rs): build.rs injects the manifest via its
#      `-tests` link-arg scope, which never touches the [[bin]]. Runs in the
#      normal target/ with no RUSTFLAGS, leaving the dev build cache intact.
#   2. Lib unit tests (--lib): reachable only by the unscoped manifest link-arg
#      (RUSTFLAGS). `--lib` builds no bin, so there's no collision; a separate
#      target dir keeps RUSTFLAGS from thrashing the main target/ cache.
#
# Plain `cargo test` on Windows still crashes the lib harness - use this script.
# A seamless single-manifest approach (tauri-build WindowsAttributes) is the
# tracked follow-up; see PROGRESS.md.
#
# MEASURE WINDOWS BEHAVIOUR FROM POWERSHELL, NOT GIT BASH. A Git Bash shell puts
# Git-for-Windows' usr/bin on PATH, which makes `sh` and the MSYS coreutils
# resolve and silently greens ~13 shell-execution tests that genuinely fail for
# a GUI-launched app. Running this script from Git Bash reports a better number
# than the product deserves.

$ErrorActionPreference = 'Continue'
$manifest = (Resolve-Path (Join-Path $PSScriptRoot '..\windows-test.manifest')).Path
$libTarget = (Join-Path $PSScriptRoot '..\target\windows-libtest')

# Save the caller's environment. Both vars are set on the live process below,
# and leaking them would silently redirect every later `cargo` in this shell to
# the wrong target dir with manifest link-args attached.
$prevRustFlags = $env:RUSTFLAGS
$prevEncoded = $env:CARGO_ENCODED_RUSTFLAGS
$prevTargetDir = $env:CARGO_TARGET_DIR

# FAIL CLOSED. These are only assigned after their `cargo test` returns, and
# with -ErrorActionPreference Continue a path where cargo never runs (not on
# PATH, a typo'd invocation, the try aborting early) would leave them $null.
# PowerShell evaluates `$null -ne 0` as TRUE, so the exit branch below is taken
# and `exit $null` yields exit code 0 - the script would report SUCCESS having
# run no tests at all. Harmless when read by hand; not harmless once this is
# wired into CI, where a green check on a script that executed nothing is
# exactly the failure the rest of this file is written to prevent.
$integration = 1
$lib = 1

try {
    # Pass 1 - integration tests, named explicitly. (`--tests` would also drag
    # in the lib unit-test harness, which has no manifest in this target dir and
    # would crash at load.) build.rs injects the manifest via -tests; no
    # RUSTFLAGS, so the bin (built because any integration-test selection sets
    # CARGO_BIN_EXE_*) keeps tauri's manifest untouched.
    #
    # Cargo derives a target name from the FILE STEM, so the `_test` suffix is
    # part of the name. Keep this list in sync with tests/*.rs - a stale name
    # aborts the whole pass before anything compiles, which is how this script
    # sat broken naming `external_mcp_test` (deleted 2026-08-17, d0661b45) while
    # four newer targets were never run at all.
    $env:RUSTFLAGS = $null
    $env:CARGO_ENCODED_RUSTFLAGS = $null
    $env:CARGO_TARGET_DIR = $null
    cargo test `
        --test codebase_map_test `
        --test phase_vote_wiring_test `
        --test retired_identifier_test `
        --test retired_symbol_prose_test `
        --test signaling_test `
        --test storage_test @args
    $integration = $LASTEXITCODE

    # Pass 2 - lib unit tests (manifest via link-args; --lib excludes the bin;
    # isolated target dir avoids thrashing the dev cache).
    #
    # CARGO_ENCODED_RUSTFLAGS, not RUSTFLAGS: cargo splits RUSTFLAGS on
    # WHITESPACE with no quote handling, so a repo path containing a space
    # ("C:\Users\Some Name\...") would split the manifest flag into two bogus
    # ones. Quoting does not help - the quotes would just become literal
    # characters in the flag and silently disable the injection, which reads
    # exactly like "the manifest workaround stopped working" and is miserable to
    # re-diagnose. The encoded form is 0x1f-separated and expresses spaces
    # correctly. RUSTFLAGS is cleared because the two are mutually exclusive
    # (encoded wins) and a leftover value would be confusing.
    $env:RUSTFLAGS = $null
    $env:CARGO_ENCODED_RUSTFLAGS =
        "-Clink-arg=/MANIFEST:EMBED" + [char]0x1f + "-Clink-arg=/MANIFESTINPUT:$manifest"
    $env:CARGO_TARGET_DIR = $libTarget
    cargo test --lib @args
    $lib = $LASTEXITCODE
}
finally {
    $env:RUSTFLAGS = $prevRustFlags
    $env:CARGO_ENCODED_RUSTFLAGS = $prevEncoded
    $env:CARGO_TARGET_DIR = $prevTargetDir
}

if ($integration -ne 0) { exit $integration }
exit $lib
