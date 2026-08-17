# bot-hq

**Agents you define. Roles you enforce. You orchestrate.**

bot-hq is a desktop app for running AI-assisted coding sessions you can actually
trust. Instead of one assistant working unchecked, you define **roles** — what each
one may do, what model it runs on, how it is briefed — and a session runs however
many of them the work needs. The usual shape is one that writes code and one that
reviews it adversarially. You sit above them as the conductor: approving the risky
steps, steering the work, and reading exactly what each participant investigated,
planned, did, and verified.

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

## The roles you orchestrate

A session spawns participants from roles you configure. Two roles ship seeded, and
they are a starting point rather than the product:

|                    | **HANDS**                                       | **EYES**                                          |
| ------------------ | ----------------------------------------------- | ------------------------------------------------- |
| Role               | Executes: writes code, runs commands, commits    | Reviews: reads the work, pushes back hard         |
| Can edit files?    | Yes                                              | No — the capability is unticked                   |
| Think of it as     | Your pair-programmer                             | Your reviewer / second opinion                    |

Both are ordinary rows in **Settings → Roles**: rename them, rewrite their
instructions, retick their capabilities, or add roles of your own. Capabilities
are enforced at the tool gate, so unticking one takes the ability away rather than
merely asking the agent not to use it. A participant is shown as `ROLE · Model`,
never as a persona.

**You are the orchestrator.** You give the task, answer the questions participants
park for you, approve the steps that need a human, and decide when the work is
done. They don't run off on their own; you conduct.

**Roles can run on different models.** A reviewer that doesn't share the author's
blind spots catches more — cross-model diversity buys a real second opinion instead
of an echo chamber.

---

## How a session flows: IPAV

Every substantial task walks through four phases — **Investigate → Plan → Apply →
Verify** — and you can watch it happen.

1. **Investigate** — the agents gather facts: read the code, the docs, your project
   notes. Nothing changes yet.
2. **Plan** — the executing role proposes an approach (which files, what changes,
   the tradeoffs); the reviewer reviews it *before* any code is written.
3. **Apply** — the executing role produces the work: the edits, commits, commands.
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
folders, search, and per-project organization. The agents read it on demand and
write to it through one guarded path — `cl_write_file`, capability-gated,
confined to the project's library folder, versioned on every write (the library
is a git repo) and re-indexed at once — typically a short "what I learned" delta
at the end of a session, which you can keep or prune. The knowledge base stays
yours.

Why it matters: the difference between an assistant that re-asks the same questions
every session and one that already knows how your project works *is* the Context
Library.

---

## Guardrails

bot-hq is the policy layer, so you can let the agents move quickly without letting
them ship something you'll regret:

- **Push approval** — pushes can be set to pause for a one-click Approve / Reject, so
  nothing reaches your remote without you.
- **Review sign-off** — when a reviewer flags a real problem as *blocking*, nobody can
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
cp .env.example .env                    # defaults are fine; see the table below
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
Tailwind** UI, with the Rust core on a Tokio runtime. Every agent is a
`claude-code` subprocess wired over stream-json — the CLI is bot-hq's only model
connector. An in-process MCP server handles UI signaling
access; storage is sqlite; policy is enforced by MCP tools plus git hooks.

The canonical docs go deeper than this README:

- **[`ARCHITECTURE.md`](ARCHITECTURE.md)** — what bot-hq is, in depth: process
  model, the in-process MCP server, storage schema, policy layer, glossary.
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
│   ├── core/              sessions, the turn ring + per-participant pumps, activity, broadcast, worktrees, terminal
│   ├── signaling/         in-process MCP HTTP server (UI tools) + SignalingBridge
│   ├── storage/           sqlite (messages, sessions, participants, session_tray, roles, models, cl_index, …)
│   ├── policy/            policy resolution, git-hook installer, session policy snapshots, tool gate, secret scan, violations log
│   ├── plugins/           plugin manifest parser, registry, catalog, asset serving, heartbeat watcher
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
| `RUST_LOG`                     | `info,bot_hq=debug` | tracing-subscriber EnvFilter                   |

A source build uses the same `~/.bot-hq/` as an installed release. Set
`BOT_HQ_DATA_DIR` (e.g. `~/.bot-hq-dev/`) only if you run both and want them kept
apart — otherwise they share one Context Library, sqlite DB, and instance lock.

</details>

### Architecture in 60 seconds

- **Stack:** single Rust binary — Tauri v2 shell + React 18 UI, with the Rust core
  on a Tokio multi-thread runtime. Tauri owns the OS main thread.
- **One agent backend:** every participant is spawned as `claude -p` in
  stream-json mode, with bot-hq's role prompt appended and a per-agent MCP
  config, and the model swapped via `ANTHROPIC_BASE_URL` /
  `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_MODEL` — which is how a non-Anthropic
  model runs through the same CLI. (A second, in-process Rust agent loop existed
  as an opt-in until rc3 D9 removed it; the CLI is the only connector.)
