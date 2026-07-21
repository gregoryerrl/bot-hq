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

bot-hq is a two-agent duo (Brian + Rain). A former solo helper agent,
**Emma**, has been removed from the core (code + data purged); she is
slated to return as the first bot-hq plugin — TBD.

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
                    ┌────────▼─────┐  ┌───▼─────────┐
                    │ claude-code  │  │ claude-code │
                    │   (Brian)    │  │   (Rain)    │
                    │ stream-json  │  │ stream-json │
                    └──────────────┘  └─────────────┘
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

Per-agent model swap via env-vars: `ANTHROPIC_BASE_URL`,
`ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_MODEL`. The model is resolved per
session from the picker stored on the `sessions` row (`brian_model_id` /
`rain_model_id`) against the saved-model `models` registry, falling back
to the `agent_configs` table then a built-in default (see "Per-agent
model selection"). `BOT_HQ_SESSION_ID` is also injected so git-hook
subprocesses can read session-scoped state.

**LLM proxy (`src/agents/llm_proxy.rs`):** agents pointed at a
non-Anthropic Anthropic-compatible gateway (e.g. Rain → DeepSeek) route
their `ANTHROPIC_BASE_URL` through a local normalizing reverse-proxy. It
hoists the `role:"system"` entry claude-code injects into the
`messages[]` array (from a SessionStart hook's `additionalContext`) up
into the top-level `system` field, which strict gateways require.
Agents on the real first-party API (Brian) bypass it.

---

## Agent role prompts (hardcoded)

Role prompts (Brian/Rain identity) are baked into the binary at
`src/agents/prompts.rs`. They are NOT CL-loaded. Reasoning:

- Role boundary (Brian writes, Rain reviews) is structural — a typo in a
  CL file shouldn't be able to break it.
- Hardcoded prompts protect the role identity through CL edits, custom
  instruction changes, etc.

CL still supplies per-project + per-user customizations on top (custom
instructions, general rules, project policy directives). The hardcoded
prompt is the floor; CL extends it.

System-prompt layering at session spawn (`src/core/session.rs::read_system_prompt`):

1. Hardcoded role prompt (Brian/Rain)
2. CL location anchor (`<data_dir>` path)
2b. Project CL index primer (when the session has a project) — the
   `cl_index_search` rows for the project (`file_path — description`,
   most-recently-updated first, capped). The table of contents only.
3. Hardcoded universal rules (`agents::general_rules::GENERAL_RULES`)
4. `<data_dir>/library/custom-general-rules.md` (optional user additions)
5. `<data_dir>/library/custom-instructions.md` (user tweaks, loaded for
   EVERY agent — consolidated from the old per-agent
   `agents/<name>/custom-instruction.md` files)
6. Resolved policy directive block (forbidden words list, push-gate
   mode, etc.)

Project conventions/notes **bodies** are deliberately NOT injected —
agents pull those via the `cl_index_search` MCP tool + `Read` on-demand.
What *is* injected (layer 2b) is the lightweight CL **index** for the
project: filenames + descriptions, so an agent that skips
`cl_index_search` on a cold start still knows what context exists to
pull. The index is fetched once in `spawn_session_handle`
(`storage.cl_index_search`) and threaded into `read_system_prompt`;
`policy.yaml` is omitted from the primer (it's already rendered as the
policy block in layer 6).

---

## Bilateral duo coordination

Stream-json prose flows Brian ↔ Rain through a central **router task**
(`src/core/router.rs::run_router`) — not peer-to-peer. Each agent's pump
(`src/core/duo.rs::pump_agent`) emits a `RouterCommand::Forward` on every
completed turn that buffered prose; the router is the single decision
point that forwards to the peer, suppresses, or lets the duo settle. All
phases are turn-based — a forward is decided on turn completion (the old
Investigate/Plan 1.5s-buffer vs Apply/Verify split was retired
2026-06-24).

**Forward ladder** (router decides, in order):
1. **Awaiting guard** — drop the forward while the session's `awaiting`
   flag is set (silent-on-hold).
2. **`peer_ack`** — if the turn called `peer_ack`, suppress (the duo
   converged; the peer is not woken).
3. **Hard cap** — after `VOLLEY_HARD_CAP` (18) consecutive user-silent
   forwards, break the volley (suppress + settle both agents to Idle).
   The counter (`user_silent_forwards` on `SessionHandle`) resets on the
   next user message.
4. **Convergence** — if a forward is ≥ `VOLLEY_SIMILARITY_THRESHOLD`
   (0.85, token-set Jaccard) similar to the previous one for
   `VOLLEY_SIMILAR_BREAK` (2) forwards running, break.
5. Otherwise **forward** to the peer's stdin.

`peer_ack` + `halt` are the *behavioral* layer (agents signal volley-end
intent); the hard-cap + convergence breaker is the *mechanical* floor for
weak models that never signal.

**Router lifecycle:** a duo session holds `Option<RouterControl>` on its
`SessionHandle` (`None` = solo, no router). `RouterControl`'s `Drop`
aborts the task, so closing/dropping a session tears the router down; its
liveness surfaces to the UI as the per-session router dot (`router_alive`).

**Suppressed from peer forwarding** (never reach the ladder):
- Tool-use events (`ask_user_choice`, `mark_awaiting_user`,
  `request_approval`, etc.) — agent ↔ UI signaling, not agent-to-agent.
- Anything emitted while `awaiting` is set (ladder step 1). The flag is
  set by `mark_awaiting_user`, `ask_user_choice` / `request_approval`
  (until resolved), and `halt`; forwarding resumes when it clears.

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

**Live freshness (filesystem watcher + command emits):** beyond the bridge event
stream, a filesystem watcher (`src/tauri_events/fs_watcher.rs` — one
`notify-debouncer-mini` over the CL dir + per-session repos, re-indexing the affected
scope before it emits) and direct `app.emit` calls from mutating Tauri commands
(`project:changed` / `model:changed`) drive UI updates that bypass the bridge. All three
channels converge on `Providers.tsx` GlobalEventSync key-set invalidation, so the CL
tree/editor, the Apply-tab `git diff`, and the project/session/model lists refresh on
external change without polling — the 60s `refetchInterval`s were dropped; only the
plugin heartbeat (10s) + a broadcast-`Lagged` `session:resync` backstop remain. Working-
repo churn is filtered by an ignore-list (`target`/`node_modules`/`.git`/dotdirs) so
builds don't thrash the A-tab diff. (Shipped 2026-06-15; see PROGRESS.md.)

**Topbar:** `Dashboard | Context Library | Plugins | Settings`.

**Dashboard:** grid of session tiles. Each tile shows title, last
activity, `[Needs Input]` badge tinting the border red. Click tile →
opens session view. Inline `+ New session` form creates rows + registers
the session with the bridge.

**Session view:** 60/40 split — chat (left) + DocumentPane (right).
Header: title + back link. Chronological chat: all messages (user,
Brian, Rain, phase_change) interleaved by `created_at` with author color
coding (brian=orange, rain=purple, user=blue, system=muted).
Pending-choice banner (purple) renders above the input with inline
choice buttons.

**DocumentPane:** IPAV tab selector (I/P/A/V chips) drives
`session_doc_search(session_id, phase=<x>)`. Each tab renders matching
`session_documents` rows; counts surface on the chips. The A tab also
renders the live color-coded `git diff` for the session's working repo
via the `compute_apply_diff` Tauri command (`src/tauri_cmd/docs.rs`,
parser `parse_diff_lines`), consumed by `DocumentPane.tsx`.

**Context Library tab:** two Settings-style subtabs — **Library Tree** |
**Context Manager** — whose pill row is the page header (no panel repeats
its label).

*Library Tree* — 2-pane file explorer + tabbed editor. The tree renders
nested collapsible folders (`cl_index_search` + `cl_folder_search`) with
substring search (no project filter — removed as YAGNI; Rescan is always
all-projects, in parallel with per-project failures surfaced). Files open a
read-write editor (`cl_read_file` / `cl_write_file`; binary + truncated
files are read-only so a lossy save can't corrupt them). Folders open a
folder-view that edits the folder description (`cl_set_folder_description`)
and, at the project root, manages the project: set the working repo, rename
(`cl_rename_project`), unbind (`cl_unregister_project` — soft: clears repo +
custom cl_path, keeps content), and hard-delete (`cl_delete_project` — purges
all index/folder rows + the row; optionally removes the managed on-disk dir,
never a custom `cl_path`). The sidebar `+` opens a **New-project** modal: the
default path (`cl_create_project`) creates a managed project at
`<data_dir>/library/projects/<name>/` (seeded `conventions.md`/`notes.md`) and
binds an optional working repo WITHOUT indexing it; an Advanced section
(`cl_register_project` + `cl_rescan`) is the power case that indexes an existing
on-disk folder AS the CL content (`cl_path`). Right-click gives VSCode-style new
file / new folder / rename / delete (`cl_create_file` / `cl_mkdir` / `cl_rename`
/ `cl_delete_path`, each followed by `cl_rescan`), plus **Promote to project**
on a top-level Global folder (moves it into `projects/` + registers). Path
inputs (working-repo, cl_path) use a native folder picker via
`tauri-plugin-dialog`. The New-session dialog can also pick an ad-hoc working
repo directly (no pre-registration).

*Context Manager* — the per-project management surface (NOT a file
explorer): a left rail of registered projects (`_globals` pinned last) and a
right panel for the selected project — header strip (repo path, per-project
Rescan, Maintain CL preselecting the project) over the **Measurement** card:
`cl_retrieval_stats` tiles over `retrieval_events` (tokens/session,
tokens/retrieval, stale-hit + retrieval-miss rates). Default selection is
the first project.

**Plugins tab:** Management UI (`PluginManager.tsx`) over
`tauri_cmd/plugins.rs` — install is two-step (`preview_plugin_manifest`
shows the requested capabilities with catalog descriptions; only an
explicit confirm installs), then enable / disable / uninstall and a
heartbeat-driven crash indicator. Enabled plugins that declare a panel
get a dynamic topbar tab (`/plugins/view/:pluginId` → `PluginPanel.tsx`
→ `PluginHost.tsx`). See "Plugin runtime" below for the execution model
and [`docs/PLUGINS.md`](docs/PLUGINS.md) for the author contract.

**Settings tab:** subtabs for the saved-model registry (Models), the
default-model + disable-Rain-by-default app settings, the global Tool
Gate keyword list, the global Claude Config surface, and a closed-session
Archive. Per-row accent dot keyed to author color. Plaintext-token
warning preserved.

**Per-agent model selection:** the user maintains a registry of saved
models (`models` table — label + provider + base_url + auth_token) in
Settings → Models. The New-session dialog exposes a Brian + Rain model
dropdown (each seeded from that agent's `agent_configs` entry when it
matches a saved model) plus a "disable Rain" checkbox (solo Brian) seeded
from the `rain_disabled_default` app setting. The picks persist on the
`sessions` row (`brian_model_id` / `rain_model_id` / `rain_enabled`) and
`resolve_spawn_config` resolves them at spawn (registry → `agent_configs`
→ built-in default). `agent_configs` is now effectively the picker
fallback. (A `default_model_id` app setting exists but is only consumed
by `summarize_session_doc`'s model resolution, not the spawn path.)
Dialog-less create paths — the Maintain-CL dispatcher and the external
driver's `create_session` — honor `rain_disabled_default` via
`Storage::default_rain_enabled` (models stay NULL = agent defaults).

**Claude Config surface** (`src/claude_config/`,
`tauri_cmd/claude_config.rs`, `frontend/src/app/ClaudeConfig.tsx`):
surfaces the user's `~/.claude` config that leaks into the headless
agent subprocesses — skills, plugins, hooks, CLAUDE.md/memory, MCP
servers, reasoning effort. The user controls it two ways: globally
(write-back to the real `~/.claude` via `claude_config/writer.rs`) and
per-agent via an override layer (`<data_dir>/config/claude-overrides.json`,
`claude_config/overrides.rs`) merged into the spawn-time `--settings`
JSON + env injection — so an inherited skill/plugin/MCP/effort can be
disabled for one agent without touching the user's own `~/.claude`.
Design: `docs/plans/2026-06-02-claude-config-surface-design.md`.

**Plugin runtime (v1, shipped 2026-07-04):** plugins are static
frontend bundles in sandboxed iframes, served over ONE `bhq-plugin://`
custom URI scheme (Builder-time registration; per-request enabled-check
via the `PluginRegistry` enabled cache, so install/enable needs no
restart; the plugin id rides the URL host on macOS/Linux and the first
path segment under the Windows `https://bhq-plugin.localhost` fold —
`plugins::serve` accepts both). Plugins never call Tauri: they
postMessage the shell (per-mount nonce + source/origin checks in
`frontend/src/lib/pluginBridge.ts`), which forwards to the single Rust
enforcement point `plugin_invoke_proxy` (`tauri_cmd/plugin_api.rs`) —
re-checking enabled ∧ granted ∧ catalog-listed per call and
dispatching through an explicit match over `plugins::catalog` (12
read-first commands + plugin-scoped KV writes; `api_version: 1`). The
heartbeat state machine (`plugins::heartbeat`) is fed by the
PluginHost's 5s ping loop (`plugin_note_ping`/`plugin_note_pong`); the
sweep loop in `main.rs` emits `plugin:crashed` after 3 misses and the
host swaps in a Reload fallback. There is NO Tauri-ACL/capability-JSON
path (the original design's generated capability files were written
where Tauri never reads — retired 2026-07-04). Author contract:
[`docs/PLUGINS.md`](docs/PLUGINS.md); working example + test fixture:
`examples/hello-plugin/`.

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

**Internal tools (36)** (see [README.md](README.md#internal-mcp-tools-served-to-child-agents)
for the documented list with descriptions): `ask_user_choice`,
`mark_awaiting_user`, `peer_ack`, `halt`, `advance_phase`, `request_phase_advance`,
`request_approval`, `action_gate`, `check_commit_message`, `eyes_flag`,
`disposition_finding`, `check_open_findings`, `override_reviewer_block`,
`approve_finding`, `close_session`, `list_my_pending_questions`, `withdraw_question`,
`supersede_question`, `session_doc_write`, `session_doc_search`,
`session_doc_read`, `cl_index_search`, `cl_retrieve`, `cl_write_file`,
`cl_register_read`, `cl_rescan`,
`cl_folder_search`, `cl_register_folder_description`, `web_search`,
`terminal_exec`, `terminal_read`,
`webview_screenshot`, `webview_click`, `webview_type`, `webview_scroll`,
`webview_press_key`.

**Session terminal (Terminal subtab).** Each session lazily spawns one PTY
shell (`core/terminal.rs`) in its working repo — rendered by the session
view's Terminal subtab (xterm.js) and shared with the agents through
`terminal_exec` (HANDS-only; BLOCKING by default — writes the command, awaits
output-settle via a quiet-window heuristic, returns the captured tail;
`block:false` for long-running processes) and `terminal_read` (both agents;
scrollback tail as evidence text). `terminal_exec` re-classifies the command
against the same two-tier Tool-Gate keyword list the PreToolUse hook uses
(session snapshot → global fallback, `tool_gate::resolve_keywords`) and
refuses gate-matched commands with a route to `action_gate` — the terminal is
not a gate bypass. The PTY is killed on `close_session`; scrollback is
in-memory only (200 KB ring).

**Review findings gate (EYES sign-off).** `eyes_flag` /
`disposition_finding` / `check_open_findings` / `override_reviewer_block` /
`approve_finding` (+ the pre-commit `check_commit_message`) implement the
EYES-sign-off gate: a `blocking` finding Rain files via `eyes_flag` gates
HANDS's `git commit` (mechanically, via the pre-commit hook) until HANDS
resolves it with `disposition_finding` (fixed / rebutted). The gate is
fail-CLOSED when the reviewer is down; `override_reviewer_block` is the
explicit escape valve. Backed by the `findings` table.

**Role enforcement at the dispatch layer:** `HANDS_ONLY_TOOLS` and its
inverse `EYES_ONLY_TOOLS` are hard-coded lists. A call to a HANDS-only
tool from Rain (or an EYES-only tool from Brian) returns a JSON-RPC error.
EYES-only = `eyes_flag`, `approve_finding` (Rain files / signs off on her
own findings; HANDS can't self-review). HANDS-only adds `disposition_finding`,
`override_reviewer_block`, `halt`, and the user-facing signaling tools. The
boundary is structural, not just convention.

**Bridge (`src/signaling/bridge/`)** owns:
- Storage handle (writes question rows, message rows, violations).
- Policy resolver (loads `general-policy.yaml` + `projects/<p>/policy.yaml`).
- Session → project mapping.
- Per-session `awaiting` halt flag (shared `Arc<AtomicBool>` with duo
  pump).
- Session permissions cache (mirrored to disk for hooks).
- Tray storage (`session_tray` table — persists awaiting-input items
  (`ask_user_choice` / `request_approval` / gated commands) so they
  survive app restart).

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
list (19 driver tools including `list_sessions`, `create_session`,
`send_message`, `wait_for_change`, `get_session_snapshot`, etc.).

---

## Policy enforcement

**Goal:** enforce per-project rules (forbidden commit words, push gate,
force-push gate) reliably even when an agent's context drifts and
forgets to call the MCP tool.

**Two layers** (`src/policy/`):

1. **MCP tools** (`request_approval`, `action_gate`, …) are
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
- `<data_dir>/config/general-policy.yaml` — defaults.
- `<data_dir>/library/projects/<project>/policy.yaml` — per-project overlay
  (lists are replaced, not merged).

Fields: `push_gate` (scalar `auto`|`ask`),
`force_push` (scalar `blocked`|`allowed`), `per_action_approval`,
`branch_pattern`. (push_gate/force_push are per-tier
toggles inherited general→project→session; there are no per-branch
"remembered approvals" or agent-side grants.)
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
- `pre-push`: resolves the session's policy. `push_gate == auto` →
  allow (exit 0). `push_gate == ask` AND `BOT_HQ_SESSION_ID` is set →
  POST the running app's `/hooks/pre-push` route (addr read from
  `<data_dir>/.local/signaling-addr`), which surfaces a per-push
  Approve/Reject prompt via `request_approval` and blocks on the user's
  pick: approve → exit 0, reject → exit 1. Fail-closed (exit 1 + a
  `PushGate`/Denied violation) if the app is unreachable; a push with no
  session context is blocked with guidance.

**Audit:** `src/policy/audit.rs` hashes each policy file at hook fire.
A hash change between fires logs a `PolicyMutation` violation
(audit-only in v1).

---

## Tool Gate

A global, user-configured keyword gate over agent **Bash** tool calls,
replacing the per-project `tool_blocklist` role (post-2026-05-29
fabricated-comment incident) with a single list that can also EXECUTE the
command on approval.

- **Config:** one global list at `<data_dir>/config/tool-gate.json` —
  `[{keyword, mode}]`, `mode` ∈ `gate | auto_allow`, edited in Settings
  ("Gated Bash Keywords"). NOT per-project, NOT in `policy.yaml` —
  bot-hq-side, so nothing is written into a working repo.
  Matching is case-insensitive substring against the tool name or command;
  `gate` wins over `auto_allow` on conflict.
- **Tripwire:** the PreToolUse Bash hook (`policy-check tool-gate`, injected
  into HANDS at spawn via `--settings` — `src/policy/hooks.rs`
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

## Session policy

Each session freezes a **policy snapshot** at spawn — the resolved
general → project → session-overlay stack (`push_gate`, `force_push`,
forbidden words, `tool_gate`). The user edits it per-session in the gear
tab (Session Settings); agents cannot write policy. There are no
agent-side commit/push grants — push and force-push are pure per-tier
toggles (`push_gate: auto|ask`, `force_push: blocked|allowed`)
inherited general → project → session.

**Storage** (`src/policy/session_policy.rs`):
- Snapshot written to `<data_dir>/.local/session-policies/<session_id>.yaml`.
  Seeded WRITE-IF-ABSENT at spawn (`core/session.rs`) by resolving the
  blueprint with `session_id=None`, so re-opening a session preserves
  gear-tab edits.
- The git hooks (`pre-push`, `commit-msg`, …) read this snapshot via
  `Policy::resolve_at_root` (threaded `BOT_HQ_SESSION_ID`), so a hook
  subprocess sees the same session-scoped policy the agent runs under.
- Purged on bot-hq startup (`main.rs`) and on `close_session`
  (`core/state.rs` → `bridge::cleanup_session_policy`).

The per-session **Tool Gate** keyword list is part of the same snapshot
(see "Tool Gate" above): `hooks.rs::run_tool_gate` reads the frozen
snapshot first, so editing the global `tool-gate.json` only affects NEW
sessions.

---

## Session worktrees

Repo-backed sessions default to an **isolated git worktree** so two or
more sessions can work the same project in parallel (per-session index,
checkout, and branch — no file races). Opt-out per session in the
New-session dialog, or globally via the `worktree_default` app setting
(Settings → Agents → Session defaults). Dialog-less paths (Maintain CL)
follow the setting.

- **Placement:** `<data_dir>/.local/worktrees/<session-id>/<repo-basename>/`.
  The repo basename stays the final path segment because
  `spawn_session_handle` derives the session's project from
  `working_repo_path.file_name()` — the worktree must map to the same
  project for policy + CL.
- **Row model:** `sessions.working_repo_path` = the WORKTREE (the path
  agents run in — action_gate, hook install, A-tab diff, and project
  derivation all read it unchanged); `sessions.base_repo_path` = the repo
  it was carved from (NULL = direct mode). Placement is decided at create
  (`tauri_cmd/sessions.rs::resolve_session_placement`); the worktree is
  materialized lazily at spawn (`core/worktree.rs::ensure_worktree`,
  idempotent across respawns, re-adds after a manual delete via
  `worktree prune`). If ensure fails, the session falls back to the base
  repo and the row is converted to direct mode so row-readers and the
  live handle agree.
- **Branch:** `bothq/<session-id>` from the base repo's HEAD at first
  ensure. Two worktrees can't share one branch, so per-session branches
  are inherent; merging back is the user's flow (push gate unchanged).
- **Hooks:** a linked worktree's `.git` is a FILE and git reads hooks
  from the shared common dir — `install_hooks` resolves the real hooks
  dir via `git rev-parse --git-path hooks` (also honors
  `core.hooksPath`), so the policy backstop covers every worktree of the
  repo. Hook identity stays per-session via `BOT_HQ_SESSION_ID` env at
  fire time.
- **Close:** `close_session` removes the worktree only when clean (plain
  `git worktree remove`, never `--force`); a dirty worktree is kept and
  logged for manual recovery. The session branch always survives.

---

## Storage (sqlite)

Schema at `migrations/0001_init.sql` + subsequent migration files.

**Tables:**
- `messages` (id PK, session_id, author, kind, content, created_at) —
  full chat history. Index on `(session_id, created_at)`.
- `sessions` (id PK, title, working_repo_path, base_repo_path,
  created_at, closed_at, archived, rain_enabled, brian_model_id,
  rain_model_id, + per-agent spawn metadata: brian/rain_model_at_spawn,
  brian/rain_claude_session_id, brian/rain_effort, brian/rain_ultracode)
  — rain_enabled + the model ids drive per-session model selection + the
  solo-Brian (disable-Rain) toggle; `base_repo_path` is set for
  worktree-isolated sessions (see "Session worktrees"). There is NO
  `project` column — the project is derived at spawn from
  `working_repo_path.file_name()` — and no `phase` column (IPAV phase is
  in-memory; see "IPAV state").
- `agent_configs` (agent_name PK, provider, model_name, base_url,
  auth_token). CHECK constraint still lists `agent_name ∈
  {'emma','brian','rain'}` (migration 0001 created it permissive;
  migration 0017 purges the `emma` row but leaves the CHECK as-is for
  legacy reasons) — only `brian`/`rain` are used. Now a fallback for the
  `models` registry below (see "Per-agent model selection").
- `models` (id PK, label, provider, model_name, base_url, auth_token) —
  saved-model registry the per-session pickers reference by id.
- `app_settings` (key PK, value) — key/value app settings
  (`default_model_id`, `rain_disabled_default`, …).
- `session_tray` (choice_id PK, session_id, agent, kind, prompt,
  options_json, command_text, status, supersedes_id, asked_at,
  answered_at, picked) — durable awaiting-input tray
  (choices/approvals/gated commands). Survives app restart. Renamed from
  `session_questions`/`questions` in migration 0010.
- `session_documents` (id PK, session_id, slug, body, phase, …) —
  per-session IPAV scratch docs.
- `findings` (id PK, session_id, finding_uid UNIQUE, agent, severity,
  summary, code_ref, status, disposition_reason, disposed_by,
  created_at, updated_at) — EYES review findings backing the commit
  gate (migration 0021; FK → sessions ON DELETE CASCADE).
- `projects` (name PK, display_name, working_repo_path, description,
  created_at) — registered-project registry; FK target for `cl_index` /
  `cl_folders` / `cl_reads`.
- `plugins` (id PK, name, version, manifest_json, dir_path, enabled,
  installed_at) — installed-plugin registry.
- `plugin_kv` (plugin_id → plugins ON DELETE CASCADE, key, value,
  updated_at; PK (plugin_id, key); migration 0029) — per-plugin
  key/value store behind the `plugin_kv_get`/`plugin_kv_set` catalog
  commands; namespaced server-side.
- `cl_index` (file_path PK, project, description, tags, size,
  modified_at, indexed_at) — SQLite-backed CL search index.
- `cl_folders` (id PK, project_id → projects, folder_path, description,
  tags, …) — folder-level CL descriptions (parallel to `cl_index`).
- `cl_reads` (id PK, cl_index_id → cl_index, session_id, agent, read_at)
  — audit of which CL files an agent read (the `cl_register_read` sink).
- `cl_atoms` (FTS5 virtual table: project_id, file_path, kind,
  heading_path, body, mtime, body_hash, code_hash; migrations
  0024/0026/0027) — heading-delimited CL sections, BM25-searchable,
  backing `cl_retrieve`. DERIVED + disposable: `cl_rescan` rebuilds it
  from disk; FTS5 column adds drop+recreate the table and the boot
  rescan repopulates.
- `retrieval_events` (id PK, session_id/agent nullable audit, project_id,
  query, atom/token/stale counts, returned_atoms JSON; migration 0028,
  deliberately FK-free append-only telemetry) — one row per
  `cl_retrieve`, feeding the Measurement view.

**Author enum:** `user` / `brian` / `rain`. (The `messages.author` CHECK
still permits `'emma'` for legacy reasons, but the Rust enum no longer
has it.) NO `system` author — phase changes synthesize as `author=user`
("phase advanced to PLAN") so chat history reads coherently and agents
see them as natural switch prompts.

---

## IPAV state

In-memory cache: `HashMap<SessionId, IpavState>` where `IpavState {
current_phase, phase_log }`. Not persisted. Subprocesses die with the
app; restart = fresh sessions.

---

## Context Library

Filesystem space at `<data_dir>/library/` — its own folder so it can be
backed up / cloud-synced independently of host-local state — holding agent
custom instructions, per-project conventions/notes. Markdown on disk stays
the source of truth; SQLite carries two DERIVED, disposable layers on top:

- **File index** (`cl_index`): one row per file (description, tags) for
  description-aware discovery via `cl_index_search`. Descriptions
  re-derive when a file's body changes on disk (rescan), so the TOC can't
  freeze at first-index.
- **Atom index** (`cl_atoms`, FTS5): each file split at headings into
  ~≤200-token *atoms* (oversized sections sub-split at bullet/paragraph
  boundaries, code fences kept whole — `util::split_into_atoms`).
  `cl_retrieve` runs ranked BM25 retrieval over them and returns atom
  BODIES inline under a token budget (convention/decision kinds win
  ties), replacing the "search → eyeball → Read the whole 38K-token
  file" loop. Atoms that cite repo paths carry a `code_hash` of the
  cited source (`bridge/cl_refs.rs`, stamped at rescan); retrieval
  recomputes it and prefixes `⚠ possibly stale` when the code has
  drifted since indexing. Every `cl_retrieve` logs one row to
  `retrieval_events` (tokens, stale/empty counts) — surfaced in the
  Library's Measurement tab. Behavioral complement: the standalone
  `bench/cl_poison/` eval measures whether agents OBEY a poisoned atom
  or VERIFY against the source.

**All-agents files** (always loaded at spawn, same content for every
agent):
- `library/custom-instructions.md` (consolidated from the old per-agent
  `agents/<name>/custom-instruction.md` files; a one-time migration in
  `Paths::init` folds user-modified legacy copies in and deletes
  untouched seeds)
- `library/custom-general-rules.md` (optional user additions; the
  universal rules are hardcoded in `agents::general_rules`)

**Per-project files** (loaded on-demand via `cl_index_search`):
- `library/projects/<project>/conventions.md`
- `library/projects/<project>/notes.md`
- `library/projects/<project>/decisions.md`
- `library/projects/<project>/policy.yaml` (CL-coupled — the policy
  resolver + audit read it here)
- Free-form: anything else under `library/projects/<project>/`

`_globals` maps to `<data_dir>/library/` itself; named projects honor a
`projects.cl_path` override (absolute path) when set, else the convention
`<data_dir>/library/projects/<name>/` resolved via `Paths::project_dir`
— the single source of truth shared by the storage resolver, policy
resolver, and policy audit (so the `library/` location can't desync them).

**Agents write CL content directly via `cl_write_file`** (HANDS-only;
EYES reviews instead of writing). The tool is a guarded create-or-replace
inside the project's CL root: relative-path + traversal checks, a 1 MiB
cap, atomic tmp+rename, mkdir-p for new subfolders, an automatic
`cl_rescan`, and a refusal on bot-hq-owned `_globals` system files
(`custom-instructions.md`, `custom-general-rules.md`, legacy `agents/`) so
an agent can't rewrite its own standing rules. The session-close learnings
delta (≤~5 non-obvious one-liners appended under `notes.md`'s
`## Learnings` with the full replacement body) rides this path, and a
`cl_write_file` lifts the close-out nudge exactly like `cl_rescan`. (The
former `cl_propose` review queue — migration 0025 — was removed in 0035:
in practice approvals were rubber-stamped, so the friction bought
nothing. The user still edits any CL file in the Context Library tab.)

**First-run init:** `templates/cl/` is baked into the binary. On first
start (no `version.txt` in the data dir), bot-hq seeds the templates
under `<data_dir>/library/`. A pre-`library/` install (root-level CL, no
`version.txt`) is migrated once into the new layout by `Paths::init`.
Missing individual files trigger an "initialize default" button in the UI
for that slot.

---

## Data locations

Defaults (env-overridable via `BOT_HQ_DATA_DIR`):

- **Data-home schema marker:** `<data_dir>/version.txt`
- **Context Library (cloud-syncable):** `<data_dir>/library/`
- **Installed plugins:** `<data_dir>/plugins/`
- **Machine policy/config (`config/` since v1.1):** `<data_dir>/config/general-policy.yaml`,
  `<data_dir>/config/tool-gate.json`, `<data_dir>/config/claude-overrides.json`
- **DB file:** `<data_dir>/.local/bot-hq.db`
- **Single-instance lock:** `<data_dir>/.local/lock`
- **External MCP token:** `<data_dir>/.local/mcp-token`
- **Violations log:** `<data_dir>/.local/violations.jsonl`
- **Policy-hash cache:** `<data_dir>/.local/.policy-hashes.json`
- **Screenshots:** `<data_dir>/.local/screenshots/`
- **Session policy snapshot:** `<data_dir>/.local/session-policies/<sid>.yaml`

Top-level dirs are **sync boundaries**: `library/` = user content
(cloud-syncable), `config/` = portable machine policy, `.local/` =
host-only runtime + secrets + logs (never synced). The binary itself ships
in a platform bundle (`/Applications/bot-hq.app` on macOS; `/usr/bin` or
AppImage on Linux; `Program Files` on Windows), NOT under `<data_dir>`.
Pre-`library/` and pre-`config/` installs are migrated once on launch (the
`config/` split landed in v1.1, 2026-06-09).

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

The plugin runtime v1 is live (see "Plugin runtime" under Tauri +
React UI): consent-gated install, `bhq-plugin://` serving, postMessage
RPC through the Rust-side catalog proxy, heartbeat/crash recovery,
panel tabs. The author contract is [`docs/PLUGINS.md`](docs/PLUGINS.md);
`examples/hello-plugin/` is the template + integration-test fixture.

Deferred plugin TIERS (extension points documented in PLUGINS.md):
plugin-contributed MCP tools (agent↔plugin), manifest-declared agents,
child-webview surface (real Browser tab), background execution,
zip/signed URL installs, per-plugin CSP overrides, inline `slot_name`
slots, host-event relay.

Concrete plugin ideas building on the runtime (each needs its own
design doc): **Cognotify** (human-comprehension deck over sessions +
CL), **Discord bridge**, **Clive** (legacy bot port), **CL cloud
sync**, **GitHub tab**.

---

## Eval harness

`bench/swebench/` is a SWE-bench rollout harness for evaluating the duo
on real GitHub issues — a Python client (`run_rollout.py`,
`bothq_client.py`, `verify.py`, …) that drives sessions through the
external MCP server and scores patches. It is a developer tool, **not
part of the runtime core**: it ships in-repo but is not compiled into
the `bot-hq` binary and does not run at app startup. See
[`bench/swebench/README.md`](bench/swebench/README.md).

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
- **Emma:** removed (former solo helper agent; planned to return as the
  first bot-hq plugin — TBD).
- **claude-code:** the upstream CLI tool that wraps a language model.
  One subprocess per agent.
- **stream-json:** claude-code's `--output-format stream-json` mode.
  One JSON event per line on stdout. See
  [`docs/stream-json-events.md`](docs/stream-json-events.md).
- **MCP (Model Context Protocol):** the protocol claude-code uses for
  external tool servers. Bot-hq runs two MCP servers in-process.
- **Policy:** machine-readable subset of CL rules — `general-policy.yaml`
  + project overlay. Drives forbidden-word grep, push gate, force-push
  gate.
- **Session policy snapshot:** the resolved general → project → session
  policy frozen per-session at spawn (`session_policy.rs`), editable in
  the gear tab. Push/force-push are pure toggles — no agent-side grants.
- **Awaiting flag:** per-session `Arc<AtomicBool>` set by user-blocking
  tools (`mark_awaiting_user`, `ask_user_choice`, `request_approval`).
  When set, the peer-forward router (`core/router.rs`) suppresses
  forwarding — silent-on-hold protocol.
- **Violations log:** append-only `violations.jsonl` at the data-dir
  root recording policy enforcement events (denied tool calls, post-
  commit greps that fired, policy file mutations).
- **Tray (`session_tray`):** durable per-session record of awaiting-input
  items — `ask_user_choice` / `request_approval` / gated commands — so
  they survive app restart. Renamed from `session_questions` (migration
  0010).
