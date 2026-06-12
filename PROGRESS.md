# bot-hq — Change Log

Recent work, newest-first. For the rebuild-era phased status (Phases
A–9 of the from-scratch rebuild), see
[`docs/rebuild-archive/PROGRESS-through-2026-05-15.md`](docs/rebuild-archive/PROGRESS-through-2026-05-15.md).

For what bot-hq IS see [`ARCHITECTURE.md`](ARCHITECTURE.md). For what's
planned next see [`PLAN.md`](PLAN.md).

---

## Current state

492 tests passing (441 lib + 33 external MCP + 7 signaling + 11 storage)
plus 87 frontend Vitest. Release build clean. Version **1.0.0** (bumped
2026-06-11; first stable). **Tauri v2 migration landed 2026-05-26** on
branch `tauri-v2-migration` (7 batches across foundation → Slint
removal). Slint UI deleted (-7,560 LOC); React frontend in `frontend/`;
zero LOC delta in `src/agents/`, `src/core/`, `src/policy/`,
`src/storage/`, `src/signaling/` per the design-doc constraint.

---

## 2026-06-12 — Full sweep: 9 fix/cleanup commits from a duo audit

Brian + Rain swept CL + codebase + docs post-1.0.0 (4 parallel review
agents, findings adversarially re-verified — 3 agent claims dismissed as
false positives). Landed `7fef038..9916514`:

- **Register-Project "doesn't work" root-caused and fixed** (`9916514`,
  closes the issues.md 2026-06-11 item). DB forensics showed the 06-11
  registration SUCCEEDED — the new project was just invisible: tree roots
  required indexed entries matching the active filter/search. Roots are
  now indexed ∪ registered (`treeProjectIds`, 5 tests), and a successful
  register clears the search + pins the tree to the new project.
- **Session project now resolves via registered-repo lookup** (`9210194`).
  Was pure basename(working_repo_path); a registered project whose repo
  dir is named differently silently got general policy + no CL context.
  Canonicalized exactly-one-match lookup (base repo first for worktree
  sessions), basename fallback, 9 tests. Also explains the 06-11
  "full forbidden list" surprise: that session ran on an UNregistered
  repo (`~/Projects/test`) — designed inheritance, now at least
  inferrable from logs. Provenance badge in the gear tab = follow-up.
- **register-from-global migration correctness** (`054d29c`): folder
  descriptions re-home from a fresh `_globals` fetch (was: view-filtered
  state — active search silently skipped descendants); partial failures
  surface in the action error.
- **Rescan failures visible** (`2380006`): single-project rescan failure
  was entirely uncaught; all-projects failures hid in console.warn. Both
  now feed a "✗N failed" chip beside the report.
- **Dialog parity** (`e4a906e`): Escape for ActionModal / ModelDialog /
  RegisterProjectModal, focus trap for SessionPolicyPanel (+1 vitest).
- **Enforcement-path observability** (`f9641c4`): check_commit_message's
  policy-audit + violation-log failures now `warn!` instead of `let _ =`.
- Docs/comments (`7fef038`): config/-split note un-staled, tool count
  25→26, cl_write_file comment. Tokens (`ca5eaf8`): blue-400→tertiary,
  neutral-600→outline-variant, red-400→error (no success/warn tokens
  exist yet — follow-up). Hygiene (`1b5437f`): root `/node_modules/`
  ignored + stray vite cache removed.

Deferred by decision: module splits (May-21 precedent), provenance
badge, semantic success/warn tokens, CL-content edits (user-gated).

## 2026-06-12 — Context Library tree overhaul + Models list redesign

Four UI improvements from user spec (categories scheme picked by user:
Projects / Global / System).

- **add: CL sidebar header icon actions** (`8de5538`). Rescan / Register
  project / Maintain CL moved from full-width block buttons into icon-only
  header buttons (RefreshIcon w/ spin-while-rescanning, PlusIcon, WrenchIcon
  in primary). No count on rescan-all per user. Search + project filter +
  rescan report stay below.
- **add: resizable context library sidebar** (`5d8c9e2`). VS-Code-style
  drag-resize, ported from SessionView's split-handle pattern in absolute px:
  clamp [180, 480], default 240, persisted to
  `localStorage["bot-hq.cl.sidebarWidth"]`.
- **add: categorized CL tree (system guard, register-from-global)**
  (`1d2f546`). Tree now groups under three collapsible category headers
  (sentinel collapse keys `@cat:*` in the existing persisted set; left-click
  only toggles — never opens a tab): **Projects** (registered, `text-primary`),
  **Global** (loose `_globals` files, header right-click → New file/folder),
  **System** (`agents/**` + `custom-general-rules.md`, `text-amber-400`,
  read+update only — no context menu, and `cl_rename`/`cl_delete_path` now
  reject protected `_globals` paths server-side via
  `assert_not_protected_globals_path`, canonicalized-path compare). Top-level
  Global folders gain right-click **Register as project**: physically moves
  the folder under `projects/` (in-place registration would double-index),
  upserts the project row, re-points folder-description rows, rescans both
  sides. `splitGlobals`/`isInternalGlobalsPath` helpers + 4 Vitest cases +
  1 cargo guard test.
- **redesign: models settings as list with edit dialog** (`dc3e70c`).
  Settings → Models card grid replaced by a 5-column list (name / provider /
  model id / updated / actions). Create + edit go through a ModelDialog
  (RegisterProjectModal scaffold); the model id is generated at save time so
  cancelling Add leaves no ghost "New model" row (the old grid pre-created
  one). Delete keeps the ConfirmDialog flow.

---

## 2026-06-11 — v1.0.0 stabilization: worktrees, dispatch defaults, prompt drafts, UX polish (shipping)

The four-area stabilization pass for the first stable release.

- **fix: dispatched sessions honor the solo/duo default** (`c215392`). The
  Maintain-CL button (`dispatch_session`) and the external driver
  (`open_session`) never called `set_session_spawn_config`, so the DB default
  (`rain_enabled=1`) always spawned the duo — `rain_disabled_default` was
  ignored. Both now resolve `Storage::default_rain_enabled` before spawn;
  models stay NULL (= agent defaults). Modal copy de-hardcodes "Brian + Rain".
- **feat: per-session prompt drafts** (`d48a02b`). ChatInput gained a
  `draftKey` prop — drafts persist to `localStorage["bothq:draft:<sid>"]`
  through navigation/restart, clear on successful send, survive failed sends.
  SessionView keys the input per session. 6 new Vitest cases.
- **fix: blank repo paths store as NULL** (`41b13d7`). A session created with
  `''` read as repo-backed everywhere `working_repo_path` is consumed
  (action_gate hard-error before its approve prompt). `create_session`
  normalizes; migration 0019 repairs pre-guard rows.
- **add: per-session git worktrees** (`b5d1d7d`). Repo-backed sessions default
  to an isolated worktree at `<data_dir>/.local/worktrees/<sid>/<repo-basename>`
  on branch `bothq/<sid>` — parallel sessions per project. `working_repo_path`
  stores the worktree (all consumers unchanged); new `base_repo_path` column
  (migration 0020) remembers the source repo. Idempotent ensure at spawn with
  direct-mode fallback (row converted so row-readers agree); clean-only removal
  at close (never `--force`); `install_hooks` resolves the hooks dir via
  `git rev-parse --git-path hooks` — a linked worktree's `.git` is a FILE and
  hooks live in the shared common dir (previously worktree repos silently
  skipped hook install). Opt-out per session or via `worktree_default`
  (Settings → Agents → Session defaults). ARCHITECTURE.md "Session worktrees"
  section added.
- **ux: 1.0 polish** (`59d39b7`). Activity-ordered dashboard tiles (newest
  message, created_at fallback), ⌘/Ctrl-N → New-session dialog + ⌘/Ctrl-, →
  Settings, welcoming empty-state copy, inline session rename
  (`rename_session` command).
- **release: version 1.0.0** across Cargo.toml / tauri.conf.json /
  frontend/package.json. Violations-log viewer deliberately deferred to v1.1
  (user scope pick).

---

## 2026-06-10 — fix the release-build CWD litter found by the smoke (shipping)

Same-day fix for the smoke's incidental find (`957d6a9`). The startup
tauri-specta export is now guarded on `frontend/src/lib` existing in the
CWD — the export creates intermediate dirs, so unguarded it littered a
`frontend/` tree into any writable launch directory. Repo-root launches
keep the documented auto-regen (verified: mtime advances, content
byte-identical at HEAD, tree stays git-clean); foreign-CWD launches skip
with a debug log (verified: temp CWD stays empty). `specta_builder`
construction stays unguarded — it also feeds `invoke_handler` (first
attempt scoped it into the guard; only the compiler caught the second
use). All five gates green (465 Rust + 71 Vitest).

## 2026-06-10 — first-run + migration smoke: PASS (shipping); MIT license

Closed the shipping.md "first-run + migration smoke" item — the GUI-startup
wiring over a legacy data dir, which the `paths.rs` unit tests don't reach.
Also shipped the MIT `LICENSE` (root + `frontend/package.json` field,
`02bba46`) and corrected INSTALL.md's Gatekeeper steps for macOS Sequoia
(right-click → Open no longer bypasses for unsigned apps; `9a2d2a0`).

- **Method.** Release binary launched 3× over TEMP `BOT_HQ_DATA_DIR` dirs
  (never the live `~/.bot-hq/`), neutral CWD, stderr captured, SIGTERM after
  ~12s. Fixtures carried per-file sentinel content with md5 manifests.
- **v0 → v2 full migration: PASS.** 13-entry root-layout fixture (all three
  stages). One "migrated legacy layout" warn; outcome `Repaired`; all 12
  content entries relocated to `library/`/`.local/`/`config/` byte-identical;
  `cl-version.txt` removed; `version.txt` stamped "2".
- **Idempotency: PASS.** Relaunch over the migrated dir: zero migration
  lines, outcome `Existing`, marker + content untouched.
- **Crash-window self-heal: PASS.** Simulated interrupted migration
  (dest-exists + conflicting root copy + unmoved entry): existing dest not
  clobbered, conflicting root copy left as residue, unmoved entry migrated.
- **Second-instance safety confirmed.** Ran beside the live prod bot-hq:
  internal signaling binds an ephemeral port; the external MCP's fixed port
  collision warned + skipped exactly as coded — app fully functional.
- **Incidental find (follow-up candidate).** The startup tauri-specta
  bindings export writes `<cwd>/frontend/src/lib/bindings.ts` from ANY
  writable CWD in release builds (it creates intermediate dirs) — launching
  the app from a terminal in `~` litters `~/frontend/`. Polish: gate the
  export to dev builds or an existing path.
- Also live-verified the new unix shutdown handler (`d039ffa`) 3× — SIGTERM
  → child reap → clean exit 0.

## 2026-06-10 — Windows compile fix: cfg-gate reaper + shutdown handler, restore CI lane (shipping)

Un-deferred Windows from the release matrix by fixing the compile blocker
found in the 2026-06-09 CI dry-run.

- **Per-platform child kill** (`d039ffa`). `spawn.rs::reap_all_children` kept
  its `try_lock` + iterate shape; the kill itself moved into a per-platform
  `kill_child` — unix keeps the verbatim `libc::kill`/SIGKILL body, Windows
  does `OpenProcess(PROCESS_TERMINATE)` → `TerminateProcess` → `CloseHandle`
  via `windows-sys` 0.59 (version already in the lock transitively; new
  `[target.'cfg(windows)'.dependencies]` entry, `libc` moved to the unix
  twin). Windows has NO kill-children-on-parent-exit semantics (no process
  tree; would need Job Objects), so an empty stub would have reintroduced
  Ghost-Brian there — the reap walk is equally load-bearing on both
  platforms. Job-Object hardening (covers hard parent kills, parity with
  un-catchable SIGKILL) noted as a follow-up candidate, not blocking.
- **Windows shutdown task** (`d039ffa`). The unix signal task is unchanged
  under stmt-level `#[cfg(unix)]`; a `#[cfg(windows)]` twin selects over
  `tokio::signal::windows::{ctrl_c, ctrl_close, ctrl_shutdown}` (≈ SIGINT /
  SIGHUP / SIGTERM) with the same reap + exit tail. Panic hook untouched.
- **CI lane restored** (`bcca313`). `release.yml` windows-latest matrix entry
  back, exactly the previously-validated shape (`args: ''` — an explicit
  `--target` would move bundle output out from under the upload globs, which
  already cover `target/release/bundle/nsis/*.exe`).
- **Verification.** All five gates green post-change (465 Rust + 71 Vitest,
  tsc clean, release + frontend builds clean; gates 4–5 run by Rain pre-halt,
  tree unchanged since). Local `cargo check --target x86_64-pc-windows-msvc`
  dies environmentally in `ring`'s C build script (no Windows SDK headers on
  macOS) before reaching our crate — the definitive Windows check is the CI
  windows-latest lane (workflow_dispatch validation run on push).
- **Still staged for later** (shipping.md): Windows runtime gaps once it
  ships — PID lock, bash hooks, `mcp-token` ACL.

## 2026-06-10 — separate personal rules from shipped standard rules (shipping)

User classified the rule inventory: commit hygiene + the forbidden-in-commits
brand list are PERSONAL conventions; everything else is standard product.

- `src/agents/general_rules.rs` — `## Commit hygiene` removed from
  `GENERAL_RULES`; replaced with a neutral `## Commit conventions` pointer
  (style + forbidden words come from policy files / custom-general-rules.md).
  Fresh installs ship NO house commit style. `Policy::default()` already has
  an empty forbidden list, so an all-default policy renders no Enforcement
  block (`is_effectively_empty` short-circuit, verified).
- `src/core/session.rs` — prompt-assembly doc comment + 4 test anchors moved
  from the deleted section to "Working directory".
- `templates/cl/custom-general-rules.md` — example line no longer references
  "baked-in hygiene".
- User-side migration (not in repo): personal hygiene block appended to
  `~/.bot-hq/library/custom-general-rules.md`; redundant outward-actions copy
  trimmed (that rule is hardcoded in the binary). Leftover `agents/emma/` CL
  dir flagged for manual deletion — blocked by a dogfooding find: `action_gate`
  errors "session has no working_repo_path" on a session that has one.

## 2026-06-09 — macOS + Linux distribution: bundle config, release CI, v0.1.0 draft, Homebrew cask (shipping)

Set up end-to-end distribution for macOS + Linux (Windows deferred) and cut the
first release. The app had no `bundle` config, no real icons, and no CI; now a
tagged `v*` push builds + drafts a GitHub release with platform artifacts.

- **Bundle config + icons** (`69ff1d1`). Added the `bundle` section to
  `tauri.conf.json`: `targets [app,dmg,deb,appimage,nsis]` (per-OS subset
  auto-selected — mac app+dmg, linux deb+appimage, win nsis), `DeveloperTool`
  category, per-OS metadata, macOS min 10.15. Generated the full icon set
  (icns/ico/PNGs) from a temporary brand-matched `>_` terminal mark (orange
  chevron = HANDS, purple cursor = EYES) kept in `icons/src/` for regeneration.
  Fixed a latent bug: `beforeBuildCommand`/`beforeDevCommand` were `cd frontend
  && …`, but Tauri v2 runs them from the frontend dir (inferred from
  `frontendDist`), so the `cd` double-cd'd — `tauri build` never worked before
  (the app was always built via separate `npm run build` + `cargo build`). Now
  `npm run build` / `npm run dev`. Verified: `bot-hq.app` bundles with the icon +
  correct Info.plist (category developer-tools, id, version).
- **Release CI** (`525ba2b`). `.github/workflows/release.yml`: a `tauri build`
  matrix (macOS universal `.dmg`, ubuntu-22.04 AppImage + `.deb`, Windows NSIS)
  on a `v*` tag (draft-release upload) or manual dispatch (validation artifacts).
  Manual-build approach, not tauri-action (whose layout auto-detection fights the
  flat layout): invokes the frontend's tauri CLI from the repo root. Unsigned;
  APPLE_* signing env stubbed. A `workflow_dispatch` dry-run validated all three
  platforms — macOS + Linux green (the runner builds the `.dmg`; the local `.dmg`
  failure was just headless GUI, `bundle_dmg.sh` needing Finder), Windows caught a
  real blocker (below).