- **One MCP server:** an **internal** one (UI-signaling tools served to the
  agents) on an ephemeral localhost port
- **Storage:** sqlite via sqlx — messages, sessions, agent/model configs, the
  durable awaiting-input tray, IPAV session documents, plugins, and the searchable CL
  index.
- **Policy enforcement:** two layers over per-project rules (push gate, force-push,
  branch pattern) — MCP tool calls as the primary path, git hooks as a deterministic
  backstop. Audited to `violations.jsonl`.

## Internal MCP tools (served to child agents)

bot-hq exposes a set of UI-signaling tools to each spawned agent (parking questions
for you, requesting approval, reading/writing session docs, searching the Context
Library, advancing the IPAV phase, and so on). A role without the matching
capability is blocked from the
action-taking tools — that role boundary is enforced server-side, not by convention.

<details>
<summary><b>Full internal tool list</b></summary>

| Tool | Purpose |
|---|---|
| `ask_user_choice(question, options)` | Park a structured question for the user. Returns a parked ack; the pick arrives out-of-band. |
| `mark_awaiting_user(reason)` | Flag the session's `[Need User Input]` badge. Non-blocking. |
| `peer_ack(final?)` | Say you have converged and end the round instead of bouncing another acknowledgment. A content-free ack (or `final: true`) ends the turn as a DONE vote toward consensus; a substantive turn stays an ordinary turn with the ack recorded as overridden, so a review is never silently downgraded to agreement. |
| `halt(reason?)` | Yield to the user and unlock the chat input (sets awaiting, which outranks busy). Like `mark_awaiting_user` framed as a yield. HANDS only. |
| `request_approval(kind, action, …)` | Per-action approval gate. Used by push gate, force-push, per-action approval. |
| `action_gate(command)` | Run a Bash command the Tool Gate blocked: bot-hq surfaces Approve/Reject and, on approve, executes it in the session repo and returns the output. |
| `check_commit_message(message)` | Pre-commit grep of a proposed message against the project's forbidden-words policy. Returns `ok` or `forbidden_word:<w>`. |
| `eyes_flag(severity, summary, …)` | **EYES only.** File a review finding; a `blocking` one gates HANDS's next `git commit` until resolved. |
| `disposition_finding(finding_id, status, reason)` | **HANDS only.** Resolve a finding (`fixed` / `rebutted`), clearing the commit gate. |
| `check_open_findings()` | Check for unresolved blocking findings before committing. Returns `ok` or the blocking list. |
| `override_reviewer_block()` | **HANDS only.** Escape valve for the fail-closed "reviewer is down" commit block. |
| `approve_finding(finding_id)` | **EYES only.** Sign off that an escalated fix HANDS marked fixed is genuinely resolved. |
| `close_session()` | Ask the host to close this session. |
| `list_my_pending_questions()` | List questions THIS agent has parked but haven't been answered. Used to avoid duplicate retries. |
| `withdraw_question(choice_id)` | Withdraw a stale parked question. |
| `supersede_question(choice_id, …)` | Replace a parked question with a rephrased one (links old→new). |
| `cl_index_search(project, query?)` | Search the SQLite-backed Context Library index — lightweight rows, so an agent can decide what is worth opening. |
| `cl_retrieve(project, query, paths?, budget_tokens?)` | Pull ranked CL CONTENT inline under a token budget, instead of the search-then-read-whole-file loop. The 95% path for getting CL knowledge on a topic. |
| `cl_folder_search(project, query?)` | Search CL folder descriptions (folder-level parallel to `cl_index_search`). |
| `cl_register_read(project, file_path)` | Audit insert recording which CL file the agent read. |
| `cl_register_folder_description(project, folder_path, …)` | Write a CL folder description (HANDS only). |
| `cl_write_file(project, file_path, content)` | Create or replace a CL file directly (HANDS only; auto-rescans). |
| `cl_rescan(project)` | Re-stat a project's CL directory after creating new files. |
| `advance_phase(target)` | Cast your vote to advance the IPAV phase; it moves when every active participant has voted at the same state of the work (D37). |
| `request_phase_advance(target, reason)` | Request a user-acknowledged phase advance before an irreversible step. |
| `session_doc_write(slug, body, phase?)` | Upsert a per-session scratch doc; `phase` surfaces it in the IPAV tabs. |
| `session_doc_search(query?, phase?)` | List this session's scratch docs; `phase` filter for cross-phase retrieval. |
| `session_doc_read(slug)` | Read a session doc by slug. |
| `web_search(query, engine?)` | Search the web via a headless webview, so non-first-party models without a server-side search tool can fetch live results. |
| `terminal_exec(command, wait_ms?, block?)` | Run one command in the session's Terminal subtab PTY (user-visible). Blocking by default: waits for output-settle and returns the captured tail; `block:false` for long-running processes. Gate-matched commands are refused (route via `action_gate`). |
| `terminal_read(lines?)` | Tail of the session terminal's scrollback as plain text (default 100 lines, max 500) — evidence agents can paste into chat or IPAV docs. |
| `webview_screenshot()` | Capture the bot-hq webview for agent-driven UI testing. |
| `webview_click(selector)` | Synthesize a click on a DOM element in the webview. |
| `webview_type(selector, text)` | Type into a webview element. |
| `webview_scroll(selector?, y)` | Scroll an element or the page in the webview. |
| `webview_press_key(key)` | Dispatch a keypress in the webview. |
| `pass_turn()` | Decline this turn — recorded in the chat so the user sees you were asked and chose to stay quiet. Not the same as being finished: a pass counts toward nothing and cannot settle the session on its own. |
| `file_feedback(kind, title, body)` | File an issue or idea about BOT-HQ ITSELF into a queue a later session works through. Never interrupts the user, never surfaces mid-session. |
| `gate_status(gate_id)` | Current state of a parked `action_gate` command: pending, approved (with output), or rejected. Read this instead of guessing whether a gated command ran. |
| `cl_stale_refs(project)` | Report CL claims that name code the repo no longer has. Report-only; never edits. |

