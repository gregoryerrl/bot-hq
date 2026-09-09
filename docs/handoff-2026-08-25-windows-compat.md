# Handoff — `windows-compat` → macOS

**Written from Windows, for the macOS machine to pick up.** Signature block at
the bottom names who verified what, and — more importantly — what nobody did.

- **Branch:** `windows-compat`, pushed, tree clean. **14 commits — this document
  is the 14th**, so don't expect a SHA quoted here to be HEAD.
- **Cut from:** `origin/main` @ `4a1eb72a`.
- **Not merged, no PR opened.**

> **Local `main` on the Windows box is stale at `64609cf4`**, so `git log
> main..HEAD` there reads 21 rather than the true 13. Run `git fetch origin
> main:main` before comparing.

---

## 1. Do this first on macOS — it closes this branch's biggest gap

**The Unix arms of everything written here are UNVERIFIED.** Rust strips
`#[cfg(not(windows))]` blocks after parsing, so syntax errors would have
surfaced but *name resolution and type checking never ran on them*. A
cross-check was attempted from Windows and is blocked environmentally —
tauri's Linux deps need a gtk sysroot (`pkg-config has not been configured to
support cross-compilation`); it never reached our sources.

macOS can settle it in one command:

```bash
git fetch origin && git checkout windows-compat
cargo test          # the whole point: this compiles the Unix arms
cd frontend && npm ci && npm test && npm run lint
```

Specifically unproven on any Unix machine:

| Site | What could be wrong |
|---|---|
| `tool_gate.rs` `gate_shell()` / `posix_shell()` | the `#[cfg(not(windows))]` arm keeps the original `$SHELL`+allowlist path; `gate_shell_label()` is new and uncfg'd |
| `terminal.rs` `spawn_shell()` + `mod script` | the Unix consts and `/bin/sh` arm are new code |
| `terminal_tools.rs` submit | `#[cfg(not(windows))]` → `\n`, unchanged behaviour but a new cfg split |
| `util.rs` `rel_key()` / `normalize_cl_path_input()` | uncfg'd, runs everywhere; `rel_key` is now on 3 call sites |
| `participants.rs` `lf()`, `files.rs`, `sessions.rs`, `hooks.rs` | test-side, uncfg'd |
| `tests/portable_home_test.rs`, `codebase_map_test`'s sync guard | new targets, never run on Unix |
| `frontend/framing.ts` `stripComments` | uncfg'd; also consumed by `overflow.ts` x2 |

**Expected on macOS — with numbers, so "green" is checkable rather than a vibe:**

| Suite | Expect |
|---|---|
| `cargo test` (lib) | **~1330 tests, 0 failures** — Windows shows 1325 passed + 5 Windows-only failures (§3) that should PASS on macOS |
| `cargo test` (integration) | **7 targets, 41 tests, 0 failures** |
| `npm test` | **56 files, 538 tests, 0 failures** |

A green with no count cannot distinguish a full run from a partial one — that
was failure #5 of this session (see the signature block). Check the counts.

Anything red is a real bug this branch introduced and could not see.

**Do NOT try to settle the `document.hasFocus()` question from macOS** (§6). It
is specifically about **WebView2**, which is Windows-only; macOS runs WKWebView,
so a probe there measures a different engine and answers nothing.

## 2. The work the user wants done on macOS: CI test jobs

**No CI job runs `cargo test` on any platform.** `.github/workflows/` has build
and release jobs only. That is why 41 Windows lib failures sat unnoticed, why
`scripts/test-windows.ps1` was a silent no-op since June, and why this branch
leaves an unverified Unix build behind it.

Suggested shape:

- **ubuntu + macos:** `cargo test`, then `npm ci && npm test && npm run lint`.
- **windows:** `.\scripts\test-windows.ps1` — **now possible for the first
  time**, because `build.rs` gained the `-tests` manifest injection on this
  branch. Run it from PowerShell, never a bash shell (see §4).
  - It exits non-zero on failure and **fails closed** if cargo never runs.
  - Windows currently has **5 known lib failures** (§3), so either allow them
    explicitly or land the PTY fix first — do not let a green CI hide them.

