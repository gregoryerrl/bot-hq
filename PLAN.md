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

The current arc is the **native agent loop** (2026-07-26/27): an agent
can run on bot-hq's own Rust loop instead of a claude-code subprocess,
opted into per saved model, EYES-only in v1. It shipped along with the
audit remediation that followed it — see PROGRESS.md for the arc and
"Native agent loop — open items" below for what's left, chiefly **B6
auto-compaction**.

Before that, the arc was duo-reliability + UX: the interrupt redesign
(stdin `control_request` cancel + `SessionActivity`), the peer-forward
router extraction (`core/router.rs`), `peer_ack` / `halt`, the
EYES-sign-off commit gate, agent-health dots, and the event-driven
UI-freshness work all landed (see PROGRESS.md). Remaining follow-ups:

- ~~Live plugin *execution*~~ — SHIPPED 2026-07-04 as the **plugin
  runtime v1** (serving + catalog proxy + PluginHost + consent +
  hello-plugin; see PROGRESS.md and `docs/PLUGINS.md`). Follow-up tiers
  now live under "Plugin runtime tiers" below.
- Replace the placeholder `icons/icon.png` with the real bot-hq mark.
- Host-mediated reroute: option (a) (centralize-only) shipped as
  `core/router.rs` (2026-06-26); the explicit-handoff (b) / hybrid (c)
  forward-policy variants remain open ideas only.

The Context Library editor write-back + folder-view + right-click disk ops
shipped 2026-05-29, and the native folder picker shipped 2026-06-16
(`71fab9a`). Still deferred from that work: rename re-derives the folder
description, hard delete (no OS trash).

**Context Library v2** (arc started 2026-06-27; brief in the project CL's
`ideas.md`, assessment at
`docs/plans/2026-06-27-context-library-v2-assessment.md`). Shipped: FTS5
atomization + `cl_retrieve` ranked retrieval, retrieval-time ⚠
stale-flagging (`code_hash`), retrieval telemetry + Measurement tab, and
the `bench/cl_poison/` obey-vs-verify eval (authored, not yet run — live
trials cost model calls). The arc's human-review queue was REMOVED
2026-07-21 (approvals were rubber-stamped in practice) in favor of
direct agent writes via `cl_write_file`. Deferred remainder,
roughly in value order:

- **§9 lifecycle / decay / pruning** — measurement made the store's ~52%
  ephemera visible (handoff + ideas atoms, no decay). Wants: staleness
  surfacing (e.g. an atom whose cited file was DELETED currently un-flags
  after the next rescan re-baselines `code_hash`), TTL/archival for
  handoffs.
- **Retrieval quality:** a real kind/pin boost (today the
  convention/decision pin only fires on exact-BM25 ties — a near no-op),
  kind-specific freshness, embeddings/hybrid scoring (deliberately
  deferred; FTS5-first).
- **Measurement follow-ups:** escape-hatch rate (whole-file CL Reads vs
  `cl_retrieve`), `used_atoms` (precision proxy), a refresh source for
  the Measurement tab (agent retrievals emit no frontend event),
  poison-eval preflight that verifies the poison is actually indexed.
- **Consolidation (audit 2026-07-02, P3):** shared path-guard +
  atomic-write helpers (`tauri_cmd/cl.rs` vs `bridge/cl_write.rs`
  duplicates), one sha256-hex util, per-file hash memoization
  in the stale recompute, extract MeasurementView from
  `ContextLibraryEditor.tsx`.

---

## Backlog

### spawn_session `task` summary field (catalog extension, deferred 2026-07-07)

Optional `task` string on the spawn_session args that the per-spawn
confirm dialog would render as a highlighted summary line (empty/omitted
→ "(no task summary provided)"). Deferred from the spawn-dialog
hardening pass: the api_version-1 arg surface stays frozen, SDK +
plugin changes are needed to benefit, and a plugin-authored summary can
itself mislead — the dialog highlighting what the plugin CLAIMS while
the real risk is the prompt tail. Revisit with api_version 2 alongside
other catalog extensions.

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

### Plugin runtime tiers (post-v1 extension points)

The v1 runtime (2026-07-04) covers panel plugins + read-first catalog
RPC. Deferred tiers, roughly in value order (all documented as
extension points in `docs/PLUGINS.md`):

- **Host-event relay** — push `agent.messages.batch` / session events
  into subscribed iframes (grant-gated) so decks like Cognotify don't
  poll.
