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

- **Rust stable** (≥ 1.92 — slint 1.16.1 MSRV). `rustup update stable`.
- **`claude-code` CLI** installed and authed. `claude --version` should
  print `2.x` or newer; `claude auth status` should be healthy.
- **macOS** is the primary target. Slint covers Linux + Windows; builds
  and smoke testing happen on macOS.

## Quickstart

```bash
git clone <repo>
cd bot-hq
cp .env.example .env       # contains BOT_HQ_DATA_DIR=~/.bot-hq-dev/
cargo run                  # opens the desktop window
```

Release build:

```bash
cargo build --release
./target/release/bot-hq
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
├── Cargo.toml
├── CLAUDE.md / ARCHITECTURE.md / PLAN.md / PROGRESS.md   ← canonical docs
├── ui/app.slint                                          ← all Slint UI
├── src/
│   ├── main.rs            entry point — tokio runtime, Slint loop, CLI dispatch
│   ├── lib.rs             module exports + slint::include_modules!()
│   ├── app.rs             top-level AppState (single Arc shared across threads)
│   ├── paths.rs           data-dir resolution + first-run init + single-instance lock
│   ├── storage/           sqlite (messages, sessions, agent_configs, questions, cl_index)
│   ├── cl/                Context Library reader + SQLite-backed index
│   ├── agents/            claude-code subprocess + stream-json I/O + hardcoded role prompts
│   ├── signaling/         in-process MCP HTTP server — internal (UI tools) + external (driver tools)
│   ├── policy/            Policy resolution, git-hook installer, session-permission grants, violations log
│   ├── core/              sessions, IPAV cache, duo coordination, broadcast
│   └── ui/view_model.rs   Slint ↔ core bridge
├── migrations/0001_init.sql + later migrations
├── templates/cl/          baked-in default CL (used on first run)
└── docs/
    ├── stream-json-events.md     claude-code CLI event schema (empirical)
    └── rebuild-archive/          original rebuild design + roadmap + decisions
```

## Architecture in 60 seconds

- **Stack:** Single Rust binary, Slint UI on the OS main thread, tokio
  multi-thread runtime for I/O.
- **Per-agent subprocess:** spawned via `claude -p --input-format
  stream-json --output-format stream-json --verbose --append-system-prompt
  <text> --mcp-config <file> --strict-mcp-config
  --dangerously-skip-permissions`. Model swap per agent via
  `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_MODEL`.
  `BOT_HQ_SESSION_ID` is set so git hooks can find the session's policy
  state.
- **Two MCP servers:**
  - **Internal** at `127.0.0.1:<ephemeral>` — UI-signaling tools served
    to child agents (13 tools — see "Internal MCP tools" below).
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
| `request_approval(kind, action, …)` | Per-action approval gate. Used by push gate, force-push, tool blocklist, per-action approval. |
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

Role boundary: Rain (EYES) is blocked from `ask_user_choice`,
`mark_awaiting_user`, `request_approval`, `grant_session_permission`,
`revoke_session_permission`. Tool calls from Rain return a
`HANDS_ONLY_TOOLS` error. Brian (HANDS) and Emma get the full set.

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
- `tool_blocklist: [prefix]` — bash command prefixes that require
  approval. Matched as a prefix on the full command string.
- `per_action_approval: [prefix]` — bash commands that always ask, no
  remembered approval.
- `branch_pattern: regex` — regex branch names must match. Empty = no
  constraint.
- `commit_style: text` — free-form note surfaced in the agent's system
  prompt.

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
cargo test                          # 165 tests
RUN_LIVE_TESTS=1 cargo test         # includes claude-code subprocess smoke
cargo build --release               # production binary
```

Breakdown: 121 lib unit tests + 29 signaling integration + 5 external
MCP integration + 10 storage integration.