Two smaller items that belong with it:
- `tests/portable_home_test.rs` and the `codebase_map_test` sync guard were
  **committed before ever executing** (a `--lib` run doesn't build integration
  targets). They have since run on Windows: 7/7 targets, 41 tests, 0 failures.
  They have still never run on Unix.
- `PLAN.md` notes the `telemetry::start` call-site guards are
  `include_str!`+`contains` — they catch deletion but not comment-out. This
  branch added a sharper instance of the same weakness: `terminal.rs:466`
  asserts `thread.contains("child.wait()")`, and that code is present,
  uncommented, and **unreachable on Windows** (§3).

## 3. Open findings — diagnosed, not fixed

### (a) The PTY never marks a terminal dead on Windows — PRODUCT BUG
`terminal.rs:238`'s reader loop exits only on `read()` → `Ok(0) | Err`, and
`child.wait()`, the `[process exited]` note and `dead.store(true)` all sit
**downstream of it**. ConPTY does not deliver EOF on the master when the child
exits — the pseudoconsole holds the pipe open until the `HPCON` closes — so the
read blocks forever.

User-facing consequences, all from `dead` never firing:
- `:393` an exited shell is **never replaced** — type `exit` and it looks alive forever
- `:409` exited terminals counted as live
- `:330` `wait_settle`'s early-exit never fires, so **every call burns its full timeout**
- `[process exited]` never renders

**Fix direction:** death must come from the child handle, not the read side — a
waiter thread on a `clone_killer()`-style handle (`ChildKiller` is already
imported at `:15`). **Wrinkle:** the reader thread still blocks forever
afterward, so the master/`HPCON` has to be closed to unblock it, or you leak a
thread per terminal. Accounts for 4 of the 5 remaining Windows lib failures.

### (b) ConPTY DSR — and a product consequence nobody had noticed
ConPTY opens by emitting a **DSR cursor-position query (`ESC[6n`)** and
withholds the child's output until something answers `ESC[<row>;<col>R`. In the
app **xterm.js** answers automatically. Headless nothing does — every PTY test's
entire scrollback was exactly `"\u{1b}[6n"`.

The test helper now answers it (`terminal.rs:553`), which cleared 3 tests. But:

> **`SessionView.tsx:262`/`:871` mount the Terminal tab LAZILY, on first
> activation.** In a session where the user never opens that tab, there is no
> responder — so `terminal_exec` would stall exactly as the headless tests did.
> A backend-driven MCP tool depending on a UI component being mounted.

**This makes the `terminal_exec` fix on this branch CONDITIONAL** (§5, bug 3).
A reactive responder belongs in `SessionTerminal`'s reader loop. It **must
trigger on observing `ESC[6n`** — writing it unconditionally races the query,
and in an interactive shell a stray `ESC[1;1R` lands as typed characters on the
command line. The caveat is written at `terminal.rs:548-560`. Accounts for the
5th remaining failure (`terminal_tools` round-trip), which has a *different*
proximate cause from (a) — the responder is in the other helper's path.

### (c) Others, recorded not investigated
- **Windows child reaping:** `spawn.rs:750` `process_group(0)` is `#[cfg(unix)]`
  and `kill_child` terminates one PID, so cancel/close orphans tool children —
  and on Windows those orphans hold file locks that block worktree removal. Fix
  is a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; `windows-sys` is
  already a dependency. **New unsafe FFI with no live mileage — land it behind
  the Windows CI job, not before it.**
- **`atomic_write` renames over a possibly-open target** (`cl_write.rs:648-656`;
  10 `fs::rename` sites overall). Windows `MoveFileEx` fails with a sharing
  violation if the destination is held open. Fails loudly, doesn't corrupt.
