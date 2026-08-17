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
saved-model registry + per-session model pickers, the
claude-code config surface, the **v1.0.0 stabilization pass** (per-session
git worktrees, dispatch defaults, prompt drafts, UX polish — 2026-06-11),
the **post-1.0 reliability arc** (the reviewer sign-off commit gate, the
interrupt redesign, `peer_ack`/`halt`, agent-health dots, event-driven UI
freshness), and — most recently — **rc3, which is now essentially complete**:
roles as user-owned data with editable prose, capability enforcement wired to
the runtime, participants identified by role rather than by name, N-participant
sessions, the turn ring as the only turn engine, and the bilateral router
deleted. Decisions D1–D37 are recorded in
[`docs/plans/2026-08-11-rc3-decisions.md`](docs/plans/2026-08-11-rc3-decisions.md).

Test + build status (live counts) lives in PROGRESS.md, not here — it
drifts every commit.

---

## Direction — the agent harness is the focus (stated 2026-08-05, unscheduled)

bot-hq is an **agent harness/system** — the gates, the transparency surfaces
(tray / IPAV docs / terminal), and the memory hierarchy — and that, not any
particular agent lineup, is the core's identity.

**rc3 delivered most of this direction ahead of the plugin migration.** Roles are
user-owned rows the user creates and edits, a session picks N participants from
them, and nothing in the runtime is keyed on a role existing: the tally filters
on `participation_mode`, the gate reads capabilities, the prompt generates its
peer section from the live roster. A reviewer is now a role the user configures
rather than a hardwired co-agent — which was the substance of "EYES becomes a
plugin only". What remains for an actual plugin migration is packaging and the
plugin runtime tiers, not core surgery.

Vision wording: project CL `vision.md` ("The harness, not the crew"); decision
record: CL `decisions.md` (2026-08-05) and rc3 D1–D37.