- **Windows deferred** (`d1e94e0`). bot-hq doesn't compile on Windows: the
  Ghost-Brian reaper (`spawn.rs` `libc::kill`/`SIGKILL`) and the shutdown signal
  handler (`main.rs` `tokio::signal::unix`) aren't `#[cfg(unix)]`-gated. Commented
  the windows-latest matrix entry (restore TODO) so `v*` tags yield clean mac +
  Linux releases instead of a failed run. Follow-up: cfg-gate both + add Windows
  equivalents (windows-sys TerminateProcess, tokio ctrl_c).
- **v0.1.0 draft release.** Tagged + pushed `v0.1.0`; CI produced a draft release
  with `bot-hq_0.1.0_universal.dmg` (15.8 MB), `bot-hq_0.1.0_amd64.AppImage`
  (86 MB), `bot-hq_0.1.0_amd64.deb` (9 MB). First real exercise of the
  draft-upload path — one clean release, no matrix race.
- **Homebrew cask + docs** (`4c00c23`, `58b9bd5`). `packaging/homebrew/bot-hq.rb`
  (real sha256, livecheck, claude-code + Gatekeeper caveats, deliberately no zap
  of `~/.bot-hq`), `INSTALL.md` (per-platform), `docs/SIGNING.md` (notarization
  upgrade path + Windows / auto-update follow-ons). Ships via an own tap
  (`gregoryerrl/homebrew-bot-hq`).

Remaining (manual/user): publish the draft `v0.1.0`; create the tap repo + add the
cask. Deferred: the Windows compile fix, the real app icon (swap `icons/src/` +
re-run `tauri icon`), and macOS notarization when an Apple cert exists.

---

## 2026-06-09 — check-for-updates: GitHub-release update banner (shipping)

