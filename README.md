# bot-hq — Rust + Slint rebuild

Desktop GUI for driving AI-assisted coding sessions through a bilateral-duo
agent model (**Brian** = HANDS, **Rain** = EYES) with an optional helper
(**Emma**). The user is the orchestrator; the app is the conductor between
user and agents.

This is a **from-scratch Rust + Slint rebuild** of the current Go-daemon /
tmux / MCP-hub bot-hq. See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the
single source of truth on architectural decisions and [`PLAN.md`](PLAN.md)
for the phased roadmap.

## Status

This is **v0.1.0**, an autonomous rebuild milestone. The binary builds, all
test suites pass (42 lib + 5 HTTP integration + 9 storage = 56 tests), and
the UI launches and runs without crashing. The full agent lifecycle (spawn
claude-code subprocesses, drive duo coordination, render real chat
streams) requires manual end-to-end verification on a machine with a
display + an authed `claude` CLI. See [PROGRESS.md](PROGRESS.md) for the
status banner + what's verified vs. caveated.

## Prerequisites

- **Rust stable** (≥ 1.92 — slint 1.16.1 MSRV). `rustup update stable`.
- **`claude-code` CLI** installed and authed (used as subprocess by each
  agent). `claude --version` should print `2.x` or newer; `claude auth
  status` should be healthy.
- **macOS** for the initial target. Cross-platform later (Slint covers
  Linux + Windows but the build & smoke testing happened on macOS).

## Quickstart

```bash
git clone <repo>           # this is the bot-hq repo, on the rebuild path
cd bot-hq-rebuild
cp .env.example .env       # contains BOT_HQ_DATA_DIR=~/.bot-hq-dev/
cargo run                  # opens the desktop window
```

For a release build:

```bash
cargo build --release
./target/release/bot-hq
```

## Configuration

bot-hq reads the following env vars at startup:

| Var                | Default        | Purpose                                    |
| ------------------ | -------------- | ------------------------------------------ |
| `BOT_HQ_DATA_DIR`  | `~/.bot-hq/`   | Context Library + sqlite DB location       |
| `RUN_LIVE_TESTS`   | unset          | Set to `1` to include subprocess tests     |
| `RUST_LOG`         | `info,bot_hq=debug` | tracing-subscriber EnvFilter          |

**During development** keep `BOT_HQ_DATA_DIR=~/.bot-hq-dev/` in `.env` so
you don't collide with the still-running current bot-hq's `~/.bot-hq/`.

## Layout

```
bot-hq-rebuild/
├── Cargo.toml
├── CLAUDE.md / ARCHITECTURE.md / PLAN.md / PROGRESS.md  ← canonical docs
├── ui/app.slint                              ← all Slint UI
├── src/
│   ├── main.rs            ← entry point, tokio runtime + Slint loop
│   ├── lib.rs             ← module exports + slint::include_modules!()
│   ├── paths.rs           ← data-dir + first-run + single-instance lock
│   ├── storage/           ← sqlite (messages, sessions, agent_configs)
│   ├── cl/                ← Context Library (Phase 8 placeholder)
│   ├── agents/            ← claude-code subprocess + stream-json I/O
│   ├── signaling/         ← in-process MCP HTTP server (2 tools)
│   ├── core/              ← sessions, IPAV, duo coordination
│   └── ui/view_model.rs   ← Slint↔core bridge
├── migrations/0001_init.sql
├── templates/cl/          ← baked-in default CL (used on first run)
└── docs/
    ├── decisions.md
    └── stream-json-events.md
```

## Architecture in 60 seconds

- **Stack:** Single Rust binary, Slint UI on the OS main thread, tokio
  multi-thread runtime for I/O.
- **Per-agent subprocess:** spawned via `claude -p --input-format
  stream-json --output-format stream-json --verbose --append-system-prompt
  <text> --mcp-config <file>`. Model swap per agent via
  `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_MODEL`.
