Rebuild from Go/tmux to Rust/Slint

Replaces the Go-daemon + tmux + MCP-hub + Emma-forwarder + 29-MCP-tool
architecture with a single-binary Rust + Slint desktop GUI where the user
is the conductor between agents.

See ARCHITECTURE.md for the decision record and PLAN.md for the phased
plan. PROGRESS.md tracks what's verified vs. caveated as of this commit.

Highlights:

- Single-binary Rust app. Stack: slint 1.16.1, tokio multi-thread, sqlx
  sqlite, hyper 1.x for the in-process MCP HTTP server.
- Per-agent claude-code subprocess via `claude -p --input-format
  stream-json --output-format stream-json --verbose --append-system-prompt
  <text> --mcp-config <file>`. Model swap per agent via
  ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN / ANTHROPIC_MODEL.
- In-process MCP HTTP server with two UI-signaling tools:
  `ask_user_choice(question, options)` (blocking) and
  `mark_awaiting_user(reason)` (non-blocking). Hand-rolled JSON-RPC over
  hyper 1.x — see Decisions Made Autonomously in PROGRESS.md.
- sqlite schema: `sessions`, `messages`, `agent_configs`. Emma is a
  singleton session row seeded by the migration.
- IPAV in-memory cache + duo coordination: I/P phases use a 1.5s buffered
  peer-forward window; A/V is pure turn-based. Tool-use events never
  forward to peer.
- Slint UI: Dashboard / Context Library / Settings + Emma half-pane
  overlay. Single-column session view (Brian/Rain/user/phase messages
  interleaved chronologically), interactive IPAV phase selector,
  broadcast prompt bar, choice-button rendering. Coherent design system
  with token-driven palette, typography, and spacing.
- Phase A complete: rebuilt minimal CL distilled into ~/.bot-hq-dev/
  (60K, 398 lines across general-rules + per-agent startups + 5 projects
  of conventions/notes). Replaces the 860-file legacy CL.

Existing Go files in this tree appear as deletions in the commit. They
are intentionally removed alongside the new Rust code — current bot-hq
keeps running from its own install location until decommissioned.

What's tested in CI (cargo test):
- 42 lib unit tests (protocol parse/serialize, storage round-trips, paths
  + lockfile, IPAV state, signaling bridge + jsonrpc dispatch).
- 5 HTTP integration tests for the in-process MCP server.
- 9 storage integration tests.
All 56 tests pass on this commit. Release build clean.

What needs a human-driven smoke pass:
- Drive a real session through I→P→A→V with live claude-code subprocesses
  on a screen-attached machine. The Slint window opens and the runtime
  comes up cleanly (verified — see PROGRESS.md), but live agent dialog
  needs an authed CLI + display.
- UX polish (keyboard shortcuts, scroll-to-bottom) deferred to a
  follow-up.

This commit is the autonomous build milestone. Next steps documented in
PROGRESS.md "Phase 9".