**Explicitly DEFERRED behind this migration** (user call, 2026-08-05,
after the Aug-5 session study — CL `issues.md` #26–#31): the
pump-shaped fixes ride the teardown instead of patching it —
`#26` held-forward flush audit (MOOT since the router's deletion on
2026-08-13: there is no hold queue to flush — see the router behaviour
inventory), `#30` duplicate spawn warmup, and the peer-wait watchdog
classification (the false
NEEDS DIRECTION on review handoffs, #25-family). **Not deferred:**
`#27` (tray answers don't preempt a running turn) is user↔agent
plumbing that survives any composition — it stays an open standalone
fix, along with `#29` (gate-refusal UX), `#31` (close-out staleness
sweep), and `#28` (auto-answerable gate classes).

---

## Next deliverables (planned 2026-08-05, after the Aug-5 session study)

Two tracks, ordered within each by recommended sequence. Full evidence
for every item: CL `issues.md` #26–#31 + the archived s-b69a5c01
session docs (investigate/apply/verify) + CL `session-study-method.md`.
Read those before starting; this section is the map, not the territory.

**Track 1 — harness fixes (standalone; none touch the pump):**

1. **#27 — tray answers preempt a running turn** — shipped 2026-08-06
   (`d71c4d1`) and **REVERSED by rc3 D34** (`7e1e04d`). The preempt interrupt is
   deleted on purpose: it aborted the holder's whole in-flight turn, which made
   a tray click a hidden interrupt when Pause is meant to be the only one
   (`core/state.rs`, "The `deliver` flag … is gone, and that is rc3 D34"). This
   entry read as shipped-and-standing until the audit's T pass; the issue is
   open again in the sense that the exposure it named is now bounded by the
   remainder of the current turn rather than cured.
2. ~~**#29 — gate refusals**~~ — SHIPPED 2026-08-06 (`c0a66b7`), with the
   issue's premise corrected by measurement: no same-command retry loop
   exists (the "19 refusals" were 19 `tool_blocklist` rows = 18 approved
   + 1 denied; real refusals were 5 across Aug 4–5). The measured failure
   is **reword-evasion** (2/5) plus a `ToolSearch` round-trip on every
   correct conversion (#14). The exact-call text was already shipped, so
   the fix forbids rewriting around the gate. The **auto-park half also
   shipped** 2026-08-06 (`19ec620`, user-picked): the hook POSTs the new
   `/hooks/tool-gate` route and the refusal becomes "already queued as
   gate_id X". The route calls `park_gated_command`, NOT `action_gate` —
   the latter re-resolves keywords and executes on `auto_allow`/no-match,
   so wiring it there could run a command unapproved whenever its resolve
   disagreed with the hook's. Every failure degrades to the old wording;
   exit stays 2.
3. ~~**#31 — close-out staleness sweep**~~ — SHIPPED 2026-08-06
   (`525d452`). Seed list comes from the old→new body diff `cl_write_file`
   already reads, not from decisions.md appends or agent self-declaration;
   sweep runs at `close_session` before/independently of the learnings
   nudge, advisory and once-only.
4. **#28 — auto-answerable gate classes** (medium; design first).
   Extend the `push_gate=auto` pattern: per-class auto-approval
   (read-only gated commands first), same-class batch approve in the
   tray. Target: engaged-session checkpoint churn (27 asks @ 7.7 min
   avg in one session).

**Track 2 — CL mechanisms (closing the vision gaps in `vision.md`'s
"reading notes" + "memory hierarchy" bullets):**

5. **Populate `retrieval_events.used_atoms`** (small-medium). The
   column is reserved and empty. Define "used" simply first: files
   Read/Edited by the same agent within the turns after retrieval.
   Unblocks every utility question below.
6. **Growth telemetry** (small). Per-project atoms/files/tokens over
   time + serve-rate joins (queries prototyped in
   `session-study-method.md`); render in the Context Manager tab
   beside the existing measurement card.
7. **Gardening surfacing** (medium). Mechanical prune candidates:
   never-served files, age+never-served, superseded drafts
   (`ideas.md`-class), dated learnings whose keepers were folded.
   Surface as a Library-tab list; the user prunes — no auto-delete.
8. **The cap — research item, explicitly open** (from vision.md: "we
   have to find that cap"). Candidate metrics to evaluate once 5–7
   exist: serve-precision decay, conflicting-atom rate (same topic,
   contradicting bodies), tokens-per-used-atom. No mechanism until the
   metric proves out.
9. **Retention-leak closers** (folds into the existing "Tighten CL ↔
   agent stitching" item below — same memory-controller arc): the
   write-then-prune close safety net; a periodic archived-session
   mining pass (method: CL `session-study-method.md`); three-store
   rule (CL canonical, auto-memory mirrors it, repo docs get pointers).

**Deferred behind the reviewer-plugin migration** (recorded above): #30
duplicate spawn warmup, peer-wait watchdog classification (#26 is moot —
router deleted).

---

## In flight

**Nothing blocking.** The **rc3** arc closed 2026-08-13 — decisions D1–D37 in
[`docs/plans/2026-08-11-rc3-decisions.md`](docs/plans/2026-08-11-rc3-decisions.md).
D17 (summon by `@mention`), D18 (two participation modes), D19/D19a/D19b (the
ring is the only delivery path) all shipped that day.

Also shipped that day: **D16** (`close_session` gates on the role's tick),
**D22** (a parked question finished the lap before halting — superseded by
**D35** on 2026-08-14: a halt stops the ring where it stands, and a question
no longer reaches the ring at all), **D23** (a
delivered row says who wrote it) and **D24** (a straggler cannot bind the next
turn's epoch).

Spec'd and **since SHIPPED** (both were still listed here as unstarted on
2026-08-16, which the rc3 audit's T pass caught):

- **D20, second half** — the user-set label overriding the ordinal, plus its
  editor in the New Session dialog: shipped `4e531c8` + migration
  `0053_participant_label.sql`.
- **D21** — the parallel BOOT phase, every participant orienting at once with
  nobody acting until the ring starts: shipped `584f06f` / `db7c3a6`.

Found by live sessions, not yet decided:

- ~~**The backlog is N stdin writes, not one.**~~ **FIXED `7060d97`** — a page is
  one write, so rows 2..N no longer land inside the turn row 1 opened.
- **The tail of the ring starves.** Every user message resets to the front, so at
  N=3 with an actively-sending user slot 2 gets roughly half the turns of slot 0 —
  2-vs-6 in `s-534b8761`, 2-vs-6 in `s-206e8921`.
- ~~**`sessions.round_number` has no writer**~~ **FIXED `1984e61`** — written
  beside the lap counter it measures. (`current_turn_participant_id`, the column
  it was compared to, is now also cleared on every halt — `92eeba5`.)

Also open: the proposals in the project CL's
`improvements-2026-08-12-visibility-and-verification.md` that were not taken —
P3 (the reviewer should REPRODUCE, not read), P5 (post-merge re-verification of
cross-branch claims) and P6 (the CL remote goes stale; a push path needs a
pre-push secret check). P8's pass volley is closed by D35 (a question parks
without touching the ring, a halt stops it at once) rather than by D22's
bounded lap.

The arc before it was the **native agent loop** (2026-07-26/27): an agent
could run on bot-hq's own Rust loop instead of a claude-code subprocess,
opted into per saved model, EYES-only. **rc3 D9 deleted it** (2026-08-12):
the claude CLI is the only model connector, and the native connector
returns later as a plugin built from `git show c7bba28:src/agents/native/`.
See "Native agent loop — CLOSED by rc3 D9" below for what closed with it.

Before that, the arc was duo-reliability + UX: the interrupt redesign
(stdin `control_request` cancel + `SessionActivity`), the peer-forward
router extraction (`core/router.rs`), `peer_ack` / `halt`, the
EYES-sign-off commit gate, agent-health dots, and the event-driven
UI-freshness work all landed (see PROGRESS.md). Remaining follow-ups:

- ~~Live plugin *execution*~~ — SHIPPED 2026-07-04 as the **plugin
  runtime v1** (serving + catalog proxy + PluginHost + consent +
  hello-plugin; see PROGRESS.md and `docs/PLUGINS.md`). Follow-up tiers
  now live under "Plugin runtime tiers" below.
- ~~Host-mediated reroute~~ — MOOT. Option (a) (centralize-only) shipped as
  `core/router.rs` in 2026-06-26 and that file was deleted by task 14
  (2026-08-13). The explicit-handoff (b) / hybrid (c) forward-policy variants
  were variants of a forward path the turn ring does not have: a participant
  reads the channel off its own cursor, so there is nothing to reroute.

The Context Library editor write-back + folder-view + right-click disk ops
shipped 2026-05-29, and the native folder picker shipped 2026-06-16
(`71fab9a`). Still deferred from that work: rename re-derives the folder
description, hard delete (no OS trash).

**Context Library v2** (arc started 2026-06-27; brief in the project CL's
`ideas.md`, assessment at
`docs/plans/2026-06-27-context-library-v2-assessment.md`). Shipped: FTS5
atomization + `cl_retrieve` ranked retrieval, retrieval-time ⚠
stale-flagging (`code_hash`), retrieval telemetry + Measurement tab, and
the `bench/cl_poison/` obey-vs-verify eval (authored, never run — live trials
cost model calls; DELETED with `bench/` in round 6, recoverable from git
history if the eval is wanted once a driver plugin exists). The arc's human-review queue was REMOVED
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

### Ring latency levers (raised 2026-08-14, all USER decisions)

The serialized ring is why sessions feel slow — measured in `s-f6a441ff`:
HANDS 10.3 min, EYES 5 min, HANDS 3 min, one holder at a time, 18.5 minutes
before the user got the floor. The candidate levers, none of which bot-hq
should pick unilaterally:

- **Turn-picker** — the user routes the next turn to a named participant
  instead of the ring running full laps (EYES takes a verify turn only when
  summoned). The user has mentioned wanting this; it is unspec'd — theirs to
  define before anything is built.
- **Per-role model/effort presets for review passes** — already expressible
  per participant at spawn; the lever is defaults/UX, not new machinery.
- **Solo sessions for non-adversarial tasks** — already possible; a
  one-participant roster runs without review laps.

### Park on external signal — RESOLVED by the halt model (2026-08-15)

Raised 2026-08-14 after `s-f6a441ff` burned pass-laps watching CI. Answered
by the user's state-model decree, not by new machinery: **every stop is a
HALT, and an external wait is a halt whose recap names the signal and the
wake time** ("waiting for the 03:15Z sweep — timer wakes me 03:42Z"). The
self-wake is the ghost mechanism claude-code already provides (a background
timer/watcher re-invokes its subprocess; it posts findings and re-declares a
fresher halt), and the ring stays frozen until the USER's message — release
is theirs by decree, so there is no auto-resuming park state to design.
Lived end-to-end in `s-d6352684` the same night. Nothing left to build.

### Release-autonomy gate profile (raised 2026-08-14, release-scoped)

The vision's line: a fully autonomous run requires the user to authorize it
in the first prompt AND dangerously open every gate. The owner will likely
never use it; released users might. Today "every gate" is a set of separate
toggles (`push_gate`, `action_gate`, per-action approvals, and now the
reviewer-down override gate) with no single profile that opens — or
audits — them together. Before release this wants: one explicit
"autonomous" profile the user opts into per session, a visible banner while
it is active, and the violations log recording that the profile (not the
agent) authorized each pass-through. Not scheduled; parked until release
planning.

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

- Responsive participant vertical stack at content widths < 1200px (the
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

A participant granted the `Agent` tool can already dispatch sub-agents within
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
  interim lever WAS the external MCP driver server, now removed (a backend-style
  plugin is an ordinary process driving sessions; that server was its transport).
- **Child-webview surface** — real Browser tab (arbitrary sites refuse
  iframing).
- **Background execution** — daemon-style plugins (CL cloud sync);
  today plugins run while mounted.
- **Zip/signed URL installs** — URL install is manifest+entry only;
  multi-file bundles need local-dir install.
- **Per-plugin CSP overrides; inline `slot_name` slots** — reserved.

### First plugins (each needs its own design doc)

- **Cognotify** — the human-comprehension deck (user's flagship idea).
  ALREADY BUILT as an external app at `~/Projects/cognotify` — this
  entry is about *integrating* it as a bot-hq plugin (panel over
  sessions + CL), not building it. (Corrected 2026-07-28; the stale
  "buildable on v1 today" wording misled a session into asserting it
  was unbuilt — the user had to correct the record.)
- **Discord plugin** — bridge sessions to/from a Discord channel.
  Probably a backend-style plugin (the external driver is now itself a planned plugin).
- **Clive plugin** — port of legacy bot-hq's Clive bot (Twitch/IRC).
- **CL cloud sync** — `library/` is the sync boundary (see shipping.md
  hook); wants the background-execution tier.
- **GitHub tab** — panel plugin; OAuth via system browser.

### Cross-platform builds

Tauri covers macOS, Linux, Windows, and `.github/workflows/release.yml`
builds all three (universal `.dmg`, `.deb` + AppImage, NSIS `.exe`) on tag
pushes. What remains is runtime, not build: the Windows PID lock and the
bash git hooks (see the workflow's own notes and `docs/WINDOWS-TESTING.md`),
keychain backends (auth-token v2), and signing (`docs/SIGNING.md`). The icon
font this line used to name was the Slint UI's; the web UI ships its own
fonts under `frontend/public/fonts`.

---

## Architectural ideas (no commit yet)

- **A guard that pins a doc against another DOC.** Raised in review 2026-08-17,
  after the third instance in one session of a claim outliving its truth: F8's
  never-executed merge instructions, a bench README describing a tool whose
  transport had been deleted, and ARCHITECTURE.md still describing the pre-vote
  two-server product.

  The shape of the gap is precise. Four guards already exist and **every one pins
  a doc against the CODE** — `codebase_map_test` → CODEBASE.md,
  `every_registered_tool_is_documented` → the tool table, `retired_identifier_test`
  → `src/` identifiers, `framing.ts` → user-visible strings. Only the tool-table
  test reads `ARCHITECTURE.md` and `README.md` (`include_str!`); nothing reads
  `CLAUDE.md` or `PLAN.md`.

  So CLAUDE.md and README can quote ARCHITECTURE's self-description indefinitely
  and no gate notices when it changes underneath them — which is exactly what
  happened: the driver sweep updated ARCHITECTURE because that is the file it was
  editing, and left the two files that quote it. The failure is not carelessness
  about docs; it is that **each fix updates the artifact in hand and not the one
  quoting it**, and no edge exists between them.

  Cheapest useful version: a test asserting that a small set of cross-quoted
  claims (the MCP-server count, the canonical-docs list, the storage-schema
  summary) appear identically wherever they are restated. Not attempted here —
  the arc that found it was already closing, and a guard written to catch one's
  own last mistake is the worst time to design it.

- **Move CL writes to a transaction model.** Partially shipped: CL writes
  are now atomic (adjacent temp file → rename, `a040c08`), which hardens
  against partial-write failures. What remains is folding the write + the
  index update into one sqlite transaction so they can't diverge.
- **Hot policy reload.** Today the policy block in an agent's system
  prompt is fixed at session spawn. Editing `policy.yaml` mid-session
  requires session restart for the agent to see new rules (though hooks
  + MCP tools always re-resolve on call). Consider a "policy reload"
  banner that re-spawns the session's participants.
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
  - *Reviewer CL write path:* a role without `write_context_library` has no CL
    write at all (by design today);
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
  unaffected (substring Tool Gate + PreToolUse hook). This was once
  described as moot on the native loop, whose EYES had no shell at all;
  rc3 D9 deleted that loop, so the CLI case is the only case and this is
  simply open.

---

## Native agent loop — CLOSED by rc3 D9 (2026-08-12)

The loop shipped 2026-07-26/27 and was **deleted** on 2026-08-12: the user
committed to the claude CLI as the only model connector, "for uniformity",
and will rebuild the native connector as a plugin. `src/agents/native/`
(6,290 lines), the `models.native` flag's readers, `may_run_native`, the
native spawn branch and `AgentRole` all came out.

Every open item that lived here — **B6 overflow handling**, the no-native-HANDS
dependency, `search_files`'s 500-file cap, `user_mcp_servers_for_agent`'s
placement, and the missing live-run coverage — was a property of that loop and
is closed with it, not deferred. Two of them are worth carrying forward when
the plugin is built rather than rediscovering:

- **Overflow is still unsolved, just not ours right now.** claude-code
  auto-compacts silently; whatever replaces it in a connector plugin should be
  designed against measurement rather than defaulted into. The measurement
  input was `<data_dir>/.local/native-accounting.jsonl` — kept unrotated for
  exactly that, and no longer written.
- **`HANDS_ROLE` assumes the CLI.** It promises `terminal_exec`, the visible
  PTY and ordinary `Bash`. A connector that does not implement those makes the
  prompt wrong with no test failing. That was true of the native loop and will
  be true of the plugin.

Start from `git show c7bba28:src/agents/native/`.


## Out of scope

- **Web UI.** bot-hq is desktop-only by design. Programmatic access is
  the planned driver PLUGIN's job now; a web frontend
  would be a separate product.
- **Multi-user / multi-tenant.** Single-developer-workstation is the
  design target. Shared workstations are out of scope (auth-token
  threat model assumes single user).
- **Migration of legacy bot-hq runtime state.** Sessions / hub history
  / last-state files from the Go/tmux/MCP-hub bot-hq do NOT carry over.
  Project CL was distilled once at rebuild time; further sync is
  manual.