- **Plugin-contributed MCP tools** (agent↔plugin) — prerequisite for an
  agent-drivable Browser tab.
- **Manifest-declared agents** — the "add an agent to sessions" tier;
  interim lever is the external MCP driver server (a backend-style
  plugin is an ordinary process driving sessions over it).
- **Child-webview surface** — real Browser tab (arbitrary sites refuse
  iframing).
- **Background execution** — daemon-style plugins (CL cloud sync);
  today plugins run while mounted.
- **Zip/signed URL installs** — URL install is manifest+entry only;
  multi-file bundles need local-dir install.
- **Per-plugin CSP overrides; inline `slot_name` slots** — reserved.

### First plugins (each needs its own design doc)

- **Cognotify** — the human-comprehension deck (user's flagship idea):
  panel plugin over sessions + CL with user-tuned lenses (spatial graph
  / spec sheet / narrative brief). Buildable on v1 today (catalog reads
  + KV for lens prefs); wants the event-relay tier for liveness.
- **Discord plugin** — bridge sessions to/from a Discord channel.
  Probably a backend-style plugin on the external MCP driver.
- **Clive plugin** — port of legacy bot-hq's Clive bot (Twitch/IRC).
- **CL cloud sync** — `library/` is the sync boundary (see shipping.md
  hook); wants the background-execution tier.
- **GitHub tab** — panel plugin; OAuth via system browser.

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
- **Persistent IPAV phase log.** Phase transitions ARE already persisted
  — `advance_phase` writes a `messages` row with `kind='phase_change'`
  (`core/state.rs`), so the per-session phase history survives in storage.
  What's missing is a dedicated *queryable view* / retrospective surface
  (which phases consumed the most time); the data layer already exists.
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
  unaffected (substring Tool Gate + PreToolUse hook). **Moot on the native
  loop** (2026-07-27): a native EYES has no shell at all — commands are
  validated against an allow-list and executed as argv, and shell
  metacharacters are refused outright. This only remains open for a
  CLI-backed EYES.

---

## Native agent loop — open items

The loop shipped 2026-07-26/27 (see PROGRESS.md). What's left:

- **B6 — overflow handling (auto-compaction or otherwise).** The largest
  open item. claude-code auto-compacts silently; the native loop does
  nothing yet — it reports occupancy every turn, says so once past 85%,
  and keeps working. Past 100% the gateway decides: DeepSeek truncates
  rather than erroring, so the agent quietly loses its oldest turns.

  **This is a deliberate interim, decided 2026-07-27.** An earlier
  version hard-stopped at 85%, which on a 1M window discarded 150K
  tokens of usable capacity and ended the session for the user rather
  than by them. The current stance is that losing old context is a real
  cost but the *user's* to weigh against finishing the work — so bot-hq
  shows the number and they choose. What replaces this (compaction,
  summarisation, selective eviction) is open and intended to be designed
  deliberately rather than defaulted into.

  Design input already exists: `<data_dir>/.local/native-accounting.jsonl`
  records tokens occupied, conversation length and stop reason for every
  native turn, append-only and unrotated precisely so the long-horizon
  growth curve survives. Measure before designing — the question is what
  to drop and when, and the answer depends on how a real session actually
  fills.

- **No native HANDS.** `AgentRole::Hands.may_run_native()` is `false`:
  Brian's subscription binds server-side to claude-code, and he depends
  on skills, plugins and the full built-in tool surface the native path
  doesn't implement. If that ever changes, `BRIAN_ROLE` in `prompts.rs`
  becomes wrong (it promises `terminal_exec`, the visible PTY, ordinary
  `Bash`) with no test failing — the dependency is only recorded here.

- **`search_files` caps at 500 files.** Enumeration asks git, which is
  the right set, but a repo with more than 500 tracked files still
  truncates. The cap is *visible* on every outcome now, including "no
  matches", so it reads as incomplete rather than as absence — but it is
  not raised.

- **`user_mcp_servers_for_agent` sits outside `AgentRole`.** It is a
  capability question by the same logic as the four that were collapsed;
  left alone because the role mapping would be behaviour-identical there
  and it is on the CLI spawn path.

- **No live-run coverage of the audit fixes.** Every remediation batch
  was verified at unit level plus real-tree measurement; none of it has
  been exercised by an actual native session. The project's own history
  says live runs find what unit tests structurally cannot — three times
  out of three during the build.

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
