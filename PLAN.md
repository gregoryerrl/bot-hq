# bot-hq — Forward Plan

What's next for bot-hq. The original rebuild roadmap (Phases A–9 of the
from-scratch rebuild) shipped — that document is preserved at
[`docs/rebuild-archive/PLAN-rebuild-era.md`](docs/rebuild-archive/PLAN-rebuild-era.md).

For what bot-hq is right now see [`ARCHITECTURE.md`](ARCHITECTURE.md).
For recent changes see [`PROGRESS.md`](PROGRESS.md).

---

## Current state (TL;DR)

bot-hq is built and used. The rebuild milestone (v0.1.0) shipped and the
**Tauri v2 migration landed 2026-05-26** on branch `tauri-v2-migration`
(7 batches; see PROGRESS.md). React frontend in `frontend/`, Slint
deleted, Rust core untouched. Since then a long arc shipped (see
PROGRESS.md): the 3-tier session-policy toggles, the global Tool Gate, the
saved-model registry + per-session model pickers + solo-Brian toggle, the
claude-code config surface, the **v1.0.0 stabilization pass** (per-session
git worktrees, dispatch defaults, prompt drafts, UX polish — 2026-06-11),
and the **post-1.0 duo-reliability arc**: the EYES-sign-off commit gate,
the interrupt redesign, the peer-forward router extraction,
`peer_ack`/`halt`, agent-health dots, and event-driven UI freshness.

Test + build status (live counts) lives in PROGRESS.md, not here — it
drifts every commit.

---

## In flight

Past the Tauri v2 migration and the v1.0.0 stabilization pass, the active
arc has been duo-reliability + UX: the interrupt redesign (stdin
`control_request` cancel + `SessionActivity`), the peer-forward router
extraction (`core/router.rs`), `peer_ack` / `halt`, the EYES-sign-off
commit gate, agent-health dots, and the event-driven UI-freshness work all
landed (see PROGRESS.md). Remaining follow-ups:

- Live plugin *execution*: the per-plugin iframes at
  `https://plugin-<id>.localhost` + their ping/pong channel. The
  management UI (install / enable / disable / uninstall) and the
  heartbeat-driven crash indicator already shipped in `PluginManager.tsx`;
  what remains is rendering + driving the plugin iframes themselves
  (the frontend `PluginSlot.tsx` component was removed as dead code and
  needs rebuilding for this; the Rust `PluginSlot` manifest type stays).
- Replace the placeholder `icons/icon.png` with the real bot-hq mark.
- Host-mediated reroute: option (a) (centralize-only) shipped as
  `core/router.rs` (2026-06-26); the explicit-handoff (b) / hybrid (c)
  forward-policy variants remain open ideas only.

The Context Library editor write-back + folder-view + right-click disk ops
shipped 2026-05-29, and the native folder picker shipped 2026-06-16
(`71fab9a`). Still deferred from that work: rename re-derives the folder
description, hard delete (no OS trash).

---

## Backlog

### UX polish (deferred from rebuild Phase 9.2)

Shipped 2026-06-11 in the v1.0.0 stabilization pass: keyboard shortcuts
(Cmd-N / Cmd-,), tile sort by last activity, welcoming Dashboard
empty-state, inline session rename, persistent prompt drafts.
(Scroll-to-bottom had already shipped.) Remaining:

- Responsive Brian/Rain vertical stack at content widths < 1200px (the
  single-chronological-chat redesign mooted this, but keep the option
  on the table if the two-pane view is requested back).

### Auth-token v2 — OS keychain

Migrate `agent_configs.auth_token` from plaintext sqlite to OS keychain
via `keyring-core`. Per-platform backends: macOS Keychain Services,
Windows Credential Manager, Linux Secret Service (dbus).

**Migration logic** (runs once, gated by a `schema_version` row):
1. Read each non-NULL `auth_token` from `agent_configs`.
2. For each, `Entry::set_password` under
   `("bot-hq", format!("{project}:{agent}:{provider}"))`.
3. NULL the column.
4. Bump `schema_version`.

Fall back to plaintext-sqlite mode with a startup warning on keychain
failure (headless CI, Linux without Secret Service daemon).

