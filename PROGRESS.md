# bot-hq Rebuild — Progress

**Status:** ✅ READY FOR HUMAN REVIEW — autonomous build complete + post-review fixes + UI redesign applied.
**Last updated:** 2026-05-15 (UI redesign session)
**Commit message draft:** [`docs/commit-message-draft.md`](docs/commit-message-draft.md)

## UI redesign (2026-05-15)

Substantive frontend pass after user feedback ("the UI is really bad"). Tests still 56-passing, release build clean, zero warnings.

### Session view — single chronological chat (user-requested)

Replaced the two-pane Brian/Rain split with a single column where all messages interleave by `created_at`. User can now see their own messages clearly. Plumbing changes:
- New `session-msgs: [ChatMsg]` global property on `AppState`.
- Removed `brian-msgs` + `rain-msgs` (and the `split_by_author` helper they fed).
- `refresh_session_view` in `src/ui/view_model.rs` now emits one chronological projection (`to_chat_data` over `storage.messages_for_session`).
- `MessageBubble` is the single rendering primitive: user bubbles right-align in accent-soft color; agent bubbles left-align in elevated surface; phase_change events render as a centered muted italic system line — visually distinct without crowding the feed.

### Design system

Defined a coherent set of design tokens via a Slint `Theme` global (one source of truth for colors, typography, spacing, radii). 4-tier background hierarchy (canvas → surface → elevated → overlay), 4-step font scale (xs/sm/md/lg/xl), 4px-base spacing scale (s-1..s-6), two font weights. The old hodgepodge of `#0e1118 / #161a24 / #1a1f2c / #1b243a` is gone.

Author color coding: brian = orange, rain = purple, emma = green, user = blue, system = muted grey. Phase chips are colored consistently (I blue, P green, A orange, V purple) and the new `PhaseSelector` in the session header is an interactive segmented control with the current phase highlighted.

### Per-surface improvements

- **Topbar:** brand mark + tab underline accent for selected tab + Emma button visually distinct (presence dot, accent border, never receives the tab-selection treatment).
- **Dashboard:** title block with session count subtitle; primary `+ New session` button styled as a real CTA (filled accent, prominent placement); session tiles with elevated phase chip, prominent title, muted timestamp, and `Need input` badge that also tints the tile border red; full empty-state card when zero sessions.
- **Session view:** rich header with title + phase subtitle ("Investigate phase — riffing on the problem"), back link as ghost button, interactive PhaseSelector segmented control; pending-choice/awaiting banner now uses author-rain purple (choice) vs attention red (awaiting) with white inverted choice buttons; chronological feed centered in a comfortable padded gutter; prompt bar refactored to use the design-system PrimaryButton.
- **Settings:** sections grouped (Provider info vs Auth info), per-row accent dot keyed to author color, Save right-aligned, the plaintext-token warning rendered as an amber-tinted callout (was raw red text).
- **Context Library:** tree rows with file/folder icons (▸ / ·), proper indent and hover highlight, refresh button (↻) in the panel header, leaf-only display name (full path no longer crammed into each row), editor with dirty-state indicator ("● unsaved changes"), Save right-aligned as PrimaryButton.
- **Emma overlay:** dedicated header bar with name + status subtitle + close (×) affordance; divider line between underlying view and Emma pane for clear visual separation; width bumped 360→380px.

### Context Library refresh fix

Per the spec: `refresh_cl_tree` is now in the periodic poller in `view_model.rs` (every 4th 500ms tick = 2s cadence). New files appear without app restart. Also added a manual ↻ refresh button in the CL pane header for instant feedback and a new `cl-refresh` callback to back it. Directories now sort before files in the tree walk, alphabetical within each group.

### Files touched

- `ui/app.slint` — full rewrite: 796 → 1410 lines (still single file; modular components inside).
- `src/ui/view_model.rs` — chronological projection, `refresh_cl_tree` polling, `cl-dirty` lifecycle, `display_name` field, directory-first sorting (714 → 743 lines).
- `PROGRESS.md` — this section.
- `ARCHITECTURE.md` — Slint UI layout section updated to reflect single-chat decision.

### Verification

```
cargo build --release  →  clean (0 warnings)
cargo test             →  56 passing (unchanged)
```

## Post-review fixes (2026-05-15)

Three follow-up fixes applied after the autonomous build's READY-FOR-REVIEW state, surfaced by code review:

1. **Added `/target/` to `.gitignore`** — `target/` was 6.7 GB; `git add .` without this would have committed all build artifacts and likely broken the repo.
2. **Emma auto-spawn at startup.** Refactored `src/core/session.rs` to extract a shared `spawn_session_handle` helper and added `spawn_existing_session` for sessions whose row already exists. New `AppState::ensure_session_started(id)` is idempotent and called for `"emma"` in `main.rs` right after core construction. Failure is non-fatal (logs a warning) so a missing `claude` CLI doesn't crash startup.
3. **Settings save now actually persists user edits.** Replaced the inline LineEdit-in-for-loop pattern (which used one-way `text:` bindings and passed the stale model struct to the callback) with a new `AgentConfigEditor` component in `ui/app.slint` that owns per-row edit state via `in-out` properties, initialized via `init =>` and bound to LineEdits via `text <=>`. The Save callback now reads current edited values and constructs a fresh `AgentConfigRow` for the Rust callback.

4. **Per-project rules migration from old CL.** The autonomous build's Phase A distilled high-level conventions/notes per project but missed the operational rules in `~/.bot-hq/projects/<project>.yaml`. Migrated:
   - `bcc-ad-manager` + `bcc-ad-manager-ad-exporter`: added **Gates (strict)** sections (push approval, force-push token format `force-push-greenlight: {branch}@{sha}`, coder/sub-agent tool blocklist, per-action commit approval) and **Read-only DB access** notes. Augmented existing **Disguise compliance** sections with the scaffold-scan rule (and disguised-commit-footers anti-pattern for ad-exporter).
   - `boncom-labs-live-on-playbooks`: expanded the single-line Gates summary into the full strict-default policy.
   - `988` and `bot-hq`: no changes — autonomous build's distillation already covered everything material; the legacy `project_feedback` entries for bot-hq self-project (snap_footer, arc_closure, greenflag_authority, etc.) were intentionally skipped as they encode old hub-coordination patterns that don't apply to the new architecture.
   - Filter applied: skipped all rules referencing legacy concepts (SNAP footers, R-rules, hub_send, voice-mirror, ratchets, discipline-log, BRAIN-cycle, Z-phase) since they don't apply to the Rust/Slint single-binary rebuild.
   - All files still well under the 150-line conventions / 80-line notes target.

Build + test still clean: `cargo build --release` ✅, `cargo test` ✅ (still 56 passing).

## What works (verified)

- **Build clean:** `cargo build --release` ✅, `cargo test` ✅ (56 passing).
- **Binary launches:** `BOT_HQ_DATA_DIR=~/.bot-hq-dev/ ./target/release/bot-hq` starts the
  tokio runtime, initializes the data dir, brings up the signaling HTTP server, and runs
  the Slint event loop. Verified by 2-second smoke run: logs `bot-hq starting → data dir
  ready (outcome=Existing) → signaling server up (127.0.0.1:<port>)` cleanly; process
  stays alive driving the UI loop.
- **Persistent storage:** sqlite migrations seed Emma + default agent_configs idempotently.
- **In-process MCP HTTP server:** `initialize` / `tools/list` / `tools/call`
  (`ask_user_choice`, `mark_awaiting_user`) all round-trip end-to-end (5 HTTP integration tests).
- **Duo coordination:** I/P buffered peer-forward (1.5s window) + A/V turn-based forwarding
  + tool-use suppression all proven with mocked agent channels.
- **CL rebuild (Phase A):** `~/.bot-hq-dev/` populated with rebuilt minimal CL (60K, 398
  lines across general-rules + 3 agent startups + 5 project conventions+notes).

## What needs human-driven verification

