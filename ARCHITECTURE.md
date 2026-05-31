# bot-hq — Architecture

This is the single source of truth for what bot-hq IS right now. It
describes the running system, not the original rebuild design — that
lives at [`docs/rebuild-archive/ARCHITECTURE-rebuild-era.md`](docs/rebuild-archive/ARCHITECTURE-rebuild-era.md).

For user-facing setup see [`README.md`](README.md). For planned work
see [`PLAN.md`](PLAN.md). For recent change log see
[`PROGRESS.md`](PROGRESS.md).

---

## Overview

bot-hq is a desktop GUI app for driving AI-assisted coding sessions
through a bilateral-duo agent model with policy enforcement. Each
session spawns two `claude-code` subprocess agents:

- **Brian** (HANDS) — executes: edits, commits, runs bash, calls tools.
- **Rain** (EYES) — reviews: read-only, adversarial counterpart. Write and
  mutation tools (Edit/Write/NotebookEdit/Task + git/gh) are denied via
  `--disallowedTools` on her claude-code subprocess; HANDS-only signaling
  MCP tools are additionally gated server-side.

A third agent, **Emma**, is a singleton solo helper (not a duo). User
summons her for one-off questions; she lives at `session_id="emma"` and
persists across app restarts.

The user is the orchestrator; the app is the conductor between user and
agents. Policy enforcement runs at two layers (MCP tool calls + git
hooks). Two MCP servers run in-process: one for agent ↔ UI signaling,
one for external driver clients.

**Stack:** Tauri v2 shell + React 18 frontend, Rust core on a tokio
multi-thread runtime. Tauri owns the OS main thread.

---

## Process model

```
                    ┌────────────────────────────────────────┐
                    │  bot-hq (Rust binary, main thread)     │
                    │                                        │
                    │  Tauri webview ◄──── AppState (Arc) ───┤
                    │                       │                │
                    │   ┌───────────────────┴─────────────┐  │
                    │   │  tokio runtime (worker threads) │  │
                    │   │   - signaling::SignalingBridge  │  │
                    │   │   - internal MCP HTTP server    │  │
                    │   │   - external MCP HTTP server    │  │
                    │   │   - per-session duo coordinator │  │
                    │   └─────────────────────────────────┘  │
                    └────────┬────────────┬──────────────────┘
                             │            │
                ┌────────────▼─┐  ┌───────▼─────┐  ┌──────────────┐
                │ claude-code  │  │ claude-code │  │ claude-code  │
                │   (Brian)    │  │   (Rain)    │  │   (Emma)     │
                │ stream-json  │  │ stream-json │  │ stream-json  │
                └──────────────┘  └─────────────┘  └──────────────┘
```

Each agent subprocess is spawned with:

```
claude -p \
  --input-format stream-json --output-format stream-json --verbose \
  --append-system-prompt <inline-text> \
  --mcp-config <per-agent-config.json> \
  --strict-mcp-config \
  --dangerously-skip-permissions
```

`--dangerously-skip-permissions` is intentional: bot-hq IS the policy
layer. claude-code's own permission prompts would double-gate and hang
the agent (the bot-hq policy gates already prompt the user). Enforcement
is provided by the policy layer + git hooks.

Per-agent model swap via env-vars sourced from the `agent_configs`
table: `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_MODEL`.
`BOT_HQ_SESSION_ID` is also injected so git-hook subprocesses can read
session-scoped state.

---

## Agent role prompts (hardcoded)

Role prompts (Brian/Rain/Emma identity) are baked into the binary at
`src/agents/prompts.rs`. They are NOT CL-loaded. Reasoning:

- Role boundary (Brian writes, Rain reviews) is structural — a typo in a
  CL file shouldn't be able to break it.
- Hardcoded prompts protect the role identity through CL edits, custom
  instruction changes, etc.

CL still supplies per-project + per-user customizations on top (custom
instructions, general rules, project policy directives). The hardcoded
prompt is the floor; CL extends it.

System-prompt layering at session spawn (`src/core/session.rs::read_system_prompt`):

1. Hardcoded role prompt (Brian/Rain/Emma)
2. CL location anchor (`<data_dir>` path)
3. `<data_dir>/general-rules.md`
4. `<data_dir>/agents/<name>/custom-instruction.md`
5. Resolved policy directive block (forbidden words list, push-gate
   mode, etc.)

Project conventions/notes are deliberately NOT injected — agents use
the `cl_index_search` MCP tool + `Read` to fetch project context
on-demand.

---

## Bilateral duo coordination