Original Phase 0 research: [`docs/rebuild-archive/decisions.md`](docs/rebuild-archive/decisions.md#auth-storage).

### Sub-agent dispatcher integration

Brian can already use the `Agent` tool to dispatch sub-agents within
claude-code. Worth wiring a visualization so the UI knows which session
spawned a sub-agent — currently sub-agents are invisible to bot-hq.
Open question: do we surface them as nested message threads, or as
phantom sessions on the dashboard?

---

## Deferred (separate plans)

### Discord plugin

Bridge bot-hq sessions to/from a Discord channel. Original scope from
the rebuild plan. Contract TBD: probably a per-channel session, with
message-author mapping (Discord user → bot-hq user / agent author).
Needs its own design doc before implementation.

### Clive plugin

Port of legacy bot-hq's Clive bot (Twitch/IRC integration). Needs its
own design doc.

### Cross-platform builds

Tauri covers macOS, Linux, Windows. Initial focus is macOS. Linux + Windows
builds need: per-platform CI, install paths (`directories` crate likely
already handles this), keychain backends (see auth-token v2), font
loading for the icon font.

---

## Architectural ideas (no commit yet)

- **Move CL writes to a transaction model.** Partially shipped: CL writes
  are now atomic (adjacent temp file → rename, `a040c08`), which hardens
  against partial-write failures. What remains is folding the write + the
  index update into one sqlite transaction so they can't diverge.
- **Hot policy reload.** Today the policy block in an agent's system
  prompt is fixed at session spawn. Editing `policy.yaml` mid-session
  requires session restart for the agent to see new rules (though hooks
  + MCP tools always re-resolve on call). Consider a "policy reload"
  banner that re-spawns the duo.
- **Persistent IPAV phase log.** IPAV is in-memory only. Phase
  transitions are visible in chat as synthetic user messages, but the
  per-session phase history isn't queryable. Worth keeping for
  retrospectives (which phases consumed the most time).
- **Tray garbage collection.** ✅ Shipped — `purge_resolved_tray(90)` runs
  a boot-time sweep of resolved tray rows older than 90 days
  (`5d8d9f2`, `storage/tray.rs` + the main.rs boot sweep), keeping
  `session_tray` bounded.
- **Tighten CL ↔ agent stitching further** (deferred from the 2026-06-08
  pass — context window = cache, session-docs = RAM, CL = disk). F-A
  (gate phase-tagged `session_doc_write` to HANDS) + F-B (spawn-time CL
  index primer) shipped; what remains is the "memory-controller" layer
  the analogy wants:
  - *Model-agnostic adherence:* a push/interrupt layer (MemGPT-style
    memory-pressure reminders at decision points) so a weaker
    non-Anthropic model doesn't rely purely on prompt instruction-
    following to page CL / session-docs in and out.
  - *Write-then-prune close-loop safety net:* nothing catches a HANDS
    agent that forgets the bounded learnings delta before
    `close_session`.
  - *Rain CL write path:* EYES has no CL write at all (by design today);
    revisit only if review-time annotations prove valuable.
  - *`cl_register_read` feedback view:* the read-audit rows are written
    but the "what context did this agent have?" view was never built.
- **EYES compound-`&&` read Bash — git-branch cause RESOLVED 2026-06-17 (`e375828`).**
  The observed denials were content-based, not pure-`&&`: the blanket
  `Bash(git branch:*)` deny matched the git-branch segment of compound reads like
  `git branch --show-current && echo …`, taking the whole compound down. Replaced
  it with deny-by-write-verb (read git-branch forms now fall through), so those
  reads pass. If any pure-`&&` denial independent of a denied segment remains,
  that's a separate claude-code matcher question — untested (needs a live
  non-Anthropic EYES session to confirm); not a known bot-hq gate bug. HANDS is
  unaffected (substring Tool Gate + PreToolUse hook).

---

## Out of scope

- **Web UI.** bot-hq is desktop-only by design. The external MCP
  driver server already enables programmatic access; a web frontend
  would be a separate product.
- **Multi-user / multi-tenant.** Single-developer-workstation is the
  design target. Shared workstations are out of scope (auth-token
  threat model assumes single user).
- **Migration of legacy bot-hq runtime state.** Sessions / hub history
  / last-state files from the Go/tmux/MCP-hub bot-hq do NOT carry over.
  Project CL was distilled once at rebuild time; further sync is
  manual.
