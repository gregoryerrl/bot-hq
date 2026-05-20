# bot-hq — Change Log

Recent work, newest-first. For the rebuild-era phased status (Phases
A–9 of the from-scratch rebuild), see
[`docs/rebuild-archive/PROGRESS-through-2026-05-15.md`](docs/rebuild-archive/PROGRESS-through-2026-05-15.md).

For what bot-hq IS see [`ARCHITECTURE.md`](ARCHITECTURE.md). For what's
planned next see [`PLAN.md`](PLAN.md).

---

## Current state

165 tests passing (121 lib + 29 signaling + 5 external MCP + 10
storage). Release build clean. The session-permission grant subsystem
is in flight (functional, uncommitted in working tree).

---

## 2026-05-20 — Session permission grants (in flight)

New module `src/policy/session_permissions.rs` plus integration across
the bridge, the duo, the spawn path, and the `pre-push` git hook.

**What changed:**
- `SessionPermissions { commit: GrantScope, push: GrantScope }` with
  `None` / `AllBranches` / `Specific { branches }` scopes.
- In-memory cache on `SignalingBridge` is the source of truth; mirrored
  to `<data_dir>/.local/session-permissions/<session_id>.json` so the
  `pre-push` git hook (separate subprocess) can read it.
- All mirror files purged on bot-hq startup; per-session file deleted
  on `close_session`.
- MCP tools added: `grant_session_permission(action, scope, branches?)`,
  `revoke_session_permission(action)`, `list_session_permissions()`.
  HANDS-only — Rain (EYES) cannot call them.
- `pre-push` hook checks the mirror before the static
  `policy.push_gate.remembered_approvals` list.

**Documentation cross-refs:** ARCHITECTURE.md → Session permissions
section; README.md → Internal MCP tools + Policy enforcement.

---

## 2026-05-19..05-20 — Doc refresh

Full rewrite of canonical docs (README, ARCHITECTURE, PLAN, PROGRESS,
CLAUDE) to reflect the post-rebuild state. Original rebuild design +
roadmap + Phase 0 research archived under `docs/rebuild-archive/`.

---

## 2026-05-15 — UI redesign

