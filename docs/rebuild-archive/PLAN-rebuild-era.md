# bot-hq — Implementation Plan (Rebuild)

**Version:** 2026-05-14
**Working dir during dev:** `~/Projects/bot-hq-rebuild` (same repo as `~/Projects/bot-hq`, `main` branch, fresh tree). Product name remains **bot-hq** — the `-rebuild` suffix is just the dev tree.
**Stack:** Rust + Slint single-binary desktop GUI

---

## Scope

Replace current bot-hq (Go daemon + tmux + MCP hub + Emma forwarder + 29 MCP tools) with a desktop GUI app where the user drives orchestration directly. See **`ARCHITECTURE.md`** (in this repo) for the full decision record.

**In scope:**
- Rust binary with Slint UI: **Dashboard | Context Library | Settings | Emma** topbar layout
- Dashboard with quick-view session tiles (phase chip, last activity, `[Need User Input]` flag, inline clickable choices)
- claude-code subprocess per agent via `claude -p --input-format stream-json --output-format stream-json`
- Model swap per agent via `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_MODEL` env-vars (settings tab)
- `bot-hq.db` (sqlite): `messages`, `sessions`, `agent_configs` tables
- IPAV state in in-memory cache only (HashMap<SessionId, SessionState>)
- Bilateral duo coordination: 1.5s buffer in I/P, pure turn-based (message_stop) in A/V
- UI-signaling embedded MCP server with 2 tools: `ask_user_choice`, `mark_awaiting_user`
- Context Library: `agents/<name>/startup.md`, `general-rules.md`, `projects/<p>/{conventions,notes}.md`
- Emma as singleton chat helper (half-view panel)
- **CL rebuild** from existing `~/.bot-hq/` content (Phase A — parallel content track)

**Out of scope this iteration:**
- Discord plugin (1st plugin — separate plan)
- Clive plugin (2nd plugin — separate plan)
- WebUI
- Migration of session/runtime data from current bot-hq

**Preserved from current tree:** `.claude/`, `.env.example`, `.gitignore`. All other Go files are unstaged deletions to be committed alongside the new code.

---

## Prerequisites

Before starting any phase, confirm:

- **Rust toolchain installed.** Stable channel via rustup. Check: `rustc --version && cargo --version`.
- **`claude-code` CLI installed + authed.** Used as subprocess by the rebuild AND needed for live tests (Phase 2.5 and 9.1). Check: `claude --version && claude auth status`.
- **Working tree state.** Existing Go files appear as unstaged deletions in `~/Projects/bot-hq-rebuild`. Do NOT stage or revert — they commit alongside new Rust code at Phase 9.5 (human-driven). Check: `git status`.
- **Env override for dev:** `BOT_HQ_DATA_DIR=~/.bot-hq-dev/` in `.env` (avoids colliding with running bot-hq's `~/.bot-hq/`).

If any prerequisite is missing, document the gap in PROGRESS.md as a blocker and continue with non-blocked work.

---

## Canonical docs

Fresh sessions read in this order:

1. **`CLAUDE.md`** — operating rules + read-order
2. **`ARCHITECTURE.md`** — architectural decisions, source of truth
3. **`PLAN.md`** (this file) — phased roadmap
4. **`PROGRESS.md`** — current state + handoff baton

---

## Data locations

Defaults (env-overridable via `BOT_HQ_DATA_DIR`):

- **CL root:** `~/.bot-hq/` (will eventually be the canonical location once current bot-hq is decommissioned)
- **DB file:** `~/.bot-hq/.local/bot-hq.db` (hidden subdir keeps it visually separate from user-editable CL content; same backup boundary as CL)

**During development:** set `BOT_HQ_DATA_DIR=~/.bot-hq-dev/` in `.env` to avoid colliding with current bot-hq's `~/.bot-hq/` CL. Add `BOT_HQ_DATA_DIR` to `.env.example` with this note.

**Security caveat:** the DB stores auth tokens in plaintext for v1. Any backup tool capturing `~/.bot-hq/` captures these. Document this in README. v2 upgrade: OS keychain integration.

---

## Project structure

Single Rust binary crate at the repo root. Modular `src/` tree:

```
bot-hq-rebuild/                  (working-dir name; same git repo as bot-hq)
├── Cargo.toml
├── CLAUDE.md                    Phase A.7 — points claude-code at rebuilt CL
├── PLAN.md                      this file
├── README.md                    Phase 9 deliverable
├── .env.example                 documents BOT_HQ_DATA_DIR, model API keys
├── ui/                          Slint .slint files
│   ├── app.slint
│   ├── dashboard.slint
│   ├── context_library.slint
│   ├── settings.slint
│   ├── session.slint
│   └── emma.slint
├── src/
│   ├── main.rs                  entry point + Slint app init
│   ├── app.rs                   top-level app state (single Arc<AppState>)
│   ├── paths.rs                 data-dir resolution + env-var override
│   ├── storage/                 sqlite (messages, sessions, agent_configs)
│   ├── cl/                      Context Library filesystem reader/writer
│   ├── agents/                  subprocess management + stream-json IO
│   ├── signaling/               embedded MCP server (ask_user_choice, mark_awaiting_user)
│   ├── core/                    session lifecycle, IPAV cache, duo coordination
│   └── ui/                      Slint bindings, view-model adapters
├── migrations/                  sqlite schema migrations
├── docs/
│   ├── decisions.md             Phase 0 research outputs
│   └── stream-json-events.md    claude-code event reference
└── tests/                       integration tests
```

---

## Phases

Task tags: `[P]` parallelizable within phase (sub-agent friendly) · `[S]` sequential within phase · `[R]` research single-threaded.

Phase A is a **parallel content track** that runs alongside the code phases — not blocking until Phase 8 integration.

---

### Phase A — CL Rebuild (parallel content track)

**Status:** Not started
**Goal:** Distill existing `~/.bot-hq/` CL into the new minimal structure. Output seeds the rebuild's data dir and gives claude-code the right context during dev.

- A.1 `[R][S]` Audit current `~/.bot-hq/`. Walk the tree with `find ~/.bot-hq -type f` (excluding `.git`, `hub.db*`, `live.log`, `bridge/`, `diag/`, `sentinels/`). Produce **`phase-a-audit.csv`** at `~/Projects/bot-hq-rebuild/` with columns: `path,classification,target_slot,rationale`. Classifications: `keep` (carries forward as-is), `merge` (folds into a new CL slot — name slot in `target_slot`), `drop` (runtime artifact replaced by new bot-hq layers). **Time bound:** target ≤ 2 hours; if blowing past, stop and note in PROGRESS.md what remains.
- A.2 `[P]` Distill **`general-rules.md`** from current `rulebook.md`, `glossary.md`, `roles.md`, `mcp-tool-manifest.md`, `agent-onboarding.md`. Universal rules only (commit style, no AI co-author trailers, security, the `ask_user_choice`/`mark_awaiting_user` MCP conventions). Slim and tight.
- A.3 `[P]` Distill **`agents/emma/startup.md`**, **`agents/brian/startup.md`**, **`agents/rain/startup.md`** from current per-agent rules + last_state.json content + role docs. Each ≤ 1 page.
- A.4 `[P]` For each relevant project, distill **`projects/<p>/conventions.md`** (patterns, glossary, anti-patterns) + **`projects/<p>/notes.md`** (learnings worth carrying forward). Initial targets: `bot-hq` (self-referential, light), `bcc-ad-manager`, `ad-exporter`.
- A.5 `[P]` **Drop list** (do not carry forward): `phase/*`, `ratchets/`, `sessions/`, `discipline-log.md`, `voice-mirror-log.md`, `gates/`, `plugins/`, `tasks.md`, `<agent>/last_state.json`, `<agent>/discipline-anchors.md`, session manifests. These were runtime/coord artifacts replaced by new bot-hq's runtime layers (IPAV cache, messages table, agent_configs).
- A.6 `[S]` Stage the rebuilt CL at `~/.bot-hq-dev/` (per env-var override). Add `cl-version.txt` with "1" at root.
- A.7 `[S]` Create `CLAUDE.md` at `~/Projects/bot-hq-rebuild/` that points claude-code at `~/.bot-hq-dev/` for context during development of the rebuild itself. Include a brief project README pointer.

**Verification:** A fresh claude-code session reading just the rebuilt CL should be productive as Brian/Rain/Emma within ~5 minutes of scanning. Total CL size budget: << current `~/.bot-hq/` (target ≤ 30% by file count).

---

### Phase 0 — Bootstrap & Research

**Status:** Not started
**Goal:** Cargo project initialized, Slint hello-world runs, foundational decisions resolved.

- 0.1 `[S]` Initialize Cargo project at `~/Projects/bot-hq-rebuild`. Package name `bot-hq`. Add deps (use latest-stable as of 2026-05-14; pin in `Cargo.toml` once 0.3 research confirms compat):
  - `slint = "1"` (latest 1.x — confirm exact version in 0.3 research)
  - `tokio = { version = "1", features = ["full"] }`
  - `sqlx = { version = "0.8", features = ["sqlite", "runtime-tokio", "macros"] }`
  - `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`
  - `tracing = "0.1"`, `tracing-subscriber = "0.3"`
  - `anyhow = "1"`, `thiserror = "2"`
  - `directories = "5"` (resolving `~/Library/...` style paths)
  - MCP server crate from 0.4 (deferred until 0.4 decides)
- 0.2 `[S]` Create directory skeleton (src/, ui/, migrations/, tests/, docs/) and stub modules with `pub mod` declarations.
- 0.3 `[P][R]` Research Slint. Sources: `https://slint.dev/`, `https://docs.slint.dev/latest/docs/slint/`, `https://crates.io/crates/slint`. Capture: latest stable version, app patterns (single-window default; Slint Globals for app state vs callback prop-drilling; `Model<T>` + `ModelRc<T>` for list/grid views; `Backend::set()` for app init). Output → `docs/decisions.md#slint` with pinned version + chosen state pattern. **Default lean:** single window, Slint Globals for shared state (`AppState` global), `Model<T>` for tile/sidebar lists. Adopt these unless research surfaces a clear better alternative.
- 0.4 `[P][R]` Research Rust MCP SDK. Search: `https://crates.io/search?q=mcp` and `https://lib.rs/search?q=mcp`. Candidates: `rmcp`, `mcp-server-rs`, or hand-rolled JSON-RPC over stdio/HTTP. **Decision criteria (in order):** (1) maintained as of 2026, (2) supports server mode with tool registration + custom handlers, (3) stdio or HTTP transport (matches `claude --mcp-config` expectations), (4) Rust 2021/2024 edition compat, (5) minimal dep tree. **Default lean:** pick the most-maintained crate meeting all criteria; fall back to a hand-rolled minimal JSON-RPC server (~200 lines) if none qualify. Output → `docs/decisions.md#mcp-server` with chosen crate name + version + rationale (or "hand-rolled" + module path).
- 0.5 `[P][R]` Capture claude-code stream-json schema empirically. Save raw output to `docs/stream-json-samples/` (one file per probe). Probes to run:
  - **Plain reply:** `echo '{"type":"user","message":{"role":"user","content":"say hello"}}' | claude -p --input-format stream-json --output-format stream-json > docs/stream-json-samples/plain.jsonl`
  - **With Bash tool use:** prompt to list files in current dir using Bash. Captures `tool_use` and `tool_result` events.
  - **Malformed input:** intentionally bad JSON to surface error event shapes.
  Document event types in `docs/stream-json-events.md` with Rust struct sketches using `#[derive(Deserialize, Serialize)]` and tag-discriminator (`#[serde(tag = "type", rename_all = "snake_case")]`). Cover at minimum: `text` / `text_delta`, `tool_use`, `tool_result`, `message_start`, `message_stop`, `error`. Note any other event types discovered.
- 0.6 `[P][R]` Research: macOS keychain (`keyring` crate) vs sqlite plaintext for auth-token storage. Output → `docs/decisions.md#auth-storage`. v1 = plaintext; document the upgrade path.
- 0.7 `[S]` Slint hello-world: empty window with topbar tabs (Dashboard | Context Library | Settings) and Emma button float-right. Compiles + runs. Tab clicks switch panes (no content yet).
- 0.8 `[S]` **Filesystem layout & first-run init.** Implement `src/paths.rs`:
  - Resolve `BOT_HQ_DATA_DIR` env var, default to `~/.bot-hq/`.
  - On app start, check if data dir exists. If missing, run **first-run init**: write default CL (from Phase A output if available, else baked-in minimal templates) + `cl-version.txt`.
  - If data dir exists but `cl-version.txt` is missing or files are partial: surface a one-time toast ("CL was missing — initialized with defaults. Custom rules from a previous session are lost.") and re-init the missing slots.
  - Single-instance lock: lockfile at `<data_dir>/.local/lock` (PID-based). Second instance refuses to start with clear error.
  - Edge cases handled:
    1. Data dir missing entirely → first-run init.
    2. Individual CL file missing → treat as empty in concatenation, log warning, UI shows "initialize default" button for that slot.
    3. CL file unreadable (permission/encoding) → surface error in UI, fall back to empty for that slot.
    4. Concurrent app instances → lockfile rejects.
    5. DB missing but CL intact → run migrations from scratch (preserves CL).
    6. Both missing → first-run flow.
    7. Disk full / permission errors during init → surface in UI; don't leave half-written state.
    8. User-editable CL while session running → fine (CL read only at spawn).
    9. CL version drift → check `cl-version.txt`, migrate if needed (no-op for v1).

**Verification:** `cargo run` opens a window. All 4 tabs visible. All research docs filed. App handles missing data dir gracefully.

---

### Phase 1 — Storage layer

**Status:** Not started
**Goal:** sqlite schema + migration runner + storage API.

- 1.1 `[S]` `migrations/0001_init.sql`: messages, sessions, agent_configs tables matching memory spec. Index on `messages(session_id, created_at)`.
- 1.2 `[P]` Migration runner via `sqlx::migrate!` on app startup. Auto-creates `<data_dir>/.local/bot-hq.db` if missing.
- 1.3 `[P]` Storage API in `src/storage/mod.rs`:
  - `insert_message(session_id, author, kind, content) -> i64`
  - `messages_for_session(session_id, since_id: Option<i64>) -> Vec<Message>`
  - `create_session(id, title, working_repo_path) -> Session`
  - `close_session(id, archive: bool)`
  - `list_active_sessions() -> Vec<Session>`
  - `get_agent_config(name) -> Option<AgentConfig>`
  - `upsert_agent_config(config)`
- 1.4 `[P]` Seed Emma singleton session row on first migration (`INSERT OR IGNORE` with id="emma", title="Emma", working_repo_path=null).
- 1.5 `[P]` Tests in `tests/storage_test.rs`: round-trip insert/query, migration runs cleanly on empty db, Emma seed idempotent.

**Verification:** `cargo test storage` passes.

---

### Phase 2 — Agent runtime (subprocess management)

**Status:** Not started
**Goal:** Spawn claude-code subprocesses with stream-json IO and route events.

- 2.1 `[P]` `src/agents/protocol.rs`: Rust types for stream-json events (text_delta, tool_use, tool_result, message_stop, error, etc.), derived from research in 0.5. Implement `Deserialize` for inbound, `Serialize` for outbound.
- 2.2 `[S]` `src/agents/spawn.rs::spawn_agent(name, config, system_prompt) -> AgentHandle`:
  - Sets ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN / ANTHROPIC_MODEL env vars from `AgentConfig`.
  - Writes concatenated system prompt to a temp file.
  - Runs `claude -p --input-format stream-json --output-format stream-json --append-system-prompt-file <tmp> --mcp-config <mcp-config-file>`.
  - Returns `AgentHandle` with: `event_rx: mpsc::Receiver<AgentEvent>`, `input_tx: mpsc::Sender<UserMessage>`, `kill: oneshot::Sender<()>`.
- 2.3 `[P]` `src/agents/events.rs`: stdout reader, line-by-line JSON parse, dispatch to `event_rx`. Handle parse errors gracefully (log + continue).
- 2.4 `[P]` `src/agents/input.rs`: stdin writer, serializes `UserMessage` as stream-json, flushes after each message.
- 2.5 `[P]` Smoke test in `tests/agent_runtime_test.rs` (gated by env `RUN_LIVE_TESTS=1`): spawn agent, send "hello", capture events, verify `message_stop` arrives.

**Verification:** Manual: spawn agent, send a message, observe events in console log.

---

### Phase 3 — UI signaling MCP server

**Status:** Not started
**Goal:** In-process MCP server exposing `ask_user_choice` and `mark_awaiting_user` tools.

- 3.1 `[S]` Implement minimal MCP server per Phase 0.4 decision in `src/signaling/mod.rs`. Supports tool registration + invocation handlers.
- 3.2 `[S]` Tool: `ask_user_choice(question: string, options: array<string>) -> string`. On invocation, push a `PendingChoice` event to the core layer and await response (oneshot channel). Returns the picked option.
- 3.3 `[S]` Tool: `mark_awaiting_user(reason: string) -> void`. On invocation, push an `AwaitingUser` event to the core layer. Returns immediately.
- 3.4 `[S]` Generate per-agent mcp-config JSON pointing to this server at app startup. Path passed to `spawn_agent` via Phase 2.2.
- 3.5 `[P]` Integration test: spawn an agent with a prompt that triggers `ask_user_choice`, simulate user response, verify agent gets the result and continues.

**Verification:** Integration test passes. Manual: prompt agent to call each tool.

---

### Phase 4 — Core orchestration

**Status:** Not started
**Goal:** Session lifecycle, IPAV cache, duo broadcast + buffered peer forwarding.

- 4.1 `[S]` `src/core/session.rs::open_session(scope, working_repo) -> SessionId`:
  - Creates DB row.
  - Reads system prompt via CL reader (Phase 8; uses inline `std::fs::read_to_string` until 8 lands).
  - Spawns Brian + Rain via Phase 2.
  - Returns SessionId.
- 4.2 `[S]` `close_session(id)`: kills subprocesses, archives DB row.
- 4.3 `[P]` `src/core/ipav.rs`: `IpavState { current_phase, phase_log }`. `advance_phase(id, target)` updates cache + emits synthetic user message ("phase advanced to PLAN") via broadcast.
- 4.4 `[S]` `src/core/broadcast.rs::broadcast_user_message(session_id, text)`: writes to both Brian and Rain `input_tx`. Persists to messages table as `author=user`.
- 4.5 `[S]` `src/core/duo.rs`: per-agent event loop. For each outbound agent event:
  - Persist to messages table.
  - If phase ∈ {Investigate, Plan}: buffer up to 1.5s OR until `message_stop`, then forward batched content to peer's `input_tx`.
  - If phase ∈ {Apply, Verify}: forward only on `message_stop` (pure turn-based).
  - Tool-use events (`ask_user_choice`, `mark_awaiting_user`) are NOT forwarded to peer (they're UI signaling).
- 4.6 `[P]` Tests with mocked agent handles: verify broadcast hits both, buffer-rule fires correctly per phase, synthetic phase-advance message emitted.

**Verification:** `cargo test core` passes. Manual integration with real agents in Phase 9.

---

### Phase 5 — UI scaffold

**Status:** Not started
**Goal:** Slint views for each tab + view-model bridge to core state.

- 5.1 `[S]` `ui/app.slint`: top-level Window with topbar (Dashboard / Context Library / Settings tabs + Emma button float-right). State property `current_tab`, `emma_open`.
- 5.2 `[P]` `ui/dashboard.slint`: grid of session tiles from a `[SessionTileModel]`. Each tile: scope title, phase chip, last-activity timestamp, `[Need User Input]` badge slot, choice-buttons slot. Click → open session view (replaces dashboard grid in content area; topbar Dashboard tab stays highlighted). **Suppression rule:** when a session is the active session view, its tile suppresses choice buttons and the `[Need User Input]` flag (those render inline in the session view instead — never duplicated).
- 5.3 `[P]` `ui/settings.slint`: list of agents (Emma/Brian/Rain), per-agent edit form (provider dropdown, model field, base_url field, auth_token field with show/hide). Save button → upsert agent_configs.
- 5.4 `[P]` `ui/session.slint`: top header with phase chip + scope title + `← All sessions` link; left sidebar (200px, resizable) listing all active sessions (active session highlighted, other entries show `[Need User Input]` badges + phase chip); two-pane chat (Brian left, Rain right); `[Need User Input]` banner above the prompt bar when the current session is awaiting input; broadcast prompt bar at bottom. Choice buttons render inline in chat at the agent message position. **Responsive:** at content widths <1200px, Brian/Rain panes stack vertically inside the chat area.
- 5.5 `[P]` `ui/context_library.slint`: file tree on the left, editor pane on the right, save button. Reads/writes filesystem under `<data_dir>/`.
- 5.6 `[P]` `ui/emma.slint`: half-pane chat (slides over right half when `emma_open=true`). Subscribes to messages where session_id="emma". When Emma is open + session view is active, layout becomes `Sidebar | Brian | Rain | Emma`. Brian/Rain panes use vertical-stack responsive behavior from 5.4 when content width is tight. Sidebar icons-only collapse toggle deferred to Phase 9 polish.
- 5.7 `[S]` `src/ui/view_model.rs`: bridges Slint Models/Properties to `AppState`. Wires Slint callbacks (button clicks, form submits) to core API calls.

**Verification:** Manual: every tab renders, Settings save works, Dashboard tiles render from active sessions list, prompt-bar input persists as a user message.

---

### Phase 6 — UI signaling integration

**Status:** Not started
**Goal:** Tool calls from agents render as buttons on dashboard tiles AND in session chat. `[Need User Input]` flag works.

- 6.1 `[S]` Wire `PendingChoice` event (from Phase 3.2) to view-model: sets a per-session pending-choice state.
- 6.2 `[S]` UI renders the choice buttons in: (a) dashboard tile inline; (b) session chat at the message position of the tool call.
- 6.3 `[S]` On user click, view-model invokes the MCP server's response callback → tool returns the picked option to the agent.
- 6.4 `[S]` Wire `AwaitingUser` event (from Phase 3.3) to view-model: sets per-session `[Need User Input]` flag. Clears on next user message in that session.
- 6.5 `[P]` Tests: integration test that spawns agent, has it call each tool, drives UI state machine, verifies correct rendering + agent continuation.

**Verification:** Manual: prompt agent to call `ask_user_choice`, see buttons on tile + chat. Click → agent continues. Prompt agent to call `mark_awaiting_user`, see flag → send any user message, flag clears.

---

### Phase 7 — Emma chat

**Status:** Not started
**Goal:** Emma as a singleton always-running session, half-view chat panel.

- 7.1 `[S]` On app startup, ensure Emma subprocess is running against `session_id="emma"`. Respawn if it dies.
- 7.2 `[S]` Wire prompt-bar input in Emma view to write only to Emma's stdin (not broadcast — Emma is solo).
- 7.3 `[P]` Emma chat history pulls from messages where session_id="emma", ordered by created_at.
- 7.4 `[P]` Toggle behavior: clicking the Emma button in topbar slides Emma chat over right half of the window. Click again to dismiss.
- 7.5 `[P]` Tests: send Emma a message, verify response persists; close + reopen Emma chat, verify history retained.

**Verification:** Open Emma, chat, close, reopen — works.

---

### Phase 8 — Context Library integration + system prompt assembly

**Status:** Not started
**Goal:** CL is the source of truth for startup prompts; concatenation logic lives in one place; CL UI tab edits write through. Phase A's output seeds the CL.

- 8.1 `[S]` First-run init seeds CL from Phase A output (if `~/.bot-hq-dev/` or destination contains a complete rebuilt CL, copy it). Fallback to baked-in minimal templates if Phase A output not present.
- 8.2 `[P]` `src/cl/reader.rs::read_startup(agent_name, project: Option<&str>) -> String`: concatenates `general-rules.md` + `agents/<name>/startup.md` + (if project provided) `projects/<p>/conventions.md` + `projects/<p>/notes.md`. Skip missing files gracefully (log warning).
- 8.3 `[S]` Refactor Phase 4.1 to use this reader. Project name derived from `working_repo_path.file_name()`.
- 8.4 `[P]` Context Library UI (Phase 5.5): edits write back to filesystem via `src/cl/writer.rs`. No auto-accumulation from agent activity — explicit user saves only.
- 8.5 `[P]` Tests: round-trip CL read, project detection, edit-and-save round-trip, missing-file handling.

**Verification:** Edit Brian's startup.md from the UI, spawn a new session, verify the change reflected in agent behavior.

---

### Phase 9 — End-to-end + polish + commit

**Status:** Not started
**Goal:** Shippable first version.

- 9.1 `[S]` End-to-end smoke: open app → create session pointed at a real repo → drive a small task through I/P/A/V using the broadcast prompt bar and IPAV chip → verify all UI signaling works → close session.
- 9.2 `[P]` UX polish: keyboard shortcuts (Cmd-N new session, Cmd-, settings, Cmd-K Emma toggle), scroll-to-bottom on new messages, tile sort order (active first), empty-state copy.
- 9.3 `[P]` Error handling: agent crash recovery (offer respawn in UI), DB write failures (surface error), MCP errors surfaced inline.
- 9.4 `[P]` Documentation: `README.md` with setup steps (Rust toolchain, claude-code CLI, first-run flow, `BOT_HQ_DATA_DIR` override), `ARCHITECTURE.md` mirroring the project memory.
- 9.5 `[S]` **Terminal state — ready for human review.** Do NOT commit or push autonomously. Instead:
  - Verify `cargo build --release` succeeds, `cargo test` passes (live tests gated by `RUN_LIVE_TESTS=1` may pass or skip).
  - Manual E2E smoke (9.1) runs cleanly — document the run in PROGRESS.md.
  - Draft a candidate commit message in `docs/commit-message-draft.md` (subject: `Rebuild from Go/tmux to Rust/Slint` + body summarizing what was built, referencing ARCHITECTURE.md).
  - Update PROGRESS.md top banner to **READY FOR HUMAN REVIEW** with a summary: what works, what doesn't, any known caveats, link to `docs/commit-message-draft.md`.
  - **Stop.** Wait for human to inspect and run the commit + push themselves.

**Verification:** `cargo build --release` exits 0. `cargo test` exits 0. PROGRESS.md shows READY FOR HUMAN REVIEW state. `docs/commit-message-draft.md` exists.

---

### Deferred phases (separate plans)

- **Phase 10:** Discord plugin (1st plugin) — contract design + implementation
- **Phase 11:** Clive plugin (2nd plugin) — port from current bot-hq

---

## Decisions deferred to implementation

1. **Async runtime:** tokio (default lean — confirm during 0.3).
2. **DB driver:** sqlx-sqlite (async + compile-time queries — confirm during 0.3).
3. **MCP SDK:** decided in 0.4.
4. **Auth-token storage:** v1 = sqlite plaintext, document keychain upgrade path during 0.6.
5. **Slint state pattern:** decided in 0.3.
6. **Tile-update mechanism:** Slint Model invalidation cadence — settled during 5.2.

---

## Sub-agent dispatch guidance

Tasks marked `[P]` within a phase can run in parallel under independent sub-agents. Each sub-agent briefing should include:

1. **Goal:** the task subject from this plan.
2. **Files:** exact paths to create/modify.
3. **Interface:** function signatures expected, types referenced.
4. **Tests:** specific `cargo test` invocations to verify.
5. **Definition of done:** the verification line at the end of each phase.

Recommended cadence:
- **Phase A (CL rebuild) runs in parallel** with all code phases. Best done early so dev sessions have rebuilt CL context.
- **One code phase at a time.** Don't dispatch Phase N+1 work before Phase N's verification passes.
- **Research phase (0) first**, single-threaded or lightly parallel. Findings unblock implementation choices in later phases.
- **Storage + agent runtime + MCP server (Phases 1, 2, 3)** are mostly independent — can be dispatched in parallel after Phase 0 completes.
- **Core (Phase 4)** depends on 1+2; gate it on those.
- **UI scaffold (5)** can start in parallel with Core (4) as long as the view-model interface is agreed upfront.
- **UI signaling integration (6), Emma (7), CL integration (8)** depend on both Core and UI scaffold landing. Phase 8 also depends on Phase A's output being staged.
- **Phase 9** is integration + manual user-driven verification.

---

## Orchestrator workflow

**Primary mode:** single claude-code session as orchestrator, dispatching sub-agents for parallel work.

**Dispatch cadence:**
- **Within a phase:** all `[P]`-tagged tasks dispatch in parallel as separate sub-agents (single message, multiple `Agent` tool calls).
- **Across phases:** gate on prior phase's verification line before dispatching the next phase's tasks.
- **Sequential `[S]`** tasks: orchestrator handles inline or dispatches one at a time.
- **Research `[R]`** tasks: dispatch as `general-purpose` or `Explore` sub-agents with the specific source URLs from the plan.

**Sub-agent brief template** (use for each dispatch — see "Sub-agent dispatch guidance" below for the full version):
- **Goal:** task subject from PLAN.md
- **Files:** exact paths to create/modify
- **Interface:** function signatures, types referenced
- **Tests:** specific `cargo test` invocations to run
- **Definition of done:** the verification criterion from PLAN.md

**After each sub-agent returns:** orchestrator reviews changes, runs tests, integrates if needed, ticks the task in PROGRESS.md, appends to "Sub-agent dispatch log".

## Context management (fallback)

If orchestrator context exhausts mid-build, the next fresh session continues from PROGRESS.md:

1. Tick completed checkboxes (`[ ]` → `[x]`) in PROGRESS.md.
2. Update "Currently working on" with file paths + status.
3. Log any blockers.
4. Document autonomous decisions.
5. Append to "Session handoff log" with a clear resume hint.
6. Update ARCHITECTURE.md if you discovered architecturally-relevant info.

The NEXT session reads `CLAUDE.md` → `ARCHITECTURE.md` → `PLAN.md` → `PROGRESS.md`, then continues. This is contingency, not the primary mode — design for single-session completion.

---

## Autonomous completion criteria

The project is **complete (autonomously)** when:

1. All Phase 0–9 tasks except 9.5's commit/push are checked off in PROGRESS.md.
2. `cargo build --release` succeeds.
3. `cargo test` passes (non-live tests; live tests gated by `RUN_LIVE_TESTS=1` may pass or skip).
4. Manual E2E smoke (Phase 9.1) is documented as working in PROGRESS.md.
5. `docs/commit-message-draft.md` exists with a clean candidate commit message.
6. PROGRESS.md top banner shows **READY FOR HUMAN REVIEW**.

At that point: **stop**. Do not commit. Do not push. The human will inspect and complete the commit step.

---

## Progress log

(append entries as phases complete)

- 2026-05-14: Plan drafted.
- 2026-05-14: Phase A (CL rebuild) and Phase 0.8 (filesystem layout) added. Path defaults standardized to `~/.bot-hq/` with `BOT_HQ_DATA_DIR` env-var override for dev.
