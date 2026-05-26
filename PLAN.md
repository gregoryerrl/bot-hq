# bot-hq — Forward Plan

What's next for bot-hq. The original rebuild roadmap (Phases A–9 of the
from-scratch rebuild) shipped — that document is preserved at
[`docs/rebuild-archive/PLAN-rebuild-era.md`](docs/rebuild-archive/PLAN-rebuild-era.md).

For what bot-hq is right now see [`ARCHITECTURE.md`](ARCHITECTURE.md).
For recent changes see [`PROGRESS.md`](PROGRESS.md).

---

## Current state (TL;DR)

bot-hq is built and used. The rebuild milestone (v0.1.0) shipped, the
session-permission grants work landed, and the **Tauri v2 migration
landed 2026-05-26** on branch `tauri-v2-migration` (7 batches; see
PROGRESS.md). React frontend in `frontend/`, Slint deleted, Rust core
untouched.

253 Rust tests passing (205 lib + 31 external_mcp + 7 signaling + 10
storage) plus 12 frontend Vitest. Release build clean.

---

## In flight

### Tauri v2 migration follow-ups

Status: branch `tauri-v2-migration` shipped 7 batches; awaiting user
manual-smoke + merge to main.

**To finish:**
- Run `cargo run --release` from the worktree; verify dashboard, new
  session, IPAV tabs, Emma overlay all work.
- Wire `broadcast_to_session` Tauri command — `ChatInput` callbacks are
  inert until a `core::broadcast` helper lands. Needed for chat-driven
  agent prompts to actually reach the agents.
- Port `view_model::parse_diff_lines` to a Rust-side `compute_apply_diff`
  Tauri command + a TypeScript renderer for the A tab git-diff view.
- Plugin install flow (manage `tauri_cmd/plugins.rs`) + iframe
  ping/pong frontend channel for the heartbeat watcher.
- Replace the placeholder `icons/icon.png` with the real bot-hq mark.
- Update `~/.bot-hq/projects/bot-hq/{conventions,notes}.md` to drop the
  Slint sections + slint-rust-docs cross-references (CL is shared, so
  deferred until the migration merges to main).

---

## Backlog

### UX polish (deferred from rebuild Phase 9.2)

- Keyboard shortcuts: Cmd-N (new session), Cmd-, (settings), Cmd-K
  (Emma toggle), Cmd-Enter (submit prompt).
- Scroll-to-bottom on new messages (sticky if user is at bottom; idle
  if user has scrolled up).
- Tile sort order (active sessions first, then by last activity).
- Empty-state copy on the Dashboard (currently shows a card; needs the
  prose to be welcoming).
- Responsive Brian/Rain vertical stack at content widths < 1200px (the
  single-chronological-chat redesign mooted this, but keep the option
  on the table if the two-pane view is requested back).

### Auth-token v2 — OS keychain

Migrate `agent_configs.auth_token` from plaintext sqlite to OS keychain
via `keyring-core`. Per-platform backends: macOS Keychain Services,
Windows Credential Manager, Linux Secret Service (dbus).

**Migration logic** (runs once, gated by a `schema_version` row):
1. Read each non-NULL `auth_token` from `agent_configs`.
2. For each, `Entry::set_password` under
   `("bot-hq", format!("{project}:{agent}:{provider}"))`.
3. NULL the column.
4. Bump `schema_version`.

Fall back to plaintext-sqlite mode with a startup warning on keychain
failure (headless CI, Linux without Secret Service daemon).

Original Phase 0 research: [`docs/rebuild-archive/decisions.md`](docs/rebuild-archive/decisions.md#auth-storage).

### Violations log — UI viewer

The `violations.jsonl` file exists and is written, but the Settings →
Violations viewer is a stub. Build it out:
- Tail the file (mtime poll, ~2s cadence).
- Filter by kind (`CommitGrep`, `PushDenied`, `PolicyMutation`, …),
  by session, by date.
- Click-through to the source message in chat where possible.

### Sub-agent dispatcher integration

Brian can already use the `Agent` tool to dispatch sub-agents within
claude-code. Worth wiring a visualization so the UI knows which session
spawned a sub-agent — currently sub-agents are invisible to bot-hq.
Open question: do we surface them as nested message threads, or as
phantom sessions on the dashboard?

---

## Deferred (separate plans)

### Discord plugin

Bridge bot-hq sessions to/from a Discord channel. Original scope from
the rebuild plan. Contract TBD: probably a per-channel session, with
message-author mapping (Discord user → bot-hq user / agent author).
Needs its own design doc before implementation.

### Clive plugin

Port of legacy bot-hq's Clive bot (Twitch/IRC integration). Needs its
own design doc.

### Cross-platform builds

Slint covers macOS, Linux, Windows. Initial focus is macOS. Linux + Windows
builds need: per-platform CI, install paths (`directories` crate likely
already handles this), keychain backends (see auth-token v2), font
loading for the icon font.

---

## Architectural ideas (no commit yet)

- **Move CL writes to a transaction model.** Today CL writes are
  filesystem-direct + index re-stat. Wrapping in a transaction (write
  to temp file → rename → index update in one sqlite tx) would harden
  against partial-write failures.
- **Hot policy reload.** Today the policy block in an agent's system
  prompt is fixed at session spawn. Editing `policy.yaml` mid-session
  requires session restart for the agent to see new rules (though hooks
  + MCP tools always re-resolve on call). Consider a "policy reload"
  banner that re-spawns the duo.
- **Persistent IPAV phase log.** IPAV is in-memory only. Phase
  transitions are visible in chat as synthetic user messages, but the
  per-session phase history isn't queryable. Worth keeping for
  retrospectives (which phases consumed the most time).
- **Question tray garbage collection.** The `questions` table grows
  unbounded — resolved questions stay forever. A periodic purge of
  resolved rows older than N days would keep it bounded.

---

## Out of scope

- **Web UI.** bot-hq is desktop-only by design. The external MCP
  driver server already enables programmatic access; a web frontend
  would be a separate product.
- **Multi-user / multi-tenant.** Single-developer-workstation is the
  design target. Shared workstations are out of scope (auth-token
  threat model assumes single user).
- **Migration of legacy bot-hq runtime state.** Sessions / hub history
  / last-state files from the Go/tmux/MCP-hub bot-hq do NOT carry over.
  Project CL was distilled once at rebuild time; further sync is
  manual.