Substantive frontend pass triggered by user feedback ("the UI is really
bad").

- **Single chronological chat.** Replaced the two-pane Brian/Rain split
  with one chronological column where all messages interleave by
  `created_at`. User can now see their own messages clearly.
- **Design system.** Slint `Theme` global owns colors, typography,
  spacing, radii. 4-tier background hierarchy
  (canvas → surface → elevated → overlay), 4-step font scale, 4px-base
  spacing scale. Author color coding: brian=orange, rain=purple,
  emma=green, user=blue, system=muted grey.
- **Per-surface polish.** Topbar gains brand mark + tab underline +
  Emma button distinct treatment. Dashboard title block + primary
  `+ New session` CTA + elevated session tiles with `Need input` badge
  tinting border red. Session view: rich header (title + phase subtitle
  + back link + interactive PhaseSelector segmented control); banner
  uses author-rain purple (choice) vs attention red (awaiting). Emma
  overlay: dedicated header bar + close affordance + divider.
- **CL refresh.** New files in CL appear without app restart via a 2s
  periodic poll plus a manual ↻ refresh button. Directories sort before
  files in the tree.

Files touched: `ui/app.slint` (full rewrite: 796 → 1410 lines),
`src/ui/view_model.rs` (714 → 743 lines). Tests still 56-passing at the
time, release build clean.

---

## 2026-05-15 — Post-review fixes

Follow-ups after the autonomous rebuild's READY-FOR-REVIEW state.

1. Added `/target/` to `.gitignore` (was 6.7 GB; `git add .` without
   this would have committed all build artifacts).
2. **Emma auto-spawn at startup.** Extracted `spawn_session_handle`
   helper in `src/core/session.rs`; added `spawn_existing_session` for
   sessions whose row already exists. `AppState::ensure_session_started`
   is idempotent and called for `"emma"` in `main.rs` post-core
   construction. Failure is non-fatal.
3. **Settings save persists user edits.** Replaced inline LineEdit-in-
   for-loop pattern with `AgentConfigEditor` component owning per-row
   edit state via `in-out` properties bound via `text <=>`.
4. **Per-project rules migration from legacy CL.** Distilled
   operational rules from `~/.bot-hq/projects/<project>.yaml` into the
   new minimal CL. Per-project policy gates + disguise rules captured.

---

## 2026-05-14 — Rebuild milestone (v0.1.0)

From-scratch rebuild of bot-hq landed: single Rust + Slint binary
replacing the Go daemon + tmux + MCP hub + Emma forwarder + 29-tool
surface. Built autonomously across multiple claude-code sessions per
the original rebuild plan.

**Result:** 56 tests passing, release build clean, binary launches and
runs the UI loop cleanly, full agent lifecycle implemented (subprocess
spawn, stream-json IO, sqlite storage, internal MCP server with 2
initial tools, IPAV duo coordination, Slint UI with topbar +
dashboard + session view + Emma overlay).

**Phase A complete:** rebuilt minimal CL distilled into `~/.bot-hq-dev/`
(60K, 398 lines across general-rules + 3 agent startups + 5 projects of
conventions + notes). Replaces the 860-file legacy CL.

For full phase-by-phase progress + Phase 0 research findings + sub-
agent dispatch log + initial decisions, see
[`docs/rebuild-archive/PROGRESS-through-2026-05-15.md`](docs/rebuild-archive/PROGRESS-through-2026-05-15.md).

---

## Decisions made autonomously (across the build)

Things that diverged from the original PLAN / decision-doc and shipped
that way. Captured for future reference.

1. **MCP transport: in-process HTTP, not stdio + UDS bridge.** Original
   design sketched claude-code spawning bot-hq as an MCP child process
   and bridging back via Unix-domain socket. That's two subprocesses
   per agent + ~150 LOC of IPC framing. Ship version runs a single
   in-process HTTP MCP server, per-agent `mcp-config.json` files
   pointing at unique URLs. Direct AppState access, no IPC layer.
2. **MCP server: hand-rolled JSON-RPC, not `rmcp` crate.** Phase 0
   research recommended `rmcp` 1.7.0; orchestrator chose hand-roll
   (~300 LOC at `src/signaling/{jsonrpc,server,protocol}.rs`) for
   simpler in-process transport. Drop-in `rmcp` upgrade later is
   straightforward.
3. **`claude --append-system-prompt` is a string, not a file.** Plan
   said `--append-system-prompt-file`; CLI only accepts inline
   `--append-system-prompt <prompt>`. Concatenated text passed inline.
4. **`--verbose` required with `-p --output-format stream-json`.**
   Empirically discovered. Spawn command includes it.
5. **`--dangerously-skip-permissions` set on agent spawn.** bot-hq IS
   the policy layer; claude-code's own permission prompts would
   double-gate and hang. Enforcement provided by `src/policy/` + git
   hooks.
6. **Role prompts hardcoded in `src/agents/prompts.rs`.** Not CL-loaded.
   Reasoning: role boundary (Brian writes, Rain reviews) is structural
   and must survive CL edits + custom-instruction changes.
7. **System-prompt layering with policy block at the end.** Session
   spawn concatenates: hardcoded role → CL anchor → general-rules →
   custom-instruction → policy directives. Project conventions/notes
   NOT injected — agents use `cl_index_search` + `Read` on-demand.
8. **`HANDS_ONLY_TOOLS` enforced at the JSON-RPC dispatch layer.** Rain
   is structurally blocked from `ask_user_choice`, `mark_awaiting_user`,
   `request_approval`, `grant_session_permission`,
   `revoke_session_permission`. Returns a JSON-RPC error, not a
   convention.
9. **Two-layer policy enforcement.** MCP tool calls (probabilistic
   primary path, audited via `violations.jsonl`) PLUS git hooks
   (deterministic backstop). Per DeepSeek-V4-Pro's review during the
   policy module work — single-layer enforcement would fail when
   agents' context drifted.
10. **`BOT_HQ_SESSION_ID` env var injected into agent subprocesses** so
    git-hook subprocesses (spawned by git, separate from the agent's
    subprocess) can re-resolve session-scoped state (session
    permissions in particular).
11. **External MCP token at `<data_dir>/mcp-token`** auto-generated
    (UUIDv4, 0600). Constant-time comparison via `subtle` crate. Read
    once at startup, never re-read — rotation requires restart.
12. **Slint pin: `slint = "1.16"`** (resolves to 1.16.1). MSRV 1.92 per
    Phase 0.3 research.
13. **Sessions/agent_configs tables have CHECK constraints on
    `agent_name` ∈ `{'emma','brian','rain'}`** so a typo from Settings
    UI doesn't silently create a bogus row.
14. **First-run detection key: `cl-version.txt` existence** — not data-
    dir existence (test setup creates the data-dir before binary touches
    it).

---

## How to verify the human-driven parts

```bash
cd ~/Projects/bot-hq
cp .env.example .env             # already contains BOT_HQ_DATA_DIR=~/.bot-hq-dev/
cargo run --release

# In the window:
#   - Click "+ New session" on the Dashboard.
#   - Type a small task in the broadcast prompt bar.
#   - Watch Brian + Rain stream. Click the I/P/A/V chips to advance phase.
#   - If an agent calls ask_user_choice, choice buttons should appear inline.
#   - Toggle the Emma button (top-right) — half-pane chat slides in.

# Richer logs:
#   RUST_LOG=trace cargo run --release
```
