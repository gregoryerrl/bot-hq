# Tauri v2 Migration — Design Plan

**Date:** 2026-05-26
**Status:** Brainstorm complete, awaiting fresh-session implementation
**Brainstormed:** Brian (HANDS) + Rain (EYES) co-brainstormed; user validated each section through bot-hq's `ask_user_choice` gates
**Implementation handoff:** big-bang branch off main, fresh claude-code session executes against this doc

---

## Context

bot-hq's UI is a Slint+Rust desktop app. The Slint UI shell (`ui/app.slint` 3,846 LOC + `src/ui/view_model.rs` 2,840 LOC = ~6,700 LOC) is the friction surface:

- ~28% of recent commits are layout fixes (ChatInput auto-grow saga, ScrollView viewport-width, width+min-width compile errors, chat-column width scoping, divider, etc.)
- The project mandates a 36-file `slint-rust-docs` knowledge base lookup before any Slint work (commit `41d7bb6`)
- The planned plugin roadmap (Discord, Clive, themes, future plugins that mutate UI) is structurally hostile to Slint's compile-time component model — Slint has no dynamic children, no portals, no runtime-loaded slots

The valuable parts of bot-hq — agent orchestration, both MCP servers, policy enforcement, sqlite storage, session permissions, signaling bridge — are stack-agnostic Rust. They port unchanged to any frontend.

This document captures the design decisions and architecture for migrating the UI shell to Tauri v2 + React, preserving the Rust core unchanged.

---

## Decisions (validated)

1. **Migration shape:** Big-bang. Branch off main, focused UI-shell rebuild. No parallel Slint maintenance.
2. **Frontend stack:** React 18 + TypeScript + Tailwind + shadcn/ui (Vite build).
3. **Plugin model:** Slot-extend + custom panels. Defer full UI-mutation capability until concrete need.
4. **IPC architecture:** Tauri-native. All React↔Rust via Tauri commands + Tauri events. No HTTP from frontend.

---

## Operating principle

> We never use HTTP unless absolutely needed (e.g., external agent drivers).

### HTTP audit

| Path | HTTP? | Justification |
|------|-------|---------------|
| React frontend ↔ Rust core | NO | Tauri commands + events |
| claude-code subprocesses ↔ Rust core | YES (localhost) | claude-code's MCP transport contract |
| External agent drivers ↔ Rust core | YES (bearer auth) | External network access |
| Plugin iframes ↔ Rust core | NO | Tauri `invoke` gated by capabilities |

---

## Architecture

Three layers.

### Layer 1: Rust core (unchanged)

`src/agents/`, `src/core/`, `src/policy/`, `src/storage/`, `SignalingBridge`, session permissions, claude-code subprocess management, sqlite schema, all 19 internal + 16 external MCP tool implementations. **~12,000 LOC preserved, zero rewrites.**

### Layer 2: Dual dispatcher over single `SignalingBridge`

- **MCP JSON-RPC dispatcher** (existing, `src/signaling/`): claude-code subprocesses speak to it over HTTP localhost. No change.
- **Tauri command dispatcher** (new, `src/tauri_cmd/`): thin `#[tauri::command]` wrappers calling the **same `SignalingBridge` methods**. Frontend + plugin iframes use `invoke('name', args)`. ~500–1000 LOC of pure wrappers, no business logic.

Single source of truth: `SignalingBridge`. Two dispatch surfaces (HTTP+JSON-RPC for agents, Tauri IPC for frontend+plugins).

### Layer 3: React frontend (new)

`frontend/` workspace. Vite + React 18 + TypeScript + Tailwind + shadcn/ui. Talks to Rust only via Tauri commands + Tauri events. No `fetch`, no HTTP. Plugin iframes use the same `invoke` API gated by Tauri v2 capabilities.

### Streaming

