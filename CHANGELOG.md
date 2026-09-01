# Changelog

Releases, newest first — [Keep a Changelog](https://keepachangelog.com/) shape.
Work between releases accumulates under **[Unreleased]**; a release moves that
block under its version heading. Development history before 1.0.0 lives in git
and in `docs/rebuild-archive/`.

## [Unreleased]

## [1.0.4] — 2026-09-01

The work-scope release: sessions kept losing the thread between days — each
one re-interpreting what the project was actually working on, with the
knowledge living in voice calls, chat scroll, and post-hoc reconstructions.
This release gives every project a durable, user-readable answer. It is also
the first tagged release since 1.0.2, so installed apps pick up 1.0.3's
review-layer work with it.

### Added

- **Per-project `focus.md` — the work-scope knowledge base.** A project CL
  may now carry a `focus.md` holding the work scopes currently in flight —
  one section per open scope (a project can run several at once): what is
  actually being worked on plus the absolute truths established about it,
  each with its provenance (what was measured, when, and as which identity),
  written to be read by the user directly, not as agent shorthand. When it
  exists, its whole body rides every participant's system prompt at
  spawn — reviewer included, since scope-watch is now a named review
  dimension; when it doesn't, the prompt carries a one-line creation trigger
  instead. The universal rules gained the full discipline: open a scope's
  section at the Plan boundary (defaulting to opening one when a scope's
  length is unknown), append truths as you learn — each entry naming its
  scope, corrections opening with line-start `SUPERSEDES:` markers —
  reorganize only at a Plan boundary, and clear PER SCOPE once that scope
  settles (graduate its residue to the project CL, remove its section,
  `confirm_shrink: true`; the file survives for the scopes still open) —
  never at close-out, where writers are context-poorest. Oversized bodies
  render head + tail around a loud truncation marker (the tail carries the
  newest truths), and a size or supersession-density advisory schedules the
  reorganize. Distilled from the 2026-08-31 ad-manager session dissection,
  where the scope's knowledge lived in a voice call and a post-hoc
  reconstruction.

## [1.0.3] — 2026-08-27

The review-layer release: a full-day dissection of why agent errors were
reaching the user found the turn ring silently starving the reviewer in 49%
of two-participant sessions (137 flagged gaps, worst 478 minutes, always the
reviewer's side), a review channel with no reverse direction, and advisory
findings dying undispositioned at close (92%). Everything here follows from
that diagnosis, and the fixes were field-verified live before release: the
same session shape that starved its reviewer for 97 minutes in the morning
dealt it within one second, all afternoon.

### Fixed

- **Tray and gate answers no longer reset the turn rotation to the front.**
  Every approval used to re-deal the executor, so an executor chaining gate
  parks starved the reviewer of turns entirely — both errors that shipped to
  GitHub from the measured session went out inside such a window. A typed
  message still resets (the user steering); an answer releases the ring and
  steps onward from the anchor, so the participant after the asker is served.
- **Anti-starvation backstop:** on every user-row deal, any active
  participant sitting on 10+ undelivered peer texts is served one pre-empting
  turn through the summons queue — covers the typed-message-chain shape the
  root fix cannot.
- **Phase-doc routing keys on reviewer shape** (`file_finding` without
  `edit_files`), so granting an executor `file_finding` — the reverse review
  channel — no longer reroutes its I/P/A/V docs into the reviewer co-doc slot.
- **Chat file links no longer point at paths quoted inside command text.** A
  path inside a quoted query, grep pattern, or pasted output became a
  clickable candidate the viewer then refused; bare-path extraction now
  qualifies whole shell words only.

### Added

- **Outward-publish review precondition:** a gated `gh`/`curl` command may
  park for approval only after the session's reviewer has been *delivered*
  its content (full-body match, file or inline; content-free mutations check
  the turn timeline instead). The refusal teaches the two-turn ritual; a
  solo roster skips the check loudly; a downed reviewer is escaped by the
  existing user-approved override; an unchanged body re-parks after a reject
  without re-review.
- **Starvation visibility:** the session roster shows an amber
  `N unread` chip when a participant crosses the summons threshold — computed
  backend-side from the scheduler's own constant, so the UI and the ring
  cannot disagree. 137 flagged gaps went undiagnosed because a starved
  reviewer was indistinguishable from a quiet one.
- **Open advisory findings surface at session close:** `close_session`
  lists them once before proceeding (never blocking), and the findings
  banner counts them while the session runs. Field-verified at first
  contact: all 7 of the measured session's advisories were dispositioned in
  the final minute instead of archiving silently.
- **"Set all" on Gated Commands** (global Settings and the session gear):
  one control flips every keyword row between Gate and Auto-allow.
- **HANDS role guidance for the reverse review channel** (suggested-pair
  prose): when the role holds `file_finding`, prefer advisory severity for
  reviewer-directed findings — a blocking finding the executor files gates
  its own commits.

## [1.0.2] — 2026-08-26

Hotfix for the Windows upgrade path. The items 1.0.1 deferred "to 1.0.2"
(PTY death, DSR responder, the Windows CI job going required, AppImage
de-bundling, Job Object reaping, macOS signing) are untouched here and carry
to the next release.

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
