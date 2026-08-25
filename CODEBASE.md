# bot-hq — Codebase Map

**What this is.** The repository split into fourteen live AREAS (plus one recorded as REMOVED) so that
exploring, studying, auditing or fixing bot-hq can happen one area at a time
instead of hopping across the tree. For each area: what it does, its files and
their roles, its entry points, its SEAMS (the joins that cross into other areas —
where a change is most likely to break something and where a test must pin), its
tests, and recipes for the common "where do I add X" questions.

**What this is not.** It does not describe behaviour — [`ARCHITECTURE.md`](ARCHITECTURE.md)
does that (what bot-hq IS), [`PLAN.md`](PLAN.md) says what is next,
[`PROGRESS.md`](PROGRESS.md) what changed. The binding rc3 decisions are in
[`docs/plans/2026-08-11-rc3-decisions.md`](docs/plans/2026-08-11-rc3-decisions.md)
(grep the `D<n>` you need — it is long). Project conventions, gotchas and
learnings live in the Context Library (`~/.bot-hq/library/projects/bot-hq/`), not
here. The dated audit that produced this map is
[`docs/plans/2026-08-15-rc3-audit.md`](docs/plans/2026-08-15-rc3-audit.md).

**How to use it.**
1. A task names a feature or a file → find its area in the index (§0) or search
   this file for the path.
2. Read that area's section top to bottom (they are short), then the matching
   ARCHITECTURE.md section for behaviour.
3. Cross an area boundary only through the seams listed for it (§14). A change
   that spans two areas touches a seam; the seam row says whether a test pins the
   join today (PINNED) or whether the join could be deleted with a green suite
   (UNPINNED — add the pin before you rely on it).
4. When you move, add, rename or delete a source file, update its row here.
   `tests/codebase_map_test.rs` fails `cargo test` if a source file is unmapped
   or a mapped path no longer exists.

