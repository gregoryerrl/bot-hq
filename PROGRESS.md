# bot-hq — Change Log

Recent work, newest-first. For the rebuild-era phased status (Phases
A–9 of the from-scratch rebuild), see
[`docs/rebuild-archive/PROGRESS-through-2026-05-15.md`](docs/rebuild-archive/PROGRESS-through-2026-05-15.md).

For what bot-hq IS see [`ARCHITECTURE.md`](ARCHITECTURE.md). For what's
planned next see [`PLAN.md`](PLAN.md).

---

## Current state

288 tests passing (240 lib + 31 external MCP + 7 signaling + 10 storage)
plus 14 frontend Vitest. Release build clean. **Tauri v2 migration landed
2026-05-26** on branch `tauri-v2-migration` (7 batches across foundation
→ Slint removal). Slint UI deleted (-7,560 LOC); React frontend in
`frontend/` (~3,000 LOC); zero LOC delta in `src/agents/`, `src/core/`,
`src/policy/`, `src/storage/`, `src/signaling/` per the design-doc
constraint.

---

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