Role boundary (enforced server-side, from the role's capability ticks): a reviewer is blocked from the
HANDS-only tools — `ask_user_choice`, `mark_awaiting_user`, `halt`, `request_approval`,
`action_gate`, `supersede_question`, `disposition_finding`, `override_reviewer_block`,
`terminal_exec` (EYES reads the terminal via `terminal_read`, never types into it),
and `cl_register_folder_description` (a reviewer converges via `peer_ack`, which is not gated);
`cl_write_file` needs `write_context_library` and `close_session` needs `close_session` (D16).
A role without `file_finding` is blocked from the reviewer tools — `eyes_flag` and `approve_finding`
(HANDS can't file or sign off on findings against its own work).

</details>

## Policy enforcement

Each project can carry a `policy.yaml` under
`<data_dir>/library/projects/<project>/`, layered over a machine-wide
`config/general-policy.yaml`. Fields:

- `push_gate: auto | ask` — `auto` lets pushes through; `ask` makes the `pre-push`
  hook surface a per-push Approve/Reject prompt and block on your pick.
- `force_push: blocked | allowed` — controls `git push --force` /
  `--force-with-lease`.
- `per_action_approval: [prefix]` — bash commands that always ask, with no
  remembered approval.
- `branch_pattern: regex` — branch names must match. Empty = no constraint.

**Tool Gate.** Beyond `policy.yaml`, a global keyword list (Settings → "Gated Bash
Keywords") gates agent Bash commands: a `gate` keyword blocks the command and routes
it to the `action_gate` tool (Approve/Reject → bot-hq executes on approve); an
`auto_allow` keyword lets it run with no prompt.

**Two layers.** (1) MCP tools (`request_approval`, `action_gate`, …) are
the primary path — agents call them before the corresponding bash op, and skipping
logs a `Denied` violation. (2) Git hooks (`commit-msg`, `pre-commit`, `post-commit`,
`pre-push`), installed in the working repo by `bot-hq install-hooks`, re-resolve the
policy and decide the exit code — a deterministic backstop for when an agent's
context drifts. Hooks are idempotent and respect foreign hooks (write a `.bot-hq`
sidecar instead of clobbering). The audit trail lives at
`<data_dir>/.local/violations.jsonl` (viewer in Settings → Violations).

## Driving bot-hq from another MCP client — REMOVED

bot-hq used to expose a second MCP HTTP server so an external agent could manage
sessions without the GUI. **It was removed on 2026-08-17** and demoted to a future
plugin; the endpoint, its bearer token and the `BOT_HQ_EXTERNAL_MCP_*` env vars
are gone. See `ARCHITECTURE.md` § "The external driver — REMOVED" for what was
deleted and where the design record lives.

## Security caveats (v1)

- **Plaintext auth tokens.** `models.auth_token` (and the legacy `agent_configs.auth_token`) is stored as plaintext
  sqlite at `<data_dir>/.local/bot-hq.db` (default user-only mode bits). Any backup
  of `<data_dir>` (Time Machine, cloud sync, rsync) captures these. v2 will move to
  the OS keychain — see [`PLAN.md`](PLAN.md).
- **Policy audit is local-only.** `violations.jsonl` is an append-only audit trail;
  nothing ships it off-host. Hook a sidecar reader if you need it centralized.

## Testing

```bash
cargo test                          # Rust unit + integration suites
cargo build --release               # production binary
```

The suite covers the lib units plus the signaling and storage integration tests,
the repo guards (`tests/codebase_map_test.rs`, `tests/retired_identifier_test.rs`,
`tests/phase_vote_wiring_test.rs`), and the frontend Vitest suite. Live pass counts are tracked in
[`PROGRESS.md`](PROGRESS.md) (they drift each commit).

---

Licensed under [MIT](LICENSE).