Added the first user-facing "you can update" path (shipping/market-prep track).
On launch bot-hq polls the GitHub releases API, semver-compares the latest tag to
the running version, and shows a dismissible download banner when a newer build
exists — plus a Settings → Updates subtab (installed-vs-latest + manual "Check
now"). This is the **check-and-notify** scope (A): no code-signing / updater
plugin, so the install is manual; the command + banner shell graduate cleanly to
full auto-install later. Decided scope with the user up front — real auto-install
is blocked on code-signing (a separate roadmap item), check-and-notify is not.

- **`core::updates`** — the testable core, split from the network glue:
  `is_newer` (semver compare, strips leading `v`, false on garbage),
  `release_from_response` (**404 → no release, NOT an error** — the current
  zero-releases state), `build_update_info` (`None` → not-available). Thin async
  `fetch_latest_release` / `check_for_update` set the GitHub-required
  `User-Agent`. 13 unit tests, all network-free.
- **`check_for_update` command** (`tauri_cmd/updates.rs`) — returns `UpdateInfo`,
  compares against `app.package_info().version` (the shipped version, not a
  constant). Registered in `collect_commands!`; bindings regenerated.
- **`tauri-plugin-opener`** (+ `opener:default` capability) opens the release page
  in the system browser — `window.open` isn't reliable cross-platform per the
  Tauri v2 docs.
- **Frontend** — app-wide `UpdateBanner` (Shell) with per-version localStorage
  dismissal; Settings Updates subtab. Both share one `check_for_update` query.
- Fails quiet: offline / rate-limit / no-release never nags. The banner shows
  nothing live until a release > installed is cut (`gh release list` is empty
  today); Settings "Check now" proves the round-trip now. Live endpoint verified
  returning 404 for the zero-releases state, handled gracefully.
- All 5 gates green; commit `4c054f8`.

---

## 2026-06-09 — v1.1 config/ split: host machine config moved under config/

Followed the `library/` carve-out (`cf72e72`) with the deferred v1.1 step from
the shipping roadmap: the three host-side machine-config files —
`general-policy.yaml`, `tool-gate.json`, `claude-overrides.json` — moved from the
data-dir root into `<data_dir>/config/`. The root now holds only `version.txt`
plus the four subtrees (`library/`, `config/`, `plugins/`, `.local/`).

- **`config_dir` is part of `Paths`.** New `config_dir` field + a free
  `paths::config_dir_path(data_dir)` helper (mirrors `read_signaling_addr` — the
  policy / claude-config path builders receive a bare `data_dir`, not a `Paths`,
  e.g. the CLI hook subprocess). `policy::general_policy_path`,
  `tool_gate::config_path`, and `overrides::config_path` route through it; the
  policy audit reuses `general_policy_path` so the resolver and the
  mutation-audit can't desync on the new location.
- **Schema bumped 1 → 2 with a REQUIRED migration.** Not a pure rename: an
  existing v1 install carries these files at the root, so changing the paths
  without moving them would make the loaders read an empty `config/` and
  silently fall back to defaults — dropping the user's configured policy /
  tool-gate / overrides. `migrate_legacy_layout` is now gated on
  `schema_version() < SCHEMA_VERSION` (was `version.txt` absence) and stages
  cumulatively: v0→v1 (root CL → `library/`, host state → `.local/`) then v1→v2
  (root config → `config/`), each exists-guarded + idempotent; `init` stamps the
  marker afterward.
- **Docs re-pointed:** `paths.rs` header layout, ARCHITECTURE.md storage map +
  policy hierarchy (also fixed a stale `projects/` → `library/projects/` left
  over from the carve-out), README policy/tool-gate paths + de-hardcoded a stale
  "288 tests" line, and the in-code doc comments across `policy/hooks`,
  `agents/spawn`, `tauri_cmd/*`, and the signaling bridge.
- New test `migrates_v1_config_files_into_config_dir`; the existing migration
  suite still passes under the schema-version gate. The deferred README
  install-docs item rode along in the same batch (prerequisites were already
  present from `cf72e72`).

---

## 2026-06-09 — Context Library carved into its own `library/` folder (market-prep layout)

Reshaped the `~/.bot-hq/` data home so the Context Library lives in its own
`library/` subtree — separable for backup / a future cloud-sync-CL plugin, and
so host-only state stops intermixing with user content at the data-dir root.
Decided the full install topology first (the binary stays platform-bundled —
`/Applications/bot-hq.app` etc., NOT under `~/.bot-hq/`); shipped the v1
`library/` carve-out and deferred a `config/` split to a cheap v1.1. Target
platforms now macOS + Linux + Windows; the layout is base-agnostic so no
platform branches were needed.

- **`Paths` is the single source of truth.** New `cl_dir` (`<dd>/library`),
  `plugins_dir`, `version_path` (`version.txt`, renamed from `cl-version.txt`),
  and `.local/`-rooted `mcp_token_path` / `violations_path` /
  `policy_hashes_path` / `screenshots_dir`. A `project_dir(name)` helper is the
  one per-project convention path, shared by the storage resolver, policy
  resolver, and policy audit so the `library/` location can't desync them.
- **One-time migration** (`Paths::migrate_legacy_layout`, gated on `version.txt`
  absence): moves root CL → `library/`, host-only state → `.local/`, renames the
  marker. Idempotent; explicit-`cl_path` projects untouched. Uses a
  rename-with-copy-fallback (`move_path`) robust to cross-filesystem EXDEV /
  locked files.
- **Resolvers + policy re-pointed** through `cl_dir` / `project_dir`:
  `cl_path_for_project`, `cl_project_root`, `walk_cl_dir`, `cl_startup_init`,
  `read_system_prompt` (also fixed a pre-existing missing-slash bug in the CL
  anchor), `resolve_at_root`, `audit_policy_files`. Host-only files
  (violations.jsonl, .policy-hashes.json, screenshots/, mcp-token) moved under
  `.local/`.
- **Cleanup + docs:** removed the dead Emma template; refreshed ARCHITECTURE.md,
  README.md, .env.example. Added migration tests (×2) + a resolver/audit parity
  test. All 5 gates green; `config/` split + a `bin/` symlink are deferred
  follow-ups.

---

## 2026-06-09 — under-the-hood health sweep: dashboard halt bug, spawn invariant, enforcement tests, cleanup

A full-codebase audit (5 read-only sub-agents + Rain's adversarial sweep) on an
otherwise-healthy codebase surfaced one real silent bug, one latent invariant
gap, under-tested enforcement seams, and a staleness tail. Remediated as 8
self-contained commits, all 5 gates green per commit.

- **fix: dashboard tiles flag halt waits via the durable tray** (`ce4d49b`). Tiles
  counted pending input from the in-memory `list_pending_choices`, which
  `mark_awaiting_user` / `request_phase_advance` never populate — so a halted
  session showed no badge and counts reset on restart, while the header bell (on
  the durable source) disagreed. Point the tile at `list_pending_tray`;
  `SessionTile` is indicate-only so its prop is now a plain `pendingCount`.
- **fix: exclude max-effort and ultracode at the spawn merge** (`e155897`). A
  persistent `effort=max` + a session `ultracode=1` (or the reverse) emitted both,
  which claude-code treats as mutually exclusive. Reconcile at the overlay
  honoring session-wins; tests for both collision directions.
- **test: push-gate approve/reject classification** (`315e05a`). Extract a pure
  `classify_push_response` seam from `decide_push` and lock the fail-closed
  property (reject / missing / malformed / non-2xx never resolve to Approved).
- **test: external MCP soft-fail + same-length token reject** (`63724f9`). Auth was
  already integration-tested; filled the two genuine gaps — port-in-use soft-fail,
  and a same-length wrong token (exercises ct_eq's content path, not just length).
- **refactor: rename `tauri_cmd/questions.rs` → `tray.rs`** (`faec167`), decouple
  ChoicePrompt from the dropped `PendingChoiceView` (`37a0036`), regen bindings
  (`a88b58b`). Drops the orphaned `list_pending_choices` Tauri command; the bridge
  method stays for the external driver.
- **chore: prune stale Emma/PluginSlot refs, the dead author.emma tailwind token,
  and a broken rustdoc link** (`d128185`).

Tests now 448 (397 lib + 33 external MCP + 7 signaling + 11 storage) + 63 frontend
Vitest. Landed on branch `brian/health-sweep-2026-06-09`.

---

## 2026-06-08 — June 8 QoL batch: metadata toggle, resizable split, collapsible diff, doc TL;DR

Four independent frontend QoL features from `ideas.md` (June 8 list), built
easiest→hardest, one commit each. All five gates green per commit.

- **feat: CL metadata editor behind a toggle** (`5d24f4c`). The CL file editor's
  description/tags panel is collapsed by default behind an "Edit metadata" header
  button (amber dot when collapsed with unsaved metadata); it stays mounted
  (CSS-hidden) so an in-progress edit survives a toggle.
- **feat: resizable chat/document split** (`1853c81`). SessionView's fixed
  `grid-cols-[3fr_2fr]` became a flex layout with a drag handle; the ratio clamps
  to [25,75]% and persists to `localStorage["bothq:split:leftPct"]`.
- **feat: collapse the Apply-tab git diff per file** (`9cab075`). A new pure
  `lib/diffGroups.ts` (`groupDiffByFile`, unit-tested) splits the classified diff
  on `diff --git` headers; each file renders as a native `<details open>` with a
  `+adds −removes` summary. No backend change — reuses `compute_apply_diff`.
- **feat: TL;DR summarize button on session docs** (`aa54329`, bindings `827f254`).
  New `summarize_session_doc` command resolves a model (`default_model_id` → session
  Brian model → agent config via the now-`pub(crate)` `resolve_spawn_config`) and runs
  a one-shot headless `claude -p … --max-turns 1 --strict-mcp-config` (60s timeout,
  kill-on-drop), rendered in a dispose-on-close dialog. The live model path is
  static-verified only (compiles, wired, binding present) — not runtime-exercised.

---

## 2026-06-06 — web_search, prompt fixes, UI outline pass + health-audit sweep

Backfills the 2026-06-06 feature work (shipped but previously unlogged) plus a
CL + codebase health sweep.

- **feat(web_search): model-agnostic web search via a headless webview.** A new
  `web_search(query, engine?)` internal MCP tool navigates a hidden webview and
  reads the rendered DOM from Rust (`eval_with_callback`), cascading
  Google→Startpage→Bing with a title-filter + nav-junk drop. Lets agents on
  gateways without a server-side search tool fetch live results. Rain's `--bare`
  was dropped (`fa57a92`) so her client-side tool loader works again (the
  llm_proxy already handles the system-message hoist `--bare` was guarding).
- **fix(prompts): interpolate the project name + sharpen investigate guidance**
  (`1bf3faa`). The `<your project>` placeholder in the CL anchor + role prompts
  was never substituted; now resolved to the session's project (or `_globals`
  when repo-less). Added a "tight turns while coordinating" nudge.
- **feat(ui): outline icons + confirm dialog** (`2fa33f1`). Replaced text/glyph
  icons with a hand-rolled outline SVG set (`components/icons.tsx`); a reusable
  `ConfirmDialog` replaced all `window.confirm` sites; the session-view close
  button is now a force-close danger dialog. Dropped the redundant role chip.
- **fix: bell self-heal on out-of-band resolve** (`e43e5f3`) — the OOB resolve
  paths now emit `ChoiceResolved` so the notifier badge clears.

Health-audit sweep (CL + codebase):
- **refactor: rename `bridge/questions.rs` → `tray.rs`** (`8035b38`) to match the
  `session_tray` table (renamed in migration 0010).
- **refactor(ui): share the phase→bucket mapping** (`9c48390`) between PhasePill
  and SessionPhaseChip via `lib/phase.ts` (`phaseBucket`); +3 frontend tests.
- **docs: correct drifted counts** — internal MCP tools 24→25 (web_search),
  external driver tools 21→19, test counts 410→425 (377 lib + 31 ext + 7 sig +
  10 storage + 56 frontend). Documented the `bench/swebench/` eval harness in
  ARCHITECTURE.md.

---

## 2026-06-05 — Rain read-only gh, bell self-heal, Emma removed

- **feat(policy): give Rain read-only `gh` access (write-verbs denied).** Rain
  (EYES) could not touch `gh` at all. Loosened to read-only: write verbs (e.g.
  `pr create`/`merge`, `issue close`, `release`) stay denied, and `gh api` stays
  fully blocked (it can mutate behind an innocuous-looking call). Rain can now read
  PR/issue/CI state for review without gaining any write path.
- **fix(ui): boot-time sweep withdraws stale tray rows on dead sessions.** The
  notification bell counted pending tray rows from sessions that had since closed or
  orphaned, so the badge showed phantom items that never cleared. Added a startup
  sweep that withdraws pending tray rows whose session is closed/missing — the count
  now self-heals stale cruft on launch instead of accumulating it.
- **chore: remove Emma from the core entirely (migration 0017).** The solo helper
  agent Emma is gone — prompt, auto-spawn, overlay, signaling, and her seeded data
  are purged (migration 0017 drops the `emma` row; the legacy CHECK constraints stay
  permissive). The duo (Brian = HANDS, Rain = EYES) is the whole agent model now.
  Emma is slated to return as the first bot-hq plugin — TBD. Canonical docs
  (ARCHITECTURE.md, CLAUDE.md) updated to match.

---

## 2026-06-04 — audit remediation (continuous pass)

Working through the full-codebase audit (findings in the session's investigate doc),
priority order, one commit per cohesive batch. Newest bullet first.

- **perf(ui): lag-resync recovery, then drop the redundant polls (E2).** The
  PendingTray 2s, Dashboard per-tile phase 5s + pending-choices 5s, and Emma 3s
  `refetchInterval` polls were the only recovery if the bridge subscriber dropped a
  `session:*` event on `Lagged` (it just logged). Added that recovery properly: the
  subscriber now emits a `session:resync` on `Lagged`, and `GlobalEventSync`
  invalidates every event-backed query on it — so a dropped burst self-heals. With
  that net in place the four event-backed polls are redundant and dropped (the
  60s/10s polls on projects/models/plugins stay — no event source). Net: no constant
  background refetch churn, and lag is now self-healing instead of silently stale.
- **perf(ui): lazy-mount Settings subtabs, keep once visited (E8).** Settings
  rendered all 6 panels up front (CSS-`hidden`), firing every panel's queries on
  first visit. Now a panel mounts only once its tab is visited and then stays
  mounted — so the default "agents" tab's queries fire on open, the rest only when
  the user actually clicks that tab. Keeps the intentional edit-survival across
  subtab switches (panels stay mounted once shown).
- **fix(storage): cl_index/cl_folders upsert returns the real id (F1).** Both
  `upsert_cl_index` and `upsert_folder_description` returned `last_insert_rowid()`,
  which on an upsert that takes the DO UPDATE branch can report the bumped (unused)
  AUTOINCREMENT value instead of the real row id — the exact footgun already fixed
  in `session_docs.rs`. Switched both to `RETURNING id`. Latent (no caller trusts
  the returned id today), so zero behavior change — purely removes the landmine.
- **fix(ui): Enter sends in the chat; Shift+Enter for a newline (D8).** The chat
  textarea required ⌘/Ctrl+Enter — bare Enter just inserted a newline, which read
  as "Enter doesn't work". Now bare Enter sends (⌘/Ctrl+Enter still works as an
  alternate), Shift+Enter inserts a newline, and IME composition is respected so
  multibyte input isn't cut off. Hint updated to `↵`. (Emma's single-line input
  already sent on Enter.)
- **refactor: parse ViolationKind via serde, not a hand match (C3).** The
  request_approval `kind` parser (`jsonrpc::parse_violation_kind`) duplicated the
  enum's snake_case wire names in a hand-written match that had to be kept in
  lockstep with `ViolationKind`. Parse through serde so it can't drift. (Only delta:
  it now also accepts `policy_mutation` — benign, since command execution gates on
  `ToolBlocklist` specifically, not on the kind being parseable.)
- **a11y(ui): focus-trap the dialog modals (D7).** New `useFocusTrap` hook: focuses
  the first focusable on open, traps Tab/Shift+Tab inside the dialog, and restores
  focus to the trigger on close. Applied to the four dialog modals (ActionModal,
  New-session, MaintainCL, Register) — keyboard/screen-reader users could
  previously Tab out into the obscured page behind the scrim. (The Emma/Policy
  slide-over drawers follow the same pattern — left for a focused follow-up.)
- **fix(ui): don't offer file/folder creation under the `_globals` root (D12 rem.).**
  The CL right-click menu offered "New file/New folder" on the `_globals` virtual
  bucket (cross-project system files), which would create files at the CL system
  root. Guarded. (Left as intentional/low-value: D11 bell counts sessions — by
  design, the dropdown already shows per-session item counts; #31 ok/yes→Approved —
  reasonable approval semantics.)
- **perf(cl): de-quadratic cl_rescan + parallelize all-projects rescan (E5).**
  Backend: `cl_rescan` did an `existing.iter().find()` per on-disk file (O(disk ×
  index)); now builds a `HashMap<path,&row>` once for O(1) lookup. Frontend: the
  "rescan all" button ran each project's rescan serially (`for…await`); now
  `Promise.all` with per-project error isolation. (Left: wrapping the per-row
  upsert/touch/delete writes in one transaction — cl_rescan is on-demand with a
  modest row count, so the sequential awaits aren't worth threading a tx through
  three storage methods.)
- **fix(ui): share the clock-time formatter (C5, partial + zone bugfix).** Emma's
  overlay had its own `formatClockTime` that used `new Date(iso)` directly — NOT
  zone-safe, so a zone-less timestamp misparsed (the staleness-bug class the
  `parseUtcMs` baseline exists to prevent). Moved a zone-safe `formatClockTime` into
  `lib/time.ts` and used it. (Left as-is: the SessionView↔Emma respawn-banner / jump
  button extraction is pure maintainability; Emma's author-color maps use a
  deliberately different terminal palette, NOT a dup of `authorColor.ts`.)
- **fix(agents): align Rain's prompt with her enforced tool blocks (C1).** RAIN_ROLE
  listed `git branch`, `gh pr view`, `gh pr list`, `gh issue view`, `gh issue list`
  as allowed read-only investigation — but `spawn.rs --disallowedTools` blanket-blocks
  `git branch:*` / `gh pr:*` / `gh issue:*`, so Rain was told she could run commands
  the mechanism denies. Tightened the prose to match enforcement (kept the security
  boundary intact — enumerating "safe" gh subcommands would risk missing a mutating
  one like `gh pr comment`/`review`). Guard test asserts the blocked commands aren't
  advertised as allowed. (Deferred: F3 — `auto_supersede_prior_pending`'s supersede
  +insert aren't transactional; the proper fix is a combined atomic storage op, a
  moderate refactor on the critical tray path to close a microsecond crash window —
  poor ROI/risk for a sweep.)
- **fix(ui): distinguish DocumentPane load errors from empty (D6).** The
  `session_doc_search` and `compute_apply_diff` queries didn't expose `error`, so a
  failed fetch rendered identically to a genuine empty ("No {phase} documents yet."
  / a blank diff). Surface the error text distinctly for each.
- **tidy: session_doc timestamp + dedup push-gate action string (F2, C4).** F2:
  `upsert_session_document` used `chrono::Utc::now().to_rfc3339()` (`+00:00`) instead
  of the project-standard `now_utc()` (`Z`) — cosmetic, but it broke the single
  UTC-baseline invariant. C4: the `git push (<branch>)` violation/approval action
  string was built independently in `policy::hooks` and `signaling::server`; hoisted
  to one `policy::push_gate_action` helper so the audit log can't show two shapes.
  (F1 — cl_index `last_insert_rowid` → `RETURNING id` — left as-is: it's latent, no
  caller trusts the returned id, and the table's PK shape makes the change higher
  risk than the dormant landmine warrants.)
- **docs: correct the PluginManager status (finding 33).** ARCHITECTURE.md called
  the Plugins tab "Placeholder UI" and PLAN.md said the frontend install/heartbeat
  wiring "is not" done — both stale: `PluginManager.tsx` has working
  install/enable/disable/uninstall + a `plugin:crashed` heartbeat indicator. What
  actually remains is live plugin *execution* (the per-plugin iframes + ping/pong;
  `PluginSlot` was removed as dead code). Updated both docs to match.
- **fix(core): stop dropping control events under load (A3).** `SessionCloseRequest`
  / `AgentAdvancePhase` shared the one 64-slot broadcast channel with per-chunk
  `MessagePersisted`, and the main.rs handler `.await`ed the slow core work (close
  kills subprocesses) INLINE in the recv loop — so a chunk flood during a close
  could lag the channel and silently drop a close/advance (subprocess kept
  running / phase never advanced on the backend). Now the recv loop only matches +
  hands off to a serial unbounded worker (never blocks), and the channel headroom
  went 64→1024. (E2 — dropping the redundant tray/phase polls — stays deferred:
  the *frontend* subscriber still only logs on Lagged without re-syncing, so the
  polls remain a cheap safety net; low value now that E1 made invalidation
  targeted.)
- **fix(agents): recover a deaf agent instead of bridging to it forever (A2).** Root
  of the #4 user→HANDS desync. The supervisor holds the public input receiver and
  forwards to the per-incarnation stdin pump with `let _ =`; when that pump died
  (stdin write failed) the error was swallowed, the public `input_tx` stayed open
  (so `is_stale()` read false), and the supervisor kept bridging to a now-deaf
  child as long as its event channel lingered — Brian silently ignored all input
  while Rain kept working, with no signal and no recovery. Now: a failed forward
  tears the supervisor down (kill + return) so the public channel closes →
  `is_stale()` true; and `core::broadcast` respawns a stale handle before
  delivering, so the next user message auto-heals the session. Test:
  `supervisor_terminates_when_incarnation_input_pump_dies`.
- **fix(core): prune bridge session maps on close; log swallowed Emma sends (A4, A5).**
  `close_session` cleaned the sessions map, tray, and policy snapshot but never the
  bridge's `session_projects` / `session_awaiting` maps — each open→close leaked a
  map entry + a dangling `Arc<AtomicBool>` for the process life. Added
  `bridge.unregister_session` and call it from `core::close_session`. Also two
  Emma stdin sends (`broadcast` + the OOB resolve-wake) used `let _ = …send()` with
  no log — if Emma's input pump died the message persisted + showed in chat but she
  never saw it, zero signal; now logged (same diagnosability fix as the duo desync
  paths).
- **fix(policy): make silent policy-disarm visible (B1, B2).** Two paths could
  silently weaken enforcement with no signal. (B1) `Policy`/`SessionPolicy` had no
  unknown-key handling, so a typo (`push-gate:`, a mistyped `tool_gate:`) was
  dropped and the setting resolved to the permissive default. Added a loud
  `tracing::warn` on unrecognized top-level keys in the policy + session-snapshot
  load paths — deliberately NOT `#[serde(deny_unknown_fields)]`, which would break
  older files carrying the retired `tool_blocklist` (failing parse → disarming the
  git-hook enforcement) and is unsupported alongside `SessionPolicy`'s
  `#[serde(flatten)]`. (B2) `audit.rs` `unwrap_or_default()` silently reset the
  policy-hash cache on a corrupt file → every file re-registered as `FirstSeen`,
  disarming mutation detection for that cycle; now logs the reset loudly.
  Non-breaking — enforcement behavior unchanged, the disarm is just no longer
  silent. Tests cover the key-detection.
- **add(ui): close-session action in the SessionView header (D1).** The
  `close_session` command had zero UI callers — a human could start/configure a
  session but never end one (only an agent could, via MCP). Added a confirm-gated
  "✕" close button (kills Brian + Rain, archives the session); the existing
  `session:closed` listener navigates back to the dashboard, and the Archive tab
  can reopen it (resumes via --resume). Surfaces a close-failed inline error.
- **fix(ui): confirm destructive actions; drop dead CL "New file" button (D5, D12).**
  Saved-model Delete (removes the stored auth token, irreversible) and Unregister
  Project now require a `window.confirm` (matching the plugin-uninstall pattern).
  Removed the permanently-disabled "New file — backend not yet wired" sidebar
  button — creation is wired via right-click (which has the folder + name context
  the header button lacked). (Deferred: guarding new-file/folder on the `_globals`
  virtual root — lives in the ContextLibrary menu builder, folded into a later CL
  batch; harmless meanwhile.)
- **remove(ui): dead top-bar search + dead footer links (D2).** The "Search
  sessions, agents, tasks…" topbar input stored state but never filtered or
  navigated anything, and the footer "API Docs"/"Support" were `href="#"`
  placeholders. Removed both (+ the now-unused `SearchIcon`/`useState`). Real
  session search is a clean follow-up feature if wanted — left out here since it
  promised searching agents/tasks (not first-class entities) and crosses routes.
- **fix(ui): surface errors on the silent HITL paths (D3, #32).** Three core
  human-in-the-loop actions failed silently: broadcast-send (`ChatInput` try/finally
  with no catch → unhandled rejection, user thought the message sent), tray-resolve
  (`DocumentPane` `console.error` only → answer stuck pending, no signal), and
  config restart-agents (`ClaudeConfig` loop with no catch). Each now catches and
  shows a dismissible inline error; the broadcast fix lives in shared `ChatInput`
  so it covers both SessionView and the Emma overlay.
- **perf(ui): scope event-driven query invalidation + concat chat batches (E1, E3).**
  `GlobalEventSync` called `invalidateQueries()` with no key on every `session:*`
  event → an app-wide refetch storm (10-20+ queries incl. `compute_apply_diff`
  spawning `git`) on a single choice-resolve. Now each event invalidates only the
  query families it can affect (tray / phase+docs+diff / close lists). Also
  `chat.ts applyBatch` spread `[...current, msg]` inside the per-message loop
  (O(N·K) — a 20-msg batch copied the history up to 20×); now accumulates per
  session and concats once. (E2 poll-removal deferred until the lossy-channel fix
  A3 — the bridge subscriber drops `session:*` events on `Lagged` without
  re-syncing, so the safety polls stay load-bearing until then.)

## 2026-06-04 — fix: resume the duo after the user answers a choice

The Brian↔Rain peer-forward went silent after the user clicked an
`ask_user_choice`/`request_approval` button, staying frozen until the user typed
free text or advanced a phase. Root cause: `ask_user_choice`/`request_approval`
set a shared `awaiting` `AtomicBool` (via `bridge::set_session_awaiting`), but the
common resolve path — `bridge::resolve_choice` → `ResolveOutcome::Delivered`
(oneshot send succeeds, agent resumes via the tool return) — never cleared it, so
`duo::flush_buffer` (gated on the flag) kept dropping every peer-forward. Only the
OOB-fallback arm, `broadcast`, and `advance_phase` cleared it. Likely the root of
the long-standing "answer didn't round-trip" symptom (notes #2).

Fix: `bridge::resolve_choice` now clears the halt (`clear_session_awaiting`) right
before delivering the pick — the bridge owns the awaiting map and set the flag, so
it clears it symmetrically. Clearing *before* `p.tx.send` (not after) avoids a
1-chunk race where the resumed agent's first reply could be suppressed before the
flag flipped; it also covers the Err/OOB fall-through (core then re-clears + wakes
stdin, harmlessly redundant). Covers choices, approvals (incl. pre-push), and
`action_gate`. Regression test `resolve_choice_delivered_clears_awaiting` asserts
the flag is set after the ask and cleared after a Delivered resolve.

## 2026-06-04 — remove user-facing screenshot button

The 📸 "share window" button (SessionView header + Emma overlay) was designed as
an agent context tool, not a user action — YAGNI for humans, who have no real
use-case for it. Removed the button + dismissible error banner + the shared
`useScreenshotCapture` hook (deleted, zero consumers) from both surfaces, plus
the frontend-only `capture_window_screenshot` Tauri command (+ its specta
registration + regenerated binding). The `webview_screenshot` MCP tool — the
agents' "eyes on the UI" — is unaffected: it uses the separate `capture_main_window`
helper, which stays.

## 2026-06-04 — remove UI manual phase-advance

User-directed removal of the UI's ability to advance a session's IPAV phase. The
interactive `PhasePillRow` in the `SessionView` header (which called the
`advance_session_phase` Tauri command) is gone; the backing command was
frontend-webview-only, so it was removed end-to-end — command + `IpavPhase`
import + specta registration + regenerated binding — rather than left dead. The
header still *displays* the current phase (read-only) and the
`session:phase_changed` listener stays: agents still drive phases via the
`advance_phase` MCP tool → `AgentAdvancePhase` → `core.advance_phase` (untouched).
Resolves the "double phase-control surface" gap — the identical-looking
`DocumentPane` pills are a view-only tab selector and stay.

## 2026-06-04 — codebase + docs cleanliness pass

Swept the codebase, CL, and docs for redundancy/staleness/dead code (clippy was
already near-clean; the debt was mostly stale docs + small frontend dup). 9
commits, gated per batch; no behavior change except the two UI fixes below.

- **Docs synced to reality.** ARCHITECTURE.md/README.md documented 3 tools that
  no longer exist (`grant`/`revoke`/`list_session_permissions` — the subsystem
  was deleted); "26 internal tools" was actually 24 and omitted `action_gate`;
  the `questions` table was renamed `session_tray` (migration 0010); the whole
  Claude Config surface + `models`/`app_settings` registry + `llm_proxy` were
  undocumented; one event name used illegal dots. Rewrote the "Session
  permissions" section for the current `session_policy.rs` frozen-snapshot model
  and fixed the in-repo `CLAUDE.md` push-grant + data-path references. Pruned
  PLAN.md (dropped hardcoded test counts; noted shipped model/Claude-config work).
- **`fix`: ToolSearch added to Rain's allowlist** — her role prompt promised it
  but `spawn.rs` blocked it (WebSearch was already allowed). Test locks the
  prompt↔allowlist contract.
- **`tidy`: dedup + clippy-clean** — hoisted the hand-synced
  `AGENT_FILTERED_MCP`/`RESERVED_MCP_KEYS` pair to one crate constant, refreshed
  stale `session_tray` comments, collapsed identical `HookKind::filename`/
  `subcommand`, resolved all clippy nits.
- **`refactor`: `storage::model` → `row_types`** — the module holds all 15 row
  types, not one Model; the name collided with the sibling `models.rs`.
- **`chore`: deleted dead frontend** — `PluginSlot.tsx` + `stores/layout.ts`
  (zero consumers).
- **`refactor(ui)`: extracted `GatedKeywordList`** shared by Settings + the
  session policy panel; dropped a `formatTimestamp` shadow of `lib/time`.
- **`fix(ui)`: purge a closed session's messages** from the chat store on
  `session:closed` (was a latent leak — `clear()` was test-only).
- **`refactor(ui)`: SessionTile indicates, doesn't answer** — removed the inline
  `ChoiceBanner` (a second answer surface duplicating the Tray tab); the tile now
  shows a `[Need User Input · N]` count and points to the session's Tray tab.
- **`refactor`: dropped the completed legacy-CL startup import** (one-shot
  `~/.bot-hq-legacy-2026-05-15` mirror; the dir is gone, so it was a no-op).

Verified-no-change (reported, not edited): push_gate `ask` docs already match the
code (the June-2 hard-block issue was fixed by the June-3 tray work); the two
`resolve_agent_overrides` call sites are intentional layering, not a double-resolve.

## 2026-06-03 — close_session withdraws pending; backfill stale closed-session pending

`close_session` left a session's pending tray rows as `pending` forever, so already-closed sessions
accumulated dead pending (61 rows across ~55 old sessions). Fixed:
- `core::close_session` now calls `storage.withdraw_pending_tray_for_session` after marking the
  session closed → a closing session's pending questions/approvals/gated commands are withdrawn.
- migration `0011`: one-time backfill — withdraws pending rows belonging to already-closed sessions
  (clears the existing 61; no-op on a fresh DB and after the first run).

Test: `withdraw_pending_tray_for_session` is session-scoped + only touches pending (already-answered
rows untouched).

## 2026-06-03 — Tray pill count + pulse; remove the in-chat question popup

Final tray polish:
- The in-chat `ChoicePrompt` popup (`SessionView`) is removed — the Tray tab is the sole answer
  surface now. Dropped the now-dead `list_pending_choices` query, resolve handler, and state.
- The Tray pill shows a pending-count badge and pulses (bell-style `animate-pulse` + primary tint)
  when count > 0, so accumulated input is visible from any tab without opening the Tray. Count comes
  from `list_session_tray` (shared query cache; `GlobalEventSync` keeps it live event-driven).

## 2026-06-03 — Event-driven UI reactivity (no more "stale until tab-switch")

The UI only refetched on mount/tab-switch, so backend state changes didn't show until the user
navigated away and back: new session docs, a tray answer reflected in chat, and session close
(which stranded the user inside the now-closed session). Fixed event-driven — emit an event for
every relevant change + invalidate queries on it, no polling. (Supersedes the 2s Tray poll
`83385fe`.)

- Backend: new `SignalingEvent::DocChanged` + `SessionClosed` (+ `tauri_events` types +
  `bridge_subscriber` routes → `session:doc_changed` / `session:closed`). `session_doc_write` emits
  DocChanged; `core::close_session` emits SessionClosed via `bridge.notify_session_closed`.
  `resolve_choice`'s two OOB arms now call `notify_message_persisted` (capturing the insert id) →
  fires `agent:messages:batch`, so a tray-answered choice shows in the chat live.
- Frontend: `GlobalEventSync` (in `Providers`, inside `QueryClientProvider`) listens to the
  `session:*` events and `invalidateQueries()` (all) — event-driven, no timers. Dropped the Tray
  `refetchInterval`. `SessionView` navigates to the dashboard on `session:closed` for the current
  session. `agent:messages:batch` is excluded from the global invalidation (the chat consumes it
  directly; invalidating everything on each message batch would be wasteful).

Tests: `bridge_subscriber` routes for the 2 new events; the OOB-fallback test now asserts
`MessagePersisted` fires.

## 2026-06-03 — Tray tab live-refreshes (2s poll)

The Tray tab fetched `list_session_tray` once on mount, so newly-parked pending didn't appear (and
answered items didn't drop) until the user switched tabs and back. Added `refetchInterval: 2_000` to
the query (same cadence as the notification bell; the query only mounts while the Tray tab is shown,
so it's idle otherwise) so the inbox updates live.

## 2026-06-03 — auto-supersede only true re-asks, so pending accumulate

`auto_supersede_prior_pending` marked ANY prior pending from the same agent as `superseded` on
every new ask — so distinct questions/gates collapsed to the latest, defeating the "pending
accumulate while the user is AFK" goal (the tray showed only the most recent of an agent's asks).
Now it matches on `prompt`: a re-ask of the SAME prompt (the timeout-retry case it was built for)
still supersedes, but distinct prompts both stay pending and accumulate.

- `bridge/questions.rs`: `auto_supersede_prior_pending` gains a `prompt` param; the find filter adds
  `q.prompt == prompt`. Callers (`ask_user_choice`, `request_approval`) pass the new question.
- Tests: re-ask of same prompt supersedes + links via supersedes_id; two distinct prompts both stay
  pending.

Known related (not yet fixed): `close_session` doesn't withdraw a closed session's pending tray
rows, so they linger as `pending` forever (currently 61 across ~55 old closed sessions). Harmless —
the notifier's open-session filter hides them — but worth a cleanup + a close-time withdraw.

## 2026-06-03 — Notifications grouped per session ("Session-X needs your input [N]")

The header notification tray (`PendingTray`) now groups pending across sessions: one row per
session — "Session {id8} · needs your input [N]" with the per-session pending count and a
go-to-session CTA — instead of one row per item. The bell badge counts SESSIONS awaiting input (not
raw items). Stays notify-only (decision #7): answering happens on that session's Tray tab.

Source is the live in-memory `list_pending_choices` (covers the normal AFK-while-running case).
Reflecting durable pending that survived a restart would need a global durable pending query —
flagged follow-up.

## 2026-06-03 — Tray tab → actionable pending inbox (not a history log)

Reframed the session Tray tab (shipped read-only in `a91a603`) into an actionable **pending
inbox**: it shows only PENDING items and answers them inline. Pending questions / approvals / gated
commands accumulate there (durable — survive AFK + restart) and the user resolves them from the tab
when they return. Resolved history is intentionally dropped (it was noise — an inbox, not an audit
log).

- `DocumentPane` `TrayList`: filter to `status === "pending"` (removed the resolved-history
  rendering), reuse the shared `ChoicePrompt` (preset options + mandatory "Other") per item, wire to
  `resolve_choice(choice_id, picked)` and invalidate the `list_session_tray` query on settle so the
  answered item drops out. action_gate rows show the gated command above the prompt.
- Notifications (header `PendingTray`) deliberately stay notify-only (go-to-session CTA); a
  per-session "needs your input [N]" count is a planned follow-up.

## 2026-06-03 — Session-view Tray tab (Tray · I · P · A · V)

Surfaced the durable `session_tray` as a tab before the IPAV phase tabs, so every accumulated
question / approval / gated command (pending + resolved history) is visible per session — including
items that survived a restart.

- `tauri_cmd/questions.rs`: `SessionTrayView` + `list_session_tray(session_id)` reading the durable
  rows via `bridge.list_questions_for_session` (decodes `options_json`; carries `command_text` /
  status / kind / timestamps). Registered in `tauri_specta_gen.rs`; `bindings.ts` regenerated.
- `DocumentPane.tsx`: a phase-independent `Tray` pill before `PhasePillRow` (now `selected: Phase |
  null` so no phase highlights while Tray is active). Read-only v1: kind/agent/status badges, prompt,
  gated command, options + picked, timestamps; pending highlighted + ordered first. A phase
  transition updates the underlying phase but does NOT pull the user off the Tray.

Read-only for now — inline Approve/Reject from the tab is a possible follow-up (the in-chat
`ChoicePrompt` already resolves the active pending choice).

## 2026-06-03 — Durable tray (session_questions → session_tray) + execute-on-approve anytime

Renamed `session_questions` → `session_tray` (it outgrew "questions" — it durably mirrors every
awaiting-input tray item: questions, approvals, action_gate gated commands, halts) and made an
approved action_gate command execute whenever it's resolved — hours/days later, or after a restart
— not just within the in-memory oneshot's lifetime. Closes the gap `ae79f3a` documented (the
post-restart `None` branch couldn't execute because the command wasn't persisted).

- migration `0010`: `ALTER TABLE session_questions RENAME TO session_tray` + `ADD COLUMN
  command_text TEXT` + recreate the partial pending index under the new name. (Type
  `SessionQuestion` → `SessionTrayEntry`; method names kept to bound churn. The type isn't surfaced
  via tauri-specta, so bindings.ts / the frontend are untouched.)
- `command_text` persists the gated command on the row (set for ToolBlocklist approvals in
  `ask_user_choice_inner`, extracted before `approval` moves into `PendingChoice`).
- `resolve_choice` executes the approved command from the durable row on BOTH receiver-gone paths —
  the same-session timeout `Err` arm (generalizes `ae79f3a`) and the post-restart `None` arm — via a
  shared `maybe_run_gated` helper. The `Delivered` (in-band) path is excluded (action_gate's own
  future runs it there) → no double-fire.
- Exactly-once is now durable: gated on `answer_question`'s atomic pending→answered flip
  (`rows_affected == 1`), so a duplicate / stale / post-restart resolve can't re-run the command.
  Replaces the in-memory oneshot's exactly-once guarantee with a DB one.

Tests: `post_restart_action_gate_executes_from_durable_row` (None arm runs from `command_text`),
`resolve_twice_executes_gated_command_once` (exactly-once via the flip gate), plus the existing
`timed_out_action_gate_still_executes_on_approve` (now flip-gated).

push-gate unchanged: it blocks a live `git push` and can't be deferred days; stays now-or-times-out.

## 2026-06-03 — action_gate executes on approve even after a client timeout

`action_gate`'s approved command runs server-side via `execute_gated`, which lived only inside the
MCP request future. When claude-code's MCP client timed out (~30s) waiting on a human, that future
was cancelled before `execute_gated` ran, and the OOB fallback (`resolve_choice`) re-delivered only
"Approve" — so a gated command the user approved would silently never execute. (Surfaced live while
testing the push-gate work: a timed-out `gh api user` approval returned no output.)

Fix: run the approved command at resolve time, decoupled from the dead request future.
- `bridge/action_gate.rs`: `execute_gated` → `pub(super)` so the sibling `bridge::questions` module
  can call it (private-to-module otherwise → E0624).
- `bridge/questions.rs` `resolve_choice`, receiver-dropped arm: when the parked approval is an
  `action_gate` request (`ViolationKind::ToolBlocklist`) resolved `Approved`, run `execute_gated`
  and append its output to the OOB message body. In-band (`Delivered`) and dropped (`Err`) paths are
  mutually exclusive on one `tx.send`, so the command runs exactly once. Scoped to ToolBlocklist —
  `ask_user_choice`, `per_action`, and `push_gate` paths are unchanged.

Distinct from the June-1 fix (#2), which made the OOB fallback deliver the user's *decision* — that
works (verified). This covers the one tool whose *action* executes server-side.

Test: `timed_out_action_gate_still_executes_on_approve` aborts the request future to simulate the
client timeout, then resolves Approve and asserts the command ran (marker file) + output is in the
OOB body.

Known limitation: the reopened-session branch of `resolve_choice` (bridge lost the in-memory Parked)
can't execute — the durable `session_questions` row stores prompt/agent/session but not the command
string. Rare (needs a bridge restart between ask and answer); would need the command re-issued.

## 2026-06-03 — push_gate "ask" prompts per-push instead of hard-blocking

`push_gate: ask` used to make the `pre-push` git hook hard-block every `git push`
(exit 1, "flip the toggle to auto") — it never actually asked, unlike `action_gate`
for other gated commands. Now `ask` surfaces a per-push Approve/Reject prompt to the
user (reusing the `request_approval` → `PendingChoice` → `resolve_choice` →
`PushGate`-violation path) and blocks on their pick: approve → push proceeds, reject
→ blocked.

The `pre-push` hook runs as a separate subprocess that can't reach the running app's
bridge, so:
- `src/main.rs` persists the internal signaling server's bound address to
  `<data_dir>/.local/signaling-addr` at startup (`paths.rs::write_signaling_addr` +
  free `read_signaling_addr`); `SignalingServer` removes it on clean shutdown (Drop).
- `src/signaling/server.rs` adds a dedicated `POST /hooks/pre-push` route that calls
  `bridge.request_approval(kind=push_gate)` directly (no HANDS-only MCP gate, no agent
  identity in the URL path) and replies `{"approved": bool}`.
- `src/agents/spawn.rs` exports `BOT_HQ_AGENT` so the prompt attributes to the pushing
  agent (covers solo Emma; Rain can't push).
- `src/policy/hooks.rs::run_pre_push` POSTs that route inside a current-thread runtime
  (reqwest, 30-min timeout) and maps Approve→0 / Reject→1. Fail-closed (exit 1 + its
  own `PushGate`/Denied violation, distinct reason per failure: no addr / connect /
  timeout / non-200 / malformed) when the app is unreachable. A push with no
  `BOT_HQ_SESSION_ID` (manual human terminal push) stays hard-blocked with guidance —
  avoids an `env -u BOT_HQ_SESSION_ID` bypass.

Lockstep prompt/doc text (7 spots) flipped from "ask = hard block, flip the toggle" to
"just run `git push`; the hook prompts the user Approve/Reject per push; you don't call
a grant tool or flip a toggle": `policy/mod.rs` (field doc, `Ask` variant, system-prompt
block), `policy/hooks.rs` (module + `run_pre_push` doc), `agents/general_rules.rs`,
`agents/prompts.rs`. ARCHITECTURE.md + README.md push-gate sections corrected (also fixed
adjacent pre-existing B-series drift: `push_gate`/`force_push` are scalar `auto|ask` /
`blocked|allowed`, no `.mode` / `remembered_approvals`).

Known follow-up: ARCHITECTURE.md's "Session permissions" / `grant_session_permission`
section + the Tool-Gate push-grant reconcile line still describe the pre-B-series grant
mechanism (removed when push/force-push became pure toggles) — left for a separate
doc-sync pass, out of scope here.

Tests: +1 paths (addr round-trip), +2 server (`/hooks/pre-push` approve/reject + missing
session_id → 400), +2 hooks (ask-without-session block, no-addr fail-closed Blocked).

## 2026-06-02 — Surface + control Claude Code config in Settings

bot-hq's agents are `claude-code` headless subprocesses, so the user's
`~/.claude` config (skills, plugins, hooks, CLAUDE.md/memory, MCP, effort)
**leaks into the agents** — a self-invoking skill or a plugin hook can derail a
Brian/Rain workflow, and that inherited config was invisible in the UI. New
**Settings → Claude Config** subtab surfaces it and lets the user control it,
both globally (edit their real `~/.claude`) and per-agent (an override layer
bot-hq injects at spawn), without bot-hq ever writing its own config into
`~/.claude`. Design: [`docs/plans/2026-06-02-claude-config-surface-design.md`](docs/plans/2026-06-02-claude-config-surface-design.md).

- **Read/resolve layer** (`src/claude_config/reader.rs`): resolves the config
  dir (honors `CLAUDE_CONFIG_DIR`), reads `settings.json` + `~/.claude.json` +
  `CLAUDE.md`/memory + `skills/` + `enabledPlugins`, with secret masking and the
  known traps flagged (e.g. `settings.json` `mcpServers` is ignored by
  claude-code — it loads MCP from `~/.claude.json`; bot-hq forwards both).
- **Inheritance lens** (`src/claude_config/mod.rs`): the single source of truth
  for which agents pick up each surface (Brian/Emma inherit; Rain `--bare` skips
  skills/plugins/hooks/CLAUDE.md; model/permissions overridden). Drives the
  per-surface badges in the UI.
- **Override store** (`src/claude_config/overrides.rs`):
  `<data_dir>/claude-overrides.json` (0600), `_all` fan-out + per-agent entries.
  Wired into `spawn.rs::build_command` (merged into the injected `--settings`
  `skillOverrides`/`enabledPlugins`/`ultracode` + effort/auto-memory/CLAUDE.md
  env) and `session.rs` (per-agent MCP filtering). `skillOverrides` (verified on
  claude 2.1.160) is the clean lever for "disable a self-invoking skill for the
  agents only" — the headline use case.
- **Global write-back** (`src/claude_config/writer.rs`): read-modify-write of
  `settings.json` that preserves all other keys + secrets; typed commands for
  string/bool knobs + plugin enablement. Malformed `settings.json` errors
  without clobbering.
- **UI** (`frontend/src/app/ClaudeConfig.tsx`): the Settings tab is now tabbed
  (Agents · Claude Config · Tool Gate). Claude Config is a 2-pane tree
  (surface-first) reusing the Context Library shell idiom, with the inheritance
  lens, global editors, and per-agent override controls. **All edits (global +
  override) batch behind one Save** (review before writing `~/.claude`); after
  saving, a banner offers to **restart running agents** so they pick up the new
  config (read at spawn). New `export-bindings` CLI subcommand regenerates the
  frontend bindings headlessly.
- **Force-restart primitive**: `CoreAppState::restart_session` + the
  `restart_session` Tauri command evict a live duo and re-spawn it (re-reading
  overrides + per-agent mcp-config; agents resume via `--resume`). Distinct from
  `respawn_session`, which is the idempotent on-mount "ensure started" and a
  no-op on a healthy session.
- **Out of scope** (deferred, noted in the design): SKILL.md global edit, MCP
  list/markdown/hooks rich widgets, full precedence engine. `policy.yaml` is
  intentionally excluded (bot-hq-internal, not user Claude config).

## 2026-06-02 — Auto-resume agents on transient API errors

A transient upstream API error (e.g. Anthropic `529` Overloaded) killed an
agent's claude-code subprocess and **nothing respawned it** — the session sat
dead on "API Error: Overloaded" indefinitely (observed on `s-c9f509d2`:
Brian died mid-Apply on 2026-06-01 after B1+B2 of the policy redesign had
landed + pushed). The only restart path in the codebase was the manual
`restart_emma` tool; an agent that hit a self-clearing blip was stranded.

Root cause: `events.rs` collapsed the result event's `api_error_status` (the
HTTP code) into a bare `is_error` bool, discarding the transient-vs-permanent
signal; on the subsequent `Exited`, `pump_agent` drained the buffer and the
supervisor task simply ended — no retry, no backoff, no respawn.

- **Signal plumbing.** `AgentEvent::TurnComplete` now carries
  `api_error_status: Option<u16>`; `events.rs::extract_api_status` coerces the
  wire value (number or string) and `spawn::is_transient_api_error` classifies
  it (`408/425/429/500/502/503/504/529` transient; `400/401/403/…` permanent —
  the DeepSeek system-role `400` stays a hard stop).
- **Retry supervisor.** New `spawn_supervised_agent` wraps the per-incarnation
  `spawn_agent` in a respawn loop that exposes STABLE event/input channels, so
  the peer-forward and `SessionHandle` survive a respawn with zero rewiring. On
  a transient failure it resumes the agent (`--resume <uuid>`, UUID captured
  from the tapped `init` event) after capped exponential backoff
  (2/4/8/16/30s, 5 attempts) and nudges it to continue where it left off; a
  clean turn resets the budget; a permanent error or an exhausted budget
  surfaces a clear message and unwinds. `Exited` is suppressed mid-retry —
  channel-close is the race-free end-of-incarnation signal, so the final
  errored `TurnComplete` is always seen before classifying.
- `core/session.rs` spawns Brian/Rain via `spawn_supervised_agent`
  (`RetryPolicy::default()`); `pump_agent` / `duo.rs` behaviour is unchanged
  (the error text is still persisted for UI visibility and never peer-forwarded).
- +9 lib tests (classifier ×2, status propagation ×3, backoff cap, supervisor
  resume-then-clean / permanent-no-resume / give-up-after-cap). Lib suite
  **305 passing**; clippy clean on touched files; release build green.

**Follow-up — evict stale session handles.** The supervisor closes its
channels on a *permanent* death (a non-retryable error or exhausted budget),
but the dead `SessionHandle` lingered in the in-memory map, so
`ensure_session_started` fast-pathed on `contains_key` and never re-spawned —
the session was stuck until an app restart (`ensure_emma_started` had the same
zombie). Added `SessionHandle::is_stale` / `EmmaHandle::is_stale` (true once a
supervisor drops its input receiver, closing the stable sender; stays false
during a healthy run *and* a transient-retry backoff). Both `ensure_*_started`
now treat a stale handle as absent: evict it (killing already-dead agents is a
no-op) and re-spawn via the resume path on the next interaction. Transient
deaths already self-heal via the supervisor; this closes the permanent-death
case.

## 2026-06-01 — Fix nested-runtime panic in policy-mutation audit

Sending a message to a session panicked the tokio worker with "Cannot start
a runtime from within a runtime" (`policy/audit.rs:181`), wedging session
start. Root cause: `log_sync` built a nested tokio runtime and `block_on`'d
it to append a `PolicyMutation` entry — harmless in the hookless
`policy-check` subprocess, fatal from the in-process async call sites
(`spawn_session_handle`, the signaling bridge). The Tool Gate commits had
rewritten the policy YAML, so the stale `.policy-hashes.json` made the next
session-start audit take the `Changed` branch → `log_sync` → panic.

- `ViolationsLog`: private `write_lock` switched from `tokio::sync::Mutex`
  to `std::sync::Mutex` (the guard is never held across an `.await`), and
  added synchronous `append_blocking` / `record_blocking` siblings sharing a
  `build_record` helper. The async `append`/`record` keep identical
  signatures, so all existing callers are unchanged.
- `audit.rs::log_sync` now calls `record_blocking` directly — no runtime, so
  it's valid in every context. One fix covers all three call sites.
- Self-healing: the first post-fix audit logs the (audit-only, non-blocking)
  `PolicyMutation` for the changed files and refreshes the hash cache; no
  data files touched.
- +1 regression test (`change_detected_inside_runtime_does_not_panic`) that
  runs the audit inside a `#[tokio::test]` runtime — it reproduced the exact
  panic before the fix. cargo test (347) + release build clean.

---

## 2026-06-01 — Maintain CL dispatch button (Context Library)

A "Maintain CL" button in the Context Library sidebar opens a dialog → the
user picks a project → a Brian + Rain session is dispatched pre-loaded with
a hardcoded, engineered prompt to maintain that project's CL (audit the
where-things-live map, sharpen descriptions, prune stale notes — keeping
the CL lighter than the codebase). Delegating CL upkeep to a session.

- New generic Tauri command `dispatch_session(id, title, project,
  repo_path, prompt)` (`tauri_cmd/sessions.rs`): create row → register
  project → `ensure_session_started` (spawn duo) → `broadcast(prompt)` in
  one atomic call. A fresh session spawns blank (`resume None`) and bot-hq
  doesn't replay storage to stdin, so the prompt must reach a LIVE session —
  hence spawn-then-broadcast in the command (avoids both the
  broadcast-before-spawn "no live session" race and a SessionView
  route-state hook that could double-send).
- The engineered prompt is a frontend const (`lib/maintainClPrompt.ts`,
  vitest-tested) so it's HMR-iterable; the Rust command stays generic.
- UI: `MaintainCLModal.tsx` (project picker → dispatch → navigate to
  `/sessions/<id>`), the sidebar button, wired in `ContextLibrary.tsx`.
- 7 files (3 new, 4 modified); +2 frontend tests (prompt anchors).

## 2026-06-01 — CL ⇄ IPAV workflow tightening

Tied session docs and the CL more tightly to the IPAV workflow so each
phase leaves ONE rewritable doc the next phase builds on, and the CL
stays fresh without bloating. Three commits:

- **One doc per phase, structurally** (`2d205d0`): `session_doc_write`
  keys phase-tagged docs by phase via an `effective_slug` helper
  (`bridge/session_docs.rs`) — repeated writes (even under a varied slug
  like `plan-v2`) overwrite the single `plan` doc, latest body wins. No
  migration; untagged scratch docs still key by slug. Tool descriptor
  (`protocol.rs`) now says "rewrite, never -v2".
- **Markdown doc preview, no count chips** (`3819c9b`): the chat's
  react-markdown renderer is extracted to a shared `Markdown.tsx` (GFM,
  code blocks, new-tab links, Industrial-Terminal styling) and reused in
  `DocumentPane`; the raw `<pre>` is gone and the per-phase doc-count
  indicators (`PhasePill` `·{n}` + the `{n} docs` span) are removed.
  Session docs aren't user-editable, so a rendered preview beats raw text.
- **Phase-doc chaining + CL model + close-loop** (`03d7615`): prompts now
  require Plan to build on the Investigate doc, Apply on Plan, Verify on
  Apply; HANDS authors the single phase doc while Rain reviews in chat (no
  two-author clobber on the shared, author-less `session_documents` row);
  CL is framed as "study notes, not a textbook" (a where-things-live map,
  not a code copy); and a write-then-prune close loop has HANDS append
  ≤~5 non-obvious one-liner learnings to a project's `notes.md` before
  `close_session` (user curates later in the CL tab). `ARCHITECTURE.md`
  softened to match.

Tests: 296 lib (+9: 2 session-doc helper, 4 general_rules anchors, 3
prompts anchors) + 29 frontend (+2 Markdown, −1 PhasePill count). Agents
pick up the new prompts and the markdown doc view only after a rebuild +
app restart.

**Follow-up bug fixes (same session):** `c19e0e0` — `session_doc_write`
returned a bogus row id on overwrite, because `last_insert_rowid()`
reports the bumped AUTOINCREMENT value on an upsert's UPDATE branch;
switched to `INSERT … RETURNING id`. `547d364` — agent self-advance (the
`advance_phase` MCP tool) only moved the frontend chip:
`SignalingEvent::AgentAdvancePhase` was consumed solely by the Tauri emit
subscriber and never routed to `core.advance_phase`, so the backend
`IpavState` (and every `[PHASE: X]` peer envelope) stayed stuck on the
default phase — the same no-op class as the old close_session bug. Added
the missing arm to the main.rs signaling consumer. Tests: 297 lib (+1
upsert-id regression).

## 2026-05-31 — Tool Gate: global gated-Bash keywords + action_gate

Replaced the per-project `tool_blocklist` PreToolUse gate (2026-05-29) with a
global, user-configurable **Tool Gate**: one keyword list
(`<data_dir>/tool-gate.json`, edited in Settings → "Gated Bash Keywords") over
agent Bash commands. A `gate` keyword blocks the command (PreToolUse exit 2) and
routes the agent to a new `action_gate` MCP tool, which surfaces Approve/Reject
and — on approve — EXECUTES the command in the session repo and returns its
output (an action request, not a permission request); `auto_allow`/no-match runs
normally. Gate-run pushes pre-record a session push grant for the current branch
so the pre-push hook doesn't double-gate.

- Backend: `src/policy/tool_gate.rs` (config + case-insensitive substring matcher
  + timeout-bounded executor), `action_gate` MCP tool
  (`src/signaling/bridge/action_gate.rs`), hook rework `run_tool_blocklist` →
  `run_tool_gate` (`hooks.rs` + `spawn.rs`).
- Frontend: Settings "Gated Bash Keywords" section; removed the commit/push
  GrantPills (+ their now-unused session-permission Tauri commands — the bridge
  methods + MCP tools are retained).
- Cleanup: retired `.claude/hooks/approval-gate.js` (+ its settings wiring);
  `policy.yaml` `tool_blocklist` marked RETIRED (parses, unenforced); reconciled
  agent prompts + canonical docs.

NB: the global keyword list defaults EMPTY — configure `gh` / `git` / `push` /
etc. in Settings to restore the 2026-05-29 outward-command protections.

Gates green: cargo test (287 lib + 49 integration), frontend vitest 28, tsc,
release build, frontend build.

---

## 2026-05-31 — SWE-bench Verified harness + test-feedback loop

Added `bench/swebench/` — a harness that benchmarks the duo (Brian/Opus-4.8 +
Rain/DeepSeek-V4-Pro) on SWE-bench Verified by driving the external MCP
(create_session → send_message → poll snapshot for the SWEBENCH_DONE sentinel →
`git diff` → predictions.jsonl), then scoring with the stock swebench harness.
Stdlib-only rollout driver; dataset via the HF datasets-server REST API.

**Result:** 27/39 resolved (69%) across all 12 Verified repos, 0 scoring errors.
The duo trails strong single-model Opus-4.8 — a structural gap (no test-feedback),
not the model.

**Test-feedback loop** (`--verify`): after the duo signals done, run the repo's
EXISTING tests against the patch in the prebuilt container (model_patch only, no
test_patch — leakage-free), bounce regressions back with their errors, revise,
cap at K rounds. On 3 known-wrong instances it flips SHALLOW regressions
(astropy-13398: 6 broken tests → resolved) but not CATASTROPHIC ones (requests:
43–45 broken tests → unresolved even with error-rich feedback). Also surfaced a
duo discipline gap: it signalled DONE while existing tests still failed.

Notes: `--instance-ids` (hand-picked diverse sets), incremental-save,
`.git/info/exclude` artifact guard (an agent's venv got swept into a 39MB diff via
`git add -A`), datasets-server retry. On Apple Silicon, score with prebuilt images
under emulation (`--namespace swebench` + `DOCKER_DEFAULT_PLATFORM=linux/amd64`);
never `--namespace none` (forces a rebuild that hits SWE-bench's bit-rotted
`setup_env.sh`). Run outputs gitignored.

---

## 2026-05-29 — Mechanical gate for outward/mutating agent commands

After an agent confabulated a "third party confirmed X" instruction inside its
own reasoning and published it as a GitHub issue comment under the user's
identity — via the honor-system `request_approval` path, with no mechanical
gate — added defense-in-depth so a fabricated or assumed instruction can't
reach an outward action. (Authored by the Brian+Rain trio in a session that
wedged on a `cargo test` turn before it could commit; verified — `cargo test`
+ release build clean, 314/314 — and landed by the maintenance operator.)

- **Anti-confabulation rule baked into `GENERAL_RULES`**
  (`src/agents/general_rules.rs`), not just the deletable
  `custom-general-rules.md`: ground every action in real inputs (actual user
  messages + actual tool results); never publish a claim about what a third
  party said/did without a verbatim in-session source; outward actions under
  the user's identity need a real in-session instruction + an approval gate.
- **PreToolUse `tool-blocklist` hook injected at spawn for HANDS/Emma**
  (`src/agents/spawn.rs` → `run_tool_blocklist` in `src/policy/hooks.rs`). They
  run `--dangerously-skip-permissions`, where claude-code SILENTLY IGNORES a
  JSON `{"decision":"deny"}` result — so the gate blocks via **exit code 2** (a
  "blocking error" honored under bypass), matching the project `tool_blocklist`
  against the Bash command before it runs. Fail-open on parse/IO error.
  Injected via `--settings` (a process arg) so nothing lands in the working
  tree. Rain is exempt — `--bare` skips hooks and she is already read-only.
- **`approval-gate.js` corrected** to whole-command prefix matching
  (`startsWith` on the trimmed command, same semantics as the Rust gate) —
  prior substring/`&&`-split versions over-blocked commands that merely
  *mentioned* a pattern (`echo "git push"`). Note: the JS hook uses
  claude-code's JSON-deny form, a no-op under bypass — the Rust exit-2 hook is
  the real gate for the trio; the JS hook backstops interactive sessions.

+9 tests (314 total: 265 lib + 49 integration).

## 2026-05-29 — Context Library view rework (post-migration UX fixes)

The Tauri v2 migration left the Context Library tab with several regressions vs
the old Slint UI. Brian + Rain triaged all five user-reported issues (plus
extras) and shipped four batches, each holding all five gates (cargo test,
release build, tsc, vitest, vite build):

- `fix: make Context Library files editable with save guards` (`49ff094`) —
  files were read-only (no `cl_write_file`). Added the command (sharing
  `cl_read_file`'s path-traversal guard via a new `resolve_existing_cl_file`
  helper), made the editor a real textarea with dirty tracking + a single
  primary Save, demoted the description editor to a secondary "Metadata" action
  (killing the duplicate-Save-button confusion), and added a `binary` flag so
  non-UTF-8 / truncated files stay read-only (can't be corrupted on save).
  Renamed the sidebar header "WORKSPACE" → "Library Tree".
- `add: Context Library recursive folder tree and folder-view` (`0869fe4`) —
  the tree was a flat per-project file list; rebuilt it as nested collapsible
  folders (`buildTree`). A folder click now toggles collapse AND opens a
  folder-view tab that edits the folder's description/tags
  (`cl_set_folder_description`). `OpenTab` became a file|folder union.
- `add: register and unregister Context Library projects from the UI`
  (`d43ce20`) — a sidebar "Register project" modal promotes an arbitrary
  on-disk folder to a project (`cl_register_project`, path validated as a real
  dir); the project-root folder-view configures the working-repo + soft-
  unregisters (`cl_unregister_project`). `cl_path` added to `ProjectView`.
- `add: Context Library right-click menu and file/folder disk ops` (`b2d1a6c`)
  — VSCode-style right-click: new file, new folder, rename, delete
  (`cl_create_file` / `cl_mkdir` / `cl_rename` / `cl_delete_path`, all path-
  guarded; delete is confirm-gated). Each op runs `cl_rescan` to resync the index.

Net: 15 files, +2076/−134. The Rust side stayed within the existing thin
`#[tauri::command]`-over-storage/bridge pattern. Deferred: native folder picker
(text-input path for now — needs the Tauri dialog plugin), rename re-derives
descriptions, hard delete (no OS trash).

## 2026-05-29 — round 6 refactor sweep (docs + plugin-module organization)

Another maintenance sweep. Brian + Rain ran parallel scans; Brian verified each
finding against the tree before applying. The codebase remains clean after round
5 (no dead code, no Slint staleness, no unused deps), so the round is small —
three commits, all zero-behavior-change:

- `docs: fix stale bridge.rs paths and test count` (`c0f5617`) — ARCHITECTURE.md
  referenced `src/signaling/bridge.rs` at two sites, but Batch 6 split it into
  the `bridge/` submodule tree; PLAN.md's test count (288) lagged the real suite
  (300).
- `refactor: extract InstalledPluginView::from_row constructor` (`267720c`) —
  `install_plugin_inner` and `list_installed_plugins_inner` built the view from
  `(row, manifest, heartbeat)` with the same `status_of(id).unwrap_or(Healthy)`
  resolution; collapse both to a `from_row` constructor. (install previously
  keyed status off `manifest.id`, list off `row.id` — equal, since the row was
  just inserted from that manifest.)
- `refactor: move PluginRegistry to plugins module` (`28de0bf`) — `PluginRegistry`
  has zero Tauri deps (wraps `plugins::Loader` + `plugins::Heartbeat` over plain
  `PathBuf`/`Mutex`). Moved the struct + impl + its three runtime tests from
  `tauri_cmd/plugins.rs` to a new `plugins/registry.rs`, re-exported as
  `crate::plugins::PluginRegistry`, so the command file holds only Tauri shims.

**Deliberately NOT done** (recorded so they aren't re-proposed): R3 — unifying
`jsonrpc::parse_optional_phase`'s error message onto `IpavPhase::error_hint()`
would be an *accuracy regression*: `error_hint()` advertises chip-form
(`I/P/A/V`), but `parse_optional_phase` accepts only lowercase full names, so the
message would tell agents to send inputs the validator rejects. A real unify
(make the two session_doc tools accept chip-form via `parse_phase_arg` +
normalize) is a behavior change beyond a polish round. F1 — moving the
`ContextLibrary*` components from `src/app/` to `src/components/` is
organizational-only with no duplication saved; deferred per the same precedent as
the Round 2 F8/F9/F10 splits. R4 — `arg_clear_on_empty` is not duplicated (single
def in `external_jsonrpc.rs`, used twice locally).

Gotcha worth carrying: removing `PathBuf` from `tauri_cmd/plugins.rs`'s top-level
imports (it lost its last non-test user when the struct moved) cleared an
`unused_import` warning in the non-test build — but the `#[cfg(test)]` helper
`write_plugin_source() -> PathBuf` still needed it. A warnings-only or
non-test-build gate masks this; only a full `cargo test` build surfaces the
`cannot find type PathBuf`. The fix is a test-module-scoped `use std::path::PathBuf;`.

300 Rust + 14 frontend tests green; release build clean.

## 2026-05-29 — round 5 refactor sweep (storage / signaling / tauri cleanups)

A maintenance sweep after the Rain fix. The codebase came back clean (zero
TODO/dead-code, no Slint staleness, docs accurate), so the round is small and
low-risk — three commits:

- `refactor: dedupe SQL column lists in storage queries` — `MESSAGE_COLUMNS` /
  `QUESTION_COLUMNS` / `DOCUMENT_COLUMNS` consts so a projection can't drift
  between the query branches / sibling methods that select the same row shape.
- `refactor: simplify signaling closures and clearable-arg parsing` — drop the
  redundant closures wrapping `internal_err_no_prefix`; extract
  `arg_clear_on_empty` for the base_url/auth_token "empty clears, absent keeps"
  parsing that `set_agent_config` repeated.
- `refactor: use ? for anyhow-sourced internal errors in tauri commands` —
  collapse 15 `.map_err(|e| AppError::Internal(e.to_string()))` sites that wrap
  bot-hq's own anyhow calls to `?` (via the existing `From<anyhow::Error>`).

**Deliberately NOT done** (recorded so they aren't re-proposed): a shared
CL-result-shape helper for `cl_index_search`/`cl_folder_search` (the two map
*different* row structs; a generic helper ≈ the duplication it removes), and
`&*TOOLS`→`&TOOLS` (would revert the deliberate Round-2 explicit-deref choice
in F13). Also scoped OUT of the tauri error sweep: `DbError`/`NotFound`/
`Validation` sites (the frontend `useInvoke` switches on kind for retry /
redirect) and `Internal(format!("ctx: {e}"))` sites over io/reqwest errors
(they add a context message `?` would drop). 300 Rust + 14 frontend tests
green; release + frontend builds clean.

## 2026-05-29 — fix: normalize role:system in Rain's gateway requests (local proxy)

The `--bare` spawn fix (`c0fa928`) for Rain's DeepSeek 400 was **insufficient**:
a fresh `--bare` Rain still 400s on a fixed `messages[11].role: unknown variant
`system``. Evidence: the live `--bare` Rain transcript logged 25 such error
turns this session (prior Rain sessions: 140, 65); every DeepSeek session errors
heavily, every Brian/Emma (real Anthropic) session ≈0.

**Root cause:** claude-code 2.1.156 injects a `SessionStart` hook's
`additionalContext` (and possibly other request-build-time context) as a
`role:"system"` entry inside the request `messages` array. It is NOT stored in
the transcript (so bot-hq can't sanitize it at the source) and `--bare` does
NOT suppress it. DeepSeek's Anthropic-compat gateway rejects `role:"system"`;
the real Anthropic API tolerates it (hence Brian/Emma are fine).

**Fix** (`src/agents/llm_proxy.rs`): a local normalizing reverse proxy. Any
agent with a custom `base_url` gets `ANTHROPIC_BASE_URL` pointed at it
(`http://127.0.0.1:<port>/<hex(real-upstream)>`); per request the proxy hoists
every `role:"system"` message out of `messages[]` into the top-level `system`
field, then forwards to the real upstream (reqwest + rustls + the `stream`
feature) and streams the SSE response straight back. Source-agnostic — it
strips the alien role regardless of which hook injected it. Brian/Emma (no
`base_url` → real Anthropic) never touch the proxy. Started at boot in
`main.rs`; address held in a process-global `OnceLock` and read by
`spawn::build_command` through the pure, unit-tested `resolve_anthropic_base_url`
helper. `--bare` is retained as defense-in-depth + Rain leanness; the misleading
spawn.rs comment claiming it fixes the 400 was corrected.

+11 tests (hex round-trip, base-url resolution, body normalization across
string/array/absent `system` shapes, and an end-to-end test asserting a body
that would 400 on a strict gateway returns 200 through the proxy with
`role:system` stripped and hoisted). 300 Rust tests green; release + frontend
builds clean.

**Live confirmation pending:** the fix only takes effect after bot-hq is rebuilt
+ restarted — the running instance keeps the old binary, so the current Rain
keeps 400ing until then. Rebuilding the binary does not disrupt the running
process (the running image keeps its old inode).

## 2026-05-29 — fix: break agent API-error spam loop (turn-failure signal)

A Rain session resumed on a pre-`--bare` (contaminated) transcript 400s on
*every* turn (DeepSeek rejects the injected `system`-role message). claude-code
emits that "API Error: 400…" as an assistant **text** block, which bot-hq
peer-forwarded to the other agent; the peer replied, that re-triggered the
failing agent, and the volley looped unbounded — burning tokens with zero user
input. (Same family as the idle-volley heartbeat loop fixed in `79114bf`, but
the error text is long + non-ack so the heartbeat breaker didn't catch it.)

**Root cause:** bot-hq discarded the turn-failure signal. claude-code's `result`
event carries `is_error` / `api_error_status`, but `ResultEvent`
(`agents/protocol.rs`) never parsed them — so a failed turn looked identical to
a successful one and its text was peer-forwarded like any prose.

**Fix** (`83c72f7`): parse `is_error` + `api_error_status` on `ResultEvent`;
propagate `is_error` onto `AgentEvent::TurnComplete` (`spawn.rs` + `events.rs`,
derived as `is_error || api_error_status.is_some()` — deliberately *not* from a
non-`success` subtype alone, to avoid false-positive suppression of legit
turns); in `core/duo.rs::pump_agent`, a failed turn drains its buffer WITHOUT
peer-forwarding (the error stays in the agent's own transcript for UI
visibility). +4 tests (1 duo: errored turn not forwarded; 3 events: error/
api_error_status/success derivation). 240 lib tests green (288 total).

**Known limit:** forward-looking — does NOT heal an already-contaminated
transcript. A resumed pre-fix Rain still 400s; restart her for a clean session.
This stops the loop/spam; it does not recover the agent.

## 2026-05-29 — fix: Rain spawns `--bare` (DeepSeek 400 after claude 2.1.156)

After upgrading claude-code to **2.1.156**, Rain (EYES — routed to DeepSeek
via `ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic`) began failing
*every* turn with `API Error: 400 ... messages[1].role: unknown variant
`system``. Brian + Emma (real Anthropic API) were unaffected.

**Root cause:** claude-code ≥ 2.1.156 serializes a `SessionStart` hook's
`additionalContext` — the user's global **superpowers** plugin injects one —
as a `role:"system"` entry *inside* the request's `messages` array. The real
Anthropic API tolerates that; DeepSeek's Anthropic-compatible gateway only
accepts `user`/`assistant` roles and rejects it. Captured + diffed the raw
HTTP body across versions: **2.1.153 → `messages:[user]`** (clean);
**2.1.156 → `messages:[user, system]`** (broken). bot-hq builds none of this
body — it's claude-code reacting to a globally-installed plugin hook.

**Fix** (`src/agents/spawn.rs`): spawn Rain's subprocess with `--bare`
(minimal mode — skips plugin sync, so the offending hook never loads and the
body stays clean). Verified end-to-end against the *real* DeepSeek gateway
with Rain's actual token: identical flags, `--bare` turns the 400 into a clean
reply. `--bare` still honors `--mcp-config` (signaling) and the
`ANTHROPIC_AUTH_TOKEN` bearer header. Scoped to Rain; Brian/Emma keep
CLAUDE.md autodiscovery + LSP. +1 test (`rain_gets_bare_minimal_mode`); 236
lib tests green.

**Known caveat:** `--bare` prevents *new* contamination but does NOT heal
transcripts written before the fix — every existing Rain transcript already
has the superpowers attachment baked in, so **resuming a pre-fix session still
400s**. New sessions are clean. Heal-existing options if needed: start fresh,
sanitize the stored `.jsonl` transcripts, or front DeepSeek with a
system-message-normalizing proxy.

## 2026-05-28 — post-rebuild cleanup (7 batches)

A cleanup pass after the Tauri v2 migration: a tray-delivery bug fix,
doc/CL reconciliation, and four pure refactors (zero behavior change).
Batches 1–5 shipped first; batches 6–7 (the two big module splits) were
deferred to a clean context window and landed last.

- **Batch 1** (`8dd3198`) — fix: route UI `resolve_choice` through core so a
  tray answer arriving after an MCP client-timeout still wakes the agent
  (the `AgentReceiverDroppedFellBack` stdin-injection path).
- **Batch 2** (`28db6d9`) — docs: correct MCP tool counts (26 internal /
  21 external), drop stale Slint references, archive spent CL files.
- **Batch 3** (`99530db`) — refactor: `once_cell` → std `LazyLock`/`OnceLock`.
- **Batch 4** (`0cc5ab8`) — refactor: split `ContextLibrary.tsx` into shell
  + sidebar + editor + shared modules.
- **Batch 5** (`c24a4b2`) — refactor: extract shared `webview_*` JS builders
  into `signaling/webview_js.rs` (+3 tests → 283 baseline).
- **Batch 6** (`8118247`) — refactor: split `signaling/bridge.rs` (1965 LOC)
  into a `bridge/` directory — `mod.rs` (types + struct + constructors +
  session/policy/event-bus methods), `questions.rs`, `permissions.rs`,
  `cl_facade.rs`, `session_docs.rs`, `util.rs`. Each submodule carries its
  own `impl SignalingBridge` block; the `pub use bridge::{…}` re-exports are
  unchanged. Cross-sibling private fns bumped to `pub(super)`; private
  fields stay private (submodules are descendants of the bridge module).
- **Batch 7** (`5d1da96`) — refactor: split `storage/mod.rs` (1197 LOC) into
  per-table submodules (`sessions`, `messages`, `agent_config`, `questions`,
  `projects`, `cl_index`, `session_docs`, `plugins`); `mod.rs` keeps the
  `Storage` struct, `open`/`memory`/`pool`, and the shared `cl_search_table`
  generic. No visibility bumps — every query method is `pub` and
  `cl_search_table` stays a private parent method reachable from descendants.

Batches 6–7 are pure file-splits: 283 Rust tests + 14 frontend Vitest stay
green, release build clean, after each commit.

---

## 2026-05-26 — Tauri v2 migration landed (7 batches)

After the design doc (`docs/plans/2026-05-26-tauri-v2-migration-design.md`,
committed at `7d5d400` + `a9c0abf` on main) and a Plan-phase correction
to the Batch 1 BatchEmitter design (event-triggered batch fetch via the
existing `messages_for_session(since_id)` query, not content-pushing —
the bridge is zero-delta), the migration shipped across 7 batches on
branch `tauri-v2-migration`:

- **Batch 0** (`eba536e` + `83d4ca7` + `3f39ce2`) — Tauri v2 + Vite +
  React 18 + Tailwind + Vitest foundation. `tauri-specta` smoke-tested
  with empty command set; frontend smoke test renders.
- **Batch 1** (`6bc81ee`) — Tauri events layer. `src/tauri_events/`
  with `BatchEmitter` (since_id watermark, N=20 / 50ms coalesce) +
  `bridge_subscriber` routing `SignalingEvent` variants to typed Tauri
  emits. 12 new tests.
- **Batch 2** (`1579eb7`) — Tauri command layer. `src/tauri_cmd/` with
  19 commands across sessions / messages / agent_configs / cl / policy /
  questions / docs domains + `AppError` enum + view types. tauri-specta
  exports to TypeScript with i64 → number bigint behavior.
- **Batch 3** (`30432d4`) — Plugin module scaffolding. `src/plugins/`
  with manifest parser (strict id validation), loader, per-plugin
  capability JSON generator (`https://plugin-<id>.localhost/*`),
  heartbeat watcher (3-strike model). 25 new tests including the
  design-doc coverage-gap (dummy iframe origin chain).
- **Batch 4** (`6aa9f1e`) — main.rs Tauri bootstrap. Slint event loop
  out, `tauri::Builder` in. Tokio multi-thread on workers, Tauri on OS
  main thread. All existing setup (CLI dispatch, panic hook, child
  reaper, signal task, MCP servers, Emma auto-spawn, CL init,
  tauri-specta TS export) preserved verbatim. Bridge subscriber wired in
  Tauri `setup()`.
- **Batch 5** (`84cddb4`) — React frontend. App shell + 5 routes
  (Dashboard, SessionView, Settings, ContextLibrary, PluginManager) +
  Emma overlay. shadcn-style minimal primitives by hand. Zustand stores
  (chat watermark dedupe), TanStack Query hooks (`useTauriQuery`,
  `useTauriMutation`), `useTauriEvent` wrapper. 12 Vitest passing.
- **Batch 6** (`8dbb03d`) — Slint removal. Deleted `src/ui/`, `ui/`,
  dropped `slint` + `slint-build` deps. Updated `ARCHITECTURE.md` +
  `CLAUDE.md` to reflect the new UI. -11,875 LOC across the diff
  (Cargo.lock shed Slint's transitive dep tree).

**Zero-delta verified:** `src/agents/`, `src/core/`, `src/policy/`,
`src/storage/`, `src/signaling/` untouched through every commit. The
Rust core's 202 baseline tests (now 253 with new Tauri layer tests)
stay green at each batch boundary.

**Path A locked** for force-flush on turn-end: design doc's
`SignalingEvent::TurnEnded` variant deferred (would be ~10 LOC core
delta). Accepting ≤50ms tail latency at turn-end as the cost of true
zero-delta. Revisit only if profiling shows perceived lag.

**Push grant:** session-level `scope=specific`, `branches=["tauri-v2-migration"]`
granted at start of Apply phase. Each batch pushed without per-action
prompt; main branch protections unaffected.

**Open items deferred:**

- `broadcast_to_session` Tauri command — `ChatInput` callbacks wired but
  inert until a `core::broadcast` helper lands.
- Live `compute_apply_diff` rendering in the A tab — port
  `view_model::parse_diff_lines` to a Rust-side command + frontend
  renderer.
- Plugin install flow + heartbeat ping/pong frontend channel.
- Real bot-hq app icon (current `icons/icon.png` is a 32×32 placeholder).
- Manual smoke checklist run-through (new-session → agent streams →
  Emma overlay → IPAV tabs → close).
- CL doc updates (`~/.bot-hq/projects/bot-hq/conventions.md` + `notes.md`)
  to drop Slint references — deferred until merge to main since the CL
  is shared across sessions.

**Reference:** Elves (mvmcode.github.io/elves) — Tauri v2 + sqlite + PTY
+ AI agents, validates the architecture in the same domain.

---

## 2026-05-26 — Tauri v2 migration decided (big-bang)

After ~28% of recent commits going to Slint layout fixes and the planned
plugin roadmap (Discord, Clive, themes, future UI-mutation plugins) being
structurally hostile to Slint's compile-time component model, the user +
Brian + Rain brainstormed a migration to Tauri v2 + React. All four
anchors validated through `ask_user_choice` gates:

1. **Migration shape:** Big-bang — branch off main, focused UI-shell
   rebuild, no parallel Slint maintenance.
2. **Frontend stack:** React 18 + TypeScript + Tailwind + shadcn/ui
   (Vite build).
3. **Plugin model:** Slot-extend + custom panels via iframes (per-plugin
   origin via Tauri custom URI scheme + capability JSON). Defer full
   UI-mutation tier.
4. **IPC architecture:** Tauri-native. All React↔Rust via Tauri commands
   + Tauri events. No HTTP from frontend.

**Operating principle locked:** HTTP only where protocol mandates it.
External agent driver server stays HTTP. Internal MCP server (HTTP
localhost) stays — that's claude-code's MCP transport contract.
Everything else is Tauri IPC.

**What's preserved:** Entire Rust core (`src/agents/`, `src/core/`,
`src/policy/`, `src/storage/`, `src/signaling/`, `SignalingBridge`,
session permissions, sqlite schema, all 19+16 MCP tool implementations).
~12,000 LOC zero-delta. The 202 existing tests are the migration's
regression baseline.

**What's getting replaced:** ~6,700 LOC of Slint+view_model
(`ui/app.slint` + `src/ui/view_model.rs`) → ~3,000–5,000 LOC React
frontend + ~500–1,000 LOC thin Tauri command layer + new plugin module.

**Canonical blueprint:** `docs/plans/2026-05-26-tauri-v2-migration-design.md`
(committed `7d5d400` + `a9c0abf`). All five design sections (architecture
/ components / data flow / error handling / testing) user-validated
through structured `ask_user_choice` gates. Rain's 8 review flags all
incorporated as section content or addenda. Session brainstorm artifact
preserved as session doc `brainstorm-tauri-migration` (phase=investigate).

**Status:** Plan-phase output complete. Awaiting fresh-session
implementation handoff (worktree off main + `superpowers:writing-plans`
+ `superpowers:executing-plans`).

**Reference:** Elves (https://mvmcode.github.io/elves/) — Tauri v2 +
sqlite + PTY + AI agents, Homebrew-installable. Validates the exact
domain.

---

## 2026-05-24 — IPAV pills become document tabs (10-batch implementation)

User-requested redesign of the session view: the I/P/A/V pills no longer
advance the IPAV phase (agents do that via the `advance_phase` MCP tool —
two sources of truth was a latent bug). Instead the pills are document-
tab selectors driving a new right-pane DocumentPane in an always-visible
60/40 split (Chat left ~60%, Documents right ~40%). User-decided layout
over Brian+Rain's drawer-toggle recommendation.

**Data model**: `session_documents` gains a nullable `phase` TEXT column
(values `investigate`/`plan`/`apply`/`verify`) via `migrations/0008_
session_documents_phase.sql`. Existing rows pass through as NULL —
invisible to tabs + phase-filtered searches. The `session_doc_write` and
`session_doc_search` MCP tool descriptors gain optional `phase` enum
params + dispatch-layer validation. Agents tag plans/findings/etc. and
retrieve cross-phase context via `session_doc_search(phase="plan")`
instead of scrolling chat history. Hardcoded agent prompts updated in
`prompts.rs:72` + `general_rules.rs:63,83` so the pattern is discoverable.

**Apply tab — git diff path**: the in-memory `SessionHandle.session_
start_sha` (new field) captures `git rev-parse HEAD` via `spawn_blocking`
at session spawn. The view's `compute_apply_diff` runs `git diff --no-
color <sha>` (one-arg form covers committed + staged + unstaged in one
shot — `git diff HEAD` alone is empty right after commits land, which
is the moment the user wants to inspect what just shipped). Fallback
chain: SHA-diff → `git diff HEAD` with anchor-lost note → latest
`phase='apply'` session doc → empty state. No schema column for the SHA;
in-memory is enough since live session state already resets on app
restart.

**Slint changes**: `AppState.advance-phase` callback + the `on_advance_
phase` handler in `view_model.rs` fully stripped (Liars That Compile —
leaving dead callbacks invites future re-wiring that reintroduces the
bug). New `select-doc-tab` callback + `selected-doc-tab` property +
five `active-doc-*` properties (content/slug/updated-at/count/empty-msg).
PhasePill rewritten: top-border accent on selected tab (keeps per-phase
`tint` color), monochrome text. SessionView outer `VerticalLayout` now
wraps the chat + DocumentPane in a `HorizontalLayout` with `horizontal-
stretch: 1.5` / `1`. PhaseSelector relocated from session header to the
DocumentPane header. LabelChip remains the sole phase indicator.

**View-model wiring**: new `refresh_session_docs` async helper (called
both from the 500ms poll loop and the immediate tab-click handler);
new `compute_apply_diff` helper; new `current_selected_doc_tab_async`
+ `push_doc_pane_state` utility. "N more" chip surfaces in the
DocumentPane header when the active tab has >1 phase-tagged doc;
expansion UI deferred per YAGNI.

**Verification**: `cargo build` clean (dev + release), 202 tests pass
(was 196 → +6 from 2 storage phase tests + 3 MCP phase tests + 1 round-
trip). 11 files modified, 1 new migration. Diff stat: +714 / -113.
Visual smoke (60/40 split renders, tabs switch, doc loads from session_
documents, git diff appears in A tab after agent commits) is the user's
gate — bot-hq's desktop nature precludes automated UI testing.

---

## 2026-05-22 — Audit Round 4 cleanup (11 findings landed)

Brian + Rain adversarial sweep of the post-Round-2/3 codebase using
`~/.bot-hq/projects/slint-rust-docs/` Tier 1/2/3 as the Rust+Slint
reference. Two independent passes (session docs `findings-fresh-sweep-2026-05-22`
+ `findings-rain-sweep-2026-05-22`); 11 findings consolidated; all
shipped per the verified commit order.

**Landed (in order):**

- **C1 — `c54a8ea` (N7, real bug)** — main.rs's shutdown-signal
  `tokio::select!` had no `else` arm. If all three `signal()`
  registrations failed (non-Unix host, container without signal
  support) the select panics ("all branches disabled"). Added a
  `future::pending()` arm that parks the task — children still get
  reaped via the panic-hook path.
- **C2 — `99ffd62` (N8, lint)** — `panic_payload_string(&Box<dyn Any>)`
  → `&(dyn Any)`. clippy::borrowed_box. 2 lines.
- **C3 — `1c3a103` (N10, doc)** — PLAN.md said "165 tests passing";
  actual is 196.
- **C4 — `57d80e7` (N2, dispatch helper)** — `IpavPhase::parse + same
  error_hint format!` was triplicated across jsonrpc.rs:155, :174,
  external_jsonrpc.rs:387 — each with the same shape but different
  wire field names ("target" vs "phase"). Extracted
  `protocol::parse_phase_arg(field, value)` preserving the
  wire-compatible error string via the `field` param. Dropped the
  now-unused `IpavPhase` import in external_jsonrpc.rs. Net -3 LOC.
- **C5 — `303db61` (N1, response helper)** — `result_json(&json!({"ok":true}), "{}")`
  was repeated 6× in external_jsonrpc.rs as the standard "operation
  succeeded" payload. Extracted `ok_response()` next to `result_json`
  in response.rs.
- **C6 — `57fbd6d` (N3, error helper)** — F6 added file-private
  `internal_err(op, e)` to external_jsonrpc.rs but jsonrpc.rs still
  had 15× `map_err(|e| JsonRpcError::new(INTERNAL_ERROR, e.to_string()))`
  (no-op-prefix shape). Lifted `internal_err` into response.rs; added
  `internal_err_no_prefix` sibling; replaced all 15 sites with
  `.map_err(internal_err_no_prefix)?`.
- **C7 — `b9e1cb0` (N6, consistency)** — `on_set_session_permission`
  inlined a 4-line `weak.upgrade().map(...)` block instead of calling
  the existing `current_session_id(&weak)` helper (used by
  `on_advance_phase` + `on_broadcast`). Dropped the inline form.
- **C8 — `40f7868` (N5, bridge dedupe)** — `resolve_policy_for` and
  `audit_policy_files_for_session` had identical 12-line
  project→project_root resolution chains. Extracted private
  `resolve_project_and_root(data_dir, sid)` returning
  `(Option<String>, Option<PathBuf>)`. Both callers collapse to one
  line.
- **C9 — `0172ba4` (N4, bridge dedupe)** — `grant_session_permission`,
  `revoke_session_permission`, and `add_branch_to_session_grant` all
  replicated the same lock→entry-or-default→mutate→snapshot→drop→mirror
  sequence (~14 lines each). Extracted `mutate_session_permission(sid, FnOnce)`;
  each caller reduces to its one-line mutation closure. Side-effect:
  the mirror-write side can't be forgotten in a future variant.
- **C10 — `bea60bd` (R4-F1, real failure-mode fix)** — `catch_unwind`
  (ffi_safe) prevents Slint-callback panics from aborting, but does
  NOT clear a poisoned `Mutex`. Before C10: first panic inside e.g.
  `ANSWER_ACCUMULATOR.lock()` poisoned the mutex; every subsequent
  tray interaction re-locked → `.unwrap()` → panic → caught → toast-
  spam until session restart. Replaced `.lock().unwrap()` /
  `.lock().expect()` with `.lock().unwrap_or_else(|p| p.into_inner())`
  at all 8 sites (5× view_model.rs, 3× spawn.rs). This is the
  hardening pass logged in decisions.md (2026-05-22) as deferred.
- **C11 — `cd3a6b8` (R4-F2, paths dedupe)** — `directories::BaseDirs::new().context("locating user home dir")?.home_dir().to_path_buf()`
  was repeated 3× in paths.rs. Extracted `home_dir() -> Result<PathBuf>`;
  all three sites reduce to one line. Net -2 LOC.

**Wire compatibility:** every refactor preserved exact existing wire
error strings and tool-call result shapes. C1 + C10 are the only
commits with behavior changes (both eliminating failure modes that
previously aborted or toast-spammed the daemon).

**Round 4 metrics:** 11 commits, 12 files touched, ~41 duplication
sites collapsed into 6 new shared helpers (`parse_phase_arg`,
`ok_response`, `internal_err_no_prefix`, `resolve_project_and_root`,
`mutate_session_permission`, `home_dir`). Net +12 LOC because each
helper carries a load-bearing docstring; the raw repetition is gone.
196 tests still pass.

**Deferred from Round 2/3 still deferred:** view_model.rs (3,038 LOC),
bridge.rs (1,841 LOC), storage/mod.rs (960 LOC), app.slint (3,846 LOC)
splits — all organizational. Re-open when actively painful.

---

## 2026-05-22 — Audit Round 3 cleanup (S4, S1, S5 landed)

Acted on `findings-slint-rust-audit` (session doc) — Brian + Rain
adversarial audit of the codebase against
`~/.bot-hq/projects/slint-rust-docs/` Tier 1/2 reference docs.
Five findings produced; three actionable (S4 LOW, S1 HIGH, S5 LOW)
shipped; two (S2, S3) deferred organizationally per the same precedent
that deferred Round 2's F8/F9.

**Landed:**

- **S4 — `41ef278`** — `build.rs` was using Slint's default
  `std-widgets` style, which is platform-dependent (fluent on Windows,
  qt on Linux, native on macOS), so widget chrome (LineEdit /
  ScrollView / TextEdit focus rings, scrollbar handles, input borders)
  drifted across builds even though the rest of the app paints from
  the Theme global. Switched to `compile_with_config(..., with_style(
  "material"))` to match the app.slint header's stated "Material 3
  dark theme".
- **S1 — `a88bc0a`** — `view_model.rs` used the LLM anti-pattern
  `slint::invoke_from_event_loop(move || { if let Some(handle) =
  weak.upgrade() {...} })` at 38 call sites — exactly what
  `slint-rust-docs/patterns/weak-handle.md` calls out as duplicating
  what `Weak::upgrade_in_event_loop` packages. Migrated all 38 sites
  to the canonical primitive (closure receives the upgraded handle,
  silently skips if the component dropped). Two edge cases handled per
  the audit spec: TreeState init moved inside the new closure body;
  `current_session_id_async`'s oneshot dropped the explicit empty-send
  branch (the receiver's `rx.await.unwrap_or_default()` covers tx-drop
  identically). Also updated 4 doc/inline comments naming the old
  primitive. Net -126 LOC (view_model.rs: 2966 → 2840).
- **S5 — `bfbde16`** — `AppState` global in `ui/app.slint` defaulted
  `in-out property` for one-way Rust-pushed values. Per
  `slint-rust-docs/conventions/slint-syntax-for-rust.md` + 
  `patterns/globals.md`, `in-out` should be justified, not the default.
  Verified each property by grepping for `.slint`-side writes
  (`AppState.foo = ...`, `<=>` two-way binds). Converted 33 to
  `in property`; kept 27 as `in-out` where TextEdit/LineEdit `<=>`
  binds, UI click-toggles, modal state, or drag-resize legitimately
  write from the `.slint` side. Three audit-table corrections caught
  during the grep pass — `cl-dirty`, `cl-metadata-dirty`, and
  `external-mcp-token-revealed` ARE UI-written (initial table was
  wrong) and stayed `in-out`.

**Deferred:**

- **S2 — persistent `Rc<VecModel<ChatMsg>>` with incremental
  mutation.** Reference pattern in `slint-rust-docs/patterns/models.md`.
  Current behavior uses fresh `ModelRc::new(VecModel::from(rows))` per
  poll with a `MSG_FINGERPRINTS` cache to short-circuit identical
  refreshes. The fingerprint workaround is load-bearing and correct;
  the canonical pattern is perf+correctness polish, not a fix. Re-open
  if rebuild churn surfaces in profiling or selection-loss appears as
  a UX complaint.
- **S3 — split `ui/app.slint`** (3846 LOC) into conventional
  `ui/{theme,types,components/,views/,main}.slint` layout per
  `slint-rust-docs/conventions/project-structure.md`. Organizational,
  not correctness — same conclusion Round 2 reached for F8/F9. Re-open
  if the mono-file becomes painful to edit / merge.

**What was already correct (no change needed):** Tokio/Slint event-loop
boundary (multi-thread Tokio + Slint on main thread, matches "Fix (a)"
in `pitfalls/tokio-event-loop-conflict.md`), zero `clone_strong()`
usage, correct weak-handle capture in all callbacks, correct
`export component AppWindow` shape, no `set_X(format!()).into()`
allocation anti-patterns on hot paths.

---

## 2026-05-21 — Audit Round 2 cleanup (F12, F2, F1, F5, F11, F6, F13, F4 landed)

Acted on `~/.bot-hq/projects/bot-hq/investigations/audit-round-2-2026-05-21.md`
— the Brian+Rain adversarial codebase audit produced earlier in the
session. Seven findings shipped, one remains queued.

**Landed:**

- **F12 — `05249b8`** — `request_phase_advance` used a hardcoded
  `matches!()` against full names, rejecting chip-form targets while
  `advance_phase` accepted both via `IpavPhase::parse`. Real behavioral
  bug — `request_phase_advance(target="I")` returned INVALID_PARAMS.
  Same SSOT issue in `view_model.rs:250-255` (manual chip-to-phase
  reimplementation). Added `IpavPhase::error_hint()` so internal +
  external MCP dispatch quote the canonical
  `"I/P/A/V or Investigate/Plan/Apply/Verify"` string instead of three
  divergent ones. Two regression tests lock in chip-form acceptance.
- **F2 — `ac4db22`** — `PROTOCOL_VERSION` was duplicated in
  `external_jsonrpc.rs:21` alongside the public const in
  `protocol.rs:11`. Silent-desync risk on MCP version bumps. Deleted
  the local copy; imported the public const.
- **F1 — `39efd51`** — `result_json()` helper from `jsonrpc.rs:108`
  was never propagated to `external_jsonrpc.rs`, which inlined the same
  `serde_json::to_string(...).unwrap_or_default()` shape at 16 call
  sites. Lifted the helper into `signaling/response.rs` as
  `pub(super)`; replaced all 16 sites. Net -26 LOC. Intentional
  behavior diff: serialize failures now return `"{}"` instead of
  `""` — valid JSON shape, matches the existing internal pattern.
- **F5 — `5e46844`** — `Message → json!({...})` projection was
  copy-pasted 4× across `external_jsonrpc.rs`
  (`get_session_messages`, `get_emma_messages`, `wait_for_change`,
  `get_session_snapshot`). Extracted file-private
  `message_to_json(&Message) -> Value` near the top; all 4 sites
  collapsed to `.iter().map(message_to_json).collect()`. Switched
  `.into_iter()` → `.iter()` per-site after verifying none reuse the
  source vec. Internal `jsonrpc.rs` has zero matching sites — F5 is
  external-only. Net -22 LOC. Same 5-field shape preserved;
  `session_id` stays dropped (DB-only, not MCP view).
- **F11 — `6a423c9`** — `SignalingBridge` had 3 constructors
  (`new` / `with_violations_log` / `with_policy`) each copy-pasting
  the same 9-field `Arc::new(Self {...})` struct literal, differing
  only in `violations: Option<ViolationsLog>` and
  `data_dir: Option<PathBuf>`. Added private
  `new_with(Option, Option)` containing the single struct-literal
  build; collapsed the 3 public fns to thin wrappers. Zero call-site
  changes across the ~41 callers (1 prod in `main.rs:59`, ~40 in
  tests). Doc comments preserved on the public wrappers. Net -13 LOC.
- **F6 — `8ef5203`** — `JsonRpcError::new(INTERNAL_ERROR,
  format!("op: {e}"))` was repeated 16× across `external_jsonrpc.rs`
  (audit counted 8 single-line sites; rediscovered 8 more in 4-line
  rustfmt-wrapped form at deeper nesting). Added file-private
  `internal_err(op: &str, e: impl Display) -> JsonRpcError`. Each
  multi-line site collapses 4 lines → 1; single-line sites get
  shorter. Internal `jsonrpc.rs` uses a different shape
  (`e.to_string()`, no op prefix) — helper stays external-only. One
  static-message site (line 558, "violations log not configured...")
  left untouched as it doesn't fit the helper signature. Net -20 LOC.
- **F13 — `136e924`** — `tool_descriptors()` (19 internal tools,
  `protocol.rs`) and `external_tool_descriptors()` (16 external
  tools, `external_jsonrpc.rs`) rebuilt their full
  `Vec<ToolDescriptor>` — including all the `serde_json::json!`
  schema trees — on every MCP `tools/list` handshake. Wrapped each
  in `static LazyLock<Vec<ToolDescriptor>>`, returning
  `&'static [ToolDescriptor]`. Three caller sites updated (drop
  `: Vec<_>` annotation; slice serializes through `json!` the same
  as the owned Vec). Rain caught that `&TOOLS` would lean on a
  multi-step `Deref` coercion — switched to explicit `&*TOOLS`. Net
  +4 LOC; perf win is one alloc per process instead of per call.
- **F4 — `fb2deb0` (tests) + `fab33e9` (extract)** — both HTTP
  handlers (`signaling/server.rs::handle_request` and
  `external_server.rs::handle_request`) had identical body-collect →
  serde_json::from_slice → PARSE_ERROR-envelope blocks and identical
  dispatch-outcome match arms (~30 LOC each, copy-paste-divergent
  waiting to happen). Rain's gate required external HTTP smoke
  coverage of the paths first — `tests/external_mcp_test.rs` already
  exercised the full HTTP stack but neither parse-error nor
  202-ACCEPTED were covered explicitly. First commit (`fb2deb0`)
  added 4 tests pinning those contracts on both servers; second
  commit (`fab33e9`) extracted `decode_jsonrpc_body(Incoming) ->
  Result<JsonRpcRequest, Response>` and `dispatch_outcome_to_response
  (outcome, id_for_err) -> Response` into `signaling/response.rs`.
  Per-server pre-dispatch logic (path parse for internal; method +
  path + bearer auth for external) and debug log lines stay in the
  callers since they carry caller-specific fields. Net: each handler
  drops ~28 LOC; response.rs gains ~50; -6 LOC overall, but the more
  meaningful win is removing the last RPC-handling drift surface
  between the two servers.

**Rejected (recorded for future re-evaluation):** F3 (generic
`dispatch_jsonrpc<F>` extraction — async closure overhead exceeds
savings), F10 (per-table storage split — import sprawl without
discoverability gain). See the audit file for re-open triggers.

**Audit round 2 complete.** Last F-series code commit `fab33e9` (F4).
F8 / F9 (view_model.rs / bridge.rs splits) remain deferred — both are
organizational preference rather than duplication; defer until either
file is actively painful or the user requests the split. Audit file
`investigations/audit-round-2-2026-05-21.md` archived as the
source-of-truth for the round.

---

## 2026-05-20 — Session permission grants (in flight)

New module `src/policy/session_permissions.rs` plus integration across
the bridge, the duo, the spawn path, and the `pre-push` git hook.

**What changed:**
- `SessionPermissions { commit: GrantScope, push: GrantScope }` with
  `None` / `AllBranches` / `Specific { branches }` scopes.
- In-memory cache on `SignalingBridge` is the source of truth; mirrored
  to `<data_dir>/.local/session-permissions/<session_id>.json` so the
  `pre-push` git hook (separate subprocess) can read it.
- All mirror files purged on bot-hq startup; per-session file deleted
  on `close_session`.
- MCP tools added: `grant_session_permission(action, scope, branches?)`,
  `revoke_session_permission(action)`, `list_session_permissions()`.
  HANDS-only — Rain (EYES) cannot call them.
- `pre-push` hook checks the mirror before the static
  `policy.push_gate.remembered_approvals` list.

**Documentation cross-refs:** ARCHITECTURE.md → Session permissions
section; README.md → Internal MCP tools + Policy enforcement.

---

## 2026-05-19..05-20 — Doc refresh

Full rewrite of canonical docs (README, ARCHITECTURE, PLAN, PROGRESS,
CLAUDE) to reflect the post-rebuild state. Original rebuild design +
roadmap + Phase 0 research archived under `docs/rebuild-archive/`.

---

## 2026-05-15 — UI redesign

Substantive frontend pass triggered by user feedback ("the UI is really
bad").

- **Single chronological chat.** Replaced the two-pane Brian/Rain split
  with one chronological column where all messages interleave by
  `created_at`. User can now see their own messages clearly.
- **Design system.** Slint `Theme` global owns colors, typography,
  spacing, radii. 4-tier background hierarchy
  (canvas → surface → elevated → overlay), 4-step font scale, 4px-base
  spacing scale. Author color coding: brian=orange, rain=purple,
  emma=green, user=blue, system=muted grey.
- **Per-surface polish.** Topbar gains brand mark + tab underline +
  Emma button distinct treatment. Dashboard title block + primary
  `+ New session` CTA + elevated session tiles with `Need input` badge
  tinting border red. Session view: rich header (title + phase subtitle
  + back link + interactive PhaseSelector segmented control); banner
  uses author-rain purple (choice) vs attention red (awaiting). Emma
  overlay: dedicated header bar + close affordance + divider.
- **CL refresh.** New files in CL appear without app restart via a 2s
  periodic poll plus a manual ↻ refresh button. Directories sort before
  files in the tree.

Files touched: `ui/app.slint` (full rewrite: 796 → 1410 lines),
`src/ui/view_model.rs` (714 → 743 lines). Tests still 56-passing at the
time, release build clean.

---

## 2026-05-15 — Post-review fixes

Follow-ups after the autonomous rebuild's READY-FOR-REVIEW state.

1. Added `/target/` to `.gitignore` (was 6.7 GB; `git add .` without
   this would have committed all build artifacts).
2. **Emma auto-spawn at startup.** Extracted `spawn_session_handle`
   helper in `src/core/session.rs`; added `spawn_existing_session` for
   sessions whose row already exists. `AppState::ensure_session_started`
   is idempotent and called for `"emma"` in `main.rs` post-core
   construction. Failure is non-fatal.
3. **Settings save persists user edits.** Replaced inline LineEdit-in-
   for-loop pattern with `AgentConfigEditor` component owning per-row
   edit state via `in-out` properties bound via `text <=>`.
4. **Per-project rules migration from legacy CL.** Distilled
   operational rules from `~/.bot-hq/projects/<project>.yaml` into the
   new minimal CL. Per-project policy gates + disguise rules captured.

---

## 2026-05-14 — Rebuild milestone (v0.1.0)

From-scratch rebuild of bot-hq landed: single Rust + Slint binary
replacing the Go daemon + tmux + MCP hub + Emma forwarder + 29-tool
surface. Built autonomously across multiple claude-code sessions per
the original rebuild plan.

**Result:** 56 tests passing, release build clean, binary launches and
runs the UI loop cleanly, full agent lifecycle implemented (subprocess
spawn, stream-json IO, sqlite storage, internal MCP server with 2
initial tools, IPAV duo coordination, Slint UI with topbar +
dashboard + session view + Emma overlay).

**Phase A complete:** rebuilt minimal CL distilled into `~/.bot-hq-dev/`
(60K, 398 lines across general-rules + 3 agent startups + 5 projects of
conventions + notes). Replaces the 860-file legacy CL.

For full phase-by-phase progress + Phase 0 research findings + sub-
agent dispatch log + initial decisions, see
[`docs/rebuild-archive/PROGRESS-through-2026-05-15.md`](docs/rebuild-archive/PROGRESS-through-2026-05-15.md).

---

## Decisions made autonomously (across the build)

Things that diverged from the original PLAN / decision-doc and shipped
that way. Captured for future reference.

1. **MCP transport: in-process HTTP, not stdio + UDS bridge.** Original
   design sketched claude-code spawning bot-hq as an MCP child process
   and bridging back via Unix-domain socket. That's two subprocesses
   per agent + ~150 LOC of IPC framing. Ship version runs a single
   in-process HTTP MCP server, per-agent `mcp-config.json` files
   pointing at unique URLs. Direct AppState access, no IPC layer.
2. **MCP server: hand-rolled JSON-RPC, not `rmcp` crate.** Phase 0
   research recommended `rmcp` 1.7.0; orchestrator chose hand-roll
   (~300 LOC at `src/signaling/{jsonrpc,server,protocol}.rs`) for
   simpler in-process transport. Drop-in `rmcp` upgrade later is
   straightforward.
3. **`claude --append-system-prompt` is a string, not a file.** Plan
   said `--append-system-prompt-file`; CLI only accepts inline
   `--append-system-prompt <prompt>`. Concatenated text passed inline.
4. **`--verbose` required with `-p --output-format stream-json`.**
   Empirically discovered. Spawn command includes it.
5. **`--dangerously-skip-permissions` set on agent spawn.** bot-hq IS
   the policy layer; claude-code's own permission prompts would
   double-gate and hang. Enforcement provided by `src/policy/` + git
   hooks.
6. **Role prompts hardcoded in `src/agents/prompts.rs`.** Not CL-loaded.
   Reasoning: role boundary (Brian writes, Rain reviews) is structural
   and must survive CL edits + custom-instruction changes.
7. **System-prompt layering with policy block at the end.** Session
   spawn concatenates: hardcoded role → CL anchor → general-rules →
   custom-instruction → policy directives. Project conventions/notes
   NOT injected — agents use `cl_index_search` + `Read` on-demand.
8. **`HANDS_ONLY_TOOLS` enforced at the JSON-RPC dispatch layer.** Rain
   is structurally blocked from `ask_user_choice`, `mark_awaiting_user`,
   `request_approval`, `grant_session_permission`,
   `revoke_session_permission`. Returns a JSON-RPC error, not a
   convention.
9. **Two-layer policy enforcement.** MCP tool calls (probabilistic
   primary path, audited via `violations.jsonl`) PLUS git hooks
   (deterministic backstop). Per DeepSeek-V4-Pro's review during the
   policy module work — single-layer enforcement would fail when
   agents' context drifted.
10. **`BOT_HQ_SESSION_ID` env var injected into agent subprocesses** so
    git-hook subprocesses (spawned by git, separate from the agent's
    subprocess) can re-resolve session-scoped state (session
    permissions in particular).
11. **External MCP token at `<data_dir>/mcp-token`** auto-generated
    (UUIDv4, 0600). Constant-time comparison via `subtle` crate. Read
    once at startup, never re-read — rotation requires restart.
12. **Slint pin: `slint = "1.16"`** (resolves to 1.16.1). MSRV 1.92 per
    Phase 0.3 research.
13. **Sessions/agent_configs tables have CHECK constraints on
    `agent_name` ∈ `{'emma','brian','rain'}`** so a typo from Settings
    UI doesn't silently create a bogus row.
14. **First-run detection key: `cl-version.txt` existence** — not data-
    dir existence (test setup creates the data-dir before binary touches
    it).

---

## How to verify the human-driven parts

```bash
cd ~/Projects/bot-hq
cp .env.example .env             # already contains BOT_HQ_DATA_DIR=~/.bot-hq-dev/
cargo run --release

# In the window:
#   - Click "+ New session" on the Dashboard.
#   - Type a small task in the broadcast prompt bar.
#   - Watch Brian + Rain stream. Click the I/P/A/V chips to advance phase.
#   - If an agent calls ask_user_choice, choice buttons should appear inline.
#   - Toggle the Emma button (top-right) — half-pane chat slides in.

# Richer logs:
#   RUST_LOG=trace cargo run --release
```