Stream-json events flow Brian → Rain and Rain → Brian via the duo
coordinator (`src/core/duo.rs`). Forwarding rules per phase:

- **Investigate / Plan:** 1.5s buffer OR until `result` event, whichever
  first. Preserves live adversarial riff.
- **Apply / Verify:** pure turn-based — forward only on `result` event.
  Less interleaving, more execution focus.

**Suppressed from peer forwarding:**
- Tool-use events (`ask_user_choice`, `mark_awaiting_user`,
  `request_approval`, etc.) — these are agent ↔ UI signaling, not
  agent-to-agent coordination.
- Anything emitted while the session's `awaiting` flag is set —
  silent-on-hold protocol. The flag is set by `mark_awaiting_user`,
  `ask_user_choice` (until resolved), and `request_approval` (until
  approved/denied). Forwarding resumes when the flag clears.

---

## Tauri + React UI

**Stack:** Tauri v2 shell + React 18 + TypeScript + Tailwind + minimal
shadcn-style primitives (Vite build). Tauri owns the OS main thread; the
Rust core runs on a multi-thread Tokio runtime.

**IPC:** Tauri commands + Tauri events. No HTTP from the frontend. The
existing `SignalingBridge` stays the single source of truth — a thin
command layer in `src/tauri_cmd/` wraps bridge methods, and a
broadcast-subscriber bridge in `src/tauri_events/` translates
`SignalingEvent` into typed Tauri events. The hot path
(`MessagePersisted` IDs → batched message fetch via existing
`messages_for_session(session_id, since_id)`) goes through a
`BatchEmitter` (N=20 / 50ms coalesce).

**Topbar:** `Dashboard | Context Library | Plugins | Settings` + Emma
button.

**Dashboard:** grid of session tiles. Each tile shows title, last
activity, `[Needs Input]` badge tinting the border red. Click tile →
opens session view. Inline `+ New session` form creates rows + registers
the session with the bridge.

**Session view:** 60/40 split — chat (left) + DocumentPane (right).
Header: title + back link. Chronological chat: all messages (user,
Brian, Rain, phase_change) interleaved by `created_at` with author color
coding (brian=orange, rain=purple, emma=green, user=blue, system=muted).
Pending-choice banner (purple) renders above the input with inline
choice buttons.

**DocumentPane:** IPAV tab selector (I/P/A/V chips) drives
`session_doc_search(session_id, phase=<x>)`. Each tab renders matching
`session_documents` rows; counts surface on the chips. The A tab also
renders the live color-coded `git diff` for the session's working repo
via the `compute_apply_diff` Tauri command (`src/tauri_cmd/docs.rs`,
parser `parse_diff_lines`), consumed by `DocumentPane.tsx`.

**Emma overlay:** fixed half-pane on the right, toggled from the topbar.
Subscribes to the `agent.messages.batch` event filtered to
`session_id="emma"`.