**Conventions in this file.** Size buckets, never line counts: S <200 lines,
M <600, L <1500, XL ≥1500 (a bucket does not go stale every commit). Rust files
carry their `#[cfg(test)]` module inline — an XL file is often half tests.
"Tests" name what the important tests PIN, not how many there are. Paths are
repo-relative; frontend test files (`X.test.tsx`) belong wherever `X.tsx` is
listed. `PINNED`/`UNPINNED` verdicts are as of HEAD `f8127b0` (2026-08-15); later rounds re-verified the ones they touched (the round-12 joins — the push re-run wire, the gate runner's env, the summons deal — are pinned by the tests named in their commits).

---

## 0. Area index

| area | purpose | where | read-first behaviour doc |
|---|---|---|---|
| **A** Agent runtime | build/launch/feed/read one claude-code subprocess per participant; capabilities; hardcoded prompt layers | `src/agents/` | ARCHITECTURE §Process model, §Role prompts; D3 D4 D9 D10 |
| **B1** Core — the ring | the turn engine: deal turns, deliver backlogs, yield (consensus / halt / gate / all-pass / cap / spin / Stage); the per-participant pump; activity; user send | `src/core/` (sequencer, pump, activity, broadcast, state, mentions, ipav) | ARCHITECTURE §Turn coordination; D17 D19 D21–D35 |
| **B2** Core — lifecycle | open → compose prompts → spawn → ring start; close → epilogue; watchdog; worktrees; terminals; updates | `src/core/` (session, close_learnings, watchdog, worktree, terminal, updates, telemetry) | ARCHITECTURE §Process model, §Session worktrees; D8 D15 D21 |
| **C1** Internal MCP dispatch | the 40-tool agent↔UI signaling server: routing, capability gate, tool descriptors | `src/signaling/` (server, jsonrpc, protocol, …) | ARCHITECTURE §Internal MCP server; D10 D16 |
| **C2** Signaling bridge | the process-wide hub the tools call into: tray, halt slot, ring gates, SignalingEvent fan-out, findings, session docs, CL facade/write/push | `src/signaling/bridge/` | ARCHITECTURE §Internal MCP server, §Context Library; D22 D34 D35 |
| **D** External MCP | REMOVED 2026-08-17 (`d0661b4`) — the driver is a future plugin | — | ARCHITECTURE §The external driver — REMOVED |
| **E** Policy | git hooks + PreToolUse tool gate + session policy snapshots + violations + secret scan | `src/policy/` | ARCHITECTURE §Policy enforcement, §Tool Gate, §Session policy |
| **F** Storage | sqlx sqlite: every table, the immutable migrations | `src/storage/`, `migrations/` | ARCHITECTURE §Storage (sqlite) |
| **G** Host shell | `main.rs` boot, data dir, Tauri commands + events, batch emitter, fs watcher | `src/main.rs`, `src/paths.rs`, `src/tauri_cmd/`, `src/tauri_events/` | ARCHITECTURE §Tauri + React UI, §Data locations |
| **H** Plugins | `bhq-plugin://` iframe runtime, catalog-gated RPC, manager UI | `src/plugins/`, `src/tauri_cmd/plugin*.rs`, FE Plugin* | ARCHITECTURE §Plugins; `docs/PLUGINS.md` |
| **I** Claude config | inheritance lens over `~/.claude` + per-role overrides injected at spawn | `src/claude_config/`, FE `ClaudeConfig.tsx` | ARCHITECTURE "Claude Config surface" |
| **J** FE — session view | the live session room + realtime plumbing (Providers, stores, hooks) | `frontend/src/` (SessionView, Chat*, DocumentPane, Halt/Approval/Tray, stores, hooks) | ARCHITECTURE §Tauri + React UI; D20 D28 D30 D32–D35 |
| **K** FE — shell/dashboard/settings | app shell, Dashboard + New Session dialog, all Settings panels, shared atoms/libs | `frontend/src/app/`, `frontend/src/components/`, `frontend/src/lib/` | ARCHITECTURE §Tauri + React UI; `docs/design/industrial_terminal/DESIGN.md` |
| **L** FE — Context Library UI | the 2-pane CL explorer/editor, ContextManager, FileViewerDialog | `frontend/src/app/ContextLibrary*` | ARCHITECTURE §Context Library |
| **M** Docs + repo hygiene | canonical docs, `docs/`, examples, packaging, deps | repo root, `docs/`, `examples/`, `packaging/`, `site/` | `CLAUDE.md` read order |

Cross-cutting traces (the paths a request actually takes) are in §13; the seams
table is §14.

---

## A. Agent runtime — `src/agents/`

**What it does.** Builds, launches, feeds and reads one `claude-code` subprocess
per session participant: argv/env/`--settings` assembly, stream-json stdout →
`AgentEvent`, stdin writes, retry supervision, the capability model, and the
hardcoded prompt layers (general rules, role prose, capability prose). Since rc3
D9 the claude CLI is the only connector; everything past `AgentHandle` is
runtime-agnostic. `llm_proxy` is LIVE (not D9 debt): it normalises claude-code's
request body for non-Anthropic gateways when a model has a `base_url`.

| path | role | size |
|---|---|---|
| `src/agents/mod.rs` | re-export hub | S |
| `src/agents/spawn.rs` | `SpawnConfig`/`AgentHandle`/`AgentEvent`; `build_command` (argv/env/settings, permission posture by capability); `spawn_supervised_agent` retry supervisor; `ParticipantInput::{deliver,deliver_batch}` (receipt-gated stdin); child reaping (`CHILD_PIDS`) | XL |
| `src/agents/events.rs` | stdout/stderr pumps; `translate` stream-json → `AgentEvent`; context-window arithmetic (`parse_context_usage`) | L |
| `src/agents/protocol.rs` | stream-json wire types both directions | M |
| `src/agents/input.rs` | `pump_inputs` stdin writer (messages + control/interrupt) | S |
| `src/agents/prompts.rs` | layer-3 hardcoded role prose (`HANDS_ROLE`/`EYES_ROLE`, keyed by ROLE SLUG) — seeds `roles.description_prompt`; live prose is the DB row | L |
| `src/agents/general_rules.rs` | layer-1 `GENERAL_RULES` (compiled in) | M |
| `src/agents/capability.rs` | `Capability` (16), `CapabilitySet`, `ResolvedCapabilities`, `required_for` = THE tool→capability map | L |
| `src/agents/capability_prompt.rs` | layer-2 generator: grants/denials + live roster prose from a `CapabilitySet` (`phrasing()` exhaustive) | L |
| `src/agents/llm_proxy.rs` | localhost reverse proxy for non-Anthropic gateways (started by `src/main.rs`) | L |
| `docs/stream-json-events.md`, `docs/stream-json-samples/` | empirical claude-code stream schema + captured fixtures (frozen since 2026-05; fixtures unused by tests) | — |
| `templates/cl/` | seed text for the user's `custom-general-rules.md` / `custom-instructions.md` (layers 4/5) | — |

**Entry points.** `spawn_supervised_agent` (production spawn) · `build_command`
(branches on `capabilities.grants(EditFiles)`, never on a name) ·
`ParticipantInput::deliver_batch` (one stdin write per backlog page) ·
`translate` · `required_for` · `capability_prompt::render` · `builtin_prose_for_role`.

**Seams (§14):** 1–7 (prompt compose → file → argv; caps → tools; overrides →
settings/env; mcp-config; spawn env `BOT_HQ_SESSION_ID`/`BOT_HQ_AGENT` → hooks —
UNPINNED at the producer; PreToolUse hook), 9 (deal → stdin), 10 (pump), 11
(epoch cell), 29 (capability ↔ tool gate, DERIVED).

**Gotchas → pointers.** Prompt stack order is documented identically in
`src/agents/prompts.rs` and `src/agents/general_rules.rs` headers. Posture is
capability-driven; `build_read_only_disallowed_tools` (renamed 2026-08-17). Do not
add a second unrecorded stdin write (see `docs/plans/2026-08-13-next-queue.md`
"What NOT to do").

**Tests pin.** posture follows the capability set (`spawn.rs`), receipt
session-scope refusal, retry/backoff, process-group reaping; the three-token
context arithmetic (`events.rs`); `unknown_event_doesnt_panic` (`protocol.rs`);
preset parity oracle (`capability.rs`); exhaustive capability coverage
(`capability_prompt.rs`); regression guards named after incidents
(`prompts.rs`, `general_rules.rs`).

**Where to add X.** New capability → variant + `Capability::ALL` +
`phrasing()` arm (won't compile otherwise) + `required_for` if it gates a tool +
preset. · New spawn flag/env → `build_command`, branch on a capability. · Prompt
wording → pick the layer (role prose is DB data: needs a reseed migration; general
rules are compiled in).

---

## B1. Core — the turn ring — `src/core/`

**What it does.** `sequencer.rs` is THE turn engine: one `run_sequencer` task per
session owns holder/epoch, deals turns (`hand_turn_to` = `set_current_turn` +
busy mark), delivers a participant's unread backlog as one stdin write per page
(`deliver_backlog` → `deliver_batch` → `commit_delivery`), and yields
(`halt()` = holder None, epoch+1) on consensus, declared halt, open gate,
all-pass lap, round cap, spin, or a Stage boundary. `pump.rs` (renamed from `duo.rs`, `90bec09`) is the
per-participant PUMP: stream-json → `messages` rows → one `TurnComplete{epoch}`
per turn. `activity.rs` derives `SessionActivity` for the UI input lock.
`state.rs` is `AppState`: user send → row + `UserMessage`, cancel/resume, close +
D15 epilogue, phase advance, tray answers, Stage.

| path | role | size |
|---|---|---|
| `src/core/mod.rs` | module list + `post_system_notice` re-export | S |
| `src/core/sequencer.rs` | the ring: `run_sequencer`, `SequencerCommand`, `advance_turn`, `start_turn`/`hand_turn_to`, `deliver_backlog`, consensus/spin/cap/all-pass/Stage yields, `TurnEnding` — the long module doc is a design diary, not current behaviour | XL |
| `src/core/pump.rs` | `pump_agent` + `PumpConfig`: rows, boot rows, provider-limit/error-streak halts, pass row, epoch bind + `TurnComplete` mint | XL |
| `src/core/activity.rs` | `ActivityTracker` (per-slug busy map + latches) → `SessionActivity` + `session:activity` emit | L |
| `src/core/state.rs` | `AppState`: open/ensure/restart/`reopen_session`, `cancel_and_escalate` (Pause: decide → interrupt → SIGKILL, atomic-op deferral via `await_atomic_op_or_cap`), resume, `close_session` + epilogue (join arm applies the archive) + `teardown_session`, `broadcast`, `send_user_response`, Stage, `advance_phase`, `resolve_choice`, `halt_declared` | XL |
| `src/core/broadcast.rs` | `broadcast_user_message`: envelope + `post_to_channel("user")`; writes no stdin (D19) | S |
| `src/core/mentions.rs` | `parse_mention_slugs` (D17) | S |
| `src/core/ipav.rs` | `IpavPhase` chip/name/parse/transition notice | S |

**Entry points.** `run_sequencer` · `SequencerCommand` (TurnComplete, UserMessage,
MessageStaged/Unstaged, ParticipantJoined, HaltDeclared, GateOpened/Resolved,
Pause/Resume — Pause is minted by `cancel_session_turn`; Resume still has NO production producer) · `pump_agent` ·
`SessionActivity::derive` · `AppState::{broadcast,user_responded,close_session,
resolve_choice,stage_user_response,deliver_staged}`.

**Seams (§14):** 8 (ring ↔ roster, PINNED), 9 (deal → stdin + failure path:
warn-only, no UI surface), 10 (pump → row → UI), 11 (epoch cell pairing —
UNPINNED join), 12–14 (tray/halt/gate ↔ ring), 18 (Stage — main.rs hop UNPINNED),
28 (mentions), 30 (boot orphan sweep).

**Gotchas → pointers.** Halt = the session row's slot (D35), gates latch the ring
via `notify_ring_gate`; a parked QUESTION stops nothing (D35 — its answer
batches into Send, D34). A release that lands while the halted holder is still
finishing is held (`RingState::winding_down` / `stashed_release`, round 10)
and replayed on that holder's completion, its respawn, or
`HALT_WIND_DOWN_GRACE` — `release_ring` is the one restart body. Ring in-memory
state (deferred/held/summons/staged/laps) is lost on restart; only `open_gates`
reseeds. Gate rows are recognised by `is_gate_options`/`GATE_OPTIONS_JSON`
(`storage/tray.rs`; the frontend mirrors it in `HaltBanner.tsx::isApproval`) —
`session_tray.kind` is `'choice'` for questions AND approvals (round 7 A8, T2).
Staged text lives in `AppState` memory AND `sessions.staged_message`
(B1-F11). CL `notes.md` "Session runtime" for the interrupt model.

**Tests pin.** epoch guard / stale completions discarded, consensus + pass tally,
declared halt stops where it stands, one-write pages, summons order, all-pass,
round cap from policy, spin fills the halt slot, gate latch seeding, Stage
boundary, busy-mark ⇔ input lock (`sequencer.rs`); TurnComplete carries the bound
epoch, boot readiness, provider-limit row+halt (`pump.rs`); pure decision tables +
source-grep pins (`state.rs`); derive priority (`activity.rs`). Not pinned: a
closed-stdin deal, an empty-page deal, pump+ring end to end.

**Where to add X.** New yield reason → decide in `run_sequencer`/`advance_turn` →
`halt()` → visible `system_notice` row → `bridge.mark_awaiting_user(sid,"system",…)`
→ pin with a ring test + slot assert. · New `SequencerCommand` → variant → arm →
drain behaviour in `deliver_backlog`'s select → producer `notify_ring_*` in
`src/signaling/bridge/mod.rs`. · New user-input path → ALWAYS
`broadcast_user_message` then `user_responded` (never write a stdin directly).

---

## B2. Core — session lifecycle — `src/core/`

**What it does.** Owns a session's life: roster seed → per-participant
`compose_system_prompt` → `participant_spawn_config` → `spawn_supervised_agent`
→ ring/pump/watchdog wiring → `boot_then_start` (D21 BOOT); close with the
optional D15 CL-learnings epilogue; the stall/idle watchdog (chip → nudge →
escalate to halt); git-worktree isolation; the Terminal-subtab PTY; the update
check.

| path | role | size |
|---|---|---|
| `src/core/session.rs` | `open_session`/`spawn_existing_session` → `spawn_session_handle` (the big glue fn); `compose_system_prompt`, `resolve_role_prose`, `resolve_roster_facts`, `read_system_prompt`, `participant_spawn_config`, `participant_capabilities`, `resolve_participant_overrides`, `mcp_config_json`; `resolve_participant_config` (model: explicit → role default → legacy `agent_configs` → hardcoded); `spawn_ring`, `boot_then_start`, `RingKick`; `SessionHandle` | XL |
| `src/core/close_learnings.rs` | pure `decide`/`plan(decision, claimed, in_flight)` for the D15 close-out epilogue (in-flight decides first, round 11), `outcome_notice` | M |
| `src/core/watchdog.rs` | `run_stall_watchdog`: stall + idle-unflagged loop → `session:attention` chip / nudge / halt escalation | L |
| `src/core/worktree.rs` | `ensure_worktree`/`remove_worktree_if_clean` (argv-array `git`, no shell) | M |
| `src/core/terminal.rs` | `TerminalRegistry`/`SessionTerminal`: one PTY per session, bounded scrollback, `wait_settle` (Notify, not polling) | M |
| `src/core/updates.rs` | GitHub-releases version check (pure logic + thin fetch) → `UpdateBanner` | M |
| `src/core/telemetry.rs` | opt-in diagnostics: hash-only panic/error events, `$HOME`→`~` redaction, 1MB drop-oldest jsonl queue, never-blocks flusher (runtime-config endpoint), panic-capture chain, `TELEMETRY_ENABLED` atomic | M |

**Entry points.** `open_session` · `spawn_session_handle` · `compose_system_prompt`
· `participant_spawn_config` · `boot_then_start` · `AppState::close_session` →
`close_epilogue_decision` → `run_close_epilogue`/`teardown_session` (in
`src/core/state.rs`) · `run_stall_watchdog` · `TerminalRegistry::ensure`.

**Seams (§14):** 1–5 (compose/spawn — PINNED), 11 (epoch cell wiring inside
`spawn_session_handle` — UNPINNED), 19 (session open registrations — four inline
bridge registrations UNPINNED), 20 (close — main.rs hop UNPINNED), 22 (CL push
flags read before teardown), 24 (watchdog → attention → UI, UNPINNED), 30.

**Gotchas → pointers.** `spawn_session_handle` spawns subprocesses, so no test
reaches its body — every load-bearing step must be an extracted fn.
`agent_configs` seeds nothing and takes any key since migration 0060 dropped its
`emma|brian|rain` CHECK and rows — the tier-2 model fallback used to miss for
every role-slug roster and is now reachable. Two close entry points
(UI + MCP `close_session`) can race the epilogue (audit B2-5). The epilogue's
"wait for busy" is currently inert (audit T-1). Worktree rules: ARCHITECTURE
§Session worktrees; CL `conventions.md` (dist missing in a fresh worktree kills
`cargo test`).

**Tests pin.** roster → spawn order + capability gating, the D8 model chain,
byte-identical prompt composition, layer-2 roster facts, project resolution,
`the_spawn_roster_registers_every_reviewer_it_returns`,
`starting_the_ring_registers_it_so_a_parked_question_can_halt_it` (`session.rs` — the
name is pre-D35: its body declares a HALT via `mark_awaiting_user`, and a
question no longer reaches the ring);
`decide`/`plan` tables (`close_learnings.rs`); decision tables + the two-row nudge
wire (`watchdog.rs`); worktree/terminal/update pure logic. Not pinned: the
concurrent double-close, PTY child reaping, the escalation branch.

**Where to add X.** New per-session background loop → spawn from
`spawn_session_handle` next to the watchdog, hold `Weak` refs. · New close-time
behaviour → an `Epilogue`/`ClosePlan` arm in `close_learnings.rs` (exhaustive
match), not a branch in `close_session`. · New spawn knob → `seed_default_roster`
→ `participant_spawn_config` → `SpawnConfig`; never a new `sessions.*_id`
column (D8).

---

## C1. Internal MCP server — dispatch — `src/signaling/`

**What it does.** The in-process HTTP JSON-RPC MCP endpoint
(`/sessions/<id>/<agent>/<token>/mcp`) every spawned agent's `--mcp-config` points at.
Defines the 40-tool signaling surface, gates each tool by the caller's
`CapabilitySet` (resolved from `session_participants` by the URL's session+slug;
the URL also carries a per-spawn secret, checked in `server.rs` — C1-1 closed
`a4e5c00`), dispatches into the bridge (C2). Two extra localhost routes let the
git-hook subprocesses park approvals (`/hooks/pre-push`, `/hooks/tool-gate`).

| path | role | size |
|---|---|---|
| `src/signaling/mod.rs` | module wiring; `RESERVED_MCP_KEYS`; `parity` is `#[cfg(test)]` | S |
| `src/signaling/server.rs` | hyper server: bind/route (3 shapes), `CallerIdentity` from the URL, mcp-config render, hook routes | L |
| `src/signaling/jsonrpc.rs` | `dispatch` (initialize/ping/tools/list/tools/call) + `call_tool` (one flat match over 40 tools), capability gate, refusal rows, `PARITY_HOLD` (empty) | XL |
| `src/signaling/protocol.rs` | wire types + the 40 `ToolDescriptor`s; `gated_by()` derives the gate sentence from `required_for` | L |
| `src/signaling/response.rs` | shared HTTP/JSON-RPC response builders | S |
| `src/signaling/tool_args.rs` | shared string-arg helpers | S |
| `src/signaling/parity.rs` | TEST-ONLY oracle: pre-rc3 name gate vs the capability gate, every (tool, agent) pair | L |
| `src/signaling/webview_js.rs` | JS builders for `webview_*` tools | S |
| `src/signaling/web_search.rs` | `web_search` via a hidden Tauri webview; one process-global permit | M |
| `tests/signaling_test.rs` | raw-TCP end-to-end HTTP contract (routing, parse errors, notifications, `ask_user_choice` parks) | S |

**Entry points.** `start_signaling_server` (from `src/main.rs`) · `handle_request`
· `resolve_caller_capabilities` · `dispatch` → `call_tool` · `tool_descriptors()`.

**Seams (§14):** 7 + 15 (tool-gate hook ↔ `/hooks/tool-gate` — HTTP shape
duplicated, join UNPINNED), 12 (ask_user_choice trace), 16–17 (findings gate,
commit-message check), 29 (capability ↔ tool gate: DERIVED, PINNED).

**Gotchas → pointers.** Caller identity is the URL's session+slug PLUS a
per-spawn secret (`server.rs::mcp_token_matches`; audit C1-1, closed). Tool
descriptions are prompt text every agent reads — `no_tool_description_an_agent_
reads_names_an_agent` sweeps names AND two-of-them phrasing, and
`stop_and_phase_tool_descriptions_match_the_shipped_semantics` (round 7) pins
the halt/question/vote descriptions to D35/D37; the driver's `wire` module and
its exemptions are gone. Change what a tool requires ONLY in
`required_for`.

**Tests pin.** `every_gated_tool_names_the_capability_its_gate_reads` (derived),
`no_tool_description_an_agent_reads_names_an_agent`, the parity oracle, per-tool
dispatch tests, `check_commit_message_finds_forbidden_word`. Not pinned: webview
tool dispatch, unknown-tool refusal.

**Where to add X.** New tool → `ToolDescriptor` → `required_for` arm (if gated) +
`gated_by(...)` → `call_tool` arm → bridge fn; parity sweep auto-covers. · New
hook route → `handle_request` branch + `handle_*` fn (400 on missing fields, one
bridge method, outcome → status).

---

## C2. Signaling bridge — `src/signaling/bridge/`

**What it does.** `SignalingBridge` is the process-wide, session-agnostic hub the
tool handlers call into: parks tray items (question / approval gate) with an
in-memory oneshot + a durable `session_tray` row; flips the awaiting flag /
halts or latches the ring via per-session registries; fans out `SignalingEvent`s
over one broadcast (UI via `src/tauri_events/bridge_subscriber.rs`, the main.rs
control handler, the plugin proxy's `wait_for_change`); fronts storage for findings,
session docs, CL index/write/push, feedback, terminals. Everything per-session
lives in 14 registries on one struct (12 keyed by session id, 2 by `(session, agent)`).

| path | role | size |
|---|---|---|
| `src/signaling/bridge/mod.rs` | struct + 14 registries, `SignalingEvent` (17 variants), register/`unregister_session`, `notify_*` emitters, close-gate + retired-terms, policy resolution, reviewer-override request | XL |
| `src/signaling/bridge/tray.rs` | `ask_user_choice_inner` (`kind = approval` for a host GATE = the ring's gate marker; `kind = request` for an agent's `request_approval` — tray, audited, no latch, round 12), `request_approval(_parked)`, supersede/withdraw (owner-scoped), `resolve_choice_confirmable`, `deliver_oob`, `emit_halt_row`/`mark_awaiting_user`, `request_phase_advance` | XL |
| `src/signaling/bridge/action_gate.rs` | `park_gated_command` (dedupe) / `execute_gated` (`tool_gate::run_in_repo`), `gate_status` | L |
| `src/signaling/bridge/findings.rs` | `eyes_flag`/`approve`/`disposition`/`check_open_findings` + reviewer-down gate + override | M |
| `src/signaling/bridge/session_docs.rs` | doc write (phase-keyed, `-eyes` twin), search, read, archive-on-rewrite (cap 50) | M |
| `src/signaling/bridge/terminal_tools.rs` | `terminal_exec`/`terminal_read` over the PTY registry + Tool-Gate parity | M |
| `src/signaling/bridge/feedback.rs` | `file_feedback` | S |
| `src/signaling/bridge/cl_facade.rs` | `cl_index_search`/`cl_retrieve`/`cl_stale_refs`/folder/register/`cl_rescan` | L |
| `src/signaling/bridge/cl_write.rs` | `cl_write_file`: path guards, atomic write, shrink guard, retired-term diff, `git_version_library` (add -A + commit), push trigger, `sweep_project` | L |
| `src/signaling/bridge/cl_push.rs` | secret scan of tracked files, then `git push` of the library | M |
| `src/signaling/bridge/cl_refs.rs` | code refs + sha256 | S |
| `src/signaling/bridge/cl_staleness.rs` | CL body claims vs repo (P4) | M |
| `src/signaling/bridge/util.rs` | `walk_cl_dir`, `split_into_atoms`, `extract_description`, OOB body | L |

**Entry points.** `SignalingBridge::new_with` · `ask_user_choice_inner`
(`is_approval = approval.is_some()` is the only authoritative gate signal at park
time) · `resolve_choice_confirmable` → `deliver_oob` · `emit_halt_row` ·
`notify_ring_gate` / `notify_ring_user_message` / `notify_ring_stage` ·
`unregister_session` · `cl_write_file`.

**Seams (§14):** 12–14 (tray/halt/gate — the gate LIFT `notify_ring_gate(false)`
is UNPINNED and its cut wedges the ring for the process lifetime), 15, 16 (open-
blocking predicate restated 3×), 21–22 (CL write chain PINNED end-to-end; push
after write), 24 (attention dedupe UNPINNED).

**Gotchas → pointers.** Restarting the app severs live agents' signaling tools
(CL `conventions.md`). Only `RefusedSecrets` push outcomes reach chat; other push
failures are log-only. `unregister_session` leaks `session_sequencer`,
`turn_passes`, `agent_rpc_seen`, `pending_override_requests`,
`session_reviewers`, `session_attention` (audit C2-1). Known-flaky:
`ask_user_choice_auto_supersedes_reask_same_prompt` (sleep-coordinated).

**Tests pin.** halt reaches the ring / a question does not; a declared halt
halts + a user message releases (`a_parked_question_halts_the_ring_and_a_user_message_releases_it`
is named for the pre-D35 behaviour; it calls `mark_awaiting_user`); any approval opens a gate; parks immediately; OOB
fallbacks (receiver dropped, reopened session); gate execute-once + post-restart
execute (`action_gate.rs`); dedup/raise + reviewer-down decision (`findings.rs`);
slug collapse, archive-on-rewrite, append (`session_docs.rs`); traversal, atomic
write, git versioning, push-unless-credential, close-gate flags (`cl_*`).

**Where to add X.** New `SignalingEvent` → variant → `notify_*` fn → arm in
`src/tauri_events/bridge_subscriber.rs::route` + `EVENT_NAME` const in
`src/tauri_events/types.rs` → `useTauriEvent` in `frontend/src/Providers.tsx` →
store/query invalidation → seed from `get_session_runtime` if it must survive a
missed event; if `main.rs` must react, add an arm to its control consumer AND its
filter. · New tray-parking tool → protocol + gate → `ask_user_choice_inner(...,
approval_ctx, ...)` — pass `Some(ApprovalContext)` ONLY if it must halt the ring,
options exactly `["Approve","Reject"]` if it is a gate.

---

## D. External MCP server — REMOVED

The bearer-token driver server (`src/signaling/external_*.rs`, 20 tools) was
deleted on 2026-08-17 (`d0661b4`) when the external driver was demoted to a
future plugin (ARCHITECTURE §The external driver — REMOVED). `wait_for_change`
survives in `src/tauri_cmd/plugin_api.rs` for the plugin proxy; the round-7
`session:created` wiring is in `AppState::notify_session_created`.

---

## E. Policy enforcement — `src/policy/`

**What it does.** Two layers: agent-facing MCP tools are the primary path
(`check_commit_message`, `action_gate`, `check_open_findings`, …) and installed
git hooks + a per-agent PreToolUse hook are the mechanical backstop that fires
whether or not the agent remembered. `Policy` = resolved rules (forbidden commit
words, push/force-push gates, per-action approvals; general → project → session
snapshot, REPLACE not merge). The Tool Gate = a global keyword→gate/auto-allow
substring matcher over Bash calls.

| path | role | size |
|---|---|---|
| `src/policy/mod.rs` | `Policy`, `resolve_at_root`/`merge`, `first_forbidden_word`/`contains_word` (one impl, word-boundary, case-insensitive), prompt block render, `write_config_atomically` (the one temp+rename config writer: policy YAMLs, `tool-gate.json`, the hash cache) | L |
| `src/policy/hooks.rs` | `policy-check` CLI: commit-msg / pre-commit (forbidden words on added lines + immutable-migration guard + EYES findings gate) / post-commit / pre-push (the EYES findings gate again, then `decide_push` HTTP round-trip; the prompt names the PUSHED refs from git's stdin lines — `parse_push_updates`/`pushed_ref_names`, read once and lazily — HEAD only as fallback) / tool-gate (PreToolUse → `park_gate`); `install_hooks`; `check_findings_gate` on its OWN read-only sqlite, over storage's `OPEN_BLOCKING_FOR_SESSION` predicate | XL |
| `src/policy/tool_gate.rs` | keyword list resolution (session snapshot → global), `match_keyword` (substring, gate wins), `run_in_repo` in the AGENT's shell (`gate_shell`: `$SHELL` if POSIX-family, else zsh→bash→sh — macOS bash is 3.2 and dies on heredoc-in-`$()`) | M |
| `src/policy/session_policy.rs` | `.local/session-policies/<sid>.yaml` snapshot write-if-absent (enforced by the caller) / read / purge at boot | S |
| `src/policy/violations.rs` | append-only `violations.jsonl` (unbounded) | M |
| `src/policy/audit.rs` | policy-file tamper detection (sha256 cache) | M |
| `src/policy/secret_scan.rs` | credential-shaped content/filename scan for the CL push | M |

**Entry points.** `hooks::run_cli` (from `src/main.rs`; the CLI catch-all turns
every internal `Err` into exit 0 — audit E1) · `install_hooks` (at session spawn
+ `bot-hq install-hooks`) · `run_tool_gate` → `park_gate` → `/hooks/tool-gate` ·
`check_findings_gate` · `tool_gate::{resolve_keywords,match_keyword}`.

**Seams (§14):** 6 (spawn env → hooks, UNPINNED at producer), 7/15 (PreToolUse
hook → HTTP park), 16 (findings gate: hook enforces only open-blocking rows; the
reviewer-down branch lives only in the MCP tool), 17 (`check_commit_message` ↔
commit-msg hook: ONE fn).

**Gotchas → pointers.** CL `notes.md` "The commit hooks (load-bearing)": whole-word
+ case-insensitive matching means writing the real forbidden footer anywhere
self-blocks your commit; the Tool Gate also matches quoted text inside the
command (issues.md #12); hooks run without a tracing subscriber; hook subprocess
reads sqlite READ-ONLY + fail-open. Immutable-migration guard escape:
`BOTHQ_ALLOW_IMMUTABLE_EDIT=1`, never `--no-verify`.

**Tests pin.** `added_lines_only`, immutable violations, force detection, push
response classification, `tool_gate_exit`, findings-gate query through the real
`Storage` writer, forbidden-word boundaries. Not pinned: `run_pre_push` with a
session present (`ask` → HTTP → exit code).

**Where to add X.** New forbidden-word-style check → `Policy` field → prompt block
→ `run_commit_msg`/`run_pre_commit` → `check_commit_message` (keep the 3 sites on
`first_forbidden_word`). · New hook kind → `HookKind` + `run_cli` arm + handler +
`install_hooks` loop. · New Tool-Gate caller → reuse `resolve_keywords` +
`match_keyword`.

---

## F. Storage — `src/storage/`, `migrations/`

**What it does.** The sole persistence layer: `Storage` owns one `SqlitePool`
(8 connections, foreign keys on, WAL journal — readers never block writers); every other
area reads/writes through it. `Storage::open` runs `sqlx::migrate!`; migrations
are immutable once applied (hook-guarded), highest `0069`, `0056` reverted by
re-stamp. One timestamp helper (`now_utc()`, RFC3339-Z) is meant to be bound by
every write.

| path | role | size |
|---|---|---|
| `src/storage/mod.rs` | `Storage::open`/`memory`, pool, `migrate!`, generic CL search | S |
| `src/storage/row_types.rs` | shared `FromRow` structs/enums (Message, Session, Finding, …) | M |
| `src/storage/time.rs` | `now_utc()` | S |
| `src/storage/sessions.rs` | `sessions` CRUD, `reopen_session`/`archive_session`, spawn-model/effort columns, halt slot (`declare_session_halt`/`clear_session_halt`), boot orphan sweep | M |
| `src/storage/participants.rs` | roles · session_participants (roster seed/invite, labels, colours) · turn cycle (`next_active_participant`, `set_current_turn`, done votes, `round_number`) · cursors + deliveries (`commit_delivery`, `unread_for_participant`/`channel_page`, `UNREAD_BATCH_LIMIT`) · channel wire (`post_to_channel`, envelope) — six clean extraction seams | XL |
| `src/storage/messages.rs` | `messages` insert/read: since-id watermark (`messages_for_session`, unbounded), the chat's tail page (`messages_tail`), the spin-detection read (`participant_text_since`); the three production SQL strings are fns (`messages_since_sql`, `messages_tail_sql`, `participant_text_since_sql`) EXPLAINed by tests | M |
| `src/storage/tray.rs` | `session_tray` (questions/approvals/gated commands), `pending_gate_ids`, purge, closed/orphan withdraw | M |
| `src/storage/findings.rs` | EYES findings CRUD + `count_open_blocking_findings` | M |
| `src/storage/session_docs.rs` | per-session IPAV docs + archives | M |
| `src/storage/activity_events.rs` | activity transition ledger (read only by tests + boot sweep) | S |
| `src/storage/gc.rs` | boot-time retention purges for the five append-only telemetry tables (`participant_deliveries`, `context_readings`, `retrieval_events`, `cancel_events`, `cl_reads`), run by `main.rs` beside the tray/activity sweeps; `messages` is deliberately not swept | S |
| `src/storage/cancel_events.rs` | Stop/interrupt escalation ledger | S |
| `src/storage/retrieval_events.rs` | `cl_retrieve` telemetry | S |
| `src/storage/context_readings.rs` | per-turn context-window readings (P7) | M |
| `src/storage/feedback.rs` | agent-filed bot-hq feedback | S |
| `src/storage/models.rs` | `models` registry + `app_settings` kv | M |
| `src/storage/agent_config.rs` | `agent_configs` — read by the spawn chain. Its `emma\|brian\|rain` CHECK made it unreachable for every rc3 role slug until 0060 rebuilt the table without it; the test that pinned the refusal now pins the acceptance | S |
| `src/storage/projects.rs` | `projects` registry, CL path resolution | M |
| `src/storage/plugins.rs`, `src/storage/plugin_kv.rs` | plugin registry + per-plugin kv | M / S |
| `src/storage/cl_index.rs`, `src/storage/cl_atoms.rs` | CL index/folders/reads; FTS5 atoms + `cl_retrieve` | M / M |
| `migrations/` | 0001…0069 (0056 absent) — append-only | — |
| `tests/storage_test.rs` | cross-cutting smoke: empty-DB migration, tray scoping, message since-id, session close/list, config round-trips | M |

**Entry points.** `Storage::open` · `now_utc` · `next_active_participant` ·
`commit_delivery` · `channel_page` · `post_to_channel` · `insert_message` ·
`declare_session_halt` · `pending_gate_ids` · `cl_retrieve`.

**Seams (§14):** 8 (ring ↔ roster, PINNED), 9 (`commit_delivery`), 16 (open-
blocking predicate ↔ hook query), 21 (CL index after write), 30 (boot sweeps).

**Gotchas → pointers.** CL `conventions.md` §Migrations (immutable; DB reset
2026-08-12; re-stamp is the sanctioned escape) and the memory note on edited
migrations. `sessions.{brian,rain}_*` and `rain_enabled` are GONE (migration 0060 — nine
columns dropped, two renamed to `slot0/slot1_model_at_spawn`); `messages.author`
survives as a plain slug string, its CHECK and its `Author` enum both deleted. Heaviest callers: `src/core/sequencer.rs`,
`src/signaling/bridge/tray.rs`, `src/core/pump.rs`.

**Tests pin.** ring order/skip rules, done-vote reset, roster byte-parity, prose
migration provenance (0046/0048/0049 oracles), delivery/cursor invariants, 200-row
paging (`participants.rs`); halt slot + orphan sweep shapes (`sessions.rs`); tray
sweeps. Not testable today: upgrading an EXISTING DB across a migration revert.

**Where to add X.** New table → `migrations/00NN_*.sql` (+ index matching the
query) → `row_types.rs` struct → `storage/<table>.rs` `impl Storage` + `mod` in
`mod.rs` → make something other than a test READ it. · New participant column →
migration → `PARTICIPANT_COLUMNS` const + `participant_from_row` + struct (+
`insert_roster` if invite-frozen). · Timestamps → bind `now_utc()`, never inline
`datetime('now')`.

---

## G. Host shell + Tauri command/event layers

**What it does.** `src/main.rs` is the boot sequence: CLI arms (`policy-check`,
`install-hooks`, `export-bindings`) short-circuit before the GUI; otherwise data
dir → logging → single-instance lock → tokio → `Storage` + boot sweeps →
bridge + internal MCP + llm proxy → `CoreAppState` → signal
reapers → bindings export → plugin registry seed → Tauri builder (`.setup`:
subscriber, fs watcher, control-event consumer, heartbeat sweep). `src/tauri_cmd/*`
are thin `#[tauri::command]` wrappers (101 commands on 2026-08-17, all listed in
`src/tauri_specta_gen.rs`); `src/tauri_events/*` turn `SignalingEvent`s into typed
Tauri events (`BatchEmitter` coalesces messages, 50 ms / N=20, since_id) plus the
fs watcher (`cl:changed`, `session:worktree_changed`, `plugin:assets_changed`).

| path | role | size |
|---|---|---|
| `src/main.rs` | boot, CLI arms, control-event consumer (`SessionCloseRequest`/`HaltAcked`→`halt_declared`/`StagedDeliveryDue`→`deliver_staged`/`AgentAdvancePhase`), heartbeat sweep, logging | L |
| `src/lib.rs` | crate root | S |
| `src/paths.rs` | data-dir resolution, first-run init, legacy layout migrations (v0→v1→v2), `LockGuard` (PID, steals stale), legacy custom-instruction seeds | L |
| `src/tauri_specta_gen.rs` | `collect_commands!` (the ONLY registration; an omitted command compiles but is unreachable) + TS export | S |
| `src/text.rs` | `floor_char_boundary` / `ceil_char_boundary` — the byte-offset-to-char-boundary idiom every `&s[..n]` on arbitrary text must use (round 9: two panics — the stdout pump's log line and the PTY tail cut) | S |
| `build.rs`, `tauri.conf.json`, `capabilities/` | Tauri build/config; main-window ACL (unrelated to plugin gating) | — |
| `src/tauri_cmd/mod.rs`, `src/tauri_cmd/error.rs` | module list; `AppError` | S |
| `src/tauri_cmd/sessions.rs` | session CRUD/lifecycle, `resolve_participant_picks`, `list_session_participants`, `get_participant_system_prompt`, `get_session_runtime` (7-field snapshot the FE seeds from), respawn/cancel/resume, `reopen_session` (round 10: clears `closed_at`/`archived`/the halt slot, then spawns — `ensure_session_started` refuses a closed row on every other path), `close_session` | XL |
| `src/tauri_cmd/messages.rs` | `get_session_messages` / `broadcast_message` | S |
| `src/tauri_cmd/tray.rs` | choices/approvals/halts/staged responses (`stage_user_response`, `send_user_response`, `resolve_choice`, `discard_choice`) | M |
| `src/tauri_cmd/docs.rs` | session docs search, `compute_apply_diff` (git in `spawn_blocking`), `summarize_session_doc` (headless `claude -p`), `validate_model` | L |
| `src/tauri_cmd/findings.rs`, `src/tauri_cmd/feedback.rs`, `src/tauri_cmd/models.rs`, `src/tauri_cmd/updates.rs`, `src/tauri_cmd/telemetry.rs`, `src/tauri_cmd/terminal.rs`, `src/tauri_cmd/tool_gate.rs` | thin wrappers | S |
| `src/tauri_cmd/files.rs` | `read_workspace_file` (path-guarded, size-capped) | M |
| `src/tauri_cmd/roles.rs` | roles CRUD + capabilities | L |
| `src/tauri_cmd/policy.rs` | 3-tier policy get/set + `read_violations` (no limit) | M |
| `src/tauri_cmd/screenshot.rs` | NOT a command: `capture_main_window` helper for the `webview_screenshot` MCP tool | S |
| `src/tauri_cmd/cl.rs` | 20 CL commands (index/folder search, file/project CRUD, `cl_write_file` UI twin) — duplicates bridge helpers (tracked CL v2 consolidation) | L |
| `src/tauri_events/mod.rs`, `src/tauri_events/types.rs` | event-name consts + payload structs (every emitted name has a const, incl. the subscriber's `SESSION_RESYNC` / `SESSION_HALT_CLEARED` / `SESSION_STAGE_DELIVERED`) | S / M |
| `src/tauri_events/bridge_subscriber.rs` | `route()` SignalingEvent → emit; `Lagged` → `session:resync` | M |
| `src/tauri_events/batch_emitter.rs` | message coalescing + per-session watermark | M |
| `src/tauri_events/fs_watcher.rs` | notify watcher: CL dir + dynamic repo/plugin dirs (`WatchSet`: owner-keyed, one refcounted watch per root, every owner notified — round 11), 500 ms debounce, one rescan per project | M |

**Entry points.** `main` boot order above · `builder()` in
`src/tauri_specta_gen.rs` · `get_session_runtime` · `route()` · `flush_once` ·
`spawn_fs_watcher`.

**Seams (§14):** 10 (BatchEmitter hop PINNED), 13/18/20 (the main.rs control
consumer — UNPINNED, no harness; its match and its broadcast FILTER must list the
same variants), 23 (FE hand-mirrors of `SessionRuntime` / activity payload),
26 (`collect_commands!` completeness — UNPINNED), 27 (event names ↔ FE literals —
UNPINNED both sides), 30.

**Gotchas → pointers.** CL `conventions.md` Tauri v2 gotchas (event names allow
no dots; camelCase INPUT args; `Handle::enter()` in setup; no root
`package.json`; `cargo run -- export-bindings`). The FE calls commands by
snake_case STRING (`useTauriQuery`/`invoke`), not the generated `commands.*`
wrappers — grep snake_case when hunting dead commands. `bindings.ts` regenerates
on launch: keep it out of feature commits.

**Tests pin.** command set constructs + spot export; timer/watermark/flush
(`batch_emitter.rs`); route arms emit typed events; fs-watcher pure scoping;
paths migrations idempotent; `sessions.rs` "no Brian/Rain in any rendered
payload". Not pinned: `Lagged`→resync, N=20 immediate flush, the control
consumer.

**Where to add X.** New command → `#[tauri::command] #[specta::specta]` in the
domain file → `collect_commands!` → relaunch regenerates bindings → FE
`useTauriQuery("snake_name")`. · New event → struct + `EVENT_NAME` in `types.rs`
→ emit in `route()` or at the call site → one `useTauriEvent` line in
`frontend/src/Providers.tsx`. · New boot sweep → `main.rs` `block_on` block,
non-fatal `Ok/Err→warn!` shape.

---

## H. Plugins

**What it does.** Plugin runtime v1: static bundles served as sandboxed iframes
over `bhq-plugin://` (Windows: one `bhq-plugin.localhost` host, id as path),
talking to the host only via postMessage → `plugin_invoke_proxy` → a versioned
14-entry capability catalog (no Tauri ACL path). Consent-gated install,
heartbeat crash recovery, panel tabs, per-plugin CSP extras (shipped).

| path | role | size |
|---|---|---|
| `src/plugins/mod.rs` | architecture summary (accurate) | S |
| `src/plugins/catalog.rs` | `CATALOG` + `required_capability`/`is_dispatchable` bundling rules | M |
| `src/plugins/heartbeat.rs` | ping/pong Healthy/Slow/Crashed (5 s / 1 s / 3 misses) | M |
| `src/plugins/manifest.rs` | manifest schema/validate, CSP-extra-origins (`validate_csp_extra_origins`) | M |
| `src/plugins/registry.rs` | `PluginRegistry`: heartbeat + 4 sync caches the URI handler reads without awaiting the DB (the disk loader nothing read was deleted in round 9) | M |
| `src/plugins/serve.rs` | asset resolution with traversal/symlink guards (managed + linked), CSP builder | M |
| `src/tauri_cmd/plugins.rs` | install/reapprove/reinstall/update/list/enable/disable/uninstall/preview/heartbeat feed (`_inner` helpers testable without Tauri) | XL |
| `src/tauri_cmd/plugin_api.rs` | `plugin_invoke_proxy` → `check_plugin_grant` → `dispatch` (one arm per catalog command; `spawn_session`, `plugin_session_*` ownership-fenced) | L |
| `frontend/src/app/PluginHost.tsx` | live iframe mount: bridge, heartbeat, crash fallback, spawn-consent dialog | M |
| `frontend/src/app/PluginManager.tsx` | Plugins tab | L |
| `frontend/src/app/PluginPanel.tsx` | route wrapper | S |
| `frontend/src/lib/pluginBridge.ts` | host-side postMessage validation/dispatch (spawn fails CLOSED without a confirm channel) | M |
| `docs/PLUGINS.md` | author contract (`api_version` 1) | — |
| `examples/hello-plugin/manifest.json`, `examples/hello-plugin/index.html`, `examples/hello-plugin/bhq-sdk.js` | template + integration fixture | — |

**Seams (§14):** 25 (invoke → grant → dispatch → kv — PINNED; catalog ↔
dispatch completeness fails LOUD, not silent).

**Where to add X.** New catalog command → `CATALOG` entry → `dispatch` arm (+
ownership fencing) → `docs/PLUGINS.md`. · New push topic → `PluginEventTopic` →
emit site in `PluginHost.tsx`. · New CSP directive → `CSP_EXTRA_DIRECTIVES` →
`CspExtraOrigins` → `build_plugin_csp` → docs.

---

## I. Claude config

**What it does.** Surfaces the user's real `~/.claude` config (skills, plugins,
hooks, CLAUDE.md, MCP, effort) that every agent inherits, with a read-only
inheritance lens per surface, explicit-Save write-back to the real
`settings.json` (the one sanctioned write outside `<data_dir>`), and a bot-hq
per-ROLE override layer merged into spawn `--settings`/env/mcp-config.

| path | role | size |
|---|---|---|
| `src/claude_config/mod.rs` | view types, `Inheritance` lens (7 surfaces) | M |
| `src/claude_config/reader.rs` | reads + resolves `~/.claude` → masked view (honours `CLAUDE_CONFIG_DIR`) | L |
| `src/claude_config/overrides.rs` | `ClaudeOverrides { all, per_role }` store (`<data_dir>/config/claude-overrides.json`), `resolve_agent_overrides`, `settings_fragment`/`env_vars`/`disabled_mcp` | M |
| `src/claude_config/writer.rs` | read-modify-write to `settings.json`, preserves unrelated keys | S |
| `src/tauri_cmd/claude_config.rs` | 6 thin commands | S |
| `frontend/src/app/ClaudeConfig.tsx` | Settings → Claude Config, 7 panes; skills/plugins/MCP write only `_all` (no per-role editor yet) | L |

**Seams (§14):** 4 (overrides → `--settings`/env — PINNED end to end; the
settings fragment reaches only edit-capable roles, env reaches all — pinned as
deliberate).

**Where to add X.** New overridable surface → `AgentOverride` field →
`settings_fragment`/`env_vars` → `Surface` variant + `inheritance()` arm → pane.
· Per-role editor for skills/plugins/MCP → mirror `AgentEffortOverride` +
`patchRole`.

---

## J. Frontend — session view + realtime plumbing — `frontend/src/`

**What it does.** The live session room (header roster, IPAV chat + doc panes,
halt/approval/tray answer surfaces, Stage/Send) and the realtime plumbing:
`Providers.tsx`'s `GlobalEventSync` turns backend `session:*` events into
TanStack Query invalidations + zustand store writes so every view stays live
without polling.

| path | role | size |
|---|---|---|
| `frontend/src/main.tsx`, `frontend/src/App.tsx`, `frontend/src/Router.tsx` | root, Providers→Router, route table | S |
| `frontend/src/Providers.tsx` | QueryClient + `GlobalEventSync` (22 listeners on 2026-08-18 → key invalidation + stores + the round-7 `session:stage_delivered` draft clear) + `onResync` fan-out + runtime backfill | M |
| `frontend/src/app/SessionView.tsx` | session container: header/roster, phase, halt/approval/chat/input orchestration, Stage delivery + tray-pick sync | L |
| `frontend/src/app/SessionContextTab.tsx` | session-scoped CL tree + lean editor | M |
| `frontend/src/app/SessionTerminalTab.tsx` | xterm.js over the session PTY | S |
| `frontend/src/components/ChatPane.tsx` | virtualised message list; owns `agent:messages:batch` | M |
| `frontend/src/components/ChatInput.tsx` | compose / Stage / Send / Pause / Resume, `@`-mention picker | L |
| `frontend/src/components/ChatMessage.tsx` | one row incl. tool_use/tool_result pills | M |
| `frontend/src/components/DocumentPane.tsx` | Tray tab + I/P/A/V doc tabs + custom-document tabs (untagged docs, round 11) + Apply-tab colored diff (memoised `groupDiffByFile`); `DocArticle` shared by phase and custom docs | L |
| `frontend/src/components/HaltBanner.tsx` | HALT state + the SOLE `isApproval`/`isTrayItem` predicates; its tray pointer calls `onOpenTray` (SessionView → DocumentPane's `openTraySignal`) | M |
| `frontend/src/components/ClosedSessionBar.tsx` | what a CLOSED session shows in place of its composer: read-only history + the **Reopen** button (`reopen_session`) — the only path that revives an archived roster (round 10) | S |
| `frontend/src/components/ApprovalGate.tsx` | Approve/Reject gate replacing the input | M |
| `frontend/src/components/PendingTray.tsx`, `frontend/src/components/ChoicePrompt.tsx` | topbar bell; one tray question | S |
| `frontend/src/components/ContextMeter.tsx`, `frontend/src/components/SessionFindingsBanner.tsx`, `frontend/src/components/SessionPhaseChip.tsx`, `frontend/src/components/PhasePill.tsx`, `frontend/src/components/HealthDot.tsx` | badges/chips | S |
| `frontend/src/components/SpawnBadge.tsx` | what a participant was ACTUALLY spawned with (migration 0061) — reads the recorded `*_at_spawn` pair, never `effort`/`ultracode`, which are the user's CHOICE and are "inherit" on almost every row. `spawn_knobs_recorded` separates "no override in force" from "predates 0061" | S |
| `frontend/src/stores/runtime.ts` | `SessionRuntime` hand-mirror + `busyBySlot`/`seedRuntimeStores` | S |
| `frontend/src/stores/activity.ts` | activity + per-slot busy, `isLocked` (D33) | S |
| `frontend/src/stores/health.ts`, `frontend/src/stores/context.ts`, `frontend/src/stores/chat.ts`, `frontend/src/stores/trayStaging.ts` | agent health/attention; context occupancy; messages + watermarks; staged tray picks (D34) | S |
| `frontend/src/hooks/useTauriEvent.ts`, `frontend/src/hooks/useInvoke.ts` | event subscribe; `useTauriQuery`/`useTauriMutation` (snake_case names) | S |
| `frontend/src/hooks/useDragResize.ts`, `frontend/src/hooks/useFocusTrap.ts`, `frontend/src/hooks/useEscapeKey.ts`, `frontend/src/hooks/useListEditor.ts`, `frontend/src/hooks/useServerDraft.ts` | small hooks | S |
| `frontend/src/hooks/useOsNotifications.ts` | needs-you → OS-notification escalation while unfocused (`session:pending_choice` + `session:awaiting_user` → queue → `planFlush`; lazy permission) | S |
| `frontend/src/lib/bindings.ts` | GENERATED by tauri-specta (`@ts-nocheck`); regenerates on app launch — never hand-edit | XL |
| `frontend/src/test-setup.ts` | jest-dom setup | S |

**Seams (§14):** 10 (batch → ChatPane), 12–14 (tray/halt/gate surfaces), 18
(Stage), 23 (hand-mirrored types — UNPINNED by construction; import from
`bindings.ts` where it already exports the type), 24, 27, 28.

**Gotchas → pointers.** NO horizontal scrolling, ever: every scroll container
pairs `overflow-y-auto overflow-x-hidden` (CL `conventions.md` §Design system).
Design tokens only (`frontend/tailwind.config.ts`). The `session:activity` payload
uses `slot0_busy`/`slot1_busy` slot names (renamed from the frozen agent names, `95a9124`). Rust enums wire out
as bare strings, so unions like `SessionActivity` are unavoidable mirrors.

**Tests pin.** slot wire (`Providers.test.tsx`, 3 of the listeners); header
roster, subtab mount-once, prompt/context dialogs (`SessionView.test.tsx`, no
Stage coverage); Stage/Send/Pause state machine, mention picker, lock gating
(`ChatInput.test.tsx`); byline + live-batch filtering (`ChatPane.test.tsx`);
tray attribution/discard/staging (`DocumentPane.test.tsx`).

**Where to add X.** New realtime event → const in `src/tauri_events/types.rs` →
emit → `useTauriEvent` in `Providers.tsx` (+ `_KEYS` const, fold into
`onResync`). · New per-session store → `stores/context.ts` shape + Providers
wiring + runtime backfill + `clearX` on `onClose`. · New command consumer →
`useTauriQuery`, import the return type from `bindings.ts`.

---

## K. Frontend — shell, dashboard, settings, shared atoms — `frontend/src/`

**What it does.** App shell/nav/footer health, the Dashboard (session list +
tiles + New Session dialog with the roster editor), all of Settings (Roles /
Models / Policy / Tool Gate / Violations / Feedback / Archive / Updates / Claude Config
tabs — Plugins is its own `/plugins` route), and the shared presentational atoms + roster/colour/time libs
every other FE area imports.

| path | role | size |
|---|---|---|
| `frontend/src/app/Shell.tsx` | nav shell, shortcuts, footer health dot | S |
| `frontend/src/app/Dashboard.tsx` | session list/filter/tiles + New Session dialog (`MAX_PARTICIPANTS = 4` dialog cap; backend 8) | L |
| `frontend/src/components/SessionTile.tsx` | one tile: health/phase/attention/Quickview | M |
| `frontend/src/app/Settings.tsx` | tab container (lazy-mount-then-keep); Policy/Archive/Updates live here | M |
| `frontend/src/app/RolesPanel.tsx` | role CRUD, capability grid, participation mode | L |
| `frontend/src/app/ModelsPanel.tsx` | saved-model registry + connection test | M |
| `frontend/src/app/SessionPolicyPanel.tsx`, `frontend/src/components/PolicyForm.tsx`, `frontend/src/components/GatedKeywordList.tsx` | session gear drawer; shared policy editor; gated keyword rows | M / S / S |
| `frontend/src/app/ViolationsPanel.tsx`, `frontend/src/app/FeedbackPanel.tsx`, `frontend/src/app/MeasurementView.tsx` | violations viewer; feedback triage; CL retrieval stats | M / S / S |
| `frontend/src/app/PromptcodesPanel.tsx` | Settings → Promptcodes: `{code, prompt}` pairs the composer's `/` picker expands (`app_settings` key `promptcodes`) | M |
| `frontend/src/components/UpdateBanner.tsx` | dismissible update banner | S |
| `frontend/src/components/DiagnosticsAskCard.tsx` | one-time diagnostics opt-in card (Shell chrome; both answers mark asked) | S |
| `frontend/src/components/ui/Button.tsx`, `frontend/src/components/ui/Card.tsx`, `frontend/src/components/ui/Input.tsx`, `frontend/src/components/ui/Select.tsx`, `frontend/src/components/ui/SegToggle.tsx`, `frontend/src/components/ui/Skeleton.tsx`, `frontend/src/components/ui/Textarea.tsx` | base atoms (`Select.tsx` is the house `selectClass` — since round 11 applied by Models/Roles/ClaudeConfig/the New Session dialog with no wrapper component; `Skeleton` since round 11: the loading-rows idiom five panels spelled by hand; `Button` carries the house `rounded` and a focus-visible ring since round 11) | S |
| `frontend/src/components/icons.tsx` | THE hand-rolled SVG icon set — two bases kept deliberately (attribute-sized stroke 1.75 `Svg`; class-sized stroke 2 `ClassSvg`, the family moved in from the CL module in round 9); the near-twins `MemoryIcon`/`FileIcon` and `RescanIcon`/`RefreshIcon` await a visual decision | S |
| `frontend/src/components/Markdown.tsx`, `frontend/src/components/ErrorBanner.tsx`, `frontend/src/components/ConfirmDialog.tsx`, `frontend/src/components/SubTabButton.tsx` | shared atoms | S |
| `frontend/src/components/authorColor.ts` | label → hue class; D20 8-hue palette named by colour | S |
| `frontend/src/lib/participants.ts` | `ParticipantView`, label/slug/slot-key resolution, `isSpawnable` (stale mirror), runtime keys | M |
| `frontend/src/lib/effort.ts` | the effort model post-no-inherit: `EFFORT_LEVELS`/`ULTRACODE`/`DEFAULT_EFFORT` + choice↔stored-pair mapping shared by the dialog and the Roles tab; `DEFAULT_EFFORT` is pinned to the Rust floor by `overrides.rs::frontend_default_effort_matches_the_rust_floor` | S |
| `frontend/src/lib/participantNames.ts` | `UNKNOWN_PARTICIPANT` leaf (breaks a cycle) | S |
| `frontend/src/lib/time.ts`, `frontend/src/lib/phase.ts`, `frontend/src/lib/diffGroups.ts`, `frontend/src/lib/cn.ts`, `frontend/src/lib/sessionId.ts`, `frontend/src/lib/staging.ts`, `frontend/src/lib/filePaste.ts`, `frontend/src/lib/tokenExpand.ts` | time/phase/diff-grouping/clsx/short-session-id/staged-answer/file-paste/token-expansion helpers (`stagedKey` + `picksDiffer` are the re-stage effect's pure half; `uriListToPaths`/`pathsToInsertText` are the composer's paste-drop half) | S |
| `frontend/src/lib/telemetry.ts` | hand-mirrored `TelemetryStatus` + `shouldShowDiagnosticsAsk` + PRIVACY_URL | S |
| `frontend/src/lib/osNotifications.ts` | OS-escalation pure policy: `planFlush` dedupe/60s-cooldown/burst-coalesce (≥3 → “N sessions need you”) + the on/off localStorage pref | S |
| `frontend/src/lib/attention.ts` | idle-unflagged badge label + tooltip; single source for SessionTile and SessionView, which had it duplicated verbatim | S |
| `frontend/src/lib/framing.ts` | retired-agent / pair-assumption detection for user-facing strings; `framing.test.ts` sweeps `frontend/src/` with it | S |
| `frontend/src/index.css`, `frontend/tailwind.config.ts`, `frontend/package.json`, `frontend/vite.config.ts`, `frontend/tsconfig.json` | tokens + build config | — |

**Seams (§14):** 23 (`ParticipantView` mirror), 26–27 (command/event string
seams), 28 (mention picker slugs).

**Gotchas → pointers.** Design system: `docs/design/industrial_terminal/DESIGN.md`
+ mocks under `docs/design/`; colour is keyed to roster place + per-participant
override (D20), never to a person. Raw Tailwind palette classes are banned; the
`text-sm`/`font-medium` residue is in-progress token migration.

**Where to add X.** New Settings subtab → `SettingsSubTab` union + `SubTabButton`
+ lazy-mount block. · New participant-row field → `ParticipantRow` →
`emptyParticipant()` → dialog JSX → `handleCreate` → backend DTO. · New shared
primitive → `components/ui/`, not `contextLibraryShared.tsx`.

---

## L. Frontend — Context Library UI — `frontend/src/app/`

**What it does.** The Context Library screen: a 2-pane explorer (workspace
sidebar tree + tabbed editor) over the CL folders, plus the per-project
ContextManager (measurement + rescan) and the session-side FileViewerDialog.

| path | role | size |
|---|---|---|
| `frontend/src/app/ContextLibrary.tsx` | container: query/tab/tree state, context menu, drag-resize sidebar (no dirty-tab awareness — audit L1) | L |
| `frontend/src/app/ContextLibrarySidebar.tsx` | `WorkspaceSidebar` tree (Projects/Global/System) | L |
| `frontend/src/app/ContextLibraryEditor.tsx` | `EditorArea`/`TabStrip`/`EditorPane`/policy.yaml editor; only the active tab is mounted; dirty-vs-`cl:changed` sync | L |
| `frontend/src/app/ContextLibraryFolderView.tsx` | folder tab: description editor + project register/rename/unbind/delete (with confirms) | M |
| `frontend/src/app/ContextLibraryContextMenu.tsx`, `frontend/src/app/ContextLibraryRegisterModal.tsx` | menu + action modal; register-project dialog | S / M |
| `frontend/src/app/contextLibraryShared.tsx` | tree types, `buildTree`, `splitGlobals`, tab helpers, `terminalInputClass`/`FieldLabel` (its icons moved to `components/icons.tsx` in round 9) | M |
| `frontend/src/app/ContextManager.tsx` | per-project measurement + rescan | S |
| `frontend/src/components/FileViewerDialog.tsx` | full-screen file viewer used from tray cards | M |

**Seams.** All 20 `src/tauri_cmd/cl.rs` commands have a caller here;
`cl:changed` is handled centrally in `frontend/src/Providers.tsx` (whole-project
refetch, deliberate).

**Where to add X.** New CL file action → `CtxAction` → `menuItems()` →
`ActionModal` → a `cl_*` command. · New special-filename editor → predicate next
to `isPolicyFile` → branch in `EditorAreaImpl`.

---

## M. Docs + repo hygiene

| path | role |
|---|---|
| `ARCHITECTURE.md` | what bot-hq IS (20 H2s; the audit's drift table lists what to refresh) |
| `PLAN.md`, `PROGRESS.md`, `CLAUDE.md`, `README.md`, `INSTALL.md` | forward plan · newest-first changelog · session instructions · public doc · install |
| `docs/plans/` | 26 planning docs (2026-08-17); BINDING: `docs/plans/2026-08-11-rc3-decisions.md`, `docs/plans/2026-08-12-rc3-reframe-contract.md`; the rest are dated handoffs (stale by construction) or queues |
| `docs/PLUGINS.md`, `docs/SIGNING.md`, `docs/WINDOWS-TESTING.md`, `docs/stream-json-events.md`, `docs/design/`, `docs/rebuild-archive/` | contracts, release notes, schema notes, mocks, frozen history |
| `tests/codebase_map_test.rs` | pins THIS map to the tree both ways (every source file placed; every named path exists) — the map's anti-staleness device |
| `tests/retired_identifier_test.rs` | no source IDENTIFIER names a D10-retired agent: splits on `_` and compares SEGMENTS, which is what `-w` could not do (`_` is a word char, so three audit rounds could not see `has_rain`). Comments and bare words exempt by design; one file carved out with a reason (`src/paths.rs`) |
| `tests/retired_symbol_prose_test.rs` | no COMMENT names a deleted symbol (`HANDS_ONLY_TOOLS`, `break_volley`, `external_jsonrpc`, `set_busy(`, …) without a retirement marker on the SAME line — the CL's `RETIREMENT_MARKERS` rule applied to `src/`, `tests/`, `frontend/src/` (round 12; the class three sweeps re-edited by hand). Code lines exempt (a live reference would not compile; literals are specimens); `signaling/parity.rs` carved out with a reason; a second test refuses a listed symbol that is still alive in production code |
| `frontend/src/lib/fonts.ts` | the `@font-face` sources `index.css` declares and where each must live — `fonts.test.ts` reads the REAL `index.css` + `public/` and pins root-absolute `/fonts/…` URLs to existing files. Exists because all three Industrial Terminal fonts 404'd in every built app until 2026-08-17 (CSS-relative `./fonts/…` against files in `public/fonts/`), and no gate could see it |
| `frontend/src/lib/overflow.ts` | pure detection for containers that can scroll HORIZONTALLY — the user's absolute no-horizontal-scroll rule, mechanically enforced for the first time by `overflow.test.ts`, which is its only caller (same shape as `framing.ts`). Line-scoped, comment-exempt, exemption-free |
| `tests/portable_home_test.rs` | no source file reads `var("HOME")`/`var_os("HOME")` outside `src/paths.rs` — `HOME` is UNSET on native Windows (it lives in `USERPROFILE`), and Git Bash sets it, so the gap is invisible from a developer's terminal and only appears in the GUI-launched app. Two live sites failed SILENTLY on 2026-08-25: `signaling/server.rs` forwarded zero user MCP servers, and `core/telemetry.rs` hashed panic paths unredacted. Matches the CALL, not the bare word, so prose is exempt; asserts the source walk is non-empty so a broken walk fails rather than reporting "clean" |
| `tests/phase_vote_wiring_test.rs` | every storage method the D37 phase-advance vote needs has a caller OUTSIDE its defining file — round 5's E1, where `bump_phase_epoch` was defined, tested seven times and called by nothing, leaving the epoch at 0 through 114 live transitions. A `pub` method on a `pub struct` is never `dead_code`, and a test that calls it does not pin its mount |
| `examples/dump_role_prose.rs` | LIVE: generator for the role-prose reseed migrations |
| (deleted) the two native-loop research spikes (native_loop, subscription_loop) rc3 D9 closed — removed 2026-08-17 (round 7); recoverable from git history at a1475e7 | historical |
| `scripts/`, `packaging/`, `site/`, `templates/`, `start`, `frontend/scripts/check-import-cycles.mjs` | turn-latency tool · Homebrew cask · landing page · CL seeds · dev launcher · the import-cycle gate `start` runs |
| `Cargo.toml`, `frontend/package.json`, `.gitignore`, `.env` | deps + config |

**Where to add X.** New canonical fact → update the ARCHITECTURE.md H2 AND grep
the other sections + README for restatements. · New decision → append `D<n>` to
the decisions doc, then fix the D-range citations in PLAN.md. · New MCP tool →
mirror into ARCHITECTURE.md + README tool lists (both drift; regenerate from the
registry).

---

## 13. Cross-cutting traces

1. **User message → agent.** `ChatInput` Send → `broadcast_message` /
   `send_user_response` (`src/tauri_cmd/messages.rs`, `src/tauri_cmd/tray.rs`)
   → `AppState::broadcast` (`src/core/state.rs`) → `broadcast_user_message` row
   (`src/core/broadcast.rs`) → `user_responded` (clears the halt slot, emits
   `HaltsCleared`) → `notify_ring_user_message` → `SequencerCommand::UserMessage`
   → `advance_turn` → `hand_turn_to` (`set_current_turn`, busy) → `deliver_backlog`
   → `deliver_batch` (`src/agents/spawn.rs`) → stdin → agent → stream-json →
   `pump_agent` (`src/core/pump.rs`) → `post_to_channel` row + `notify_message_persisted`
   → `bridge_subscriber` → `BatchEmitter` → `agent:messages:batch` → `ChatPane`.
   Stage: `stage_user_response` holds the text in `AppState` memory →
   `MessageStaged` → boundary → `StagedDeliveryDue` → main.rs → `deliver_staged`.
2. **MCP tool call.** agent → `/sessions/<id>/<slug>/mcp` → `handle_request` →
   `dispatch` → capability gate → `call_tool` (`src/signaling/jsonrpc.rs`) →
   bridge fn (`src/signaling/bridge/`) → storage → `SignalingEvent` → subscriber →
   Tauri event → `Providers.tsx` → store/query. Questions park (`session_tray`
   row + `PendingChoice`); answers return via `resolve_choice_confirmable` →
   `deliver_oob` (a user-authored row) → `user_responded`.
3. **git commit / push.** hook → `bot-hq policy-check <hook>` (`src/policy/hooks.rs`)
   → forbidden words on the message / added lines → immutable-migration guard →
   `check_findings_gate` (read-only sqlite, `BOT_HQ_SESSION_ID`) → exit code;
   pre-push → `Policy::resolve` → `/hooks/pre-push` → `request_approval` (tray
   gate latches the ring) → exit code. Bash → PreToolUse hook →
   `policy-check tool-gate` → `/hooks/tool-gate` → `park_gated_command` → tray →
   Approve → `execute_gated`.
4. **Session open / close.** `create_session` → `open_session`
   (`src/core/session.rs`) → roster seed → per participant: prompt compose →
   spawn config → `spawn_supervised_agent` → pump + epoch cell → `spawn_ring` →
   `boot_then_start` (BOOT, then first `UserMessage`) → watchdog. Close (UI or
   MCP) → `close_session` → `close_learnings::decide` → epilogue turn or
   `teardown_session` (kill, `storage.close_session`, withdraw tray, policy
   snapshot cleanup, `unregister_session`, worktree, `SessionClosed`).
5. **CL write.** `cl_write_file` (MCP) → path guard → atomic write →
   `git_version_library` (library repo commit) → `cl_rescan` → index/atoms →
   `mark_cl_rescan` (close gate) → detached `scan_then_push` (secret scan, then
   push). UI twin: `src/tauri_cmd/cl.rs`. fs watcher → `cl:changed` → FE refetch.
6. **Plugin invoke.** iframe postMessage → `pluginBridge.ts` → `plugin_invoke_proxy`
   → `check_plugin_grant` (catalog ∧ enabled ∧ consent) → `dispatch` → storage /
   `AppState` (ownership-fenced) → response; heartbeat sweep in `main.rs` →
   `plugin:crashed`.

---

## 14. Seams — the joins between areas (from the 2026-08-15 audit)

A seam is PINNED only if deleting the join line reddens the suite; a test of each
half does not count. Details, evidence and the cheapest pins are in
`docs/plans/2026-08-15-rc3-audit.md` §W.

| # | seam | join | status |
|---|---|---|---|
| 1 | role prose + roster facts + rules + CL primer → system prompt (F/B2 → A) | `compose_system_prompt` `src/core/session.rs` | PINNED (extracted) |
| 2 | composed prompt → file → `--append-system-prompt-file` (B2 → A) | `participant_spawn_config`, `build_command` | PINNED |
| 3 | capabilities → allowed/disallowed tools / bypass (F → A) | `participant_capabilities` → `build_command` | PINNED |
| 4 | role Claude overrides → `--settings` + env (I → A) | `resolve_participant_overrides` → `settings_fragment`/`env_vars` | PINNED |
| 5 | per-agent mcp-config (signaling addr + user MCPs) (C1 → A) | `mcp_config_json` | PINNED |
| 6 | spawn env `BOT_HQ_SESSION_ID`/`BOT_HQ_AGENT` → git hooks (A → E) | `cmd.env` in `build_command` AND in `SessionTerminal::spawn` → `hook_session_id` | PINNED at both producers (`build_command_sets_the_session_id_the_hooks_read`, `the_pty_carries_the_session_id_the_hooks_read`) and at the consumer's block arm (`the_findings_gate_blocks_a_session_commit_and_skips_a_human_one`, round 8) |
| 7 | PreToolUse hook fragment → `run_tool_gate` (A → E) | `--settings` hook cmd → `hooks.rs` | PINNED as halves + argv |
| 8 | ring step → deal → persist (F ↔ B1) | `next_active_participant` → `hand_turn_to` → `set_current_turn`/`set_round_number` | PINNED end to end |
| 9 | deal → stdin → commit (B1 → A → F) | `deliver_backlog` → `deliver_batch` → `commit_delivery` | PINNED; failure path is `warn!` only (no event, no row) |
| 10 | pump → row → UI (B1 → C2 → G → J) | `notify_persisted` → `route` → `BatchEmitter` → `agent:messages:batch` | pump→notify UNPINNED; downstream PINNED |
| 11 | epoch cell shared by pump and ring (B2 wiring) | `deps.epochs[p.id]` vs `PumpConfig.turn_epoch` per slot | publish PINNED (`the_epoch_is_published_to_the_holder_before_its_rows_go_out`); the `session.rs` slot pairing UNPINNED (the epoch test installs its own cells) — a mis-pairing wedges the ring after turn 1; since round 11 both cells are filled in ONE pass |
| 12 | `ask_user_choice` park → answer → OOB row → ring (C1 → C2 → F → J → B1) | `ask_user_choice_inner`, `resolve_choice_confirmable`, `deliver_oob`, `user_responded` | PINNED at the bridge; core hop pinned by source-grep only |
| 13 | halt declare/clear (C2 → F → G → J; clear in B1) | `emit_halt_row`, `declare_session_halt`, `halt_declared` (main.rs), `user_responded` | bridge→ring PINNED; main.rs interrupt hop UNPINNED; clear pinned by grep only |
| 14 | approval gate latch/lift (C2 ↔ B1) | `notify_ring_gate(true/false)`, `Storage::pending_gate_ids` seed | PINNED end to end — `resolving_a_gate_lifts_the_latch_and_the_ring_deals_again` (durable row → latch → resolve → deal); the custom-menu lift by `a_custom_menu_approval_lifts_the_gate_it_latched` (round 11) |
| 15 | tool-gate hook ↔ `/hooks/tool-gate` (E ↔ C1 ↔ C2) | `park_gate` ↔ `handle_tool_gate` (inline literals both sides) | halves PINNED; HTTP join UNPINNED |
| 16 | `eyes_flag` → pre-commit gate (C2 → F → E) | `insert_finding` ↔ `query_open_blocking` (predicate restated 3×) | PINNED at the row; reviewer-down branch exists only in the MCP tool |
| 17 | `check_commit_message` ↔ commit-msg hook (C1 ↔ E) | one fn `first_forbidden_word` | PINNED (compile-time) |
| 18 | Stage (J → G → B1 → C2 → main.rs → B1 → J) | `stage_user_response` → `MessageStaged` → `StagedDeliveryDue` → `deliver_staged` | ring PINNED; core + main.rs hops UNPINNED |
| 19 | session open registrations (B2 → C2) | `register_session{,_awaiting,_activity,_phase}` inline in `spawn_session_handle` | reviewers + sequencer PINNED; the four inline ones UNPINNED |
| 20 | session close (C1 → main.rs → B1/B2 → C2 → G) | `SessionCloseRequest` → `close_session` → `teardown_session` | tool→event PINNED; main.rs hop + teardown order UNPINNED |
| 21 | CL write → index (C2 → F) | `cl_write_file` → `git_version_library` → `cl_rescan` | PINNED end to end |
| 22 | CL push after write; close-gate flags read before teardown | `push_library_after_write`; `close_gate_flags` in `close_epilogue_decision` | push PINNED; read-before-teardown order UNPINNED |
| 23 | FE hand-mirrors ↔ Rust structs (G ↔ J/K) | `stores/runtime.ts`, Providers activity payload, `ParticipantView` | UNPINNED by construction (import from `bindings.ts` where possible) |
| 24 | watchdog → attention → UI (B2 → C2 → G → J) | `notify_session_attention` → `session:attention` | decision PINNED; loop/dedupe/route/escalation UNPINNED |
| 25 | plugin invoke → grant → dispatch → kv (H → F) | `check_plugin_grant`, `dispatch` | PINNED; misses fail LOUD |
| 26 | `collect_commands!` ↔ `#[tauri::command]` set (G) | `src/tauri_specta_gen.rs` | UNPINNED (source-grep test recommended) |
| 27 | event-name consts ↔ FE listener literals (G ↔ J/K) | `src/tauri_events/types.rs` ↔ `Providers.tsx` et al. | UNPINNED both sides |
| 28 | mentions: picker → `parse_mention_slugs` → summons (J → B1) | `resolve_mentions` → `notify_ring_user_message(mentions)` | parser + ring PINNED; slug→id→ring hop UNPINNED |
| 29 | capability set → MCP tool gate (A ↔ C1) | `required_for` (one map, four consumers) | DERIVED + PINNED |
| 30 | boot orphan sweeps (G → F) | `halt_orphaned_busy_sessions`, `withdraw_pending_tray_for_closed_or_orphaned` | storage PINNED; main.rs call sites UNPINNED; predicate reads fail-open telemetry |

---

## Keeping this map current

- Add/move/delete a source file → edit its row (the test tells you which).
- A new area seam → add a §14 row with its pinning test (or `UNPINNED` + the
  cheapest pin), and cite it from both areas' "Seams" lines.
- Behaviour changes go to ARCHITECTURE.md; this file only says WHERE and WHAT
  JOINS WHAT.
