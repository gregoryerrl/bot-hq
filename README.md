# bot-hq

**Two AI agents — one builds, one reviews. You orchestrate.**

bot-hq is a desktop app for running AI-assisted coding sessions you can actually
trust. Instead of one assistant working unchecked, every session pairs **Brian**,
who writes the code, with **Rain**, who reviews it adversarially. You sit above both
as the conductor — approving the risky steps, steering the work, and reading exactly
what each agent investigated, planned, did, and verified.

It's built for people who want the speed of AI coding without giving up oversight: a
clear workflow, a project knowledge base your agents read before they touch
anything, and guardrails that stop the embarrassing mistakes before they ship.

> Desktop app for macOS, Linux, and Windows. Free and open source (MIT).
>
> **[⬇ Download the latest release](https://github.com/gregoryerrl/bot-hq/releases)**

---

## Why bot-hq?

A single AI coding assistant is fast but unaccountable — it can confidently ship a
plausible-looking bug, forget your project's conventions, or rewrite a file you
didn't want touched. bot-hq is built around three ideas that fix that:

- **A builder and a reviewer, not a lone agent.** Real review catches what the
  author misses — especially when the reviewer runs a *different* model and gives a
  genuine second opinion instead of an echo.
- **A shared, durable knowledge base.** Your conventions, gotchas, and project
  memory live in one place the agents read *before* they start — so they begin
  informed, not cold.
- **You stay in the loop on what matters.** Pushes, risky commands, and flagged
  problems pause for your approval. Everything else just flows.

---

## The duo you orchestrate

Every session spawns two agents with distinct, enforced roles:

|                    | **Brian — HANDS**                              | **Rain — EYES**                                  |
| ------------------ | ---------------------------------------------- | ------------------------------------------------ |
| Role               | Executes: writes code, runs commands, commits  | Reviews: reads Brian's work, pushes back hard    |
| Can edit files?    | Yes                                            | No — write tools are blocked                     |
| Think of them as   | Your pair-programmer                           | Your reviewer / second opinion                   |

**You are the third role** — the orchestrator. You give the task, answer the
questions the agents park for you, approve the steps that need a human, and decide
when the work is done. Brian and Rain don't run off on their own; you conduct.

Rain can run on a *different* model than Brian when you want one. A reviewer that
doesn't share the author's blind spots catches more — cross-model diversity buys a
real second opinion instead of an echo chamber.

---

## How a session flows: IPAV

Every substantial task walks through four phases — **Investigate → Plan → Apply →
Verify** — and you can watch it happen.

1. **Investigate** — the agents gather facts: read the code, the docs, your project
   notes. Nothing changes yet.
2. **Plan** — Brian proposes an approach (which files, what changes, the tradeoffs);
   Rain reviews it *before* any code is written.
3. **Apply** — Brian produces the work: the actual edits, commits, and commands.
4. **Verify** — the result is checked against the plan: tests run, output read, an
   adversarial proof-read.

Each phase leaves behind a **session document** — a short write-up of what was
found, planned, done, or verified. The session view has an **I / P / A / V tab** for
each phase, so you can open any one and read the agents' own account instead of
scrolling back through chat. The Apply tab also renders a live, color-coded `git
diff` of the session's work, so you can eyeball the real changes at a glance.

IPAV isn't bureaucracy — it's there so an AI change can't ship on momentum. The plan
gets reviewed before the code exists; the result gets verified before it's called
done.

---

## The Context Library

This is the part most AI tools are missing. The **Context Library** is a curated
knowledge base your agents read *before* they start working — the things that aren't
in the code and that a fresh assistant would never know:

- **Conventions** — how *this* project does commits, tests, formatting, branching.
- **Gotchas** — the trap that bit you last time, the test that's flaky, the file
  that looks dead but isn't.
- **Project memory** — decisions you made and why, so they don't get re-litigated
  every session.

Each project gets its own space (`conventions.md`, `notes.md`, `decisions.md`, and
anything else you add). It's indexed for fast, description-aware search, and the
agents are disciplined to consult it *first* — so a perfectly correct fix doesn't
ship in the wrong house style.

You curate it in the **Context Library tab**: a file tree plus an editor, with
folders, search, and per-project organization. The agents read it on demand and, by
design, only ever write to it in one narrow, audited way — a short "what I learned"
note at the end of a session, which you can keep or prune. The knowledge base stays
yours.

Why it matters: the difference between an assistant that re-asks the same questions
every session and one that already knows how your project works *is* the Context
Library.

---

## Guardrails

bot-hq is the policy layer, so you can let the agents move quickly without letting
them ship something you'll regret:

- **Commit hygiene** — a configurable list of forbidden words/phrases is checked on
  every commit (handy for keeping unwanted attribution lines, secrets, or banned
  terms out of your history).
- **Push approval** — pushes can be set to pause for a one-click Approve / Reject, so
  nothing reaches your remote without you.
- **Review sign-off** — when Rain flags a real problem as *blocking*, Brian can't
  commit over it until it's resolved or explicitly rebutted.
- **Sensitive commands** — you can gate specific commands by keyword so they ask
  before running.

Enforcement runs at two layers — the agents are told to check, *and* git hooks in
the repo enforce it independently — so a guardrail holds even if an agent's
attention drifts. Every enforcement event is logged for you to review.

---

## Parallel sessions

Each repo-backed session runs in its own isolated **git worktree** by default, on
its own branch. So you can run several sessions on the same project at once —
different features, different agents — with no file collisions between them. Merging
back is your call, through the normal push flow.

---

## Install

### Just want to run it

Grab the latest build for your platform from the **[releases
page](https://github.com/gregoryerrl/bot-hq/releases)**, then see
[`INSTALL.md`](INSTALL.md) for per-platform notes — including the first-launch
Gatekeeper step on macOS, since builds are currently unsigned.

You'll also need the [`claude-code`](https://claude.com/claude-code) CLI installed
and authenticated — bot-hq drives it under the hood, one process per agent.

### Build from source

```bash
git clone https://github.com/gregoryerrl/bot-hq.git
cd bot-hq
cp .env.example .env                    # sets BOT_HQ_DATA_DIR=~/.bot-hq-dev/
(cd frontend && npm install)            # React frontend deps (Vite + Tauri CLI)
cargo install tauri-cli --version '^2'  # one-time, if `cargo tauri` is missing
cargo tauri dev                         # builds the UI + opens the desktop window
```

Prerequisites: **Rust** (latest stable), **Node 22+ / npm**, and the **`claude-code`
CLI** (authenticated, `2.x` or newer), on **macOS / Linux / Windows**.

For a release build: `cargo tauri build` (bundles the app under
`target/release/bundle/`).

---

## For developers

bot-hq is a single Rust binary: a **Tauri v2** shell + **React 18 + TypeScript +
Tailwind** UI, with the Rust core on a Tokio runtime. Each agent is a `claude-code`
subprocess wired over stream-json; two in-process MCP servers handle UI signaling
and external driver access; storage is sqlite; policy is enforced by MCP tools plus
git hooks.

The canonical docs go deeper than this README:

- **[`ARCHITECTURE.md`](ARCHITECTURE.md)** — what bot-hq is, in depth: process
  model, both MCP servers, storage schema, policy layer, glossary.
- **[`PLAN.md`](PLAN.md)** — what's planned next.
- **[`PROGRESS.md`](PROGRESS.md)** — recent change log, newest-first.

The original rebuild design + roadmap are preserved under
[`docs/rebuild-archive/`](docs/rebuild-archive/).

<details>
<summary><b>Repo layout</b></summary>

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

</details>

<details>
<summary><b>Configuration (environment variables)</b></summary>

Env vars read at startup:

| Var                            | Default             | Purpose                                        |
| ------------------------------ | ------------------- | ---------------------------------------------- |
| `BOT_HQ_DATA_DIR`              | `~/.bot-hq/`        | Context Library + sqlite DB location           |
| `BOT_HQ_EXTERNAL_MCP_PORT`     | `7892`              | External driver MCP server port                |
| `BOT_HQ_EXTERNAL_MCP_DISABLED` | unset               | Set to `1` to skip external MCP server startup |
| `RUN_LIVE_TESTS`               | unset               | Set to `1` to include subprocess tests         |
| `RUST_LOG`                     | `info,bot_hq=debug` | tracing-subscriber EnvFilter                   |

**During development** keep `BOT_HQ_DATA_DIR=~/.bot-hq-dev/` in `.env` so you don't
collide with a running production bot-hq at `~/.bot-hq/`.

</details>

### Architecture in 60 seconds

- **Stack:** single Rust binary — Tauri v2 shell + React 18 UI, with the Rust core
  on a Tokio multi-thread runtime. Tauri owns the OS main thread.
- **Per-agent subprocess:** each agent is spawned as `claude -p` in stream-json
  mode, with bot-hq's role prompt appended and a per-agent MCP config. Model swap per
  agent via `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_MODEL`.
- **Two MCP servers:** an **internal** one (UI-signaling tools served to the
  agents) on an ephemeral localhost port, and an **external** one on `127.0.0.1:7892`
  (driver tools for any bearer-token-authenticated MCP client).
- **Storage:** sqlite via sqlx — messages, sessions, agent/model configs, the
  durable awaiting-input tray, IPAV session documents, plugins, and the searchable CL
  index.
- **Policy enforcement:** two layers over per-project rules (forbidden words, push
  gate, force-push, branch pattern) — MCP tool calls as the primary path, git hooks
  as a deterministic backstop. Audited to `violations.jsonl`.

## Internal MCP tools (served to child agents)

bot-hq exposes a set of UI-signaling tools to each spawned agent (parking questions
for you, requesting approval, reading/writing session docs, searching the Context
Library, advancing the IPAV phase, and so on). Rain (EYES) is blocked from the
action-taking tools — that role boundary is enforced server-side, not by convention.

<details>
<summary><b>Full internal tool list</b></summary>

| Tool | Purpose |
|---|---|
| `ask_user_choice(question, options)` | Park a structured question for the user. Returns a parked ack; the pick arrives out-of-band. |
| `mark_awaiting_user(reason)` | Flag the session's `[Need User Input]` badge. Non-blocking. |
| `request_approval(kind, action, …)` | Per-action approval gate. Used by push gate, force-push, per-action approval. |
| `action_gate(command)` | Run a Bash command the Tool Gate blocked: bot-hq surfaces Approve/Reject and, on approve, executes it in the session repo and returns the output. |
| `check_commit_message(message)` | Pre-commit grep for forbidden words. Returns `ok` or `forbidden_word:<word>`. |
| `close_session()` | Ask the host to close this session. |
| `list_my_pending_questions()` | List questions THIS agent has parked but haven't been answered. Used to avoid duplicate retries. |
| `withdraw_question(choice_id)` | Withdraw a stale parked question. |
| `supersede_question(choice_id, …)` | Replace a parked question with a rephrased one (links old→new). |
| `cl_index_search(project, query?)` | Search the SQLite-backed Context Library index. |
| `cl_folder_search(project, query?)` | Search CL folder descriptions (folder-level parallel to `cl_index_search`). |
| `cl_register_read(project, file_path)` | Audit insert recording which CL file the agent read. |
| `cl_register_folder_description(project, folder_path, …)` | Write a CL folder description (HANDS only). |
| `cl_rescan(project)` | Re-stat a project's CL directory after creating new files. |
| `advance_phase(target)` | Move the IPAV phase chip yourself — no user gate. |
| `request_phase_advance(target, reason)` | Request a user-acknowledged phase advance before an irreversible step. |
| `session_doc_write(slug, body, phase?)` | Upsert a per-session scratch doc; `phase` surfaces it in the IPAV tabs. |
| `session_doc_search(query?, phase?)` | List this session's scratch docs; `phase` filter for cross-phase retrieval. |
| `session_doc_read(slug)` | Read a session doc by slug. |
| `web_search(query, engine?)` | Search the web via a headless webview, so non-first-party models without a server-side search tool can fetch live results. |
| `webview_screenshot()` | Capture the bot-hq webview for agent-driven UI testing. |
| `webview_click(selector)` | Synthesize a click on a DOM element in the webview. |
| `webview_type(selector, text)` | Type into a webview element. |
| `webview_scroll(selector?, y)` | Scroll an element or the page in the webview. |
| `webview_press_key(key)` | Dispatch a keypress in the webview. |

Role boundary: Rain (EYES) is blocked from `ask_user_choice`, `mark_awaiting_user`,
`request_approval`, `action_gate`, `supersede_question`, and
`cl_register_folder_description`. Brian (HANDS) gets the full set.

</details>

## Policy enforcement

Each project can carry a `policy.yaml` under
`<data_dir>/library/projects/<project>/`, layered over a machine-wide
`config/general-policy.yaml`. Fields:

- `forbidden_in_commits: [string]` — words/phrases that must not appear in commit
  messages or staged diffs.
- `push_gate: auto | ask` — `auto` lets pushes through; `ask` makes the `pre-push`
  hook surface a per-push Approve/Reject prompt and block on your pick.
- `force_push: blocked | allowed` — controls `git push --force` /
  `--force-with-lease`.
- `per_action_approval: [prefix]` — bash commands that always ask, with no
  remembered approval.
- `branch_pattern: regex` — branch names must match. Empty = no constraint.
- `commit_style: text` — free-form note surfaced in the agent's system prompt.

**Tool Gate.** Beyond `policy.yaml`, a global keyword list (Settings → "Gated Bash
Keywords") gates agent Bash commands: a `gate` keyword blocks the command and routes
it to the `action_gate` tool (Approve/Reject → bot-hq executes on approve); an
`auto_allow` keyword lets it run with no prompt.

**Two layers.** (1) MCP tools (`check_commit_message`, `request_approval`, …) are
the primary path — agents call them before the corresponding bash op, and skipping
logs a `Denied` violation. (2) Git hooks (`commit-msg`, `pre-commit`, `post-commit`,
`pre-push`), installed in the working repo by `bot-hq install-hooks`, re-resolve the
policy and decide the exit code — a deterministic backstop for when an agent's
context drifts. Hooks are idempotent and respect foreign hooks (write a `.bot-hq`
sidecar instead of clobbering). The audit trail lives at
`<data_dir>/.local/violations.jsonl` (viewer in Settings → Violations).

## Driving bot-hq from another MCP client

bot-hq exposes a second MCP HTTP server so an external agent (another claude-code
session, a test driver, a custom tool) can manage sessions without the GUI.

**Endpoint:** `http://127.0.0.1:7892/mcp` (POST, JSON-RPC body).
**Auth:** `Authorization: Bearer <token>`, where the token lives at
`<data_dir>/.local/mcp-token` (UUIDv4, `0600` perms, auto-generated on first run).

Add it to another claude-code session's MCP config:

```json
{
  "mcpServers": {
    "bot-hq": {
      "type": "http",
      "url": "http://127.0.0.1:7892/mcp",
      "headers": {
        "Authorization": "Bearer <paste contents of ~/.bot-hq/.local/mcp-token>"
      }
    }
  }
}
```

Restart that claude-code; the bot-hq tools appear as `mcp__bot-hq__*`.

### Available external tools

<details>
<summary><b>Full external (driver) tool list</b></summary>

| Tool | Purpose |
|---|---|
| `list_sessions` | Read active sessions (id, title, phase, models). |
| `create_session(title, working_repo_path?)` | Spawn a Brian+Rain duo. |
| `send_message(session_id, text)` | Broadcast to a session. |
| `get_session_messages(session_id, since_id?)` | Read chat in order. |
| `advance_phase(session_id, phase)` | Move through I/P/A/V. |
| `resolve_choice(choice_id, picked)` | Answer a parked `ask_user_choice`. |
| `get_pending_choices` | List parked choices with their choice_ids. |
| `close_session(session_id, archive?)` | Kill duo + mark closed. |
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

</details>

**Security model:** localhost-only bind (`127.0.0.1`, refuses remote connections at
the bind layer); bearer token with constant-time comparison; `auth_token` redaction
in `get_agent_configs`; soft-fail on port conflict (internal MCP keeps working).
Rotate the token by editing `<data_dir>/.local/mcp-token` and restarting bot-hq.

## Security caveats (v1)

- **Plaintext auth tokens.** `agent_configs.auth_token` is stored as plaintext
  sqlite at `<data_dir>/.local/bot-hq.db` (default user-only mode bits). Any backup
  of `<data_dir>` (Time Machine, cloud sync, rsync) captures these. v2 will move to
  the OS keychain — see [`PLAN.md`](PLAN.md).
- **Policy audit is local-only.** `violations.jsonl` is an append-only audit trail;
  nothing ships it off-host. Hook a sidecar reader if you need it centralized.

## Testing

```bash
cargo test                          # Rust unit + integration suites
RUN_LIVE_TESTS=1 cargo test         # includes claude-code subprocess smoke
cargo build --release               # production binary
```

The suite covers the lib units plus signaling, storage, and external-MCP integration
tests, plus the frontend Vitest suite. Live pass counts are tracked in
[`PROGRESS.md`](PROGRESS.md) (they drift each commit).

---

Licensed under [MIT](LICENSE).
