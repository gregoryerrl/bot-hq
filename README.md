# bot-hq

Desktop GUI for driving AI-assisted coding sessions through a bilateral-duo
agent model: **Brian** (HANDS — edits, commits, runs commands) and **Rain**
(EYES — adversarial review). Optional solo helper **Emma** for one-off
questions. The user is the orchestrator; the app is the conductor between
user and agents.

Each agent runs as a `claude-code` subprocess. The app wires bidirectional
stream-json between subprocesses, persists chat history to sqlite, and
exposes two MCP servers — one internal (UI signaling) and one external
(driver tools for other claude-code sessions).

For architectural detail see [`ARCHITECTURE.md`](ARCHITECTURE.md); for
planned work see [`PLAN.md`](PLAN.md); for recent change log see
[`PROGRESS.md`](PROGRESS.md). The original rebuild design + roadmap are
preserved under [`docs/rebuild-archive/`](docs/rebuild-archive/).

## Prerequisites

- **Rust stable** (latest). `rustup update stable`.
- **Node.js 22+ and npm** — for the React frontend (Vite build).
- **`claude-code` CLI** installed and authed. `claude --version` should
  print `2.x` or newer; `claude auth status` should be healthy.
- **macOS** is the primary target; Linux + Windows are tracked in PLAN.md.

## Quickstart

```bash
git clone <repo>
cd bot-hq
cp .env.example .env                    # contains BOT_HQ_DATA_DIR=~/.bot-hq-dev/
(cd frontend && npm install)            # React frontend deps (Vite + Tauri CLI)
cargo install tauri-cli --version '^2'  # one-time, if `cargo tauri` is missing
cargo tauri dev                         # builds the React UI + opens the desktop window
```

`cargo tauri dev` runs the `beforeDevCommand` wired in `tauri.conf.json`
(`cd frontend && npm run dev`), so the Vite dev server and the Rust app come
up together. The Tauri CLI also ships locally via `@tauri-apps/cli` in
`frontend/node_modules` if you'd rather not install it globally.

Release build:

```bash
cargo tauri build            # builds the frontend, compiles release, bundles the app
./target/release/bot-hq      # the bundled app also lands under target/release/bundle/
```

## Configuration

Env vars read at startup:

| Var                          | Default             | Purpose                                             |
| ---------------------------- | ------------------- | --------------------------------------------------- |
| `BOT_HQ_DATA_DIR`            | `~/.bot-hq/`        | Context Library + sqlite DB location                |
| `BOT_HQ_EXTERNAL_MCP_PORT`   | `7892`              | External driver MCP server port                     |
| `BOT_HQ_EXTERNAL_MCP_DISABLED` | unset             | Set to `1` to skip external MCP server startup      |
| `RUN_LIVE_TESTS`             | unset               | Set to `1` to include subprocess tests              |
| `RUST_LOG`                   | `info,bot_hq=debug` | tracing-subscriber EnvFilter                        |

**During development** keep `BOT_HQ_DATA_DIR=~/.bot-hq-dev/` in `.env` so
you don't collide with a running production bot-hq at `~/.bot-hq/`.

## Layout

```
bot-hq/
├── Cargo.toml / tauri.conf.json / build.rs
├── CLAUDE.md / ARCHITECTURE.md / PLAN.md / PROGRESS.md   ← canonical docs
├── frontend/              React 18 + TypeScript + Tailwind UI (Vite)
│   └── src/{app,components,hooks,stores,lib}/   pages, components, hooks, zustand stores, tauri bindings
├── src/
│   ├── main.rs            entry point — tokio runtime, Tauri builder, CLI dispatch
│   ├── paths.rs           data-dir resolution + first-run init + single-instance lock
│   ├── agents/            claude-code subprocess + stream-json I/O + hardcoded role prompts
│   ├── core/              sessions, IPAV cache, duo coordination, broadcast
│   ├── signaling/         in-process MCP HTTP servers (internal UI tools + external driver) + SignalingBridge
│   ├── storage/           sqlite (messages, sessions, agent_configs, questions, cl_index)
│   ├── policy/            policy resolution, git-hook installer, session-permission grants, violations log
│   ├── plugins/           plugin manifest parser, loader, capability gen, heartbeat watcher
│   ├── tauri_cmd/         #[tauri::command] wrappers over bridge/storage methods
│   ├── tauri_events/      bridge subscriber → BatchEmitter → typed app.emit
│   └── tauri_specta_gen.rs  TypeScript binding generation (tauri-specta)
├── migrations/            0001_init.sql + later migrations
├── templates/cl/          baked-in default CL (used on first run)
└── docs/
    ├── design/            Industrial Terminal design spec + screen mocks
    ├── stream-json-events.md     claude-code CLI event schema (empirical)
    └── rebuild-archive/          original rebuild design + roadmap + decisions
```