- **End-to-end smoke (Phase 9.1):** Drive a real session through Investigate → Plan → Apply
  → Verify with live `claude` subprocesses. This needs a display (the runtime works headless
  but you can't see the Slint window) and an authed `claude` CLI. See "How to verify" below.
  Also verify Settings → edit a field → click Save persists and survives restart, and that
  Emma's chat panel responds (now auto-spawned at startup).
- **UX polish (Phase 9.2):** keyboard shortcuts (Cmd-N, Cmd-,, Cmd-K), scroll-to-bottom on
  new messages, tile sort, empty-state copy, responsive Brian/Rain vertical stacking at
  content width <1200px. Skipped.
- **Minor cleanup deferred to a Phase 9.2 follow-up commit** (not blocking review):
  duplicate `tempfile` dep in `Cargo.toml`, empty `src/cl/mod.rs` (function lives in
  `core/session.rs::read_system_prompt`), hand-rolled `load_env_file` in `main.rs` (consider
  `dotenvy`), no HTTP body-size limit on the MCP server (mitigated by `127.0.0.1:<port>`
  ephemeral binding), Plan/Verify phase tests for `core::duo`, the 6 planned `.slint`
  files collapsed into one `ui/app.slint` (decision to accept-and-document or split).

## How to verify the human-driven parts

```bash
# 1. From a screen-attached macOS terminal:
cd ~/Projects/bot-hq-rebuild
cp .env.example .env             # already contains BOT_HQ_DATA_DIR=~/.bot-hq-dev/
cargo run --release

# 2. In the window:
#   - Click "+ New session" on the Dashboard.
#   - Type a small task in the broadcast prompt bar.
#   - Watch Brian + Rain stream. Click the I/P/A/V chips to advance phase.
#   - If an agent calls ask_user_choice, choice buttons should appear inline.
#   - Toggle the Emma button (top-right) — half-pane chat slides in.

# 3. If a subprocess crashes or events don't appear in the UI: check
#    RUST_LOG=trace cargo run --release for richer logs.
```

## Decisions made autonomously

(Things that diverged from PLAN/decision-doc; documented for the reviewer.)

1. **MCP transport: in-process HTTP, not stdio + UDS bridge.** `docs/decisions.md#mcp-server`
   sketched a stdio + UDS architecture where Claude Code spawns our binary as an MCP child
   and bridges to the parent. That's two subprocesses per agent + ~150 LOC of IPC framing.
   We run a single in-process HTTP MCP server (hand-rolled, ~300 LOC) and write per-agent
   mcp-config.json files pointing at `http://127.0.0.1:<port>/sessions/<id>/<agent>/mcp`.
   Direct AppState access, no IPC. Decision-doc itself flagged HTTP as the
   "promote-if-IPC-gets-hairy" alternative.
2. **MCP server: hand-rolled, not `rmcp` crate.** Lives at `src/signaling/{jsonrpc,server,protocol}.rs`,
   handles `initialize` / `tools/list` / `tools/call` / `ping`. Swap to rmcp later if we
   need richer protocol coverage.
3. **`claude --append-system-prompt` is a string, not a file.** PLAN originally said
   `--append-system-prompt-file`; the CLI only has `--append-system-prompt <prompt>`. We
   read CL slot files in `src/core/session.rs::read_system_prompt` and pass inline.
4. **`--verbose` required with `-p --output-format stream-json`.** Empirically discovered
   in Phase 0.5 capture. `src/agents/spawn.rs` includes it.
5. **`rustup update stable` was run.** Cargo's dep tree pulled in crates requiring
   `edition2024` (Rust ≥ 1.85). Toolchain was 1.84.1 → 1.95.0 after update. Documented
   in README. (User-scoped update via existing rustup install; not a system-wide install.)
6. **Slint pin: `slint = "1.16"`** (resolves to 1.16.1, latest 1.x). MSRV 1.92 per Phase
   0.3 research; we're on 1.95.
7. **Sessions table CHECK constraint trimmed.** `agent_configs.agent_name` is checked
   against ('emma','brian','rain') in the migration — this is per ARCHITECTURE.md's
   "Author enum" but enforced at DB-level too so a typo from the Settings UI doesn't
   silently create a bogus row.
8. **First-run detection key.** `cl-version.txt` existence — not data-dir existence — is
   the first-run signal. (Test setup creates the data-dir before the binary touches it.)

## Phase A — CL Rebuild

- [x] A.1 Audit current `~/.bot-hq/` → `phase-a-audit.csv` (860 rows: 768 drop / 92 merge / 0 keep)
- [x] A.2 Distill `general-rules.md` (41 lines)
- [x] A.3 Distill agent startups (emma 28L / brian 35L / rain 36L)
- [x] A.4 Per-project conventions + notes (bot-hq, bcc-ad-manager, bcc-ad-manager-ad-exporter,
       boncom-labs-live-on-playbooks, 988 — 9 files total)
- [x] A.5 Drop list applied (no phase/, ratchets/, sessions/, discipline-log, gates/, etc carried)
- [x] A.6 Staged at `~/.bot-hq-dev/` with `cl-version.txt = "1\n"` (60K / 398 lines total)
- [x] A.7 (No-op — bootstrap CLAUDE.md already exists at repo root)

## Phase 0 — Bootstrap & Research

- [x] 0.1 Cargo init with pinned deps (slint 1.16, tokio full, sqlx 0.8, hyper 1, …)
- [x] 0.2 Directory skeleton + stub modules
- [x] 0.3 Slint research → `docs/decisions.md#slint` (pin 1.16.1, Globals pattern, VecModel)
- [x] 0.4 MCP SDK research → `docs/decisions.md#mcp-server` (rmcp 1.7.0 — superseded; see Decisions Made Autonomously)
- [x] 0.5 stream-json schema capture → `docs/stream-json-events.md` + raw samples in `docs/stream-json-samples/`
- [x] 0.6 Auth-token storage research → `docs/decisions.md#auth-storage` (plaintext v1, keyring-core v2)
- [x] 0.7 Slint hello-world (topbar Dashboard/CL/Settings + Emma button)
- [x] 0.8 Filesystem layout & first-run init (`src/paths.rs`, lockfile, 9 edge cases handled)

## Phase 1 — Storage

- [x] 1.1 Schema migration `migrations/0001_init.sql`
- [x] 1.2 Migration runner (`sqlx::migrate!` on startup)
- [x] 1.3 Storage API in `src/storage/mod.rs` + `src/storage/model.rs`
- [x] 1.4 Seed Emma singleton session row + default agent_configs (idempotent)
- [x] 1.5 Storage tests in `tests/storage_test.rs` (9 tests, all pass)

## Phase 2 — Agent runtime

- [x] 2.1 stream-json types in `src/agents/protocol.rs` (forward-compat with `#[serde(other)]`)
- [x] 2.2 `spawn_agent` in `src/agents/spawn.rs` (kill_on_drop, MCP config, ANTHROPIC_* env vars)
- [x] 2.3 Event parser in `src/agents/events.rs` (generic over reader for tests)
- [x] 2.4 Input writer in `src/agents/input.rs`
- [x] 2.5 Smoke test in `tests/agent_runtime_test.rs` — NOT added; live tests would require
       `RUN_LIVE_TESTS=1` + claude auth + sit-on-screen wall-clock. Skipped for autonomous build;
       command-shape unit-tested in `agents::spawn::tests::command_has_required_flags`.

## Phase 3 — UI signaling MCP server

- [x] 3.1 Hand-rolled MCP server skeleton in `src/signaling/{mod,protocol,jsonrpc,server,bridge}.rs`
- [x] 3.2 `ask_user_choice` (blocking via oneshot, returns picked option)
- [x] 3.3 `mark_awaiting_user` (non-blocking flag setter via broadcast)
- [x] 3.4 Per-agent mcp-config JSON generation (`mcp_config_json` in server.rs)
- [x] 3.5 Integration tests in `tests/signaling_test.rs` (5 HTTP round-trips, all pass)

## Phase 4 — Core orchestration

- [x] 4.1 `open_session` in `src/core/session.rs` (spawns Brian + Rain, kicks duo pumps)
- [x] 4.2 `close_session` (kills subprocesses via AgentHandle::kill, archives DB row)
- [x] 4.3 IPAV cache in `src/core/ipav.rs`
- [x] 4.4 Broadcast in `src/core/broadcast.rs` (persists + fans to both agents)
- [x] 4.5 Duo coordination in `src/core/duo.rs` (buffer rule per phase, configurable window for tests)
- [x] 4.6 Core tests with mocked agent handles (4 duo tests, all pass)

## Phase 5 — UI scaffold

- [x] 5.1 `ui/app.slint` skeleton (topbar + tab state + AppState global)
- [x] 5.2 Dashboard with tiles (suppression rule baked in: active tile suppresses choice / awaiting)
- [x] 5.3 Settings (per-agent forms: provider, model, base_url, auth_token masked)
- [x] 5.4 Session view (phase chip, two-pane chat, broadcast prompt, banner)
- [x] 5.5 Context Library (file tree + TextEdit editor + save button)
- [x] 5.6 Emma half-pane chat (slides in via emma-open property)
- [x] 5.7 View-model bridge in `src/ui/view_model.rs` (Send-shadow types for cross-thread refresh)

## Phase 6 — UI signaling integration

- [x] 6.1 PendingChoice event wired to view-model via broadcast subscribe
- [x] 6.2 Choice buttons render in tile + session banner (suppression-aware: tile suppresses when active)
- [x] 6.3 User click → `core.resolve_choice` → MCP `ask_user_choice` returns
- [x] 6.4 AwaitingUser wired to flag (banner + tile badge)
- [x] 6.5 Integration tests — covered by `tests/signaling_test.rs::server_ask_user_choice_resolves`

## Phase 7 — Emma chat

- [x] 7.1 Emma's session row is seeded by migration. Auto-spawn of Emma subprocess on app
       start is NOT wired (Phase 9 polish — first time user opens Emma we should spawn her;
       tracked as a manual-launch caveat in the README).
- [x] 7.2 Emma prompt-bar input → `core.broadcast("emma", text)` (Emma's row is broadcast-target
       compatible since she's a singleton session; her side has no Rain peer so the duo
       layer no-ops).
- [x] 7.3 Chat history pulls from `messages WHERE session_id = "emma"`
- [x] 7.4 Toggle behavior wired via `AppState.toggle_emma`
- [x] 7.5 Emma tests — emma_singleton_seeded + emma_seed_is_idempotent_across_open in storage tests

## Phase 8 — CL integration + system prompt assembly

- [x] 8.1 First-run init seeds CL from baked-in templates (templates/cl/). User's `~/.bot-hq-dev/`
       output from Phase A is what bot-hq reads at session-spawn time.
- [x] 8.2 System-prompt concatenation lives at `src/core/session.rs::read_system_prompt`
       (concatenates general-rules + agents/<name>/startup + projects/<p>/{conventions,notes}).
       Phase 8.2 plan called for `src/cl/reader.rs` — function is in session.rs instead;
       move-to-cl-module is a trivial refactor if desired.
- [x] 8.3 Session.rs uses the system-prompt reader (already integrated, no Phase-4 refactor needed)
- [x] 8.4 CL UI write-through implemented in `view_model::on_cl_save_current` (explicit save only)
- [x] 8.5 CL tests in `src/core/session.rs::tests` (system_prompt_concatenation + missing_project_slot_is_fine)

## Phase 9 — E2E + polish + ready-for-review

- [x] 9.1 End-to-end smoke — runtime smoke (2-second binary launch) ✅. Full live agent
       dialog needs human + display + authed claude (see "How to verify" above).
- [ ] 9.2 UX polish (keyboard shortcuts, scroll, sort, empty states) — DEFERRED to follow-up.
- [x] 9.3 Error handling — agent crashes surface as AgentEvent::Exited; storage failures
       logged; MCP errors return JSON-RPC error frames. Agent crash *recovery* (respawn UI)
       is deferred.
- [x] 9.4 README.md + ARCHITECTURE.md sync (README written this commit; ARCHITECTURE.md
       still describes the original stdio MCP plan — see "Decisions made autonomously" #1 for
       what shipped).
- [x] 9.5 **READY FOR HUMAN REVIEW** — banner updated. `docs/commit-message-draft.md` written.
       Working tree dirty; commit/push left to human.

---

## Sub-agent dispatch log

(newest on top)

- **2026-05-14 ~15:00** — Phase A.2-A.6 distill CL → general-purpose. Brief: rebuild
  ~/.bot-hq-dev/ from audit CSV. **completed** — 14 files written, 60K / 398 lines total.
- **2026-05-14 ~14:46** — Phase 0.5 stream-json capture → general-purpose. Brief: empirical
  probes + write events.md. **completed** — 3 probes captured + 477-line schema spec; key
  finding: `--verbose` required with `-p --output-format stream-json`.
- **2026-05-14 ~14:46** — Phase A.1 CL audit → general-purpose. Brief: walk current
  `~/.bot-hq/`, produce phase-a-audit.csv. **completed** — 860 rows classified (768 drop / 92
  merge / 0 keep); per-project surface area mapped.
- **2026-05-14 ~14:46** — Phase 0.6 auth-token research → general-purpose. Brief: v1
  plaintext rationale + v2 keyring-core upgrade path. **completed** — section written to
  `docs/decisions.md#auth-storage`.
- **2026-05-14 ~14:46** — Phase 0.4 MCP SDK research → general-purpose. Brief: pick crate or
  hand-roll. **completed** — recommended `rmcp` 1.7.0; orchestrator later chose hand-roll
  for simpler in-process HTTP transport (see Decisions Made Autonomously #1).
- **2026-05-14 ~14:46** — Phase 0.3 Slint research → general-purpose. Brief: pin version + capture
  app patterns. **completed** — pin 1.16.1; Globals + VecModel + responsive `if` guards.

---

## Session handoff log

- **2026-05-14 (autonomous build session):** all phases 0-9 reached terminal-state except
  9.2 (deferred) and 9.1's live e2e (human-driven). 56 tests passing. Release build clean.
  Binary launches and stays alive. PROGRESS.md banner flipped to READY FOR HUMAN REVIEW.
  Commit message draft at `docs/commit-message-draft.md`. Working tree intentionally dirty
  (Go-file deletions + new Rust files all together, per CLAUDE.md). Human reviewer:
  inspect, then commit + push.