**Context Library tab:** 2-pane "Library Tree" sidebar + tabbed editor. The
tree renders nested collapsible folders (`cl_index_search` + `cl_folder_search`).
Files open a read-write editor (`cl_read_file` / `cl_write_file`; binary +
truncated files are read-only so a lossy save can't corrupt them). Folders open
a folder-view that edits the folder description (`cl_set_folder_description`)
and, at the project root, configures + registers / unregisters the project
(`cl_register_project` / `cl_unregister_project`); a sidebar modal registers an
arbitrary on-disk folder as a new project. Right-click gives VSCode-style new
file / new folder / rename / delete (`cl_create_file` / `cl_mkdir` / `cl_rename`
/ `cl_delete_path`, each followed by `cl_rescan`). Substring search + project
filter.

**Plugins tab:** Placeholder UI surfaced from `tauri_cmd/plugins.rs`
(landing later). Rust scaffold in `src/plugins/` ships the manifest
parser, loader, capability JSON generator, and host-side heartbeat
watcher.

**Settings tab:** per-agent config (provider, model, base_url,
auth_token). Per-row accent dot keyed to author color. Plaintext-token
warning preserved.

**Plugin model:** iframes at per-plugin origin
(`https://plugin-<id>.localhost`) via Tauri custom URI scheme; each gets
a generated capability JSON listing only the commands its manifest
requested. Heartbeat watchers register at app-shell level (NOT
per-PluginSlot — those remount).

---

## Internal MCP server (UI signaling)

In-process HTTP MCP server, hand-rolled JSON-RPC over hyper 1.x. Lives
in `src/signaling/` (`jsonrpc`, `protocol`, `server`, and the `bridge/`
submodule tree). Surface:

- **Bind:** `127.0.0.1:<ephemeral>` (chosen at startup; ephemeral port).
- **URL per agent:** `http://127.0.0.1:<port>/sessions/<id>/<agent>/mcp`.
  Each agent's `--mcp-config` file points at its own URL so the bridge
  knows which agent is calling.
- **Methods:** `initialize`, `ping`, `tools/list`, `tools/call`.

**26 internal tools** (see [README.md](README.md#internal-mcp-tools-served-to-child-agents)
for the full list with descriptions): `ask_user_choice`,
`mark_awaiting_user`, `advance_phase`, `request_phase_advance`,
`request_approval`, `check_commit_message`, `close_session`,
`list_my_pending_questions`, `withdraw_question`, `supersede_question`,
`session_doc_write`, `session_doc_search`, `session_doc_read`,
`cl_index_search`, `cl_register_read`, `cl_rescan`, `cl_folder_search`,
`cl_register_folder_description`, `grant_session_permission`,
`revoke_session_permission`, `list_session_permissions`,
`webview_screenshot`, `webview_click`, `webview_type`, `webview_scroll`,
`webview_press_key`.

**Role enforcement at the dispatch layer:** `HANDS_ONLY_TOOLS` is a
hard-coded list of tools Rain (EYES) cannot call. Tool calls from Rain
to any HANDS-only tool return a `HANDS_ONLY_TOOLS` JSON-RPC error. The
boundary is structural, not just convention.

**Bridge (`src/signaling/bridge/`)** owns:
- Storage handle (writes question rows, message rows, violations).
- Policy resolver (loads `general-policy.yaml` + `projects/<p>/policy.yaml`).
- Session → project mapping.
- Per-session `awaiting` halt flag (shared `Arc<AtomicBool>` with duo
  pump).
- Session permissions cache (mirrored to disk for hooks).
- Question tray storage (`questions` table — persists `ask_user_choice`
  prompts so they survive app restart).

---

## External MCP server (driver tools)

Second HTTP MCP server for external agents (another claude-code
session, a test driver). Lives in `src/signaling/external_jsonrpc.rs`
+ `src/signaling/external_server.rs`.

- **Bind:** `127.0.0.1:7892` (override via `BOT_HQ_EXTERNAL_MCP_PORT`;
  disable via `BOT_HQ_EXTERNAL_MCP_DISABLED=1`).
- **Auth:** bearer token at `<data_dir>/mcp-token` (UUIDv4, 0600,
  auto-generated). Constant-time comparison via the `subtle` crate.
- **Soft-fail:** if port is taken, internal MCP keeps working, external
  marks "unavailable" — bot-hq stays usable.

Tools: see [README.md](README.md#available-external-tools) for the full
list (21 driver tools including `list_sessions`, `create_session`,
`send_message`, `wait_for_change`, `get_session_snapshot`, etc.).

---

## Policy enforcement

**Goal:** enforce per-project rules (forbidden commit words, push gate,
force-push tokens, tool blocklist) reliably even when an agent's context
drifts and forgets to call the MCP tool.

**Two layers** (`src/policy/`):

1. **MCP tools** (`check_commit_message`, `request_approval`, …) are
   the primary path. Agents are instructed in their system prompt to
   call them before the corresponding bash op. Skipping logs a
   `Denied` violation to `<data_dir>/violations.jsonl`.
2. **Git hooks** are the deterministic backstop. `bot-hq install-hooks`
   writes `commit-msg`, `pre-commit`, `post-commit`, `pre-push` into
   `.git/hooks/` of the working repo. Each hook execs
   `bot-hq policy-check <sub> --data-dir … --project … --session …`
   which re-resolves policy and decides exit code. Hooks are
   idempotent, respect foreign hooks (write `<hook>.bot-hq` sidecar
   instead of clobbering).

**Policy file hierarchy:**
- `<data_dir>/general-policy.yaml` — defaults.
- `<data_dir>/projects/<project>/policy.yaml` — per-project overlay
  (lists are replaced, not merged).

Fields: `forbidden_in_commits`, `push_gate.mode`
(`auto`|`per_branch_approval`|`always_ask`), `force_push.mode`
(`blocked`|`token_required`|`allowed`), `force_push.token_format`,
`per_action_approval`, `branch_pattern`, `commit_style`.
(`tool_blocklist` is RETIRED — superseded by the global Tool Gate
below; the field still parses for backward-compat but is no longer
enforced.)

**Hook details:**
- `commit-msg`: receives commit message file path as `$1`. Scans for
  forbidden words (stripping `#` comment lines). Exits 1 on hit.
- `pre-commit`: scans staged diff added lines only (so removing a
  forbidden word passes). Exits 1 on hit.
- `post-commit`: read-only audit. Writes `CommitGrep` violation if a
  forbidden word slipped through (--amend, --no-verify bypass). Exits
  0 — the commit already happened.
- `pre-push`: reads `BOT_HQ_SESSION_ID`, loads the session's
  permissions JSON file, allows push if (a) `push_gate.mode == auto`,
  (b) session has an active grant covering the branch, or (c) branch
  is in `policy.push_gate.remembered_approvals`.

**Audit:** `src/policy/audit.rs` hashes each policy file at hook fire.
A hash change between fires logs a `PolicyMutation` violation
(audit-only in v1).

---

## Tool Gate

A global, user-configured keyword gate over agent **Bash** tool calls,
replacing the per-project `tool_blocklist` role (post-2026-05-29
fabricated-comment incident) with a single list that can also EXECUTE the
command on approval.

- **Config:** one global list at `<data_dir>/tool-gate.json` —
  `[{keyword, mode}]`, `mode` ∈ `gate | auto_allow`, edited in Settings
  ("Gated Bash Keywords"). NOT per-project, NOT in `policy.yaml` —
  bot-hq-side, so nothing is written into a working repo (disguise-safe).
  Matching is case-insensitive substring against the tool name or command;
  `gate` wins over `auto_allow` on conflict.
- **Tripwire:** the PreToolUse Bash hook (`policy-check tool-gate`, injected
  into HANDS/Emma at spawn via `--settings` — `src/policy/hooks.rs`
  `run_tool_gate`) blocks a `gate`-matched command with **exit 2** and routes
  the agent to the `action_gate` MCP tool; `auto_allow`/no-match exits 0 (runs
  normally). Exit 2 is the only block form honored under
  `--dangerously-skip-permissions`.
- **Execute-on-approve:** `action_gate(command)`
  (`src/signaling/bridge/action_gate.rs`) re-classifies, surfaces
  Approve/Reject via the existing `request_approval` machinery, and on approve
  runs the command itself in the session's `working_repo_path` (from storage),
  returning combined output to the agent — an action request, not a permission
  request. A gate-run `git push` first records a session push grant for the
  repo's current branch so the pre-push hook doesn't double-gate.

The global list defaults EMPTY (no gating until configured in Settings).

---

## Session permissions

Session-level commit/push grants live separately from the static
`policy.yaml` `remembered_approvals` list. They exist for the chat
pattern: user types "you can push" → Brian calls
`grant_session_permission(action="push", scope="all")` → all subsequent
pushes in this session bypass the per-action approval prompt.

**Storage** (`src/policy/session_permissions.rs`):
- In-memory cache on the `SignalingBridge` is the source of truth.
- Mirrored to `<data_dir>/.local/session-permissions/<session_id>.json`
  so the `pre-push` hook subprocess can read without HTTP-ing back.
- All files purged on bot-hq startup (cache is gone after restart;
  leftover files would let fresh sessions inherit grants they never
  earned).
- Per-session file deleted on `close_session`.

**Scope** (`GrantScope` enum):
- `None` — default. Ask every time.
- `AllBranches` — granted for any branch in this session.
- `Specific { branches }` — granted only for listed branches.

---

## Storage (sqlite)

Schema at `migrations/0001_init.sql` + subsequent migration files.

**Tables:**
- `messages` (id PK, session_id, author, kind, content, created_at) —
  full chat history. Index on `(session_id, created_at)`.
- `sessions` (id PK, title, working_repo_path, project, phase,
  created_at, closed_at, archived).
- `agent_configs` (agent_name PK, provider, model_name, base_url,
  auth_token). CHECK constraint enforces `agent_name ∈
  {'emma','brian','rain'}`.
- `questions` (choice_id PK, session_id, agent, kind, prompt,
  options_json, asked_at, resolved_at, picked) — durable question
  tray. Survives app restart.
- `cl_index` (file_path PK, project, description, tags, size,
  modified_at, indexed_at) — SQLite-backed CL search index.

**Author enum:** `user` / `emma` / `brian` / `rain`. NO `system` author —
phase changes synthesize as `author=user` ("phase advanced to PLAN") so
chat history reads coherently and agents see them as natural switch
prompts.

**Emma:** singleton row (`id="emma"`), seeded by migration, never
closes. Auto-spawned at startup; respawned by `restart_emma`.

---

## IPAV state

In-memory cache: `HashMap<SessionId, IpavState>` where `IpavState {
current_phase, phase_log }`. Not persisted. Subprocesses die with the
app; restart = fresh sessions (but Emma persists since her row + history
are durable).

---

## Context Library

Filesystem space at `<data_dir>/` holding agent custom instructions,
general rules, per-project conventions/notes. Indexed in `cl_index`
table for fast description-aware search via `cl_index_search`.

**Per-agent files** (always loaded at spawn):
- `agents/<name>/custom-instruction.md`
- `general-rules.md`

**Per-project files** (loaded on-demand via `cl_index_search`):
- `projects/<project>/conventions.md`
- `projects/<project>/notes.md`
- `projects/<project>/decisions.md`
- `projects/<project>/policy.yaml`
- Free-form: anything else under `projects/<project>/`

**CL writes are EXPLICIT** — user action via the Context Library tab.
Never auto-accumulated from agent activity.

**First-run init:** `templates/cl/` is baked into the binary. On first
start (no `cl-version.txt` in data dir), bot-hq writes the templates to
`<data_dir>/`. Missing individual files trigger an "initialize default"
button in the UI for that slot.

---

## Data locations

Defaults (env-overridable via `BOT_HQ_DATA_DIR`):

- **CL root + policy:** `<data_dir>/`
- **DB file:** `<data_dir>/.local/bot-hq.db`
- **Single-instance lock:** `<data_dir>/.local/lock`
- **Violations log:** `<data_dir>/violations.jsonl`
- **External MCP token:** `<data_dir>/mcp-token`
- **Session permissions mirror:** `<data_dir>/.local/session-permissions/<sid>.json`

**Dev:** `BOT_HQ_DATA_DIR=~/.bot-hq-dev/` keeps dev data separate from a
production install.

---

## Future: auth-token keychain migration

v1 stores auth tokens plaintext in sqlite. v2 plan: migrate to OS
keychain via `keyring-core`. Per-platform stores: macOS Keychain
Services, Windows Credential Manager, Linux Secret Service (dbus). The
migration logic: on v2-first-launch, read each non-NULL token from
`agent_configs`, `Entry::set_password` it under
`("bot-hq", format!("{project}:{agent}:{provider}"))`, then NULL out
the column. Bump a `schema_version` row so it runs once.

Fall back to plaintext-sqlite mode on keychain failure (headless CI,
Linux without Secret Service daemon) with a startup warning. See
[`docs/rebuild-archive/decisions.md`](docs/rebuild-archive/decisions.md#auth-storage)
for the original Phase 0 research.

---

## Plugins

Deferred to separate plans (not in current scope):

- **Discord plugin** — bridge sessions to/from a Discord channel.
- **Clive plugin** — port of legacy bot-hq's Clive bot.

Plugin contract TBD per plugin.

---

## Glossary

- **Bilateral duo:** Brian (HANDS — edits/commits/push) + Rain (EYES —
  review, no write tools). Spawned per session.
- **IPAV:** Investigate → Plan → Apply → Verify. Discipline framework
  agents follow within a session.
- **CL (Context Library):** filesystem space at `<BOT_HQ_DATA_DIR>`
  holding agent custom instructions, rules, per-project conventions/
  notes. Indexed in SQLite for description-aware search.
- **Session:** a scope-keyed work container, holding a duo of agent
  subprocesses + chat history.
- **Emma:** chat helper agent. Singleton (one per app). Solo (no Rain
  peer, no IPAV).
- **claude-code:** the upstream CLI tool that wraps a language model.
  One subprocess per agent.
- **stream-json:** claude-code's `--output-format stream-json` mode.
  One JSON event per line on stdout. See
  [`docs/stream-json-events.md`](docs/stream-json-events.md).
- **MCP (Model Context Protocol):** the protocol claude-code uses for
  external tool servers. Bot-hq runs two MCP servers in-process.
- **Policy:** machine-readable subset of CL rules — `general-policy.yaml`
  + project overlay. Drives forbidden-word grep, push gate, force-push
  gate, tool blocklist.
- **Session permission grant:** per-session commit/push authorization
  recorded by `grant_session_permission`. Distinct from
  `policy.yaml`'s static `remembered_approvals`.
- **Awaiting flag:** per-session `Arc<AtomicBool>` set by user-blocking
  tools (`mark_awaiting_user`, `ask_user_choice`, `request_approval`).
  When set, duo coordinator suppresses peer-forwarding —
  silent-on-hold protocol.
- **Violations log:** append-only `violations.jsonl` at the data-dir
  root recording policy enforcement events (denied tool calls, post-
  commit greps that fired, policy file mutations).
- **Question tray:** the `questions` table — durable record of
  ask_user_choice / mark_awaiting_user / request_approval prompts so
  they survive app restart.