Tauri events. Rust emits typed events (`agent.message`, `session.phase_changed`, `cl.refreshed`, `mcp.tool_called`, `session.created`, `session.subprocess_died`) via `app_handle.emit()`. Frontend listens via `listen('event.name', handler)`. Replaces current 500ms poll loop. Hot path (`AgentMessage`) goes through `BatchEmitter` to coalesce stream-json bursts.

### Plugin loading

Host scans `<data_dir>/plugins/` for `manifest.json` at startup. Each plugin → iframe with capability declaration. Plugins served at `https://plugin-<id>.localhost` (per-plugin origin via Tauri custom URI scheme). Capability JSON scopes via `remote.urls`.

---

## Components

### Rust additions (~1,500–2,500 LOC new)

**`src/tauri_cmd/`** — domain-grouped command wrappers:

- `sessions.rs` — `create_session`, `respawn_session`, `close_session`, `get_session`, `list_sessions`
- `agents.rs` — `broadcast_to_session`, `get_session_messages`, `get_emma_messages`
- `mcp_tools.rs` — `call_mcp_tool` (dispatcher for internal MCP tools when frontend needs to invoke them)
- `settings.rs` — agent config + preferences
- `plugins.rs` — `list_plugins`, `install_plugin`, `enable_plugin`, `disable_plugin`
- `policy.rs` — `grant_session_permission`, `revoke_session_permission`, `list_session_permissions`
- `cl.rs` — `cl_index_search`, `cl_folder_search`, `cl_register_read`, `cl_rescan`

Each `#[tauri::command]` takes typed Rust args, gets `tauri::State<Arc<SignalingBridge>>`, calls bridge methods, returns typed responses.

**`src/tauri_events/`** — typed event structs deriving `Serialize`/`Clone` + emit helpers:

- `AgentMessage`, `SessionPhaseChanged`, `ClRefreshed`, `McpToolCalled`, `SessionCreated`, `SessionSubprocessDied`
- `BatchEmitter` — 50ms / N=20-event batching for `AgentMessage` hot path
- Forced flush on `result` stream-json event (avoids 50ms turn-end stall)

**`src/plugins/`** — new module:

- `manifest.rs` — `PluginManifest` struct (id, name, version, entry, requested_capabilities, slot_contributions, panel_contributions)
- `loader.rs` — scans `<data_dir>/plugins/`, validates manifests
- `capabilities.rs` — generates per-plugin capability JSON at startup, writes to `src-tauri/capabilities/plugin-<id>.json`
- `heartbeat.rs` — host-side iframe heartbeat watcher; **registers at app-shell level, not per-PluginSlot** (would otherwise die on slot remount)

**`src/main.rs`** — restructured for Tauri bootstrap. Tokio multi-thread runtime stays on workers; Tauri owns OS main thread. Subprocess spawn/monitor via `tokio::process::Command` — no Slint event-loop dependency to remove.

### Frontend (`frontend/`)

Stack: Vite + React 18 + TypeScript + Tailwind + shadcn/ui.

Layout:

- `src/app/` — top-level routes: Dashboard, SessionView, Settings, PluginManager
- `src/components/` — shadcn primitives + composed app components (ChatInput, PhasePill, SessionTile, DocumentPane)
- `src/features/` — feature folders mirroring backend domains: `sessions/`, `chat/`, `cl/`, `plugins/`, `settings/`
- `src/hooks/` — `useInvoke`, `useTauriEvent`, `useSession`, `useDocs`, `useCl`
- `src/lib/types.ts` — **generated from Rust via `tauri-specta`** (strong typing across the IPC bridge)
- `src/stores/` — Zustand for UI-local state (window layout, modal stack, sidebar collapsed)
- `src/slots/` — `<PluginSlot name="sidebar.bottom">` host components rendering plugin iframes
- `src/main.tsx` — entry point with `<RouterProvider>` + `<TanstackQueryProvider>`

Build pipeline:

- `cargo build` → triggers `tauri-specta` TS generation → outputs `src/lib/types.ts`
- `vite build` → produces frontend bundle
- `tauri.conf.json#beforeBuildCommand` / `beforeDevCommand` enforces the order

### Plugin rendering

Each plugin served at `https://plugin-<id>.localhost` (per-plugin origin via Tauri custom URI scheme), all hosted inside the main webview's DOM.

Capability JSON shape:
```json
{
  "identifier": "plugin-<id>-capability",
  "windows": ["main"],
  "webviews": ["main"],
  "remote": { "urls": ["https://plugin-<id>.localhost/*"] },
  "permissions": ["allow-<command-name>", "..."]
}
```

- Slot plugins render as iframes inside `<PluginSlot name="...">`
- Panel plugins render as iframes in dedicated routes
- Both call `window.__TAURI__.invoke('cmd', args)`
- Tauri matches iframe origin against capability `remote.urls`, allows or denies

Origin-scoping (NOT per-Tauri-webview): per-Tauri-webview plugins can't nest inside the main webview's React tree, which would break slot extension.

---

## Data Flow

### 1. Session creation

User clicks `+ New session`.

1. React → `invoke('create_session', { agent, task })`
2. Tauri command (`src/tauri_cmd/sessions.rs`) → `bridge.create_session(...)`
3. Storage layer writes `sessions` row
4. `core::session::spawn_session_handle()` spawns `claude-code` subprocess with `BOT_HQ_SESSION_ID` env + MCP config pointing at internal MCP server (HTTP localhost — claude-code's contract)
5. Tauri command returns `SessionInfo { id, status }`
6. Rust emits `app_handle.emit("session.created", &info)`
7. All Dashboard listeners refresh

### 2. Agent stream (hot path)

`claude-code` subprocess emits stream-json on stdout.

1. Existing parser in `src/agents/stream.rs` decodes message
2. Storage layer writes `messages` row (existing path)
3. `BatchEmitter` accumulates the `AgentMessage` — flushes at N=20 OR 50ms (whichever first)
4. On flush: `emit("agent.messages.batch", &Vec<AgentMessage>)`
5. React `SessionView` listener applies batch to chat store; React reconciles once per batch

50–100 events/sec stream-json bursts → ~20 React renders/sec → ~5× CPU saving.

**Forced flush on `result` event** before phase-forward signal — avoids the 50ms turn-end stall that would otherwise leave the final ~15 events buffered.

### 3. Plugin invoke (capability check)

Plugin iframe at origin `https://plugin-discord.localhost` calls `window.__TAURI__.invoke('cl_index_search', { project: 'bot-hq' })`.

1. Tauri IPC receives call + iframe origin
2. Capability lookup for that origin: is `cl_index_search` in allowed commands?
3. Yes → dispatch to `tauri_cmd::cl::index_search()` → `SignalingBridge` → response
4. No → capability-denied error
5. Capability JSON generated at startup from `manifest.json#requested_capabilities`
6. User approves capability set at install time via PluginManager UI

### Event-loop ownership

Tauri owns OS main thread; Tokio multi-thread runtime stays on worker threads (4 workers default). Subprocess management uses `tokio::process::Command` — no Slint event-loop dependency to remove. Confirmed: 0 `slint::invoke_from_event_loop` calls outside the Slint-removal blast radius (`view_model.rs` + `main.rs` shutdown handler, both being replaced).

---

## Error Handling

### 1. Tauri command errors

All `#[tauri::command]` functions return `Result<T, AppError>`. `AppError` typed enum:

- `Validation(String)`
- `NotFound(String)`
- `Unauthorized(String)`
- `Internal(String)`
- `DbError(String)`
- `CapabilityDenied(String)`

Frontend `useInvoke` hook auto-surfaces errors via shadcn `<Toast>` for mutations; queries throw to TanStack Query for retry/cache invalidation. Errors are part of the `tauri-specta` generated TS contract.

### 2. Subprocess lifecycle errors

Existing reaper in `src/core/session.rs` (already hardened in commits `7cc6c25`, `faa7171`, `36f6f6b`) emits `session.subprocess_died { session_id, exit_code, reason }`. React `SessionView` listener shows red banner + "Restart agent" button → `invoke('respawn_session', { session_id })`.

Mutex-poison recovery (`bea60bd`) ports forward unchanged: `.lock().unwrap_or_else(|p| p.into_inner())` discipline at all shared-state lock sites.

### 3. Plugin sandbox failures

- **Crash detection:** iframe `error` event + heartbeat ping (host sends `__ping` every 5s; expects `__pong` within 1s). Miss → assume crash.
- **Watcher scope:** Heartbeat watchers register at app-shell level (Dashboard route — never unmounts), NOT inside per-session `<PluginSlot>` components. PluginSlot remount would kill the watcher and the plugin would run unwatched.
- **Recovery (v1):** Host removes the failed iframe + shows fallback message. Manual reload from PluginManager.
- **Recovery (pre-external):** Before opening plugin model to third-party authors, add exponential-backoff auto-restart (3 attempts, then pause). Otherwise a crashing-on-load plugin is dead until user notices.
- **Capability-denied:** Standard Tauri permission-denied error → plugin iframe handles however it chooses (log, retry without that feature, show its own UI).

### Other failure modes

- **Stream parse errors** (malformed stream-json): existing log-and-skip path. No change.
- **Storage errors:** sqlite write failures bubble through Tauri `Result` → `db-error` toast.
- **Tauri event loss:** events fire even with no listeners (events get dropped). Mitigation: React components fetch initial state via `invoke('get_*_state')` on mount, then subscribe to event delta. Standard query+subscription pattern.
- **Policy violations** (git push without approval, etc.): existing two-layer enforcement (MCP tool checks + git hooks) unchanged. New Tauri command layer also checks via same `policy::*` modules — single policy SOT across both dispatch surfaces.

---

## Testing

### Rust additions

- `src/tauri_cmd/<domain>/tests.rs` — one unit test per command. Tests call the underlying `SignalingBridge` method directly. Mock state via `tauri::test::mock_builder()` for the few commands that need `AppHandle`.
- `tests/tauri_cmd_integration.rs` — exercise commands via Tauri's test harness with a real `SignalingBridge` + in-memory sqlite. Mirrors `external_mcp_test.rs` shape.
- `src/plugins/tests.rs` — manifest parse + capability JSON generation. Pure-function tests.
- `src/tauri_events/tests.rs` — `BatchEmitter` timer + flush-on-result behavior, with `tokio::time::pause()` for deterministic timing.
  - **Specific test: partial-batch + result-flush case** — emit 15 events (under N=20), fire `result` signal, assert all 15 flushed before 50ms timer elapses.
- `src/plugins/iframe_ipc_test.rs` — **dummy iframe origin + mock capability set**, verify full chain (capability JSON gen → Tauri IPC origin check → command dispatch). Closes coverage gap before real Discord/Clive plugins ship.

### Frontend

- **Vitest + React Testing Library** for components, hooks, stores
- `useInvoke` / `useTauriEvent` mocked via `@tauri-apps/api/mocks`
- Snapshot tests for stable UI surfaces (Dashboard tiles, PhasePill, SessionView header, ChatInput)
- Coverage target: 70% for hooks/stores; component-level snapshot only
- TypeScript `tsc --noEmit` runs in CI to catch tauri-specta-generated type drift

### Existing 202-test independence

The 202 existing Rust tests remain runnable as plain `cargo test` without Tauri bootstrapping. The Tauri layer wraps the core — the core tests don't spin up Tauri. This holds naturally because:

- Tauri commands depend on `SignalingBridge`; tests use the bridge directly (no Tauri runtime needed)
- Tauri events are emit-only side effects; tests don't observe them
- Plugin loading is gated by `<data_dir>/plugins/` scanning, not by Tauri lifecycle

### E2E (smoke level)

- **Playwright + tauri-driver** for critical user flows: new-session → agent streams → close. ~5–10 tests, nightly (slow).
- Plugin install + capability-grant E2E once Discord plugin lands.
- Manual smoke checklist (PROGRESS.md "verify the human-driven parts") ports forward — release builds pass that gate before merge.

### Regression strategy

- CI per-commit: `cargo test` (existing 202 + new Tauri command/events/plugins tests) + `cd frontend && pnpm test` + `cargo clippy` + `tsc --noEmit`
- CI nightly: Playwright E2E
- No green = no merge
- Big-bang branch merges to main when: parity hit + frontend Vitest ≥70% + E2E smoke green + manual smoke checklist passes

---

## Migration scope

| Area | LOC delta |
|------|-----------|
| Slint UI (`ui/app.slint` + `src/ui/view_model.rs`) | **-6,700** (deleted) |
| New Tauri command layer (`src/tauri_cmd/`) | +500–1,000 |
| New Tauri events layer (`src/tauri_events/`) | +200–400 |
| New plugin module (`src/plugins/`) | +400–800 |
| `src/main.rs` restructure | ±200 |
| React frontend (`frontend/`) | +3,000–5,000 |
| Rust core (`src/agents/`, `src/core/`, `src/policy/`, `src/storage/`, `src/signaling/`) | **0** |
| **Net** | ~-2,500 to +500 |

**Estimated time:** 2–4 weeks heads-down (Big-bang shape).

---

## Open questions / deferred

- **Plugin auto-restart:** defer until before opening plugin model to third-party authors. v1 ships with manual PluginManager reload only.
- **Full UI-mutation plugin tier:** defer until a concrete use case demands it. v1 ships with slot + panels only.
- **Cross-platform** (Linux, Windows): inherits Tauri's cross-platform support. macOS first; Linux + Windows after parity.
- **Plugin author SDK + docs:** needs its own design pass once core plugin model lands. Out of scope for this migration.
- **Stream-json transport for claude-code:** if claude-code adds stdio/UDS MCP transports in the future, the internal HTTP MCP server can be reconsidered. For now HTTP localhost is the contract.

---

## Implementation handoff

This document is the Plan-phase output of the brainstorming session. The Apply phase is **a fresh claude-code session** executing against this doc as its blueprint.

**Handoff procedure:**

1. Create a git worktree off main for the migration branch (`git worktree add ../bot-hq-tauri tauri-v2-migration`)
2. Spawn a fresh claude-code session in the worktree
3. Fresh session reads `docs/plans/2026-05-26-tauri-v2-migration-design.md`
4. Fresh session uses `superpowers:writing-plans` skill to produce a detailed implementation plan (sub-tasks, ordering, test-first checklist)
5. Fresh session executes via `superpowers:executing-plans` (batch execution with review checkpoints)
6. Each batch lands as one logical commit; CI gates every commit
7. Branch merges to main when migration scope above is complete + all regression gates green

This session does NOT execute the migration. It produces the plan only.

---

## References

- **Elves** (https://mvmcode.github.io/elves/) — Tauri + sqlite + PTY + AI agents reference. Validates the same domain (Homebrew-installable native binary, AI-agent orchestrator).
- **Tauri v2 docs** — capabilities, custom URI schemes, IPC, future mobile support
- **`tauri-specta`** — Rust → TypeScript type generation for IPC contracts
- **shadcn/ui** — copy-into-repo React component library (Vercel-built, used by Linear, Cal.com, etc.)
- **`~/.bot-hq/projects/bot-hq/PROGRESS.md`** — existing bot-hq change log
- **`~/.bot-hq/projects/slint-rust-docs/`** — knowledge base being retired post-migration
- **Session doc `brainstorm-tauri-migration`** — full brainstorming process artifact with section-by-section validation history (surfaced in the session view's I tab; tag `phase=investigate`)