- **CL key portability:** keys are now `/`-form via `rel_key`, but a DB written
  on Windows before `5024665a` holds `\` keys. A Windows-gated one-shot renames
  them in place at `Storage::open`. Nothing runs on macOS; a library synced
  between machines is the untested case.

## 4. Traps that cost this session real time

- **Measure Windows from PowerShell, never Git Bash.** Git Bash puts Git's
  `usr/bin` on PATH, so `sh` and the MSYS coreutils resolve and ~13
  shell-execution tests silently pass that genuinely fail for a GUI-launched
  app. This produced a wrong "corrected" baseline of 28 against a true 41.
- **`.gitattributes` was a 1-file commit, not the ~439 everyone expected.**
  `git ls-files --eol` field 1 shows 444 `i/lf`, 27 `i/-text`, **zero `i/crlf`**
  — the index was *always* LF and `git add --renormalize .` stages nothing. Both
  participants read that output repeatedly while tracking only field 2
  (`w/crlf`).
- **CRLF has three shapes and only one is fixed by `.gitattributes`:** a needle
  ENDING in `\n` fails on CRLF; a needle STARTING with `\n` still matches; a
  `split("\n")` leaves `\r` on every element with no needle at all. The third is
  invisible to both a panic-sweep and a needle-sweep. Fix in CODE where the
  input is arbitrary; a renormalize would only mask it. Any CRLF fixture must be
  written as `\r\n` **escapes**, or an LF working tree disarms it.
- **PowerShell: `$null -ne 0` is TRUE and `exit $null` yields 0.** An unassigned
  exit variable reports SUCCESS. Initialise exit codes to 1.
- **`RUSTFLAGS` is whitespace-split with no quote handling** — use
  `CARGO_ENCODED_RUSTFLAGS` (0x1f-separated). Quoting makes the quotes literal
  and silently disables the flag.
- **`canonicalize` returns a verbatim `\\?\` path on Windows**, verbatim paths
  cannot hold `..`, and `PathBuf::push` therefore RESOLVES hops at join time.
  A traversal built on a canonicalized base is unconstructible.
- **The `Edit` tool preserves CRLF; `sed -i` rewrites the file as LF.**

## 5. What this branch shipped

| Gate | Before | After |
|---|---|---|
| Backend lib (Windows, PowerShell) | 41 failed | **5** failed / 1325 passed |
| Integration (Windows) | **never ran** | **41 passed, 7/7 targets** |
| Frontend | 1 failed | **0** failed / 538 passed |

Four Windows product bugs found, three fixed:
1. **Tool Gate had no shell** — a GUI process has no `sh` on PATH, so every
   *approved* gated command silently failed to run. `1049a96d`.

   **CONFIRMED IN PRODUCTION**, not just by tests. Opening this branch's own PR
   was routed through `action_gate` for approval; the user approved, and the
   installed 1.0.0 build returned:

   ```
   failed to spawn `...gh.exe pr create...`: program not found
   [action_gate → exit -1 · 336 output bytes · shell sh]
   ```

   `shell sh` is `gate_shell()`'s bare fallback — the exact failure. Until this
   point F6 rested on a code reading plus 20 failing tests; this is the bug
   executing in the shipped build, on a real approved command. The fix is on
   this branch, so the running app does not have it.
2. **`HOME` is unset on Windows** — `default_user_settings_paths` returned an
   empty Vec, so every agent spawned with **zero user MCP servers forwarded**,
   silently. Telemetry's twin hashed panic text unredacted, so identical crashes
   never aggregated. `6a2c3496`, with a standing guard.
3. **`terminal_exec` submitted LF where consoles need CR** — a documented MCP
   tool dead on the platform. `94f73b86`. **Conditional** — see §3(b).
4. **PTY never marks dead** — §3(a). NOT FIXED.

## 6. T6 toasts — CLOSED, and it was never a bot-hq bug

The 1.0.0 report was a **disabled OS subsystem**: `ToastEnabled = 0`, the
Windows master notifications switch. No application could display a toast.

After the user enabled it (2026-08-25, this session), **the user confirmed
directly that notifications work**, and independently
`com.gregoryerrl.bot-hq` now appears under `HKCU\…\Notifications\Settings`.

- **AppUserModelID: REFUTED.** The identity works; that hypothesis is dead.
  Two independent legs, worth keeping distinct: the user's own confirmation is
  evidence a toast was **displayed**; the registry entry is evidence the app
  reached the notification platform with a **recognized AUMID**. The registry
  leg alone would rest on an inference about when Windows writes that key.
- **Permission explanations: eliminated from source.**
  `tauri-plugin-notification-2.3.3/src/desktop.rs` hardcodes
  `Ok(PermissionState::Granted)` for BOTH `request_permission` and
  `permission_state`, so `isPermissionGranted()` is a compile-time constant on
  desktop carrying no information about display.
- **The escalation path is CONFIRMED WORKING on Windows** (tested live,
  2026-08-25 11:02 UTC). `document.hasFocus()` reports a backgrounded WebView2
  window correctly, both focus gates behave, and the event wiring delivers.
  **T6 is closed; nothing here is left for macOS.**

  Measured objectively rather than by asking whether anyone saw a toast —
  Windows records each app's deliveries at
  `HKCU\…\Notifications\Settings\com.gregoryerrl.bot-hq`:

  | | `LastNotificationAddedTime` | `PeriodicNotificationCount` |
  |---|---|---|
  | baseline, seconds pre-park | 10:10:25 | 1 |
  | post-park | **11:02:29** | **3** |

  Count `1 → 3` because `useOsNotifications` subscribes to BOTH
  `session:pending_choice` (the park, 11:02:1x) and `session:awaiting_user` (the
  halt that followed, 11:02:27). The important part is not that two event types
  work — it is that the focus gate passed at **two different instants ~10s
  apart, in two separate flushes**. One delivery could be a timing fluke; two
  independent evaluations of `document.hasFocus()` are not.

  **What each measurement proves, kept separate:** the registry advance proves
  the APP-SIDE path end-to-end — the enabled pref, both focus gates,
  `planFlush`, and the send. It does NOT prove a banner visually appeared, since
  the user was deliberately away and was not asked to watch. That half was
  demonstrated separately at 10:10 by the test button, which the user confirmed
  directly. Two measurements, two halves — do not let a composite claim rest on
  either alone.

  **Method note, because the first attempt got this wrong.** An earlier run
  showed no delivery and was very nearly recorded as a product bug: pref,
  cooldown, DND and the OS layer had all been excluded, leaving
  `document.hasFocus()` as the only candidate. It was wrong — the user had not
  been reliably unfocused. The protocol had an instrument available (the
  registry) and asked a human anyway; a mistaken "yes, a toast appeared" was
  offered and then corrected by the user. Re-running with a **fresh baseline
  taken immediately before the park** and a read taken **before refocus** made
  the result independent of anyone's attention.
- **Real product gap, unfixed:** `sendNotification` returns `void` and wraps the
  synchronous Web Notification constructor, so the send can never report
  failure; combined with the permission constant, **bot-hq cannot tell a user
  their notifications are off**. `docs/MANUAL.md` was corrected on this branch;
  the code check was not written. On Windows the `ToastEnabled` registry read is
  the only signal that carries information — put it beside the user-facing claim
  (test button / escalation toggle), never in the fire-and-forget send path.

---

## Signature

**Session:** bot-hq `windows-compat`, 2026-08-25, Windows 11 Pro 26200.
**Participants:** HANDS (executed and measured everything) · EYES (reviewed,
declared expected deltas before each run, filed the blocking CL finding).

**Verified:** every number in §5 was measured on this machine from PowerShell,
against a declared expectation stated before the run. Commands to reproduce are
in §1 and the Verify session doc.

**NOT verified, and nobody should read this document as claiming otherwise:**
- The **Unix arms** — see §1. This is the load-bearing gap.
- `terminal_exec` end-to-end on Windows, because §3(b) blocks the headless test;
  the CR fix rests on direct observation of the live Terminal, not a test.
- The `hasFocus()` question in §6.
- EYES could not read `git stash show` (permission layer) and ran no test suite;
  **every test number here is HANDS' measurement.** EYES' contribution was
  declaring expectations in advance so they were falsifiable — which caught four
  bad numbers, including one where a green result was arithmetically impossible.

**The recurring failure this session, worth carrying:** *a number is a claim
about a measurement, not about reality.* Five times an improved or green number
meant less than it appeared to — a baseline taken in the wrong shell, a delta
extraction that read an empty failure list from a **failed build** as "all
cleared", a `--lib` run credited with integration coverage, a count that raced a
file write. The fix each time was a **fail-closed check**, not more care.