## Architecture in 60 seconds

- **Stack:** Single Rust binary — Tauri v2 shell + React 18 UI, with the
  Rust core on a tokio multi-thread runtime. Tauri owns the OS main thread.
- **Per-agent subprocess:** spawned via `claude -p --input-format
  stream-json --output-format stream-json --verbose --append-system-prompt
  <text> --mcp-config <file> --strict-mcp-config
  --dangerously-skip-permissions`. Model swap per agent via
  `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_MODEL`.
  `BOT_HQ_SESSION_ID` is set so git hooks can find the session's policy
  state.
- **Two MCP servers:**
  - **Internal** at `127.0.0.1:<ephemeral>` — UI-signaling tools served
    to child agents (26 tools — see "Internal MCP tools" below).
  - **External** at `127.0.0.1:7892` — driver tools served to any
    bearer-token-authenticated MCP client (see "Driving bot-hq from
    another MCP client" below).
- **Storage:** sqlite via sqlx. Tables: `messages`, `sessions`,
  `agent_configs`, `questions` (per-session question tray),
  `cl_index` (searchable CL file index). Emma is a singleton session row
  (`id="emma"`).
- **IPAV:** in-memory `HashMap<SessionId, IpavState>`. Phases I/P use a
  1.5s buffered peer-forward; A/V is pure turn-based.
- **Policy enforcement (`src/policy/`):** two-layer enforcement of
  per-project rules (forbidden words, push gate, force-push, tool
  blocklist, branch pattern). Layer 1: MCP tool calls
  (`check_commit_message`, `request_approval`, …) — probabilistic primary
  path, audited via `violations.jsonl`. Layer 2: git hooks
  (`commit-msg`, `pre-commit`, `post-commit`, `pre-push`) installed in
  the working repo — deterministic backstop catching cases where the
  agent context drifted. See "Policy enforcement" below.

## Internal MCP tools (served to child agents)

Bot-hq exposes these to each spawned agent via the per-agent
`mcp-config.json` (written to a temp file, points at
`http://127.0.0.1:<port>/sessions/<id>/<agent>/mcp`).

| Tool | Purpose |
|---|---|
| `ask_user_choice(question, options)` | Park a structured question for the user. Blocks the agent's turn until the user picks. |
| `mark_awaiting_user(reason)` | Flag the session's `[Need User Input]` badge. Non-blocking. |
| `request_approval(kind, action, …)` | Per-action approval gate. Used by push gate, force-push, per-action approval. |
| `action_gate(command)` | Run a Bash command the Tool Gate blocked: bot-hq surfaces Approve/Reject and, on approve, executes it in the session repo and returns the output. |
| `check_commit_message(message)` | Pre-commit grep for forbidden words. Returns `ok` or `forbidden_word:<word>`. |
| `close_session()` | Ask the host to close this session. |
| `list_my_pending_questions()` | List questions THIS agent has parked but haven't been answered. Used to avoid duplicate retries. |
| `withdraw_question(choice_id)` | Withdraw a stale parked question. |
| `cl_index_search(project, query?)` | Search the SQLite-backed Context Library index. |
| `cl_register_read(project, file_path)` | Audit insert recording which CL file the agent read. |
| `cl_rescan(project)` | Re-stat a project's CL directory after creating new files. |
| `grant_session_permission(action, scope, branches?)` | Record a session-level commit/push grant so subsequent ops skip approval. |
| `revoke_session_permission(action)` | Revoke a previously granted commit/push permission. |
| `list_session_permissions()` | List the current session's commit/push grants. |
| `advance_phase(target)` | Move the IPAV phase chip yourself — no user gate. |
| `request_phase_advance(target, reason)` | Request a user-acknowledged phase advance before an irreversible step. |
| `supersede_question(choice_id, …)` | Replace a parked question with a rephrased one (links old→new). |
| `session_doc_write(slug, body, phase?)` | Upsert a per-session scratch doc; `phase` surfaces it in the IPAV tabs. |
| `session_doc_search(query?, phase?)` | List this session's scratch docs; `phase` filter for cross-phase retrieval. |
| `session_doc_read(slug)` | Read a session doc by slug. |
| `cl_folder_search(project, query?)` | Search CL folder descriptions (folder-level parallel to `cl_index_search`). |
| `cl_register_folder_description(project, folder_path, …)` | Write a CL folder description (HANDS/Emma only). |
| `webview_screenshot()` | Capture the bot-hq webview for agent-driven UI testing. |
| `webview_click(selector)` | Synthesize a click on a DOM element in the webview. |
| `webview_type(selector, text)` | Type into a webview element. |
| `webview_scroll(selector?, y)` | Scroll an element or the page in the webview. |
| `webview_press_key(key)` | Dispatch a keypress in the webview. |

Role boundary: Rain (EYES) is blocked from `ask_user_choice`,
`mark_awaiting_user`, `request_approval`, `action_gate`,
`supersede_question`, `grant_session_permission`,
`revoke_session_permission` (the `HANDS_ONLY_TOOLS` gate), plus
`cl_register_folder_description` (the
`CL_MUTATE_TOOLS` gate — Emma is allowed). Tool calls from Rain to a
blocked tool return an error. Brian (HANDS) gets the full set.

## Policy enforcement

Each project (matched by `working_repo_path` basename) can carry a
`policy.yaml` under `<data_dir>/projects/<project>/`. The file is layered
over `<data_dir>/general-policy.yaml`. Policy fields:

- `forbidden_in_commits: [string]` — words/phrases that must not appear
  in commit messages or staged diffs.
- `push_gate.mode: auto | per_branch_approval | always_ask` — controls
  whether agents must call `request_approval` before `git push`.
- `force_push.mode: blocked | token_required | allowed` — controls
  `git push --force`. `token_required` accepts a user-typed token
  matching `force_push.token_format` (supports `{branch}`, `{sha}`).
- `tool_blocklist: [prefix]` — RETIRED. Superseded by the global
  **Tool Gate** (Settings → "Gated Bash Keywords" → `action_gate`); the
  field still parses for backward-compat but is no longer enforced.
- `per_action_approval: [prefix]` — bash commands that always ask, no
  remembered approval.
- `branch_pattern: regex` — regex branch names must match. Empty = no
  constraint.
- `commit_style: text` — free-form note surfaced in the agent's system
  prompt.

**Tool Gate.** Beyond `policy.yaml`, a global keyword list (Settings →
"Gated Bash Keywords", stored at `<data_dir>/tool-gate.json`) gates agent
Bash commands. A `gate` keyword blocks the command and routes it to the
`action_gate` MCP tool (Approve/Reject → bot-hq executes on approve); an
`auto_allow` keyword lets it run with no prompt. Case-insensitive substring
match on the command or tool name; the list is global, not per-project, and
defaults empty.

**Session-level grants** ride alongside this. When the user types
"you can push" / "you can commit" in chat, the agent calls
`grant_session_permission` which mirrors a JSON file to
`<data_dir>/.local/session-permissions/<session_id>.json`. The
`pre-push` git hook reads this file (via `BOT_HQ_SESSION_ID` env var) to
decide whether to allow the push without re-prompting. Grants are wiped
on session close and on bot-hq startup.

**Two-layer enforcement:**
1. MCP tools (`check_commit_message`, `request_approval`, …) — agents
   call them before the corresponding bash op. Skipping logs a
   `Denied` violation to `violations.jsonl`.
2. Git hooks (`commit-msg`, `pre-commit`, `post-commit`, `pre-push`) —
   `bot-hq install-hooks` writes them into `.git/hooks/` of the working
   repo. Each hook execs `bot-hq policy-check <sub> --data-dir …
   --project … --session …` which re-resolves the policy and decides
   exit code. Hooks are idempotent and respect foreign hooks (write
   `.bot-hq` sidecar instead of clobbering).

`violations.jsonl` lives at `<data_dir>/violations.jsonl`. UI exposes a
viewer (Settings → Violations).

## Driving bot-hq from another MCP client

Bot-hq exposes a second MCP HTTP server so an external agent (another
claude-code session, a test driver, a custom tool) can manage sessions
without the GUI.

**Endpoint:** `http://127.0.0.1:7892/mcp` (POST, JSON-RPC body)
**Auth:** `Authorization: Bearer <token>` where token lives at
`<data_dir>/mcp-token` (UUIDv4, `0600` perms, auto-generated on first
run)

### Setup in another claude-code session

Add to your MCP config (typically `~/.claude.json` or per-project
`mcp.json`):

```json
{
  "mcpServers": {
    "bot-hq": {
      "type": "http",
      "url": "http://127.0.0.1:7892/mcp",
      "headers": {
        "Authorization": "Bearer <paste contents of ~/.bot-hq/mcp-token>"
      }
    }
  }
}
```

Restart that claude-code; the bot-hq tools appear as `mcp__bot-hq__*`.

### Available external tools

| Tool | Purpose |
|---|---|
| `list_sessions` | Read active sessions (id, title, phase, models). |
| `create_session(title, working_repo_path?)` | Spawn a Brian+Rain duo. |
| `send_message(session_id, text)` | Broadcast to a session (or `"emma"`). |
| `get_session_messages(session_id, since_id?)` | Read chat in order. |
| `get_emma_messages(since_id?)` | Same, for Emma. |
| `advance_phase(session_id, phase)` | Move through I/P/A/V. |
| `resolve_choice(choice_id, picked)` | Answer a parked `ask_user_choice`. |
| `get_pending_choices` | List parked choices with their choice_ids. |
| `close_session(session_id, archive?)` | Kill duo + mark closed. |
| `restart_emma` | Kill + respawn Emma (e.g. after config swap). |
| `get_status` | Version, addresses, session count, uptime. |
| `get_agent_configs` | Read agent configs (auth_token redacted to last 4 chars). |
| `set_agent_config(agent_name, …)` | Upsert a row; empty string clears a field. |
| `get_violations(limit?)` | Read recent `violations.jsonl` entries. |
| `wait_for_change(session_id, since_id?, timeout_ms?)` | Long-poll: blocks server-side until new messages arrive or timeout. |
| `get_session_snapshot(session_id, msg_limit?)` | One-shot aggregate: session meta + last N messages + phase + awaiting + pending choices. |
| `webview_screenshot(session_id?)` | Capture the bot-hq webview as a base64 PNG. |
| `webview_click(selector)` | Click a DOM element in the webview. |
| `webview_type(selector, text)` | Type into a webview element. |
| `webview_scroll(selector?, y)` | Scroll an element or the page in the webview. |
| `webview_press_key(key)` | Dispatch a keypress in the webview. |

### Security model

- **Localhost-only bind** (`127.0.0.1`, not `0.0.0.0`). Refuses remote
  connections at the bind layer.
- **Bearer token** with constant-time comparison via the `subtle` crate.
  Guards against timing attacks if you ever expose the port via reverse
  proxy.
- **`auth_token` redaction** in `get_agent_configs`. Returns
  `<set:****abcd>` showing only the last 4 chars. Writing a new value
  via `set_agent_config` still requires the full token.
- **Port conflict:** soft-fails — internal MCP keeps working, Settings →
  External MCP shows "unavailable". Quickest fix: kill the conflicting
  process or set `BOT_HQ_EXTERNAL_MCP_PORT`.
- **Rotation:** edit `<data_dir>/mcp-token` and restart bot-hq. The
  token is read once at startup.

## Security caveats (v1)

- **Plaintext auth tokens.** `agent_configs.auth_token` is stored as
  plaintext sqlite. Any backup of `<data_dir>` (Time Machine, cloud
  sync, rsync) captures these. The DB file is at
  `<data_dir>/.local/bot-hq.db` with default user-only mode bits.
  v2 will move to the OS keychain (`keyring-core`) — see PLAN.md.
- **Policy audit is local-only.** `violations.jsonl` is an append-only
  audit trail; nothing ships it off-host. If you need it shipped to a
  central log store, hook it via a sidecar reader.

## Testing

```bash
cargo test                          # 288 tests
RUN_LIVE_TESTS=1 cargo test         # includes claude-code subprocess smoke
cargo build --release               # production binary
```

Breakdown: lib unit tests plus signaling, storage, and external-MCP
integration suites — 288 Rust total, plus 14 frontend Vitest.
