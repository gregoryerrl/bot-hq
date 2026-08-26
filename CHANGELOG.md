# Changelog

Releases, newest first — [Keep a Changelog](https://keepachangelog.com/) shape.
Work between releases accumulates under **[Unreleased]**; a release moves that
block under its version heading. Development history before 1.0.0 lives in git
and in `docs/rebuild-archive/`.

## [Unreleased]

### Fixed

- **Windows: upgrading from any 1.0.0-rc / 1.0.0 install no longer exits at
  launch** with "migration 1 was previously applied but has been modified".
  Those builds came off a CRLF checkout and stamped CRLF migration checksums;
  1.0.1 embedded the same migrations with LF endings and refused every
  upgraded database within a second, before any window (fresh installs and
  macOS/Linux were never affected — no migration had changed). Migration
  checksums are now line-ending independent, and a database stamped by a
  CRLF build is repaired in place on first open (one INFO log line names the
  rewrite). One-way: once this build has opened a database, 1.0.0 and
  earlier Windows builds refuse it with the same message — do not roll back
  past this release after upgrading.
- **Windows: untouched HANDS/EYES no longer show "Differs from the shipped
  default"** on installs upgraded from a 1.0.0-rc / 1.0.0 build. Those builds
  stored the role prose with Windows line endings, so the Roles tab compared
  it against the shipped default and found every line changed — offering a
  diff of the whole prose and a Reset that rewrote all of it — when nothing
  but the line endings differed. A one-shot migration normalises the stored
  prose (the roles' edit timestamps do not move), and the comparison and the
  diff now ignore line endings.

### Added

- **A native error dialog when bot-hq fails before its window opens** — a
  data-dir problem, a failed migration, "bot-hq is already running" from a
  second launch, a port that will not bind. It carries the full error chain
  and names the log directory (saying so when logging had not come up yet);
  the same text goes to the log at ERROR level and, as before, to stderr
  with exit code 1. Skipped on a non-interactive Windows window station
  (service, OpenSSH session, non-interactive scheduled task — a message box
  there would block unseen) and when `BOT_HQ_NO_STARTUP_DIALOG=1` is set
  for headless or scripted launches.

## [1.0.1] — 2026-08-26

### Fixed

- **Windows: approved gated commands actually run.** The Tool Gate resolved no
  shell in a GUI process (`sh` is not on PATH), so every *approved* command
  failed with "program not found" — confirmed in the shipped 1.0.0 build. The
  gate now resolves its shell from Git-for-Windows.
- **Windows: agents spawn with the user's MCP servers.** `HOME` is unset on
  Windows, so user-level config paths resolved to nothing and agents silently
  got zero user MCP servers; panic telemetry also hashed paths unredacted. A
  portable home resolver (with a standing guard test) replaces every direct
  `HOME` read.
- **Windows: `terminal_exec` submits with a carriage return** (consoles ignore
  a bare LF). Known limit: if the Terminal tab was never opened, the ConPTY
  startup query has no responder and the tool can still stall — deferred with
  the PTY work below.
- **Windows: Context Library keys are forward-slashed everywhere**, with a
  one-shot in-place migration for databases written before the fix.
- **Linux: the AppImage opens a window on modern Mesa** via
  `scripts/install-appimage-linux.sh`, which strips the payload's stale
  Wayland client libraries (the underlying packaging defect — the AppImage
  still bundles them — is deferred; see below). The silent no-window failure
  is documented in `docs/FEDORA-LINUX-COMPAT.md`.
- **Linux: spawned processes no longer inherit the AppImage's environment.**
  The launcher's `LD_LIBRARY_PATH`/`PYTHONHOME`/etc reached every child, which
  broke host `git` over HTTPS, `curl`, `python3` and made `gsettings` return
  wrong answers inside sessions. A structural scrub (`src/appimage_env.rs`)
  drops payload-rooted entries at all four spawn sites; source builds are
  untouched.
- **Linux: native controls (dropdowns, scrollbars) render dark on dark
  desktops** — `:root { color-scheme: dark }`, measured against a forced
  light GTK theme with two independent probes.
- **`webview_screenshot` works off macOS.** It hardcoded
  `/usr/sbin/screencapture` on every platform (and its error text pointed at
  a macOS settings pane). Now platform-gated: macOS `screencapture`; Linux
  tries Spectacle → grim → ImageMagick → gnome-screenshot; Windows returns an
  explicit unsupported error.
- **Chat-stream file viewer opens `~/...` paths and Context Library files.**
  Tilde paths were treated as repo-relative (ENOENT with a misleading
  message), and the library was outside the viewer's allowed roots. Dotted
  entries under the library (`library/.git/**`) stay unreadable. Limit: a
  project whose CL lives at a custom `cl_path` outside `~/.bot-hq/library` is
  still outside the viewer's scope.
- **Windows notifications: the app can now tell you they're off.** The
  Settings test button reads the `ToastEnabled` master switch (the one signal
  that carries information — the plugin's permission API is a compile-time
  constant on desktop) and warns when Windows has toasts disabled OS-wide.

### Added

- **Starter safety defaults, offered once** (mirrors the roles offer): a
  basic Tool Gate keyword list (destructive commands only — `rm -r`, `sudo `,
  disk writers, `git reset --hard`, `git clean -f`) and a basic general
  policy (`push_gate: ask`, `force_push: blocked`, empty commit word-list
  with commented examples). Offered on fresh installs and on upgrades that
  never wrote the config file; an existing config suppresses its offer and is
  never overwritten (asserted byte-identical). Cards on Settings → Tool Gate
  and Policy; a dismissible Dashboard banner deep-links to them. Note: the
  escalation keyword also catches `sudo dnf` / `sudo apt` — edit the list to
  taste; and a project policy cannot relax `push_gate` back to `auto` (use
  the session's gear toggle).
- **New Context Library projects seed three structured starters** —
  `conventions.md`, `notes.md`, and (new) `decisions.md`, with section
  headings instead of one-line stubs.
- **Reopening a session says when no learnings delta was recorded** for its
  project, softly (an interrupted close and a deliberate write-nothing are
  indistinguishable on disk). In practice the note is rare on actively-worked
  projects: any CL write for the project since the session began counts.
- **Fail-loud webview startup watchdog**: if no page finishes loading within
  30s the app says so on stderr, in the log, and as a `webview_launch_failed`
  diagnostics event — the 1.0.0 Fedora failure produced no text at all.
  Startup only: a webview that loads and later dies is not covered.
- **Role prose view-diff and reset-to-default** against the shipped example
  pair (the 1.0.0 release-notes promise). Reset fills the editor; you still
  save.
- **CI test jobs** (`.github/workflows/test.yml`): `cargo test` + the
  frontend suite on ubuntu-22.04 and macOS as required jobs — no CI ran any
  tests through 1.0.0. The Windows job runs the full fail-closed suite but is
  advisory (`continue-on-error`) until the five known ConPTY failures are
  fixed or named: **the first Windows run of this workflow names the failing
  tests — annotate them by measured name, then flip the job to required**
  (tracked under Deferred below).

### Changed

- `PLAN.md` and `PROGRESS.md` are retired — 1.0.0 closed the build-out. This
  changelog carries what changes per release; `ARCHITECTURE.md`/`CODEBASE.md`
  say what bot-hq is and where things live; git history carries the rest.
- Telemetry/watchdog call-site guard tests scan only uncommented lines — a
  `//` comment-out now fails them. Narrower hole, not closed: a `/* */` block
  would still pass.

### Deferred to 1.0.2

- **Windows Terminal-tab PTY death** (an exited shell looks alive; ConPTY
  delivers no EOF) and the **DSR responder** for `terminal_exec` without a
  mounted Terminal tab — needs live Windows mileage.
- **Windows CI job → required**, once the first run on main names the five
  failing tests and they are annotated by measured name.
- **AppImage de-bundling** (stop shipping `libwayland-*` + `libepoxy`) and/or
  an **RPM** — the bundled-WebKit-vs-host-libstdc++ crash cannot be fixed
  from this repo and is the third independent argument for a distribution
  package.
- **Windows child-process reaping** via a Job Object (new unsafe FFI — lands
  only behind the Windows CI job).
- **macOS signing + notarization** — consciously deferred by the user at
  1.0.0 and again here (Gatekeeper banner-only by choice). The how is in
  `docs/SIGNING.md` (cert → repo secrets → uncomment the workflow's `APPLE_*`
  env); doing it removes the right-click-Open friction and unblocks a future
  auto-updater.

## [1.0.0] — 2026-08-25

First public release: the agent harness — sessions with user-defined roles,
IPAV phase discipline with per-phase documents, the two-layer policy
enforcement (MCP tools + git hooks), Tool Gate approvals, Context Library
with indexed retrieval, session terminal, plugin runtime, opt-in diagnostics,
and the packaged installers (macOS universal `.dmg`, Windows NSIS `.exe`,
Linux `.deb`/AppImage) with the Homebrew tap.
