# bot-hq — Project Instructions (for claude-code)

You are working on **bot-hq**, a Tauri v2 + React + Rust desktop GUI app
for driving AI-assisted coding sessions through a bilateral-duo agent
model (Brian = HANDS, Rain = EYES) with policy enforcement. A former
solo helper agent, Emma, was removed from the core (planned to return as
the first bot-hq plugin — TBD).

The original from-scratch rebuild shipped at v0.1.0; subsequent work
added a UI redesign, an external driver MCP server, and a two-layer
policy enforcement layer (MCP tools + git hooks). Current work is
maintenance + feature-extension on the existing system.

## Read these files FIRST, in order:

1. **[`ARCHITECTURE.md`](ARCHITECTURE.md)** — what bot-hq IS right now
   (process model, both MCP servers, policy layer, session permissions,
   storage schema, glossary).
2. **[`PLAN.md`](PLAN.md)** — what's planned next (in-flight work,
   backlog, deferred plugins).
3. **[`PROGRESS.md`](PROGRESS.md)** — recent change log,
   newest-first.

These three are the canonical docs. The original rebuild design +
roadmap + Phase 0 research are preserved under
[`docs/rebuild-archive/`](docs/rebuild-archive/) for historical
reference — do not treat them as current.

---

## Tauri + React UI work

The frontend is React 18 + TypeScript + Tailwind in `frontend/`. Tauri
commands live in `src/tauri_cmd/<domain>.rs` as thin `#[tauri::command]`
wrappers over `SignalingBridge` / `Storage` methods. Events flow through
`src/tauri_events/`: bridge subscriber → `BatchEmitter` (since_id
watermark, 50ms / N=20 coalesce) → `app.emit(name, payload)`. TypeScript
bindings auto-generate via `tauri-specta` on each app launch (writes
`frontend/src/lib/bindings.ts`).

Plugin model (scaffolded; live plugins TBD): per-plugin origin via
custom URI scheme (`https://plugin-<id>.localhost`), capability-gated
via Tauri JSON, host-side heartbeat watcher at app-shell level.

---

## Operating mode

This is **maintenance + feature-extension** mode. The big build is
done; work is now incremental.

- **Take work in small testable chunks.** Compile + test after each
  change.
- **For non-trivial multi-step features,** spawn the `Agent` tool for
  parallel work on independent sub-tasks. Brief each sub-agent with:
  goal, files, interface, tests, definition of done.
- **Don't litigate decisions already shipped** in `ARCHITECTURE.md`
  (e.g., HTTP MCP not stdio+UDS, hand-rolled JSON-RPC, hardcoded role
  prompts, two-layer policy enforcement). Reopen only with a clear
  reason.
- **When unsure about scope or direction,** ask the user via the
  bot-hq `ask_user_choice` MCP tool (don't write prose questions —
  they don't surface cleanly in the UI).

---

## Critical rules

- **Commit conventions are config-driven, not shipped here.** Subject
  style and the forbidden-word list resolve from the project `policy.yaml`
  + the user's `custom-general-rules.md` in the Context Library — personal
  / per-project config, not product rules baked into this repo. Whatever
  the resolved policy forbids is enforced by the `commit-msg` git hook +
  the `check_commit_message` MCP tool; call that tool before every commit,
  and don't bypass the hooks (`--no-verify` / hook-skipping).
- **Push is governed by the session's `push_gate` policy toggle**
  (`auto` | `ask`, inherited general→project→session, editable in the
  gear tab). Under `ask`, just run `git push` — the pre-push hook
  surfaces a per-push Approve/Reject prompt and blocks on the user's
  pick. There are no agent-side push grants. Don't push because
  permission feels implicit.

---

## Data paths during dev

Keep `BOT_HQ_DATA_DIR=~/.bot-hq-dev/` in `.env`. The default
`~/.bot-hq/` collides with any running production bot-hq.

`<data_dir>` layout (see ARCHITECTURE.md for the full list):
- `.local/bot-hq.db` — sqlite
- `.local/lock` — single-instance lock
- `.local/session-policies/<sid>.yaml` — per-session policy snapshots
- `mcp-token` — external MCP bearer token (UUIDv4, 0600)
- `violations.jsonl` — policy audit trail
- `custom-instructions.md` (all agents), `general-rules.md`,
  `projects/<p>/{conventions,notes,policy.yaml,…}.md` — CL content

---

## How a typical session looks

1. Read ARCHITECTURE.md, PLAN.md, PROGRESS.md (in that order) to
   refresh context.
2. Identify the task. If it's in-flight per PROGRESS.md / PLAN.md,
   pick up where it left off. Otherwise scope it.
3. Write code in small chunks. Run `cargo test` + `cargo build` after
   each chunk.
4. For multi-file or multi-day work, update PROGRESS.md with a
   newest-first entry summarizing what changed and why.
5. When the work is ready for the user to see, mark the task complete
   and surface a summary.

---

## Working tree state

The working tree is normal. The original autonomous-build commit landed
long ago; Go-file deletions from the rebuild are already in history.
Don't expect "unstaged deletions" from a prior architecture — that was
the rebuild milestone state, not current.

In-flight work appears as standard staged/unstaged changes plus
untracked files. If you encounter an unexpected file or branch,
investigate before deleting (it might be the user's WIP from another
session).

---

## Handling ambiguity

If something is genuinely ambiguous and NOT decided in ARCHITECTURE.md
or PLAN.md:

1. Ask the user via `ask_user_choice` with 2–4 concrete options.
2. If the user is offline / unresponsive, default to the simplest
   reasonable choice consistent with the documented direction and
   note your call in PROGRESS.md.
3. Don't block on minor choices the user can revise later.

---

## Prerequisites

- Rust stable toolchain (rustup, latest stable).
- Node.js 22+ and npm (for the React frontend; pnpm works too).
- `claude-code` CLI installed and authed (used as subprocess by each
  agent + needed for live tests).
- macOS (initial target; Linux + Windows tracked in PLAN.md).

If a prerequisite is missing, document it and continue with
non-blocked work.