- **UI signaling:** in-process **HTTP MCP server** (hand-rolled minimal
  JSON-RPC over hyper 1.x — see Decisions Made Autonomously below). Two
  tools: `ask_user_choice(question, options)` (blocking) and
  `mark_awaiting_user(reason)` (non-blocking).
- **Storage:** sqlite via sqlx, tables `messages` / `sessions` /
  `agent_configs`. Emma is a singleton session row (`id="emma"`).
- **IPAV:** in-memory `HashMap<SessionId, IpavState>`. Phases I/P use a
  1.5s buffered peer-forward; A/V is pure turn-based.

## Decisions made autonomously

These differ from `docs/decisions.md` recommendations; logged here for the
human reviewer:

1. **MCP transport: in-process HTTP, not stdio + UDS bridge.** The
   decision-doc sketch had Claude Code spawning our binary as an MCP child
   process and bridging back to the parent via UDS. That's two
   subprocesses per agent and ~150 LOC of IPC framing. Instead we run a
   single HTTP MCP server in the parent process and write per-agent
   `mcp-config.json` files pointing at
   `http://127.0.0.1:<port>/sessions/<id>/<agent>/mcp`. Direct AppState
   access, no IPC layer. Decision-doc itself flagged this as the
   "promote-if-IPC-gets-hairy" alternative.
2. **MCP server: hand-rolled, not `rmcp`.** `rmcp` 1.7.0 compiles fine but
   the macro surface kept adding indirection for what's ~200 lines of
   JSON-RPC dispatch. The hand-rolled version sits in
   `src/signaling/{jsonrpc,server,protocol}.rs` and only handles
   `initialize`, `tools/list`, `tools/call`, `ping`. Drop-in replacement
   with `rmcp` later is straightforward if we add more tools.
3. **`claude --append-system-prompt` is a string, not a file.** The PLAN
   originally said `--append-system-prompt-file` but the CLI only accepts
   inline `--append-system-prompt <prompt>`. We read the CL slot files in
   `src/core/session.rs::read_system_prompt` and pass the concatenated
   text inline. Total system prompt size is bounded (~3-5kB) so command-
   line length isn't a concern.
4. **`--verbose` is required with `-p --output-format stream-json`.**
   Empirically discovered in Phase 0.5 — without it the CLI errors out
   with `When using --print, --output-format=stream-json requires
   --verbose`. The spawn command in `src/agents/spawn.rs` includes
   `--verbose` accordingly.
5. **`rustup update stable` was run during build.** Cargo's lockfile
   resolved deps requiring `edition2024` (Rust ≥ 1.85). The user's
   installed toolchain was 1.84.1. `rustup update stable` is a user-scoped
   update (no system install) so we ran it — bumped to 1.95.0. Documented
   in this README so future maintainers know the MSRV came from Slint.

## Security caveats (v1)

- **Plaintext auth tokens.** `agent_configs.auth_token` is stored as
  plaintext sqlite. Any backup of `~/.bot-hq/` (Time Machine, cloud sync,
  rsync) captures these. The DB file is at
  `<data_dir>/.local/bot-hq.db` and gets default user-only mode bits.
  v2 will move to the OS keychain (`keyring-core`) — see
  `docs/decisions.md#auth-storage` for migration plan.

## Testing

```bash
cargo test                          # 56 tests, all non-live
RUN_LIVE_TESTS=1 cargo test         # includes claude-code subprocess smoke
cargo build --release               # production binary
```

## What's NOT done yet

- **Manual end-to-end smoke** (Phase 9.1). The build runs, the UI renders,
  but driving a real session through Investigate→Plan→Apply→Verify with
  live claude-code subprocesses on a screen-attached machine is the human
  step.
- **UX polish** (Phase 9.2): keyboard shortcuts, scroll-to-bottom,
  empty-state copy.
- **Discord + Clive plugins** (Phases 10–11). Deferred per PLAN.md.
