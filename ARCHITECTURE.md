# bot-hq — Architecture

This is the single source of truth for what bot-hq IS right now. It
describes the running system, not the original rebuild design — that
lives at [`docs/rebuild-archive/ARCHITECTURE-rebuild-era.md`](docs/rebuild-archive/ARCHITECTURE-rebuild-era.md).

For user-facing setup see [`README.md`](README.md). For what changed
per release (and what's deferred) see [`CHANGELOG.md`](CHANGELOG.md).
For WHERE things live — the codebase split
into areas with files, entry points, seams and tests — see
[`CODEBASE.md`](CODEBASE.md) (this file says what bot-hq does; that one
says where).

---

## Overview

bot-hq is a desktop GUI app for driving AI-assisted coding sessions
through an agent harness — N role-playing participants — with policy enforcement. Each
session spawns participants the user picks from their ROLES:

- A role holds a set of **capabilities** (ticked in Settings → Roles) and its own
  instruction prose. A FRESH install seeds one neutral role — `agent`, ordinary session
  capabilities, no prose (migration 0072) — plus a one-time Roles-tab offer to
  install the example pair, **HANDS** (executes) + **EYES** (adversarial
  reviewer); declined, the offer never returns, and an upgrading install never
  sees it. The author's own install keeps its HANDS/EYES rows — they are that
  user's configuration, not bot-hq's furniture.
- A participant is displayed as `ROLE · Model` and is never named after a person.

Every participant is backed by a `claude-code` subprocess. **Tool access is gated
on the participant's invite-time capability snapshot**, not on any name —
`signaling/jsonrpc.rs` states this in its module doc.

The spawn-side write denial is `--disallowedTools` subtraction, which is
fail-open: a deny-list cannot anticipate every mutation verb — `rm -rf`, `mv`,
`chmod`, `npm install`, and file-writing shell forms like `sed -i` all pass it.
The capability gate covers the MCP tools; the shell surface is covered by role
prose and the Tool Gate, not by the deny-list.

**There used to be a second backend** — bot-hq's own native Rust agent loop,
opted into per saved model. rc3 D9 deleted it (2026-08-12): the claude CLI is
the default model connector (the Codex CLI joined it 2026-10-21, per model row — D9 amended), "for uniformity", and the native connector returns
later as a plugin. Git history is the archive — `git show
c7bba28:src/agents/native/` is where it starts from.

bot-hq is an **agent harness** — describe it that way, never by agent count. A
session runs N participants (dialog default 1, dialog cap 4, backend cap 8),
each playing a role the user defined. A fresh install is born with one neutral
`agent` role and the optional example-pair offer (1.0.0 Batch 4); every other
role is the user's own creation.

A former helper agent, **Emma**, was removed from the core; a return as a
plugin is possible but unplanned (it is not on the plugin ideas list below).

The user directs the work and owns the decisions; the app is the bridge
between user and agents. Policy enforcement runs at two layers (MCP tool calls + git
hooks). One MCP server runs in-process, for agent ↔ UI signaling.

**Stack:** Tauri v2 shell + React 18 frontend, Rust core on a tokio
multi-thread runtime. Tauri owns the OS main thread.

---

## Process model

```
                    ┌────────────────────────────────────────┐
                    │  bot-hq (Rust binary, main thread)     │
                    │                                        │
                    │  Tauri webview ◄──── AppState (Arc) ───┤
                    │                       │                │
                    │   ┌───────────────────┴─────────────┐  │
                    │   │  tokio runtime (worker threads) │  │
                    │   │   - signaling::SignalingBridge  │  │
                    │   │   - internal MCP HTTP server    │  │
                    │   │   - per-session turn ring       │  │
                    │   └─────────────────────────────────┘  │
                    └────────┬────────────┬──────────────────┘
                             │            │
                    ┌────────▼─────┐  ┌───▼─────────┐
                    │ claude-code  │  │ claude-code │
                    │ participant 1│  │ participant N│
                    │ stream-json  │  │ stream-json │
                    └──────────────┘  └─────────────┘
```

Every agent is a subprocess.

### One agent backend

`spawn_session_handle`'s per-participant loop has no branch: every participant goes through
`spawn_supervised_agent`, whatever model row it carries. A model whose
gateway does not speak the Anthropic Messages API therefore fails at spawn —
`validate_model`'s pre-flight (the **Test** button in Settings → Models) is
what surfaces that at configure time instead.

**A second backend existed until rc3 D9** (2026-08-12): a native Rust agent
loop, in-process, opted into per saved model, EYES-only. It was deleted
outright rather than feature-flagged, because a second runtime nobody builds
still costs every reviewer a re-read and every refactor a second case. What
made the deletion cheap is what made the loop additive in the first place:
`AgentHandle` is a pure channel struct — `core/pump.rs`, `core/sequencer.rs`, the
policy layer, the UI event path and the context meter speak only `AgentEvent`
and `OutgoingUserMessage` — so nothing downstream ever knew which backend it
had, and nothing downstream changed when one went away.

Each agent subprocess is spawned with:

```
claude -p \
  --input-format stream-json --output-format stream-json --verbose \
  --append-system-prompt-file <path> \
  --settings <per-role overrides json> \
  --mcp-config <per-agent-config.json> \
  --strict-mcp-config \
  --dangerously-skip-permissions
```

`--dangerously-skip-permissions` is intentional: bot-hq IS the policy
layer. claude-code's own permission prompts would double-gate and hang
the agent (the bot-hq policy gates already prompt the user). Enforcement
is provided by the policy layer + git hooks.

Two CLI behaviours are switched at spawn because they work against a ring
(both probed on CLI 2.1.284):

- **The commit attribution.** The CLI tells an agent to end commits with a
  co-author trailer. When the session's resolved policy forbids a word
  that trailer or the PR footer contains (`Policy::forbids_cli_attribution` —
  the hook's own matcher against the CLI's own text), `--settings` carries
  `"attribution": {"commit": "", "pr": ""}`, written last so no model row or
  role override brings it back. Otherwise the CLI's default is left alone.
- **The check-in reminder.** `CLAUDE_CODE_SILENT_TURN_REMINDER=0` stops the
  CLI's "the user hasn't heard from you in a while" nudge: every mid-turn line
  is stored, shown, and delivered to the peers, and the composer's status line
  shows the tool call in flight instead (`frontend/src/lib/runningTool.ts`).
  A value already present in bot-hq's own environment is inherited untouched.

Per-agent model swap via env-vars: `ANTHROPIC_BASE_URL`,
`ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_MODEL`. A model row may also carry
`models.cli_settings` (0079): a JSON object merged into the `--settings` argument
on **both** spawn postures — the read-only posture's `--settings` otherwise
carries only the compaction hooks and the zsh-trap lint. Its first use is
`modelOverrides`: the CLI resolves a model's context window from its own catalog,
so a model id newer than the installed CLI (`claude-fable-5-1` on 2.1.251) runs at
the 200k default whatever the registry says; mapping a known id to it restores the
real window. A role's own Claude-config override wins on any key both set. The
pump compares the CLI's first reported window with `models.context_window` and
posts one channel notice when they disagree by more than 20 %. The
`agent_configs` fallback tier has no such column, so it borrows `cli_settings`
from the registry row spawning the same `model_name` when one exists. The model is
resolved **per participant**, in this order
(`core/session.rs::resolve_participant_config` → `resolve_spawn_config`):

1. `session_participants.model_id` — the create dialog's pick for that
   participant;
2. else the participant's ROLE default (`roles.model_id`), re-read at spawn so a
   roster seeded before the role had a default still gets the current one;
3. else the `agent_configs` row for that agent name;
4. else a built-in default, with a warning naming the participant.

Whichever id wins is looked up in the saved-model `models` registry for the
provider, base URL and token (its `context_window` is copied into the spawn
config but read by nothing — see the two-unread-columns note below; the live
context meter divides by the window claude-code reports per turn).

`BOT_HQ_SESSION_ID` is also injected so git-hook subprocesses can read
session-scoped state.

**The child's account is explicit (migration 0090).** Before any env of its
own, every spawn — the live participant (`agents/spawn.rs::build_command`) and
the one-shot summarizer / Models "Test connection" probe
(`tauri_cmd/docs.rs::headless_claude_cmd`) — runs
`agents::spawn::apply_account_env`: the inherited auth/billing variables
(`AUTH_ENV_SCRUB`: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
`ANTHROPIC_BASE_URL`, `ANTHROPIC_PROFILE`, `ANTHROPIC_CONFIG_DIR`,
`CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_OAUTH_TOKEN` + its refresh / descriptor forms,
`CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR`, the Bedrock / Vertex / Foundry switches)
are removed, then `CLAUDE_CONFIG_DIR` is set when the model row names a
Claude config dir (`models.claude_config_dir`; blank = the CLI's default
`~/.claude`). claude-code keeps one credential per config dir, so a second
subscription runs beside the first only when each has its own dir and a model
row names it ("Fable 5.1 · acct 2"). The dir is resolved ONCE per participant,
at its first spawn, and recorded on the row
(`session_participants.account_dir_at_spawn`, with the signed-in email and
organisation id `claude auth status --json` reported —
`core/session.rs::resolve_participant_account`); every later spawn reuses the
record, because the CLI's session store lives inside the dir and a `--resume`
anywhere else starts blank. A first spawn into a custom dir the CLI reports as
logged out is refused with the login command; an unreadable status never is.
That command, and the Model dialog's setup + optional config-share commands
(`account_setup_commands`, which checks the dir by Save's rules first), come
from one generator, `agents::account_setup`, in the shell of the machine bot-hq
runs on: POSIX sh, or Windows PowerShell 5.1 on Windows (junctions for the
shared folders, `mklink` for the two files, nothing ever deleted). A
dir is a credential slot, not an account (`/login` swaps the account behind the
same dir), so a resume that finds a different email posts a notice, and the
usage-limit mark (`account_marks`, 0091) is keyed by dir + organisation +
model (claude.ai limits are per model) against the identity reported at THIS
spawn: set from the CLI's own limit message for a subscription-billed
participant, shown as "limited until …" on the Models list and the New Session
dialog for that model's rows, and cleared by a clean turn on that account and
model that started after it, or by the user — advisory, never a refusal and
never a re-route. `init`'s `apiKeySource` is read on every spawn: a
subscription participant reporting anything but `none` (the measured
subscription value; `/login managed key` is the Console key) gets one notice,
and a dir whose `authMethod` is not `claude.ai` is treated as a Console login
— refused for a first spawn into a custom dir, noticed otherwise.

**LLM proxy (`src/agents/llm_proxy.rs`):** agents pointed at a
non-Anthropic Anthropic-compatible gateway (e.g. DeepSeek) route
their `ANTHROPIC_BASE_URL` through a local normalizing reverse-proxy. It
hoists the `role:"system"` entry claude-code injects into the
`messages[]` array (from a SessionStart hook's `additionalContext`) up
into the top-level `system` field, which strict gateways require.
Participants on the real first-party API bypass it.

The proxy sits on the only path there is. It used to be described as a
"CLI-path fixup", because the native loop built its request body itself and
put `system` at the top level to begin with; with one connector left, every
gateway-backed agent routes through it.

---

## Role prompts (user-owned data, seeded from the binary)

**A role's instruction prose lives in the `roles` table and is edited in
Settings → Roles** (rc3 D8/D10). It is the user's, not the product's: migration
0046 seeded `roles.description_prompt` byte-for-byte from the constants in
`src/agents/prompts.rs`, 0048 cleared the `builtin` flag so bot-hq claims no
ownership, and 0049 removed the agent names from the seeded text. Later
reseeds (0050, 0055, 0065, 0067, 0068) move the text the same way: each UPDATE is
guarded on the previous seed (`description_prompt = '<previous>' OR IS NULL`),
so a row the user edited is never touched and a cleared row takes the new
built-in; `seeded_role_prose_is_byte_identical_to_the_hardcoded_constants`
pins the migrated text to the constants.

Since 1.0.0 Batch 4 the constants are PRESET payload only
(`PRESET_HANDS_ROLE`/`PRESET_EYES_ROLE`, installable from the Roles tab's
one-time offer): the prose FALLBACK is deleted, so an empty
`description_prompt` means an empty role — "a default role with no instruction
prompt" is expressible, and clearing the box no longer resurrects 11.8 KB the
user just removed. (The old text continued: `resolve_role_prose` →
`builtin_prose_for_role`, keyed on the role slug). So clearing the box in the
Roles tab restores the built-in text rather than producing a role with no
instructions.

This inverts the old rule. The prompts used to be hardcoded *specifically* so a
CL edit could not break a role boundary; that protection now comes from
**layering order** instead: the generated capability section (spawn layer 6
below; layer 2 of the three-layer design) is emitted after every editable
input, so free text cannot grant itself
something the gate does not enforce. Migration 0044's schema comment states the
invariant — *"a role must not be able to author rules that contradict its own
capability set."*

CL still supplies per-project and per-user customisation on top.

System-prompt layering at session spawn (`src/core/session.rs::read_system_prompt`):

1. The role's instruction prose (`roles.description_prompt`, falling back to
   the seeded constant)
2. CL location anchor (`<data_dir>` path)
2b. Project CL index primer (when the session has a project) — the
   `cl_index_search` rows for the project (`file_path — description`,
   most-recently-updated first, capped). The table of contents only.
2c. Project focus (when the session has a project root) — the FULL body
   of `<project CL>/focus.md`, the open work scopes' knowledge base (one
   section per scope; a project can run several concurrently),
   identical for every participant (scope-watch is a review dimension).
   Absent file → a one-line creation trigger instead. Oversized bodies
   render head + tail with a loud middle marker (the tail carries the
   newest truths under append-as-you-learn); >32 KB or ≥5 line-start
   `SUPERSEDES:` markers append a reorganize advisory. Position is
   load-bearing: never below layer 6, which governs who may write it.
3. Hardcoded universal rules (`agents::general_rules::GENERAL_RULES`)
4. `<data_dir>/library/custom-general-rules.md` (optional user additions)
5. `<data_dir>/library/custom-instructions.md` (user tweaks, loaded for
   EVERY agent — consolidated from the old per-agent
   `agents/<name>/custom-instruction.md` files)
6. Generated capability rules + the live roster
   (`agents::capability_prompt`, produced from the participant's grants —
   rc3 D3; nothing editable comes after it)
7. Resolved policy directive block (forbidden words list, push-gate
   mode, etc.)
Every step is additive. There used to be a step that took something
away — for native agents only, `strip_claude_code_tool_inventory` removed
the CLI tool inventory from the role prompt and `NATIVE_TOOL_ADDENDUM`
appended the loop's own tools in its place. It went with the loop (rc3 D9);
nothing subtracts from a prompt now.

Project conventions/notes **bodies** are deliberately NOT injected —
agents pull those via the `cl_index_search` MCP tool + `Read` on-demand.
What *is* injected (layer 2b) is the lightweight CL **index** for the
project: filenames + descriptions, so an agent that skips
`cl_index_search` on a cold start still knows what context exists to
pull. The index is fetched once in `spawn_session_handle`
(`storage.cl_index_search_agent`, the agent-visibility-filtered read) and
threaded into `read_system_prompt`;
`policy.yaml` is omitted from the primer (it's already rendered as the
policy block in layer 7).

---

## Turn coordination — the ring

**One participant holds the turn at a time.** `src/core/sequencer.rs` runs a
fixed rotation over the session's ACTIVE participants in `turn_position` order;
`on_mention` participants are skipped rather than handed a no-op turn. Each participant's pump (`src/core/pump.rs::pump_agent`) reports
`SequencerCommand::TurnComplete` when its turn ends — on BOTH the substantive and
the errored branch, because the ring steps on the completion, not on the text.

**Delivery is by cursor, not by forward.** A participant reads everything past
its own `participant_cursors` watermark when it takes the turn, and every
delivery is recorded in `participant_deliveries` with a nullable
`withheld_reason`. **Each wire leads with `[speaker]`** — the peer's slug (the
same handle `@mention` parses), `user`, or `system` (rc3 D23). Before that the
wire carried no author at all, and a participant handed four rows had to infer
which was the task and which was a peer's aside. **The tag also says when the
row was posted** — `[eyes · 2026-10-03T05:24Z]`, the row's own `created_at`
in UTC, to the minute (feedback #58). An agent has no clock; a turn is dealt
when a row lands, so the newest row's time is the time of the deal, and the
older rows show their age. It is the stored value rendered, never a line added
at delivery, so what a participant reads stays derivable from the row.

**A turn's backlog is ONE stdin write.** One outgoing message is one stream-json
line and claude-code opens a turn on the first line it reads, so delivering rows
one at a time handed a participant row 1 and then interrupted it with the rest:
measured over four sessions, the user's own message arrived somewhere other than
the front of the batch 37 times out of 44. Rows are joined with a blank line and
written together, so the backlog's last row — normally the user's, since a user
message is what wakes the ring — reads last. The splitter is the 200-row page
bound of `unread_for_participant` and nothing else; a deeper backlog is one write
per page, and the commit is all-or-nothing per page.

**A reviewer's first page opens with the turn digest** (feedback #76, carrying
#61; `src/signaling/bridge/turn_digest.rs`). A reviewer reads only its peers'
prose — their tool calls never reach it — so a Context Library write or a
session-doc write left nothing in the channel to review, and a gate the user
answered while the reviewer worked was a state it could contradict. The ring
asks the bridge for `turn_digest(session, slug, participant)` before the first
page of a dealt turn; for a participant the session registers as a reviewer it
lists, oldest first, the peers' (and the user's) CL writes (`cl_write_log`, fed
by `write_cl`; in memory, capped at 200 per session), the session docs changed
(`session_documents_changed_since`, by `updated_at` and `last_writer`, archives
left out) and the gated commands the user answered (`gates_answered_since`,
with `gate_ledger_state`'s verdict, exit code and output size and the first
output line), newest 30 within 6,000 bytes, earlier ones counted. It goes out
as the PREFACE of the same one stdin write (`deliver_batch_prefaced`), never as
a row — nobody else reads it. Its window is `[cursor, now]`: the `now` of the
last digest handed over (`digest_cursor`, committed by `commit_turn_digest`
only once the page LANDED, so a drain a user message cut short loses nothing),
or when there is none the time of the row the participant's channel cursor
rests on; a gate still running when listed is listed again, finished, next
time. Both maps die with the session (`unregister_session`).

**A turn's epoch is bound by the RING, not by the participant's own output.** The
sequencer publishes the epoch before delivering, and the pump snapshots it on the
turn's first event — but never on a STRAGGLER, an event arriving after a
completion and before the next handover (rc3 D24). Binding one to the epoch it
just completed with means every later completion carries a retired number, is
discarded, and the ring stops on a participant it is waiting on. There is no hold queue and no forward that can be lost:
policies gate delivery, never persistence.

An interrupted completion carries the host's interrupt epoch even when the
model produced no earlier event. Both connectors preserve their explicit abort
reason; a completion cannot claim a newer deal just because it arrived after
that deal. Busy marks also belong to an epoch, so an old completion or an empty
read clears only its own mark. Interrupted, stale, or failed reviewer turns do
not settle queued publishes; the next successful current turn can do so.

**A turn nobody dealt stays unbound, and is stopped while someone else holds the
ring.** claude-code starts a turn by itself when a background task it ran
finishes. Its events are stragglers: it opens no epoch, and its completion leaves
the last real epoch in place, so a second one in a row is still recognised. If
another participant is busy, the pump asks `AppState::stray_turn` to stop it —
once, and never in the middle of a commit, push or migration — and the
participant reads what the task reported at its next dealt turn. With nobody else
busy it runs. **Nor is it stopped while the participant still has background
tasks running** (claude-code's own count, the `background_tasks_changed` system
event → `AgentEvent::BackgroundTasks`): the stop is claude-code's interrupt, which
cancels the process's background agents with the turn, so the turn runs beside
the holder and a notice says why.

**A helper's messages are not the participant's.** claude-code streams a
subagent's assistant messages and tool results on the parent's stdout, tagged
`parent_tool_use_id` (the parent's own carry `null`). `agents::events::translate`
drops them: they are neither chat rows under the parent's name, nor a turn the
parent opened, nor its context reading. The helper's work reaches the parent
through the Agent call's own result and its completion turn.

**A participant still orienting when boot times out is not dealt.** Boot ends for
the session at its timeout, but a shared orienting set — seeded on a first spawn
only — keeps a late participant out of the rotation, the summons queue and a
typed message's interrupt until its readiness arrives as
`SequencerCommand::Oriented`. A ring left with nobody to deal, or only with
participants that have voted done or passed a lap, holds for it instead.

**How a turn ends** (`TurnEnding`):
- `Spoke { peer_ack_override }` — substantive output. Steps the ring and RESETS
  the done-tally for the whole session.
- `Done` — the consensus vote. Recorded per participant.
- `Passed` — declines the turn without voting done. Casts no vote and retracts
  its own — **both the done vote and the participant's phase vote** (D37) — so a
  participant that is blocked rather than finished cannot complete
  a tally by accident.

**Two participation modes, and both of them do something** (rc3 D18):
`active` is in the rotation, `on_mention` is not. An `on_mention` participant is
spawned and waits; the USER hands it a turn by naming it. `observer` was a third
and is gone — it was spawned, handed no turn, delivered nothing and could not
vote, so it read nothing, said nothing and billed for existing.

**A mention is a wake target, not addressing** (rc3 D17). Typing `@advisor` in
the composer opens a picker of this session's participants; the chosen slug is
parsed out of the user's row (`core::mentions`), resolved to a participant id,
and carried on `SequencerCommand::UserMessage`. That participant takes the NEXT
turn and only that one; the rotation then resumes from where it was, because the
ring steps from an ANCHOR that a summoned turn does not move. Several mentions
queue in the order written.

**Only the user may mention.** The parse has one call site, on the path that
writes the user's own row, and the function is private to `core::state` — so a
participant that types `@advisor` writes text and nothing can act on it. Peer
mentions would compose into a summon loop nothing catches: every turn
substantive, so the tally never completes, spin detection never fires, and only
the round cap ends it.

**How a cycle stops**, in order of how often it should fire:
1. **Consensus** — every active participant holds a done vote
   (`all_active_voted_done`, which filters on `enabled && participation_mode ==
   "active"`, so an `on_mention` participant never blocks a halt).
2. **A halt** — `mark_awaiting_user` / `halt` (and `request_phase_advance`)
   declare the session's halt: the bridge sends `SequencerCommand::HaltDeclared`
   carrying the DECLARER's participant id, and **the ring stops where it
   stands** (rc3 D35, 2026-08-14 — the user overruled D22's courtesy lap: *"a
   halt is a halt"*). If the declarer holds the turn, that turn ends; a
   non-holder's declaration latches the next deal. The declarer's own
   generation is interrupted so the declaration is true. An **approval gate**
   (`action_gate` — a Tool-Gate park or `require_approval: true` — the
   pre-push hook, the reviewer-down override) latches the ring the same way
   through `GateOpened`/`GateResolved`. An ordinary **question**
   (`ask_user_choice`) touches NONE of this: it parks in the tray, the session
   keeps working, and the answer arrives as a user row at the next boundary —
   and since round 12 so does an agent's **`request_approval`** (a `request`
   row: the user's split, "request_approval is tray parkable, approval_gates
   are session blockers"; it is audited in violations.jsonl but latches nothing).
   (D22's lap existed because halting at a first-turn park made a participant
   that asks every turn unreachable to its peers — `s-e8a20797`; a question no
   longer reaches the ring at all, so that failure cannot come back this way.)
   Since round 10 the RELEASE of a holder-declared halt waits for the
   halted holder's completion: the holder's turn ends ring-side at once, but
   its process is still finishing (the halt tool's result, the D35 interrupt
   on it), and a message dealt into that window is folded by claude-code
   into the OLD turn — its answer comes back under the old epoch and the ring
   discards it (five live traces, ~95 s each until the idle nudge). A
   `UserMessage` arriving while `RingState::winding_down` names that holder
   is stashed (`RingState::stashed_release`; two in the window merge, in
   order) and replayed through `release_ring` on its completion (live or
   discarded), on its respawn, or after `HALT_WIND_DOWN_GRACE` (3 s) —
   whichever comes first. The grace is a deadline armed at the first bounded
   read after the stash (round 11), not an idle timeout re-armed by other
   commands; it does not run under a pause — the ring's own flag OR the
   activity tracker's latch — and re-arms in full when the pause lifts.
   **A TEMPORARY halt** (round 12, the user's 2026-08-19 shape: *"HALT;
   TEMPORARY HALT 00:03:57"*) is the same slot with `halt_wake_at` set
   (migration 0069): `mark_awaiting_user` / `halt` with `wake_after_secs`
   (10–3600) — for a wait on something EXTERNAL with an ETA (CI, a deploy),
   so an AFK user never holds the floor for an event that already happened
   (`s-1c29c521`: two agents halted on CI, nobody awake when it finished).
   The banner counts down (`HaltBanner`, `TEMPORARY HALT · wakes in mm:ss`);
   at the instant the bridge (`fire_temporary_halt`) re-reads the slot — a
   newer halt, an answer or a close before then cancels the wake — skips
   under a pause, defers behind an open gate (re-checks every 30 s, up to 10,
   then gives up with a notice), then posts a system row, clears the slot and
   SUMMONS the declarer (`notify_ring_user_message` with that one id) for a
   turn. Armed in-process by the declaring tool and RE-ARMED from the slot
   when a ring registers (`register_session_sequencer` → `rearm_temporary_halt_for`),
   so a relaunch or lazy respawn honours a pending wake without a boot-time
   sweep over ringless sessions.
   **The last gate resolving can itself wake queued work.** Once its answer
   or command result has been delivered, an idle ring serves its stored user
   release, reviewer summons, staged response or eligible unread rows. A live
   holder, Pause, boot, explicit halt or another gate still takes precedence.
   A close approval clears the latch quietly before requesting close-out.
   The bridge returns a receipt when it queues that gate transition; AppState
   then sends no second release or cleanup. Questions and ordinary approval
   requests still send one non-preempting notice, suppressed during Pause;
   the ring decides when the live turn can safely read it.
   With no next turn, the host leaves a recap; approval alone is never called
   proof that a command finished. A non-preempting answer observed during
   backlog delivery is deferred without cutting that delivery short.
   An internal retry retires a prior mechanical host recap only after its
   first page actually reaches the participant. An empty deal, missing input
   or stopped drain does not clear it. The author-predicated halt update and
   gate/Pause checks preserve agent halts and active approval waits.

2b. **The all-pass lap** — a full lap in which every dealt participant PASSED
   yields to the user with a visible notice (rc3 D27, `announce_all_passed`):
   nobody had anything to add, and the only party who can change that is the
   user. A staged message is delivered at that boundary like any other.
   The ring counts accepted pass completions from the current lap and requires
   all currently dealable active participants to have passed. It never derives
   an all-pass lap merely from a lack of speech. A new user release clears that
   accounting; a tray answer received while a generation is still live waits
   for its completion, regardless of the UI's busy flag. That wait has no
   interrupt wind-down deadline: an answer does not interrupt ongoing work.

   **A user message releasing Pause owns the next deal.** The ring replays
   held state changes before that message, preserving gates, vote retractions
   and mentions, but does not deal or declare consensus from a held completion.
   An internal `PauseReleased` marker ends this replay and issues the one
   release. Otherwise the held completion and the message both deal, and an
   empty second deal can falsely announce all-pass while the first one runs.
3. **Spin detection** — token-set Jaccard at `SPIN_SIMILARITY_THRESHOLD` (0.85)
   over ONE participant's output across rounds, for `SPIN_BREAK_STREAK` (2)
   running. Cross-agent echo is impossible in a ring; self-repetition is not.
   A pass deliberately does not trip it.
4. **Round cap** — a crude backstop at 500 LAPS (one lap = one full pass of the
   ring), `0` = off, per-session override on the policy chain. High enough to be
   invisible in normal use; firing posts a visible row (rc3 D7).
5. **Errored turns** — a turn claude-code ends `is_error:true` (an API failure;
   a turn bot-hq itself interrupted is excluded) steps the ring like any other,
   and TWO in a row from one pump fill the halt slot with the error line as the
   banner. Round 12 put a retry ladder in front of that halt for TRANSIENT
   failures (`core/pump.rs::transient_error` — the socket errnos, `fetch
   failed`, `overloaded`, HTTP 502/503/504/529 beside an HTTP word): the pump
   posts a notice, waits 30 s → 2 min → 10 min (`RETRY_LADDER`), posts a nudge
   row and sends `SequencerCommand::Summon` — a D17 summons with no ring reset,
   dealt at once if the ring is idle, after the current turn otherwise; the
   fourth transient failure halts, the banner saying the retries happened, and
   a clean turn resets the ladder. A process that DIES is the supervisor's
   (`spawn.rs::supervise` respawns with `--resume`), never this ladder's.

**The bilateral router this replaced was deleted 2026-08-13** (task 14). It
forwarded each of two participants' turns to the other through a central task
with a hold queue, a volley hard-cap and a convergence breaker, and had no third
case — it was the last thing holding a session to two participants. Every
behaviour it encoded carries a verdict in
[`docs/plans/2026-08-06-router-behaviour-inventory.md`](docs/plans/2026-08-06-router-behaviour-inventory.md):
12 PRESERVED (each with a named test in `core::sequencer`), 6 DISSOLVED
(structurally impossible without a hold queue), 2 DROPPED with stated reasons.

---

## Context compaction — the handoff doc

claude-code compacts a participant's context when it fills: everything before
becomes a summary, and what the user said an hour ago survives only if the
summary kept it. bot-hq cannot prevent that, so it makes three things
mechanical (feedback #54 / #63 / #67 / #74 / #81):

- **A warning before.** At each turn end the pump compares the participant's
  context reading with two thresholds timed from the point claude-code
  compacts (window − 33,000: `core/pump.rs` `compaction_point`,
  `context_row_thresholds`): the first row 6 % of the window before it (at
  least 25,000 tokens), a last call 2 % before it (at least 15,000) — 907,000
  and 947,000 on a 1M window, 142,000 and 152,000 on 200k. One channel row per
  threshold (`context_threshold_notice`), ending with the state of the
  participant's handoff doc. The latch re-arms on a compaction, or once a
  reading falls under half the first threshold, so every fill of the window is
  announced, not just the first, and a reading hovering under a row never
  re-fires it.
- **A row when it happens.** claude-code writes
  `{"type":"system","subtype":"compact_boundary","compact_metadata":{…}}` to
  stdout; `agents/events.rs` turns it into `AgentEvent::Compacted`, and the pump
  posts one row every participant and the user read: who, the token counts, and
  whether the handoff doc was put back. It is handled before the turn logic,
  like the background-task count: a compaction is not the participant speaking.
- **A summary that knows the doc's age.** Every participant is also spawned
  with a `PreCompact` hook (`policy-check pre-compact`, a 10 s timeout). It
  reads the same rendered file as the put-back below and prints
  `agents/handoff.rs::compaction_instructions`; claude-code appends a
  PreCompact hook's stdout to the prompt that writes the summary (probed on
  2.1.291). The summary is written as usual and ends with a section on
  everything since the doc was last written: the user's words verbatim with
  their times, decisions, what was done with its evidence, what is in flight,
  open ids, and what in the doc is no longer true. The summary is the one
  artifact written AT the compaction, so this is what keeps the time between
  the doc's last write and the compaction from being lost. The hook always
  exits 0 and prints plain text: an exit 2, or stdout parsing as
  `{"decision":"block"}`, would block the compaction. It has no side effects,
  because the CLI may run it early (its precompute path) or twice.
- **One document put back.** The custom session doc `handoff-<participant
  slug>` is the participant's own (a peer's write to it is refused; the user
  edits it in its tab). The session-doc store mirrors every write and delete of
  it to `<data_dir>/.local/handoffs/<session>/<participant>.md`, already
  rendered (`agents/handoff.rs`: a header, the body up to a 9,000-byte budget,
  and where to read the rest). Every participant is spawned with a
  `SessionStart` hook, matcher `compact`, that runs `bot-hq policy-check
  compact-handoff`; it prints that file and claude-code puts a hook's output
  into the compacted context. It stamps the doc's age into the header as it
  prints ("last written T — 38 minutes before this compaction",
  `handoff::stamp_age`), and the header says which source wins per claim: the
  summary for anything after T, the doc's own words for anything before it
  (above all the user's instructions it quotes). Hook output over about 10,000 characters is
  replaced by a preview, hence the budget. The stream's `hook_response` for
  that hook is recognised by the file's opening marker, so the compaction row
  says "put back" only when it was.
- **Unread gate results survive a newer handoff** (feedback #112, migration
  0106). A gate's `resolution_row_id` links its latest recorded answer/output;
  a later scheduled run replaces the earlier approval receipt. After a successful
  MCP call, the issuer receives a second text block for up to eight unannounced
  results. `gate_result_notices.noticed_at` records that notice separately from
  actual channel delivery; it never advances the channel cursor.
  `gate_result_notices.delivered_at` preserves gate-specific delivery proof
  beyond telemetry retention, including delivery before a result was linked.
  Per-reader mirrors at `.local/handoffs/<sid>/<slug>.gates` retain results
  without delivery proof, independent of handoff timestamps. They refresh on
  result linking, delivery and handoff write/spawn; empty mirrors are removed.
  Claude's separate `compact-gate-results` hook and Codex's compaction injection
  consume the same mirror (up to eight records, JSON-quoted output tails, a
  6,000-byte cap). The notice and excerpts direct the agent to `gate_status`
  before reporting an outcome; an exit code alone is not a success claim.
  Legacy results whose delivery telemetry was already purged may be replayed
  conservatively. Direct `gate_status` reads do not count as channel delivery.
- **The user's own words put back verbatim.** A summarizer paraphrases, so what
  the user said since the doc was last written comes back as written: a second
  command in the same `SessionStart:compact` group, `bot-hq policy-check
  compact-user-words`, prints `<data_dir>/.local/handoffs/<session>/user-words.log`
  filtered to the entries after the participant's doc time (read off its
  rendered file; the whole session when it has no doc), newest kept under its
  own 9,000-byte budget — claude-code caps each hook's output on its own (two
  9 KB outputs arrived whole where one 18 KB output became a preview, CLI
  2.1.291), and the commands landed in settings order, so the words follow the
  doc. The file mirrors the `session_user_words` table (migration 0092),
  re-rendered on every record and at spawn. What counts as the user's words is
  the CALLER's decision (`core::Authored`): a typed Send and a staged message
  are recorded; the resume notice, the close-out prompt, a plugin's send and
  the prompt of a plugin-created session — all posted in the user's voice —
  are not; a tray answer is
  recorded as the pick with the question it answered (gate: the verdict with
  the command's first line), never the row's body with the gate's output. The
  put-back's precedence has three tiers: the user's verbatim words over the
  summary's account of them, the summary for anything else after the doc's
  time, the doc for anything before it. The compaction row counts the messages
  put back.

- **The mid-turn reminder (B1e).** The context rows are built from the
  `result` event, so they reach a participant only at its NEXT turn — and 8 of
  13 executor fills in two sessions compacted inside one turn with no row
  first (EYES' count, s-079dec53). Every top-level `assistant` message carries
  its API call's prompt size, which `translate` now emits as
  `AgentEvent::AssistantUsage` (`events::used_tokens_of`, the turn-end
  numerator's sum). The pump measures it against the window the last `result`
  reported; when it crosses a `context_row_thresholds` value not yet marked
  this fill, it writes `<data_dir>/.local/handoffs/<session>/<participant>.due`
  (`handoff::due_text`: the figures, the doc's name, what to do). A third
  PreToolUse hook on BOTH postures and every tool (no matcher) — `bot-hq
  policy-check handoff-due`, its own entry, never a branch of the Tool Gate's
  allow path — prints that file as `hookSpecificOutput.additionalContext` and
  nothing else, exit 0 (probed on CLI 2.1.291, s-42bf34b3: the context reached
  the model in `-p` stream-json, and a `Bash(git commit:*)` deny under
  `dontAsk` still refused the call beside it). So the participant reads the
  reminder on its next tool call, mid-turn, and keeps reading it until it
  writes the doc: `Storage::sync_handoff_file` — the one choke point every
  handoff-doc write passes — removes the file. A compaction removes it and
  re-arms the fill; so does a reading under half the first threshold. One
  reminder per threshold per fill (the first row and the last call).

The hooks fail open: with no file they print nothing and exit 0. The files are
rendered at spawn too (the doc's as "you have no handoff doc" until one is
written) and removed when the session closes.

## Tauri + React UI

**Stack:** Tauri v2 shell + React 18 + TypeScript + Tailwind + minimal
shadcn-style primitives (Vite build). Tauri owns the OS main thread; the
Rust core runs on a multi-thread Tokio runtime.

**IPC:** Tauri commands + Tauri events. No HTTP from the frontend. The
existing `SignalingBridge` stays the single source of truth — a thin
command layer in `src/tauri_cmd/` wraps bridge methods, and a
broadcast-subscriber bridge in `src/tauri_events/` translates
`SignalingEvent` into typed Tauri events. The hot path
(`MessagePersisted` IDs → batched message fetch via existing
`messages_for_session(session_id, since_id)`) goes through a
`BatchEmitter` (N=20 / 50ms coalesce).

**Live freshness (filesystem watcher + command emits):** beyond the bridge event
stream, a filesystem watcher (`src/tauri_events/fs_watcher.rs` — one
`notify-debouncer-mini` over the CL dir + per-session repos, re-indexing the affected
scope before it emits) and direct `app.emit` calls from mutating Tauri commands
(`project:changed` / `model:changed`) drive UI updates that bypass the bridge. All three
channels converge on `Providers.tsx` GlobalEventSync key-set invalidation, so the CL
tree/editor, the Apply-tab `git diff`, and the project/session/model lists refresh on
external change without polling — the 60s `refetchInterval`s were dropped; only the
plugin heartbeat (10s) + a broadcast-`Lagged` `session:resync` backstop remain. Working-
repo churn is filtered by an ignore-list (`target`/`node_modules`/`.git`/dotdirs) so
builds don't thrash the A-tab diff. (Shipped 2026-06-15.)

**Topbar:** `Dashboard | Context Library | Plugins | Settings`.

**Dashboard:** grid of session tiles in the USER'S order — `sort_key`, seeded
from creation order (0071) and exchanged by drag-to-SWAP (dropping tile A on
tile B trades exactly those two slots; the old last-activity order made tiles
race each other). Each tile shows title, last
activity, and a `[Need User Input · N]` pill (primary-tinted border) when
questions, gates or approvals await the user — the tile INDICATES only; the
answer surface is the session's Tray tab. Click tile → opens session view.
`+ New session` opens a modal dialog (roster editor) that creates the row +
registers the session with the bridge.

**Session view:** three subtabs — **Workspace** (chat + DocumentPane, a
drag-resizable split defaulting to 60/40, clamped 25–75), **Context** (the
Context Library scoped to this session's project — tree + a lean editor —
without leaving the session), **Terminal** (the session PTY). Header: title + back link, the
live roster rendered `ROLE · Model`, the phase picker, and the gear button
that opens the Session Settings panel (the per-session policy snapshot).
A CLOSED session's view is read-only history (round 10): its mount respawns
nothing (`ensure_session_started` refuses a closed row on every path), and the
composer's slot shows a **Reopen** bar — the one control that applies — whose
`reopen_session` clears `closed_at` / `archived`, SETS a system-declared halt
("Session reopened — waiting for your prompt." — the idle watchdog gates on the
slot, so the fresh roster is never nudged into re-closing), resets the IPAV
phase, posts a REOPENED system row for the respawned roster's bearings, and
respawns via `--resume` (1.0.0 Batch 1; an already-open row is a success
no-op). The Apply-tab diff anchor `session_start_sha` persists write-once
(0070), so a reopen never rebaselines it. Chronological chat: all
messages (user, each participant, phase_change)
interleaved by `created_at`; each participant's colour is one of an 8-hue
palette hashed on its `ROLE · Model` label, with a per-participant override
chosen in the New Session dialog (rc3 D20, migration 0052); user = blue,
system = muted. Questions and gates are answered on the DocumentPane's Tray
tab; a declared halt renders as the banner above the input.

**DocumentPane:** a **Tray** tab (pending questions, approvals and gated
commands for this session — the single answer surface) followed by the IPAV
tabs (I/P/A/V chips), which drive `session_doc_search(session_id,
phase=<x>)`, followed by one tab per **custom document** — an untagged
`session_documents` row the session wrote (a task checklist, an issue
write-up; round 11, the user's ideas.md), named by its slug; the phase docs'
archived versions (`<slug>@<n>`) are not tabs. A custom doc written with
`kind: "user_action"` (feedback #89, the user's first slice — no review step)
is one of the user's action items: its pill carries a FOR YOU badge (DONE once
dismissed), and the dashboard's **Your action items** panel
(`ActionItemsPanel`, `list_user_action_items`) lists every such doc across
sessions, open or closed, with Open (lands on the session with `?doc=<slug>`,
which the pane selects) and Dismiss (`dismiss_user_action_item` →
`dismissed_at`; the doc stays). A re-mark un-dismisses. Each IPAV tab renders matching
`session_documents` rows; counts surface on the chips. The A tab also renders
the live color-coded `git diff` for the session's working repo via the
`compute_apply_diff` Tauri command (`src/tauri_cmd/docs.rs`, parser
`parse_diff_lines`), consumed by `DocumentPane.tsx`.

**Context Library tab:** two Settings-style subtabs — **Library Tree** |
**Context Manager** — whose pill row is the page header (no panel repeats
its label).

*Library Tree* — 2-pane file explorer + tabbed editor. The tree renders
nested collapsible folders (`cl_index_search` + `cl_folder_search`) with
substring search (no project filter — removed as YAGNI; Rescan is always
all-projects, in parallel with per-project failures surfaced). Files open a
read-write editor (`cl_read_file` / `cl_write_file`; binary + truncated
files are read-only so a lossy save can't corrupt them). Folders open a
folder-view that edits the folder description (`cl_set_folder_description`)
and, at the project root, manages the project: set the working repo, rename
(`cl_rename_project`), unbind (`cl_unregister_project` — soft: clears repo +
custom cl_path, keeps content), and hard-delete (`cl_delete_project` — purges
all index/folder rows + the row; optionally removes the managed on-disk dir,
never a custom `cl_path`). The sidebar `+` opens a **New-project** modal: the
default path (`cl_create_project`) creates a managed project at
`<data_dir>/library/projects/<name>/` (seeded `conventions.md`/`notes.md`) and
binds an optional working repo WITHOUT indexing it; an Advanced section
(`cl_register_project` + `cl_rescan`) is the power case that indexes an existing
on-disk folder AS the CL content (`cl_path`). Right-click gives VSCode-style new
file / new folder / rename / delete (`cl_create_file` / `cl_mkdir` / `cl_rename`
/ `cl_delete_path`, each followed by `cl_rescan`), plus **Promote to project**
on a top-level Global folder (moves it into `projects/` + registers). Path
inputs (working-repo, cl_path) use a native folder picker via
`tauri-plugin-dialog`. The New-session dialog can also pick an ad-hoc working
repo directly (no pre-registration).

*Context Manager* — the per-project management surface (NOT a file
explorer): a left rail of registered projects (`_globals` pinned last) and a
right panel for the selected project — header strip (repo path, per-project
Rescan) over the **Measurement** card:
`cl_retrieval_stats` tiles over `retrieval_events` (tokens/session,
tokens/retrieval, stale-hit + retrieval-miss rates). Default selection is
the first project.

**Plugins tab:** Management UI (`PluginManager.tsx`) over
`tauri_cmd/plugins.rs` — install is two-step (`preview_plugin_manifest`
shows the requested capabilities with catalog descriptions; only an
explicit confirm installs), then enable / disable / uninstall and a
heartbeat-driven crash indicator. Enabled plugins that declare a panel
get a dynamic topbar tab (`/plugins/view/:pluginId` → `PluginPanel.tsx`
→ `PluginHost.tsx`). See "Plugin runtime" below for the execution model
and [`docs/PLUGINS.md`](docs/PLUGINS.md) for the author contract.

**Settings tab:** subtabs led by **Roles** (create/edit roles, their capability
ticks, instruction prose, participation mode and default model), then the
saved-model registry (Models), the global Tool Gate keyword list, the global
Claude Config surface (one block per role), Policy, Violations, Feedback, a
closed-session Archive, and Updates. The **Agents** subtab was retired by rc3 D8 — a role owns
its default model and the New Session dialog overrides it per participant.

**Per-participant model selection:** the user maintains a registry of saved
models (`models` table — label + provider + base_url + auth_token +
`context_window`) in Settings → Models. The New Session dialog adds one row per
participant, each choosing a ROLE and optionally overriding that role's default
model and effort. The picks persist on `session_participants`
(`role_id` / `model_id` / `effort` / `ultracode`).

**The model chain (rc3 D8):** the session's own pick → the ROLE's
`default_model_id` → the per-agent row → the built-in default, resolved in
`resolve_participant_config`. The role step is what makes the Roles tab the owner
of "which model does this role run on" for every create path, including the ones
with no dialog.

The dialog-less create path — the plugin proxy — seeds exactly ONE
participant, the first active role by `roles.id`
(design §1's "default 1"). The `rain_disabled_default` setting that used to
govern them was deleted by rc3 D13: the dialog picks the roster now, so "start
solo" is simply not adding a second participant.

**Two per-model columns are now unread** (rc3 D9). Both survive in the
schema: dropping either needs a rebuild-table migration that buys nothing —
0060, which did rebuild `models` for the retirement of the agent names,
carried `native` forward deliberately — so they are left in place:

- **`models.native`** (migration 0036) — opted a model into the native
  loop. Nothing reads or writes it; `MODEL_COLUMNS` does not project it,
  so an upsert leaves it at the column default. There is no checkbox for
  it in Settings → Models any more.
- **`models.context_window`** (migration 0037) — total tokens this model
  accepts. Its only reader was the native loop's own accounting; on
  claude-code the meter comes from the CLI's per-turn `contextWindow`
  report. Still round-tripped through the Models tab so a saved value is
  not destroyed by an edit, but nothing acts on it.

Both are mirrored onto `agent_configs` (migration 0038), with the same
status there.

**Claude Config surface** (`src/claude_config/`,
`tauri_cmd/claude_config.rs`, `frontend/src/app/ClaudeConfig.tsx`):
surfaces the user's `~/.claude` config that leaks into the headless
agent subprocesses — skills, plugins, hooks, CLAUDE.md/memory, MCP
servers, reasoning effort. The user controls it two ways: globally
(write-back to the real `~/.claude` via `claude_config/writer.rs`) and
per-role via an override layer (`<data_dir>/config/claude-overrides.json`,
`claude_config/overrides.rs`, `ClaudeOverrides { all, per_role }`) merged
into the spawn-time `--settings` JSON + env injection — so an inherited
skill/plugin/MCP/effort can be disabled for one role without touching the
user's own `~/.claude`.
Design: `docs/plans/2026-06-02-claude-config-surface-design.md`.

**Plugin runtime (v1, shipped 2026-07-04):** plugins are static
frontend bundles in sandboxed iframes, served over ONE `bhq-plugin://`
custom URI scheme (Builder-time registration; per-request enabled-check
via the `PluginRegistry` enabled cache, so install/enable needs no
restart; the plugin id rides the URL host on macOS/Linux and the first
path segment under the Windows `https://bhq-plugin.localhost` fold —
`plugins::serve` accepts both). Plugins never call Tauri: they
postMessage the shell (per-mount nonce + source/origin checks in
`frontend/src/lib/pluginBridge.ts`), which forwards to the single Rust
enforcement point `plugin_invoke_proxy` (`tauri_cmd/plugin_api.rs`) —
re-checking enabled ∧ granted ∧ catalog-listed per call and
dispatching through an explicit match over `plugins::catalog` (14
commands: 10 read-first, `spawn_session`, `plugin_sessions`, and the
plugin-scoped KV get/set; `api_version: 1`). The
heartbeat state machine (`plugins::heartbeat`) is fed by the
PluginHost's 5s ping loop (`plugin_note_ping`/`plugin_note_pong`); the
sweep loop in `main.rs` emits `plugin:crashed` after 3 misses and the
host swaps in a Reload fallback. There is NO Tauri-ACL/capability-JSON
path (the original design's generated capability files were written
where Tauri never reads — retired 2026-07-04). Author contract:
[`docs/PLUGINS.md`](docs/PLUGINS.md); working example + test fixture:
`examples/hello-plugin/`.

---

## Internal MCP server (UI signaling)

In-process HTTP MCP server, hand-rolled JSON-RPC over hyper 1.x. Lives
in `src/signaling/` (`jsonrpc`, `protocol`, `server`, and the `bridge/`
submodule tree). Surface:

- **Bind:** `127.0.0.1:<ephemeral>` (chosen at startup; ephemeral port).
- **URL per agent:** `http://127.0.0.1:<port>/sessions/<id>/<agent>/<token>/mcp`.
  Each agent's `--mcp-config` file points at its own URL — with a per-spawn
  secret minted in `core/session.rs` and checked in `server.rs`
  (`mcp_token_matches`, C1-1, `a4e5c00`) — so the bridge knows which agent
  is calling and a wrong or missing token is refused. The HTTP dispatch is
  where role
  enforcement lives, which is why the deleted native loop spoke the same
  HTTP JSON-RPC rather than calling `SignalingBridge` in-process: an
  in-process path would have been a second, unenforced route to the same
  tools.
- **Methods:** `initialize`, `ping`, `tools/list`, `tools/call`.

**Internal tools (49)** (see [README.md](README.md#internal-mcp-tools-served-to-child-agents)
for the documented list with descriptions): `ask_user_choice`,
`mark_awaiting_user`, `peer_ack`, `pass_turn`, `halt`, `advance_phase` (a VOTE
since D37 — see "The phase-advance vote"),
`web_search`, `request_phase_advance`, `file_feedback`, `resolve_feedback`, `request_approval`,
`action_gate`, `read_gate` (group K: the reviewer's own path to a listed read), `prod_read` (group K: read-only Postgres SQL on the project's production database, parked with its SQL), `gate_status`, `gate_history`, `check_commit_message`, `flag_finding` (alias `eyes_flag`),
`disposition_finding`, `check_open_findings`, `get_finding`, `override_reviewer_block`,
`approve_finding`, `close_session`, `list_my_pending_questions`, `withdraw_question`,
`supersede_question`, `session_doc_write`, `session_doc_edit`, `session_doc_search`,
`session_doc_read`, `session_history`, `app_control`, `cl_index_search`, `cl_retrieve`, `cl_stale_refs`,
`cl_write_file`, `cl_edit_file`, `cl_register_read`, `cl_rescan`,
`cl_folder_search`, `cl_register_folder_description`,
`terminal_exec`, `terminal_read`,
`webview_screenshot`, `webview_click`, `webview_type`, `webview_scroll`,
`webview_press_key`.

`app_control` requires the explicit `control_bot_hq` grant in BOTH the participant's
invite snapshot and its current role. It is not granted to default/example roles.
Unchecking or archiving the role revokes new calls, as do removal, session closure
and human Pause. Each accepted operation has a durable source/target/outcome audit
(0108); a started record without a finish is not proof it is still running.
The initial routes read cross-session inventory, state, history, documents, tray
summaries/choice chunks and the audit. Administrative writes are not connected yet.

`session_history` requires `read_channel` and fixes its scope to the
authenticated caller's session. It returns raw channel rows or explicit
human-word ledger records, newest first (10 per page,48KB encoded). Long bodies
use source-record IDs and resumable UTF-8 byte offsets; a channel message ID can
select multiple human-ledger records. It never advances ring cursors or records
delivery. Metadata clipping is marked; wrappers are never re-labelled as human
words. Its descriptor stays deferred.

`tools/list` is unfiltered but answered **per caller**: a core set
(`protocol::ALWAYS_LOADED_TOOLS` — the ring verbs, the session-document tools,
the Context Library readers, the gates, the commit checks) carries
`_meta["anthropic/alwaysLoad"]` on the entries that caller can use, so
claude-code keeps those schemas in context instead of deferring them behind
`ToolSearch` (feedback #88). The rest of the registry stays deferred; a test
holds the always-loaded set to a byte budget (about 23 KB for the executor
preset, 15 KB for the reviewer, of a 48 KB registry).

This list is checked against the live registry by
`protocol.rs::every_registered_tool_is_documented` — it said 36 for four tools'
worth of drift before that test existed, and a list nothing compares to the code
is a list that is wrong and does not know it.

**Session terminal (Terminal subtab).** Each session lazily spawns one PTY
shell (`core/terminal.rs`) in its working repo — rendered by the session
view's Terminal subtab (xterm.js) and shared with the agents through
`terminal_exec` (needs `run_terminal`; BLOCKING by default — writes the command, awaits
the shell's return to its prompt: 0.7 s with no output and no job in the
terminal's foreground, the foreground sampled through the window (unix
`tcgetpgrp` on the PTY master; Windows falls back to the quiet window alone,
feedback #48), and returns the captured tail — a job still holding the terminal
at `wait_ms` is named in the note; a just-spawned shell is first given its
first prompt; `block:false` for long-running processes; while a job holds the
terminal it refuses and names it — what is typed would go to that job — unless
`to_job` says so, and never into a credential prompt such as `sudo`, feedback #98)
and `terminal_read` (every participant;
scrollback tail as evidence text). Both return the text as the screen shows it
(`core::term_text::render_plain`: escapes dropped, `\r`/`\b`/erase-in-line
applied; `terminal_read`'s `raw` keeps the bytes). `terminal_exec` re-classifies the command
against the same two-tier Tool-Gate keyword list the PreToolUse hook uses
(session snapshot → global fallback, `tool_gate::resolve_keywords`) and
refuses gate-matched commands with a route to `action_gate` — the terminal is
not a gate bypass. The PTY is killed on `close_session`; scrollback is
in-memory only (200 KB ring).

**Review findings gate (reviewer sign-off).** `flag_finding` (alias `eyes_flag`) /
`disposition_finding` / `check_open_findings` / `override_reviewer_block` /
`approve_finding` (+ the pre-commit `check_commit_message`) implement the
reviewer sign-off gate: a `blocking` finding filed via `flag_finding` gates
`git commit` AND `git push` (mechanically, via the pre-commit and pre-push
hooks — `hooks.rs::check_findings_gate` runs on both) until it is resolved
with `disposition_finding` (fixed / rebutted). The gate is fail-CLOSED when the
reviewer is down — the reviewers it watches are the participants holding
`file_finding`, registered at spawn. `override_reviewer_block` is the explicit
escape valve. Backed by the `findings` table.

**Capability enforcement at the dispatch layer:** the hard-coded
`HANDS_ONLY_TOOLS` / `EYES_ONLY_TOOLS` name lists are GONE (rc3). A gated tool
resolves its required capability through `capability::required_for` and checks it
against the caller's invite-time snapshot; the refusal names the capability and,
since rc3 P2, also posts a visible row. Tool descriptions render their
requirement from `required_for` itself, so a description and its gate cannot
disagree. A parity oracle walks every gated tool against a frozen transcription
of the old name gate, so the reframe is proven equivalent rather than asserted.

`close_session` gates on `Capability::CloseSession` (D16, shipped 2026-08-13,
`6cc2b9a`); `PARITY_HOLD` is EMPTY and test-asserted so — the parity oracle
fails on any divergence between the capability model and the name gate it
replaced, and says in its own message that such a divergence is the user's
call.

**Bridge (`src/signaling/bridge/`)** owns:
- Storage handle (writes question rows, message rows, violations).
- Policy resolver (loads `general-policy.yaml` + `projects/<p>/policy.yaml`).
- Session → project mapping.
- Per-session `awaiting` halt flag (shared `Arc<AtomicBool>` with each
  participant's pump; it drives the input-lock activity state).
- Tray storage (`session_tray` table — persists awaiting-input items
  (`ask_user_choice` / `request_approval` / gated commands) so they
  survive app restart).

---

## The external driver — REMOVED (2026-08-17)

bot-hq ran a second HTTP MCP server so an external agent could drive the app
the way a human does: start sessions, send messages, answer gates. The user
removed it and **demoted it to a future plugin**, the same call rc3 D9 made for
the native agent loop.

Deleted: `src/signaling/external_jsonrpc.rs`, `src/signaling/external_server.rs`,
`tests/external_mcp_test.rs`, the `<data_dir>/.local/mcp-token` bearer token and
its generation, the `BOT_HQ_EXTERNAL_MCP_PORT` / `_DISABLED` env vars, and the
`external_server` field on `CoreAppState` — ~2,100 lines.

**What survived, and why.** `wait_for_change` moved to `tauri_cmd/plugin_api.rs`:
the plugin proxy was its only other caller, and that file already holds the
storage and bridge it needs. Nothing else in the tree depended on the driver.

**What the deletion bought beyond the deletion.** The driver's `wire` module was
the sole reason two guards carried exemptions — `no_tool_description_an_agent_reads_names_an_agent`
now runs with NO exempt strings at all, and `retired_identifier_test`'s
carve-out list dropped from two files to one. A published wire is a promise that
costs something to keep; deleting it refunded that.

The design record is the code itself: `git show 2833949:src/signaling/external_jsonrpc.rs`
holds the 20 tool descriptors, the bearer-auth handshake and the CORS handling a
plugin rebuild would want.

---

## The phase-advance vote (rc3 **D37**, migration 0062)

**The phase moves only when every active participant has voted for the same
state of the work.** `advance_phase` records a vote; consensus performs the
transition. The user's reason: a full IPAV pass had gone by in which the reviewer
was never dealt a turn, and the boundary is where that costs the most.

- **A vote is a ROW**, keyed `(participant_id, target_phase,
  artifact_fingerprint, phase_epoch)`. Not a column: `done_vote` is cleared
  session-wide on `TurnEnding::Spoke`, and this feature exists to make
  participants speak between votes, so reusing it livelocks by construction.
- **`artifact_fingerprint`** digests the session's phase documents (count, latest
  `updated_at`, total body length). Talking never invalidates a vote; changing
  the work always does. A state of the work that cannot be read is not voted on:
  the vote is refused rather than cast on an empty digest.
- **The reviewer's co-docs are not the work.** The four `<phase>-eyes` docs are
  left out of the digest (feedback #70 / #95): counting them made a reviewer's
  notes orphan the executor's standing vote, and reviewers kept findings in chat
  to avoid it. What the invalidation did is done directly instead: a co-doc
  write withdraws the WRITER's own vote, so a reviewer that records a new
  objection votes again before the phase can move, and one system row per co-doc
  per run of its turn tells the others that notes landed. The guard on an
  advance is the reviewer's own vote; a blocking finding gates commit and push,
  never the vote.
- **`sessions.phase_epoch`** is monotonic and bumps on every transition. It
  closes the axis a content digest cannot see: phases run backward, so without
  it a Plan-era vote would match again after Plan → Investigate → Plan whenever
  the work came back unchanged.
- **The electorate** is `enabled && participation_mode == "active"` — the same
  filter `all_active_voted_done` uses. A solo session advances on its own vote;
  an empty electorate is vacuously arrived.
- **`on_mention` callers are refused**, since a vote counting toward nothing
  while the tool reported an advance is a dead phase with no signal.
- **A pass retracts the passer's own vote** (see "Turn endings").
- **The tool reports what happened** — advanced / vote-recorded / refused — because
  an agent told "advanced" writes the next phase's document and begins mutating.

**D36's escape valve** fires on the ROUND CAP, not the all-pass yield: a pass
retracts, so a silent lap has no votes left to report. It names the deadlock and
points at the phase control; it does not force the advance, because D36 is
explicit that the gate stays blocking and never times out.

**The user's own phase control** is `advance_session_phase` → the header picker in
the session view. It routes to the same `AppState::advance_phase` the agent tool
reaches, with `PhaseAdvanceSource::User`, which additionally releases the ring —
an agent self-advancing is mid-turn, a user acting on a halted session is not.

---

## Policy enforcement

**Goal:** enforce per-project rules (forbidden commit words, push gate,
force-push gate) reliably even when an agent's context drifts and
forgets to call the MCP tool.

**Two layers** (`src/policy/`):

1. **MCP tools** (`request_approval`, `action_gate`, …) are
   the primary path. Agents are instructed in their system prompt to
   call them before the corresponding bash op. Skipping logs a
   `Denied` violation to `<data_dir>/.local/violations.jsonl`.
2. **Git hooks** are the deterministic backstop. `bot-hq install-hooks`
   writes `commit-msg`, `pre-commit`, `post-commit`, `pre-push` into
   `.git/hooks/` of the working repo. Each hook execs
   `bot-hq policy-check <sub> --data-dir … --project … --session …`
   which re-resolves policy and decides exit code. Hooks are
   idempotent, respect foreign hooks (write `<hook>.bot-hq` sidecar
   instead of clobbering).

**Policy file hierarchy:**
- `<data_dir>/config/general-policy.yaml` — defaults.
- `<data_dir>/library/projects/<project>/policy.yaml` — per-project overlay
  (lists are replaced, not merged).

Fields (`policy/mod.rs::Policy`): `push_gate` (scalar `auto`|`ask`),
`force_push` (scalar `blocked`|`allowed`), `per_action_approval`,
`branch_pattern`, `forbidden_in_commits`, `commit_style`, `round_cap`.
(push_gate/force_push are per-tier toggles inherited
general→project→session; there are no per-branch "remembered approvals"
or agent-side grants.) `tool_blocklist` is RETIRED — superseded by the
global Tool Gate below; the key is TOLERATED (listed among the known keys
so it does not warn) but parses into nothing and is not enforced.

**Hook details:**
- `commit-msg`: receives commit message file path as `$1`. Scans for
  forbidden words (stripping `#` comment lines). Exits 1 on hit.
- `pre-commit`: scans staged diff added lines only (so removing a
  forbidden word passes). Exits 1 on hit.
- `post-commit`: read-only audit. Writes `CommitGrep` violation if a
  forbidden word slipped through (--amend, --no-verify bypass). Exits
  0 — the commit already happened.
- `pre-push`: resolves the session's policy. `push_gate == auto` →
  allow (exit 0). `push_gate == ask` AND `BOT_HQ_SESSION_ID` is set →
  POST the running app's `/hooks/pre-push` route (addr read from
  `<data_dir>/.local/signaling-addr`), which surfaces a per-push
  Approve/Reject prompt via `request_approval` — naming the PUSHED refs
  read (lazily, once) from git's stdin lines, HEAD only as the fallback
  (round 10; before that it named the checked-out branch, which is not
  always what is being pushed) — and blocks on the user's pick: approve →
  exit 0, reject → exit 1. Fail-closed (exit 1 + a `PushGate`/Denied
  violation) if the app is unreachable; a push with no session context is
  blocked with guidance.

**Audit:** `src/policy/audit.rs` hashes each policy file at hook fire.
A hash change between fires logs a `PolicyMutation` violation
(audit-only in v1).

---

## Secret redaction (F10)

What bot-hq stores, shows and hands to a peer does not carry a secret an agent
produced. The user's rule (tray `7e3308f1`, 2026-09-26): **redact every stored
row except what the user types, from now on** — rows stored before F10 stay as
they were.

**One scanner.** `src/policy/secret_scan.rs::find_secrets` finds
self-identifying shapes only: PEM private-key blocks (a cut-short block ends at
its last key line), vendor-prefixed tokens, signed JWTs, an AWS secret beside its
key name, Laravel Sanctum tokens. `redact` replaces each span with
`[redacted: <what>]` and returns a clean string untouched (exact-match callers
depend on that). The same finder backs the Context Library push refusal and the
outward-publish refusal, so they agree on what a secret is.

**Decoded strings, never serialized text** (EYES `8eea5190`): scanning a JSON row
lets a cut-short key swallow the rest of the line and lets an escape hide a
token's boundary. So JSON is redacted leaf by leaf before it is serialized.

**Where it happens:**
- **Messages.** `Storage::post_to_channel` redacts the plain kinds (text, system
  notice, phase change, boot) through an exhaustive `MessageKind` match;
  `tool_use` / `tool_result` hold JSON the pump already redacted string by
  string (`core/pump.rs`) and pass through. The receipt the ring delivers is the
  stored string.
- **The verbatim door.** `post_to_channel_verbatim` is the one door that does not
  redact. Its production callers are pinned by a test to two:
  `core::broadcast::broadcast_user_message` (the composer and the staged Send)
  and the tray answer row, which is built from redacted pieces and keeps only the
  user's own words (a typed pick or rejection reason) as written. A door, not an
  origin check: approved-gate output is posted as origin `user` too.
- **The tray.** `ask_user_choice_inner`, which every park goes through, redacts
  the question and options once, so the in-memory park, the event and the row
  agree (a pick of a listed option still compares equal); the store redacts them
  again as the backstop for the close card and the queued outward gate. The
  re-ask dedupe compares redacted prompts.
- **Everything else agents write:** session docs (`session_doc_write`,
  `session_doc_write_eyes` — the new text, before an append is composed — and
  `session_doc_edit`'s replacement text), findings
  (summary, code ref, disposition reason; the re-raise lookup redacts its probe),
  feedback, retrieval queries, violation records (`action` / `detail` on the
  record), the halt slot and its banner (`emit_halt_row`), plugin text
  (`tauri_cmd::plugin_api::plugin_text`), and Context Library writes
  (`cl_write_file`, `cl_edit_file`'s replacement, an agent's folder description
  at its tool).
- **Outward publishes.** A `gh` / `curl` command whose PUBLISHED body carries a
  secret is refused before any reviewer lookup (`action_gate.rs`); headers and
  URLs are not published content.
- A CL write or session-doc write whose text was redacted says so in its reply,
  and a `cl_edit_file` whose `old_string` quotes a secret the file holds only as
  its marker is told to match the marker.

**Not redacted, on purpose:**
- What the user types: the composer and staged sends, a tray free-text answer,
  a custom session doc saved in the UI, Context Library tab edits and folder
  descriptions.
- A gated command's text (`session_tray.command_text`): it runs exactly as
  written, the approval card and the stale-gate confirm show exactly what runs,
  and five queries match it exactly. The user's recorded exception (tray
  `16393946`).
- The credential store (`models.auth_token`, `agent_configs.auth_token`).
- Rows stored before F10.

**Limits:**
- An agent sees its own tool results raw — claude-code hands them over before
  bot-hq stores anything — and a command the Tool Gate auto-allows through
  `action_gate` returns its raw output to the agent. Only the stored copies are
  redacted. A command that waits for the user's approval reaches the agent only
  through the redacted answer row.
- The Terminal tab shows raw live output; it is an in-memory ring, never stored.
- App log files are not redacted (the only agent free text found logged is a
  reviewer-override reason). Telemetry sends only hashes.
- The git hooks run the binary the app was launched as, so their violation lines
  are redacted only after a relaunch onto a build with F10.
- A secret the user types into a CL file and later removes still reaches the
  private library remote in history: the push scans the working tree (advisory
  `909d1150`; the user did not pick a history scan).
- Two agent questions that differ only in a secret redact to one text, so the
  second supersedes the first; two options that differ only in a secret show as
  one label. The close card's decline notice quotes the user's typed words
  redacted, while their answer row keeps them verbatim. An agent edit that
  carries a user-typed secret through a CL file rewrites it to its marker.
- Detection is by shape: a secret with no self-identifying shape (a bare
  password) is not found.

---

## Tool Gate

A global, user-configured keyword gate over agent **Bash** tool calls,
replacing the per-project `tool_blocklist` role (post-2026-05-29
fabricated-comment incident) with a single list that can also EXECUTE the
command on approval.

**Scope:** only participants granted `edit_files` — the PreToolUse hook is
injected via `--settings` on that posture alone. The read-only posture's
`--settings` carries the two compaction hooks, the zsh-trap lint (below)
and a model's `cli_settings` fragment (0079), never the Tool Gate hook: a role that holds no
`action_gate` could not answer a gate. A
reviewer is held by `--disallowedTools` instead, which is **fail-open** for verbs
a deny-list did not anticipate: `sed -i`, `tee` and `python3 -c` all write files
and none are denied. bot-hq deliberately relies on that for mutation-based
verification (see the project CL's conventions), so treat the denial as covering
the named tools, not as a write boundary. The inline allow-list gate that used to
hold a reviewer more strictly belonged to the native loop and went with it (D9).

- **Config:** one global list at `<data_dir>/config/tool-gate.json` —
  `[{keyword, mode}]`, `mode` ∈ `gate | auto_allow`, edited in Settings
  ("Gated Bash Keywords"). NOT per-project, NOT in `policy.yaml` —
  bot-hq-side, so nothing is written into a working repo.
  Matching (L, feedback #50/#62; `tool_gate::Line::hits`) is case-insensitive: a
  substring of the tool name; the keyword's words against consecutive words of a
  command the line runs (`outward_body::gate_surface` over the K parser; basename
  for the first word, short-flag clusters as letter sets, a trailing `=`/`-` word
  as a prefix); and a substring of the whole line — UNLESS the line is relaxable:
  balanced quotes, no heredoc, no substitution, nothing opaque, every command in
  `GATE_DATA_TOOLS` (grep, echo, cat, jq, ls, ps… — no option that runs, evaluates
  or writes a named file) with only assignments before it, no long option naming an
  action, and no output redirection but `/dev/null` and fd dups. A keyword holding
  shell syntax always keeps the whole-line match. `gate` wins over `auto_allow` on
  conflict.
- **Tripwire:** the PreToolUse Bash hook (`policy-check tool-gate`, injected
  at spawn via `--settings` into every participant whose role holds
  `edit_files` — `src/policy/hooks.rs`
  `run_tool_gate`) blocks a `gate`-matched command with **exit 2** and parks it
  through the authenticated `/hooks/tool-gate` route (with an `action_gate`
  fallback). The Codex hook uses the same route after checking caller posture.
  An outward `auto_allow` command also asks the host: unread content queues
  for review and automatic execution, while already-covered content or a
  roster without a reviewer may proceed. The route rechecks policy and the
  enabled caller's edit capability and never executes a command itself.
  Other `auto_allow`/no-match commands exit 0. Exit 2 is the only block form honored under
  `--dangerously-skip-permissions`.
- **Execute-on-approve:** `action_gate(command)`
  (`src/signaling/bridge/action_gate.rs`) re-classifies, surfaces
  Approve/Reject via the existing `request_approval` machinery, and on approve
  runs the command itself in the session's `working_repo_path` (from storage)
  under the AGENT's shell (`tool_gate::gate_shell`: `$SHELL` when POSIX-family,
  else zsh → bash → sh — macOS `/bin/bash` is 3.2 and choked on commands the
  agent had proved under zsh; round 10), returning combined output to the agent
  — an action request, not a permission request. (There are no session push grants: a gate-run `git push` meets the
  pre-push hook like any other push, and under `push_gate=ask` that hook is the
  gate.)
- **Automatic publication after review** (feedback #105, migration 0104):
  `approval_required` defaults true; only a validated outward `auto_allow`
  request inserts false. `reviewed_at` authorizes exactly `body_row_id`.
  Settlement records that proof; the background gate tick claims and runs it,
  rechecking current policy, findings and requester posture. Results use
  `auto_running` / `auto_resolved`, with no manufactured user answer. Body
  revisions revoke proof before changing pins. Concurrent preparations share
  a command lock; a stronger duplicate raises approval and promotes an already
  reviewed queue to a user card. Explicit approval, listed reads, urgent and
  windowed requests cannot acquire automatic authority. A configured down
  reviewer holds the request; the existing user-approved override promotes
  it to a user card. No reviewer in the roster permits direct execution.
- **An approved publish is read back** (`src/signaling/bridge/readback.rs`,
  feedback #57 #60 #97). After an approved single `gh issue|pr
  create|edit|comment` runs:
  - bot-hq takes the object's URL from the publish's stdout (gh prints its other
    lines to stderr);
  - it reads the object with a `gh api --method GET`, built only from checked
    parts of that URL and run through the same gate shell;
  - it compares the result with the bytes the approval's hash check read.

  The result row says the body equals what was approved, DIFFERS (with the first
  differing line), could not be read back, or was not read back and why. A
  pull request also gets the issues GitHub links it to close. An edit is read
  once more before a mismatch is reported.
- **A queued edit is shown against the live body.** The reviewer's message
  carries the diff against the issue's or PR's live body. That body's hash is
  stored on the gate (0088), and approval refuses the edit if the live body
  changed in between, the same shape as the body-file hash check (0084).

The global list defaults EMPTY (no gating until configured in Settings).

**A project's listed data reads** (group K, `src/policy/data_reads.rs`). A
project's `policy.yaml` may list `production_reads` and `staging_reads`.
`data_reads::match_command` matches an entry where a command RUNS
(`signaling::commands_run`, which sees through quotes, wrappers, `sh -c` and
`$(…)`):
- the entry's first word is the tool;
- flag words are spelling only;
- middle words are whole words in order;
- the last word may sit anywhere inside a word, including `NAME=` prefixes, so
  a host matches in `--host=`, a URL or `PGHOST=`.

A match parks for the user's approval whatever the keyword list says: the
executor's Tool Gate hook auto-parks it, `action_gate` parks it, and
`terminal_exec` refuses it. It is reviewed first: `data_read_review` queues it
for the reviewer every time, with no coverage shortcut, under the outward
publish's roster rules (no other reviewer → park directly; reviewer down → held
unless overridden). The card names the matched entry and the executor's
`approve_after`.

The reviewer has its own path, `read_gate` (tray `ef8fe5de`). It takes ONE
command that runs a listed entry, piped at most into head, tail, jq, grep, wc
or cut. It refuses `;`, `&&`, redirection, `$(…)`, wrappers
(`outward_body::single_read_command`) and write verbs (`write_word`), because
one list both gates the executor and limits the reviewer. It also refuses a
database client (`database_client`: `psql`, `mysql`, `sqlite3`…, whose own
command language can write where `write_word` cannot see; SQL goes through
`prod_read`) and takes a `bq query` only as one plain SELECT or WITH, with no
`;`, comment or result-storing flag (EYES `a2c4c8df`). A reviewer's read
parks straight for the user, its card reading "Reviewer's command (slug)"; an
executor's goes through review first. The read-only posture's `shell-lint`
hook refuses a listed read and points at `read_gate`.

**`prod_read`** (`bridge/prod_read.rs`, tray `afef06f0`) runs read-only SQL on the
project's production Postgres (`policy.yaml` `prod_read:`). `check_sql` admits only
SELECT, WITH, EXPLAIN, SHOW, TABLE and VALUES statements, plus whole-line `\d…`/`\x`.
It scans the SQL the way PostgreSQL and psql do (`$` inside names, dollar-quotes,
comments) and refuses wherever the two could disagree about where a string or comment
ends — a nested `/*`, an `E'…'` or `U&'…'` string, a backslash, a `$` that opens no
dollar-quote, a carriage return — or anything unterminated (EYES `1e62db22`: a
disagreement hides a statement). The command runs `psql -1 -q -f -`, and bot-hq's own
first statements make the transaction read-only, set its timeout and run one query,
after which PostgreSQL refuses to make it read-write; `PGOPTIONS` repeats the settings.
A direct call to a known function acting outside the transaction (`OUTSIDE_EFFECTS`:
`dblink`, `pg_terminate_backend`, `set_config`, `lo_`…) is refused too; a function the
database defines can still reach them, so a read-only role is the real boundary. The card
shows the SQL. The gate row is marked
`exec_kind = prod_read` (0089) by the handler alone, and only then does approval
read the password from the configured file (parsed, never sourced) and pass
`PGPASSWORD` in psql's environment. It is routed like a listed read: reviewed first
from the executor, straight to the user from the reviewer. A data read's result row keeps the first 48 KB and the last
8 KB of its output.

**The zsh `"$var:x"` trap is refused before any of this** (feedback #96,
`src/policy/shell_lint.rs`). Under zsh, `$name:` followed by a modifier letter
applies the modifier: `"$R:app/x"` is the absolute path of `$R` followed by
`pp/x`, and an error discarded with `2>/dev/null` leaves empty output that reads
as "nothing found". The lint scans the command (quoting, `$(…)`, backticks,
`${…}`, heredocs; anything it cannot follow passes) and, only when the shell is
zsh, refuses it with the braced form, `"${R}:app/x"`, which every shell reads
literally. The refusal opens "Not a Tool Gate stop": rewriting IS the fix. It
runs in four places:
- the Tool Gate hook, first, before the keyword match and the auto-park;
- `policy-check shell-lint`, the read-only posture's only PreToolUse hook. It
  runs the lint alone, never the keywords, and on a pass exits 0 printing
  nothing, so the permission layer that enforces a reviewer's deny list still
  decides;
- `action_gate`, before it parks or runs, under `gate_shell`;
- `terminal_exec`, under the terminal's `$SHELL`.

**Review authority (0104, hook/MCP activation in progress).**
`session_tray.approval_required` defaults true for existing and new user gates.
An automatic publish opts out at insertion, and `reviewed_at` is bound to its
current body row; a revision clears it and a stronger request can require user
approval, never remove that requirement. Automatic execution has a separate
atomic claim checking that proof, body identity, session openness and explicit
approval metadata. Its `auto_running` / `auto_resolved` states retain NULL user
answer fields. Reviewer settlement records that proof and returns without
executing commands. The background gate tick owns execution, through the same
body/script-pin and read-back checks as a user gate, so a slow publication never
blocks the reviewer pump's completion or the turn ring.
The background gate tick recovers reviewed unclaimed work after restart; a
claimed interrupted run is reported, never repeated. Current policy restrictions
or revoked requester capabilities promote the exact reviewed body to user
approval. The automatic claim atomically checks live requester capabilities and
open veto/revision findings too. Pause/cancellation holds new automatic work.
Results are system-origin rows; the gate ledger and chat chips distinguish
automatic authorization from a user answer. The hook and MCP entry points still
use their existing behavior until the remaining activation slice lands.

**A run window lets the user approve any time while the command waits for its
slot** (feedback #66a, the user's pick). `action_gate`'s `run_not_after`
(required; RFC3339, at most `RUN_WINDOW_MAX_AHEAD_SECS` = 24 h ahead) and
`run_not_before` (`validate_run_window`) park the command whatever the keyword
list says; `card_extra` puts the window on the card and `record_run_window`
writes it to the row (`session_tray.run_not_before` / `run_not_after`, 0098)
once the park produced a fresh gate. At approval `maybe_run_gated` reads the
window: closed → `refuse_approved_run` ("the run window closed at …"); not yet
open → the approval row carries only the promise ("Scheduled: … runs it then"),
a `⏱` notice is posted, and the row stays answered with no result and no
refusal — `is_scheduled` (`bridge/tray.rs`), the one predicate the tray view's
`scheduled` flag, the Cancel and `gate_status` ("approved, SCHEDULED") share.
`spawn_scheduled_gate_runner` (started by `main.rs` after storage is wired)
calls `run_due_scheduled_gates` every `SCHEDULED_GATE_TICK` (10 s):
`due_scheduled_gates` reads the approved windowed rows whose start has come,
each runs through the same `maybe_run_gated` (`deferred`, so the start is not
re-checked while the end is), its output lands as a `[system]` row linked by
`set_tray_run_result`, and `ChoiceResolved` refreshes the UI. A persisted,
linked result or refusal then queues a non-preempting `Summon` for the
enabled issuer (or its next active successor). Live work, Pause, declared
halts and gates retain control of when that turn runs. Closed sessions,
unrecorded results and duplicate claims produce no wake. `GateRunOutcome`
distinguishes terminal results from unchanged requests; even an expired
window's refusal takes the run claim before posting. No in-memory timer,
so a relaunch re-arms nothing. **Every run is claimed first** (EYES
2577e626; `session_tray.run_started_at`, 0100): `claim_gate_run` is one
atomic `UPDATE … WHERE run_started_at IS NULL` that both paths take before
executing — a gate approved inside its window runs at once, and while it runs
the row would otherwise look due to the scheduler; the claimed row is excluded
from `due_scheduled_gates`, `is_scheduled` is false for it, and a second claim
runs nothing ("NOT RUN AGAIN"). At boot `report_interrupted_gate_runs` turns a
claimed row with no result and no refusal — a run the previous process was
cut off in — into a refusal and a `⛔` row in its session; it is never re-run,
because whether the command did its work is unknown. `cancel_scheduled_gate`
(the tauri command behind the Cancel on the `ScheduledRuns` strip) records a
`run_refusal` the scheduler skips, with an audit record and a `⛔` row. A CLOSE
cancels the session's scheduled gates the same way (EYES 2f046a6f) — nobody
would be left to read the output — and the close recap names them on the card
first, so the user approves the close knowing what it stops.

**An URGENT gate reaches the user instead of waiting to be noticed** (feedback
#52; the user's pick was the desktop tier — no phone push). `action_gate`'s
`urgent_reason` (one line, at most 200 characters; `validate_urgent_reason`)
parks the command whatever the keyword list says, exactly like
`require_approval`, and `record_urgency` writes it to the row
(`session_tray.urgent_reason`, 0097) once the park has produced a fresh gate
(an identical re-issue returns the existing gate, whose urgency stands). A gate
that is PENDING fires `SignalingEvent::GateUrgent` → `session:gate_urgent`
(session, gate, agent, reason, command) and posts the `🚨 Gate … is URGENT`
channel row — the audit of who marked it and why; a QUEUED outward publish keeps
the mark and `promote_queued_row` fires the event when the reviewer's read makes
it the user's to answer. The frontend (`useOsNotifications`) sends that one
toast at once, focused or not, outside the burst queue and its cooldown, and
repeats it every `URGENT_RENOTIFY_MS` (10 min) until `session:choice_resolved`
names the gate — at most `URGENT_RENOTIFY_MAX` (6) repeats; at mount it reads
the pending tray once, so a relaunch with an urgent gate still parked notifies
again. The Settings Off toggle still silences it. The session tile turns red
(`NeedsYou.urgent`, "URGENT approval"), the card leads with `🚨 URGENT APPROVAL`
and the reason, and `gate_status` / `gate_history` carry the mark.

---

## Session policy

Each session freezes a **policy snapshot** at spawn — the resolved
general → project → session-overlay stack (`push_gate`, `force_push`,
forbidden words, `tool_gate`). The user edits it per-session in the gear
tab (Session Settings); agents cannot write policy. There are no
agent-side commit/push grants — push and force-push are pure per-tier
toggles (`push_gate: auto|ask`, `force_push: blocked|allowed`)
inherited general → project → session.

**Storage** (`src/policy/session_policy.rs`):
- Snapshot written to `<data_dir>/.local/session-policies/<session_id>.yaml`.
  Seeded WRITE-IF-ABSENT at spawn (`core/session.rs`) by resolving the
  blueprint with `session_id=None`, so re-opening a session preserves
  gear-tab edits.
- The git hooks (`pre-push`, `commit-msg`, …) read this snapshot via
  `Policy::resolve_at_root` (threaded `BOT_HQ_SESSION_ID`), so a hook
  subprocess sees the same session-scoped policy the agent runs under.
- Purged on `close_session` (`core/state.rs` → `bridge::cleanup_session_policy`);
  at bot-hq startup (`main.rs`) only the files of sessions that are no longer
  open go — an open session keeps its snapshot across a relaunch, gear-tab
  edits included.

The per-session **Tool Gate** keyword list is part of the same snapshot
(see "Tool Gate" above): `hooks.rs::run_tool_gate` reads the frozen
snapshot first, so editing the global `tool-gate.json` only affects NEW
sessions.

---

## The close card (0081) and the close-out learnings epilogue (rc3 **D15**)

An AGENT's `close_session` requests a current `session_tray` card of kind
`close`. Only answering that card Approve emits `SessionCloseRequest`; an old
answer is history, never permission for a later tool call, even before reopen.
The tool requires storage and serializes the whole read/nudge/recap/insert
sequence per session. `jsonrpc.rs`'s arm parks the card
(`SignalingBridge::park_close_card`: a gate — latches the ring, gate slot,
canonical Approve/Reject, no in-memory oneshot) carrying `close_recap`'s lines
(staleness sweep demoted to one advisory line, open-advisory count, whether a
CL delta was written); `resolve_choice` turns Approve into the ordinary
`SessionCloseRequest` and Reject into a keep-working system row. **With a
reviewer in the session the card is QUEUED first** (feedback #73 (2), the
user's pick): `queue_close_card` inserts a kind-`close` row with status
`queued` (`Storage::insert_queued_close`, no command), posts the recap — card
and detail — as the body row the reviewer must read, and summons the reviewer;
the same `settle_queued_outward` that settles a queued publish parks it for the
user once the reviewer's turn ends with the recap read (the promotion carries
the close card's approval context), or withdraws it when a blocking finding
names the gate, and the session keeps working. An advisory naming a close is a
note, not a #69 revise hold (`revises_queued_gate` skips kind `close`; no hold
row). The result is a `CloseQueue`: `NoReviewer` (a solo roster) parks directly,
as before; `ReviewerDown` (the reviewer's cached health stalled/dead with no RPC
inside `REVIEWER_LIVENESS_WINDOW`) parks directly too, and the arm's reply says
the recap was not read. The learnings-delta nudge below stays as the arm's one refusal; the UI
Close button (`tauri_cmd`) and `Capability::CloseSession` are untouched.

The tool returns its readable explanation plus `structuredContent` containing
`status`, `choice_id` and `next_action`. `needs_learnings` is the one retry
before a card exists; `queued` and `pending` require ending the turn, with no
further close call. A retry reads the existing card before generating a recap
or consuming advisory reports, including after a host restart. `closed` means
storage already records closure. Reject names the card and preserves the user's
next instruction. The card persists `close_archive` before review/approval
(0105); its prompt states that intent, and a retry cannot change it. Approval
passes that stored flag to the ordinary close path once, even after restart.
Advisory details never ask for another close request. `user_action` documents
are excluded from the unpromoted-scratch warning and named separately as
remaining on the dashboard until dismissed. The default role already uses
this one-card flow (0082/0105). Migration 0105 also repairs one unchanged,
standalone 0067 close paragraph inside a custom HANDS role, preserving all
surrounding text and other role fields. Edited or duplicated paragraphs stay
untouched.

A session that ends before anyone wrote anything down is D15's gap. The
agent's own `close_session` tool is soft-gated with a write-the-delta nudge
(A3b, `jsonrpc.rs`); the user's Close button reached `AppState::close_session`
directly and killed every subprocess without anyone being asked. The epilogue
closes that: `core/close_learnings.rs::decide` (pure, four arms — `Run`,
`SkipNoWriter`, `SkipBusy`, `SkipAlreadyHandled`) says whether a closing
session gets a learnings turn; `AppState::run_close_epilogue` is the live half —
close the row and release the UI FIRST, broadcast `CLOSE_LEARNINGS_PROMPT` to
the roster, wait bounded (`CLOSE_EPILOGUE_ARM_TIMEOUT` 15 s to be dealt,
`CLOSE_EPILOGUE_TIMEOUT` 180 s to finish), post one outcome row (`Wrote` /
`Declined` / `TimedOut` / `Failed` — a failed write never looks like a decline),
then tear down. A second close for a session whose epilogue is running
(`ClosePlan::JoinInFlight` — a double-clicked Close, or the epilogue's own agent
closing on the turn it was given) joins rather than tearing it down. Silence is
the documented expected outcome: an empty-handed session writes nothing.
Observed shape: an agent-path close (the agent calling `close_session` mid-turn)
decides `SkipBusy` by construction — the agent is expected to have written its
delta before calling; the epilogue exists for the USER's Close on an idle
session.

## Session worktrees

Repo-backed sessions default to an **isolated git worktree** so two or
more sessions can work the same project in parallel (per-session index,
checkout, and branch — no file races). Opt-out per session in the
New-session dialog, or globally via the `worktree_default` app setting
(Settings → Policy → Session defaults; it lived under the retired Agents
subtab until rc3 D8). The Maintain-CL dispatcher was removed by D15 — start a
session and instruct it.

- **Placement:** `<data_dir>/.local/worktrees/<session-id>/<repo-basename>/`.
  The repo basename stays the final path segment because
  `spawn_session_handle` derives the session's project from
  `working_repo_path.file_name()` — the worktree must map to the same
  project for policy + CL.
- **Row model:** `sessions.working_repo_path` = the WORKTREE (the path
  agents run in — action_gate, hook install, A-tab diff, and project
  derivation all read it unchanged); `sessions.base_repo_path` = the repo
  it was carved from (NULL = direct mode). Placement is decided at create
  (`tauri_cmd/sessions.rs::resolve_session_placement`); the worktree is
  materialized lazily at spawn (`core/worktree.rs::ensure_worktree`,
  idempotent across respawns, re-adds after a manual delete via
  `worktree prune`). If ensure fails, the session falls back to the base
  repo and the row is converted to direct mode so row-readers and the
  live handle agree.
- **Branch:** `bothq/<session-id>` from the base repo's HEAD at first
  ensure. Two worktrees can't share one branch, so per-session branches
  are inherent; merging back is the user's flow (push gate unchanged).
- **Hooks:** a linked worktree's `.git` is a FILE and git reads hooks
  from the shared common dir — `install_hooks` resolves the real hooks
  dir via `git rev-parse --git-path hooks` (also honors
  `core.hooksPath`), so the policy backstop covers every worktree of the
  repo. Hook identity stays per-session via `BOT_HQ_SESSION_ID` env at
  fire time.
- **Close:** `close_session` removes the worktree only when clean (plain
  `git worktree remove`, never `--force`); a dirty worktree is kept and
  logged for manual recovery. The session branch always survives.

---

## Storage (sqlite)

Schema at `migrations/0001_init.sql` + subsequent migration files.

**Tables:**
- `messages` (id PK, session_id, participant_id → session_participants,
  origin `participant`|`user`|`system`, kind, content, envelope JSON,
  created_at, plus the legacy `author` mirror — still read by four sites) —
  every row of every session: agent text and tool rows, the user's rows, and
  the host's injections. Indexes on `(session_id, created_at)`,
  `(session_id, id)` and `(participant_id, kind, id)` — the spin-detection
  seek; migration 0066 dropped two unearned ones. Delivered per participant
  through `participant_deliveries`.
- `sessions` (id PK, title, working_repo_path, base_repo_path,
  created_at, closed_at, archived, slot0/slot1_model_at_spawn,
  created_by_plugin, current_turn_participant_id, round_number,
  halt_declared_by/halt_reason/halt_declared_at, staged_message,
  phase_epoch, ipav_phase, roster_revision) —
  per-participant model, effort, ultracode and resume id live on
  `session_participants` (rc3 D10), which also carries **`effort_at_spawn` /
  `ultracode_at_spawn` / `spawn_knobs_recorded`** (0061) — what a participant was
  ACTUALLY spawned with, recorded because the effective value cannot be recovered
  from the user's `effort`/`ultracode` CHOICE (usually "inherit") and must not be
  re-derived later, which would answer "what it would be spawned with now".
  `spawn_knobs_recorded` separates "no override in force" from "predates 0061",
  which are otherwise the same two nulls. **Migration 0060 dropped the nine columns
  that used to shadow them** (`rain_enabled`, `brian/rain_model_id`,
  `brian/rain_claude_session_id`, `brian/rain_effort`,
  `brian/rain_ultracode`) — every one had no reader and no writer — and renamed
  the two that remain to the turn slots they always indexed. "Does this session
  run more than one participant?" is now a correlated subquery over
  `session_participants` rather than a cached boolean.
  `base_repo_path` is set for
  worktree-isolated sessions (see "Session worktrees"). There is NO
  `project` column — the project is derived at spawn from
  `working_repo_path.file_name()`. The CURRENT phase is persisted in
  **`ipav_phase`** (0063; see "IPAV state") beside **`phase_epoch`** (0062), a
  monotonic counter bumped on every transition, which is what stops a stale
  phase vote matching again after the phase runs backward and returns.
  `halt_*` (0054) + `halt_wake_at` (0069, a TEMPORARY halt's wake instant) is
  the single halt slot; `staged_message` (0058) the
  durable half of the Stage toggle; `current_turn_participant_id` and
  `round_number` (0044) are written by the ring and read by nothing in
  production (tests only) — cleared on close since round 7.
- `roles` (id PK, slug, name, description_prompt, capabilities, participation
  mode, default model, colour, …) — the user's roles, edited in Settings →
  Roles; the seeded HANDS/EYES prose lives here as user data.
- `session_participants` (id PK, session_id, slug, display_name, role_id,
  model_id, capabilities snapshot, participation_mode, turn_position,
  done_vote, effort, ultracode, claude_session_id, enabled, joined_at,
  color, label, effort_at_spawn, ultracode_at_spawn, spawn_knobs_recorded)
  — the roster; capabilities are the invite-time snapshot the runtime
  enforces.
- `session_participant_changes` (0107) records add,
  remove and model-switch intent, authenticated host-supplied origin,
  expected roster revision, a fingerprint of selected role/model settings,
  status/timestamps/error and a write-once recovery checkpoint. One pending
  or applying request per session; exact retries return the same record.
  Acceptance and claim validate inside immediate transactions and do not
  mutate the roster. A claim rechecks selections and revision; cancellation
  wins only before claim. Role/model settings are hashed, so credentials and
  full role prose are not copied into this ledger. Triggers advance the roster
  revision on membership/selection edits, excluding runtime bookkeeping.
  `prepare_participant_change` revalidates and commits the typed recovery
  checkpoint with the roster change: append a new identity/cursor, disable a
  retained member, or reset only the affected member's model/runtime/account/
  resume metadata. It returns frozen model/role inputs for the runtime;
  credentials and composed prompts are not copied into recovery data.
  A late invite's initial cursor is the session's current maximum message ID,
  captured in that transaction; no old-row delivery or gate-result receipt is
  manufactured. A switch keeps its delivered cursor. That history frontier is
  stored in the checkpoint and passed to candidate construction; publication
  refuses a cursor that moved meanwhile. Restoration never rewinds real delivery.
  Failure restores owned fields atomically; a failed addition remains disabled
  history. Current votes are retracted, never resurrected on rollback.
  Close and reopen settle unfinished operations in their own transactions;
  a newer external edit is preserved and recorded as a failed change rather
  than blocking closure. Database errors are not acknowledged as recovery.
  cold-start recovery rolls back applying work instead of replaying it.
  The host can reserve the turn ring before claim: it waits for the current
  completion, halt wind-down, boot and the shared atomic-tool flag; preserves
  queued steering and Pause; installs input and epoch together without a deal;
  and releases once. A cancelled waiter resumes normally; loss of an acknowledged
  owner keeps the ring stopped with a recap. Reservations belong to one ring
  instance and preserve the completed member's position when it is removed.
  Each installed `AgentHandle` also owns an incarnation fence shared with its
  pump. Retirement revokes new host writes and drains admitted writes across
  their async waits; late events, teardown and delayed retries cannot mutate
  a replacement's state. Ordinary process kill retains ordinary pump cleanup.
  Token revocation compares the retiring token so delayed cleanup cannot revoke
  a newer registration. Process-origin ring controls retain their incarnation
  through Pause/reservation replay and cached summons; retired controls are
  discarded when consumed, including obsolete stdin joins. Queued process
  interrupts additionally check the installed handle's identity. User mentions
  and already-persisted halt declarations remain session state; the interrupt
  accompanying a pump-origin halt targets only that pump's incarnation.
  This is host-write retirement, not OS-exit proof.
  Spawn preparation can now return an in-memory draft: candidate files and
  observed metadata, with no participant writes, MCP registration, handoff-file
  refresh or channel announcements. It uses supplied frozen model/prompt/
  override inputs. Codex may still prepare its selected hook/catalog files and shared
  trust cache; those are not participant metadata. Ordinary startup retains
  immediate persistence. `publish_participant_spawn` applies the draft's
  metadata and success status atomically only for the same prepared target,
  open session, runtime and post-preparation roster revision. Older checkpoints
  without that revision or history frontier still recover but cannot publish a candidate.
  Every spawn recovers an interrupted applying change before project setup or
  roster selection, including temporary-halt respawns. Recovery conflicts stop
  startup with a recap; pending intent stays pending and is not replayed as work.
  `core::participant_runtime` now owns the host's asynchronous operation:
  reserve, claim, drain watchdog/target writes, revoke the old token, await
  connector shutdown, prepare a frozen draft, bind the new input/epoch, then
  publish metadata and the runtime. Failed setup restores only its target from
  the prior spawn configuration; failed recovery keeps a recap and the ring held.
  Connector lifecycle tasks acknowledge child cleanup before sending their final
  event; supervisors await that receipt even with a full event queue. This does
  not claim Windows process-tree parity or control of escaped daemon processes.
  Per-session lifecycle locks serialize close/restart/start; teardown cancels and
  joins the worker outside the global session map before deleting resources.
  Publication/restoration hold that per-session barrier across database and
  review work; the global map is held only for runtime access/installation.
  The worker's barrier wait is cancellable, so a closer holding it can join the
  worker without deadlock. Request validation likewise stays outside the map.
  Runtime publication and automatic-publication authority share a per-session
  lock. Queued decisions are re-presented to the resulting reviewer, or promoted
  to a user decision when no reviewer remains. Pending user gates survive removal
  of their requester. Activity slots and the weak watchdog roster refresh together.
  Candidate prompt excerpts (phase docs, own handoff, first three/latest six
  authored-user ledger entries, including tray choices) cap encoded bytes at64KB
  and never advance an existing reader cursor. Excerpts exclude channel rows
  beyond the captured frontier, so startup-time rows remain normal unread input.
  The allowance reserves provider/tool room, estimates two bytes per token,
  counts the existing prompt and caps at64KB; it is an estimate, not a live
  context-meter reading. Small windows can receive only the coverage notice.
  Each excerpt reports source/shown bytes and truncation, and the prompt states
  which history was omitted. Channel `origin=user` is not proof of human
  authorship: it can also carry host/plugin text or command output.
  Session Settings invokes this worker through typed request/status/cancel commands.
  It shows saved role/model choices, last-active and capacity guards, a context-usage
  warning and durable operation status. Publication emits `session:participant_changed`
  before starting the replacement pump; the frontend refreshes the roster and drops
  only that participant's old live context reading. Existing peers' handoff mirrors
  gain a bounded host roster section, within the existing file budget, without
  rewriting their dated source docs or clearing handoff reminders. Omitted records
  are available through `session_history`. Delegated-agent authorization is separate
  work. A connector startup receipt is not proof of a successful model turn.
- `participant_cursors` (participant → last read message id) and
  `participant_deliveries` (participant, message_id, delivered_at,
  withheld_reason) — the ring's per-participant read position and its
  delivery record; the latter is write-only audit in production.
- `phase_votes` (participant, target_phase, artifact_fingerprint,
  phase_epoch; 0062) — the D37 vote rows; cleared on every transition.
- `activity_events` (session, recorded_at, state, slot0/slot1_busy; purged
  at 90 days), `cancel_events` (write-only telemetry), `context_readings`
  (per-turn context-window readings behind the meter, 0051),
  `agent_feedback` (the Feedback panel's rows).
- `agent_configs` (agent_name PK, provider, model_name, base_url,
  auth_token, `native` — unread, `context_window` — unread). **Seeds nothing,
  and any key is legal.** Until migration 0060 it carried
  `CHECK (agent_name IN ('emma','brian','rain'))` plus rows for those names, so
  it could not hold a key any rc3 roster produces (`hands`, `eyes`, `eyes-2`,
  `advisor`) — the spawn chain's lookup could only ever miss and fall through to
  the built-in default. 0060 rebuilt the table without the CHECK and carried no
  rows over, which makes this tier reachable for the first time since D10; it is
  still the fallback for the `models` registry below (see "Per-participant model
  selection"), and `native`/`context_window` were mirrored here by migration
  0038.
- `models` (id PK, display_name, provider, model_name, base_url, auth_token,
  `native` — unread, `context_window` — unread) — saved-model registry
  the per-session pickers reference by id. `native` (0036) opted the
  model into bot-hq's own agent loop and `context_window` (0037,
  nullable) fed that loop's context meter; rc3 D9 deleted the loop, and
  neither column is read now. Both are still in the table.
- `app_settings` (key PK, value) — key/value app settings
  (`default_model_id`, `worktree_default`, `adherence_nudges`).
- `session_tray` (id PK, choice_id UNIQUE, session_id, agent, kind, prompt,
  options_json, command_text, status, supersedes_id, asked_at,
  answered_at, picked_option) — durable awaiting-input tray
  (choices/approvals/gated commands). Survives app restart. Renamed from
  `session_questions`/`questions` in migration 0010. An approved gate's run
  is linked back (0087): `result_row_id` is the `messages` row that carries
  its output, with `exit_code` and `ran_ms`. `gate_status` reads the output
  from that row — redacted when it was written — instead of from a second
  copy, and resolves the 8-character id quoted in chat inside the caller's
  own session.
- `session_documents` (id PK, session_id, slug, body, phase, …) —
  per-session docs: one rewritable doc per IPAV phase (`phase` set), the
  reviewer's co-located `<phase>-eyes` doc, archived versions (`<slug>@<n>`,
  untagged: what a replace or an in-place edit superseded) and custom
  documents (untagged, own tab). `kind` (0099) is `user_action` for one of
  the user's action items and `dismissed_at` when they put it away. The MCP tools keep a long doc readable: a
  reply carries at most 48 KB of body (`DOC_REPLY_BUDGET_BYTES`), a doc past
  it is listed by its size and headings, and `session_doc_read` returns one
  section (`heading`), a line range (`lines`) or the matching lines (`grep`).
  `session_doc_edit` corrects a passage in place with `cl_edit_file`'s
  contract (one shared `replace_exactly`); `session_doc_write` can take its
  body from a file (`content_path`, 256 KiB); `cl_write_file(session_doc:)`
  copies a doc to the Context Library byte for byte. Every write stamps
  `last_writer` (a participant's slug, or `user` for a save in the doc's tab;
  migration 0094), and a REPLACE by a participant that has not seen the
  current version whole since someone else changed it is refused, naming who
  and when (feedback #102: the user's answers typed into `decisions-needed`
  were buried by the executor's rewrite). "Seen whole" is kept per
  participant in memory (`doc_seen`, keyed by `updated_at` + body hash): a
  whole `session_doc_read`, a `session_doc_search` reply carrying the body,
  line ranges read from 1 to the end, or its own write of the whole body. An
  append and `session_doc_edit` are never refused for that — and neither
  counts as having seen the doc unless the writer had already seen the
  version it changed, so a blind append cannot make the next replace
  acceptable. `promoted_to` / `promoted_at` (migration 0095) record where
  `cl_write_file(session_doc:)` copied a doc; the close recap lists the custom
  docs still unpromoted (handoff docs excepted), beside the open advisory AND
  blocking findings (feedback #73).
- `findings` (id PK, session_id, finding_uid UNIQUE, agent, severity,
  summary, code_ref, status, disposition_reason, disposed_by,
  created_at, updated_at) — EYES review findings backing the commit
  gate (migration 0021; FK → sessions ON DELETE CASCADE).
- `projects` (name PK, display_name, working_repo_path, description,
  created_at) — registered-project registry; FK target for `cl_index` /
  `cl_folders` / `cl_reads`.
- `plugins` (id PK, name, version, manifest_json, dir_path, enabled,
  installed_at) — installed-plugin registry.
- `plugin_kv` (plugin_id → plugins ON DELETE CASCADE, key, value,
  updated_at; PK (plugin_id, key); migration 0029) — per-plugin
  key/value store behind the `plugin_kv_get`/`plugin_kv_set` catalog
  commands; namespaced server-side.
- `cl_index` (id PK, project_id → projects, file_path, description, tags,
  created_at, updated_at, agent_visible; UNIQUE (project_id, file_path)) —
  SQLite-backed CL search index.
- `cl_folders` (id PK, project_id → projects, folder_path, description,
  tags, …) — folder-level CL descriptions (parallel to `cl_index`).
- `cl_reads` (id PK, cl_index_id → cl_index, session_id, agent, read_at)
  — audit of which CL files an agent read (the `cl_register_read` sink).
- `cl_atoms` (FTS5 virtual table: project_id, file_path, kind,
  heading_path, body, mtime, body_hash, code_hash; migrations
  0024/0026/0027) — heading-delimited CL sections, BM25-searchable,
  backing `cl_retrieve`. DERIVED + disposable: `cl_rescan` rebuilds it
  from disk; FTS5 column adds drop+recreate the table and the boot
  rescan repopulates.
- `retrieval_events` (id PK, session_id/agent nullable audit, project_id,
  query, atom/token/stale counts, returned_atoms JSON; migration 0028,
  deliberately FK-free append-only telemetry) — one row per
  `cl_retrieve`, feeding the Measurement view.

**No `Author` enum.** `messages.author` is a plain string: `'user'` or a
participant slug (`hands`, `eyes`, `eyes-2`, `advisor`). The two-variant
`Author` enum that used to bound it was deleted in the D10 hard retirement —
it could name `user`/`brian`/`rain` and therefore no participant this app
creates. `messages.author`'s own CHECK was dropped earlier for the same reason.
`origin` is the attribution that matters: host rows (phase notices, the idle
nudge, injected instructions) are `origin='system'` and reach agents tagged
`[system]`; `author` mirrors them as `'user'` for legacy readers only.
(Until round 7 the phase-change notice and the watchdog's chat notice were
written with `origin='user'` and read as the user's words.)

---

## IPAV state

`SessionHandle.ipav: Arc<Mutex<IpavState>>` — one per live handle, plus a `Weak`
registry in `signaling/bridge` (`session_phase`) so a tool call can read the phase
without reaching through the session map. `IpavState` holds exactly one field,
`current_phase`.

(This paragraph described a `HashMap<SessionId, IpavState>` with a `phase_log`
field until round 5. Neither has existed; `core/ipav.rs` carried the same wrong
sentence in its own doc comment, so correcting either one alone left the other to
re-seed it.)

Both halves are durable. The CURRENT phase is persisted in `sessions.ipav_phase`
(migration 0063): `AppState::advance_phase` writes the new phase's tag, and
`core/session.rs` restores it when it builds the handle, so a restart resumes
where the work is rather than at Investigate. The machinery that decides when the
phase may CHANGE is persisted too (see "The phase-advance vote"). NULL means the
session has never transitioned, and reads as Investigate.

(Until round 6 this paragraph said the phase was NOT persisted, and named "a
restart resets the chip to Investigate while the work continues wherever it was"
as a live consequence — which is verbatim the defect 0063 was written to fix.
That sentence sat directly below the round-5 correction note above it, in the
file every session is told to read first.)

Agents die with the app, so a restart gives fresh sessions; they resume
their own transcript via `--resume` off each participant's own
`session_participants.claude_session_id`. (It was `sessions.brian/rain_claude_session_id`
until rc3 D10 moved the read to the roster and migration 0060 dropped the
columns.) The native loop was the exception
— it held `messages` in the task, so bot-hq persisted them under
`.local/native-history/` and reloaded them at spawn. That store, its
session-close clear and its startup orphan sweep all went with the loop
(rc3 D9); a leftover directory from a pre-D9 install is inert and can be
deleted by hand.

---

## Context Library

Filesystem space at `<data_dir>/library/` — its own folder so it can be
backed up / cloud-synced independently of host-local state — holding agent
custom instructions, per-project conventions/notes. Markdown on disk stays
the source of truth; SQLite carries two DERIVED, disposable layers on top:

- **File index** (`cl_index`): one row per file (description, tags) for
  description-aware discovery via `cl_index_search`. Descriptions
  re-derive when a file's body changes on disk (rescan), so the TOC can't
  freeze at first-index.
- **Atom index** (`cl_atoms`, FTS5): each file split at headings into
  ~≤200-token *atoms* (oversized sections sub-split at bullet/paragraph
  boundaries, code fences kept whole — `util::split_into_atoms`).
  `cl_retrieve` runs ranked BM25 retrieval over them and returns atom
  BODIES inline under a token budget (convention/decision kinds win
  ties), replacing the "search → eyeball → Read the whole 38K-token
  file" loop. Atoms that cite repo paths carry a `code_hash` of the
  cited source (`bridge/cl_refs.rs`, stamped at rescan); retrieval
  recomputes it and prefixes `⚠ possibly stale` when the code has
  drifted since indexing. Every `cl_retrieve` logs one row to
  `retrieval_events` (tokens, stale/empty counts) — surfaced in the
  Library's Measurement tab. (A standalone `bench/cl_poison/` eval measured
  whether agents OBEY a poisoned atom or VERIFY against the source; it was
  removed with `bench/` — see "Eval harness — REMOVED".)

**All-agents files** (always loaded at spawn, same content for every
agent):
- `library/custom-instructions.md` (consolidated from the old per-agent
  `agents/<name>/custom-instruction.md` files; a one-time migration in
  `Paths::init` folds user-modified legacy copies in and deletes
  untouched seeds)
- `library/custom-general-rules.md` (optional user additions; the
  universal rules are hardcoded in `agents::general_rules`)

**Per-project files** (loaded on-demand via `cl_index_search`):
- `library/projects/<project>/conventions.md`
- `library/projects/<project>/notes.md`
- `library/projects/<project>/decisions.md`
- `library/projects/<project>/policy.yaml` (CL-coupled — the policy
  resolver + audit read it here)
- Free-form: anything else under `library/projects/<project>/`

`_globals` maps to `<data_dir>/library/` itself; named projects honor a
`projects.cl_path` override (absolute path) when set, else the convention
`<data_dir>/library/projects/<name>/` resolved via `Paths::project_dir`
— the single source of truth shared by the storage resolver, policy
resolver, and policy audit (so the `library/` location can't desync them).

**Agents write CL content directly via `cl_write_file`** (needs the
`write_context_library` capability — a role without it flags corrections in
chat instead; the seeded EYES role does not hold it). The tool is a guarded
create-or-replace
inside the project's CL root: relative-path + traversal checks, a 1 MiB
cap, atomic tmp+rename, mkdir-p for new subfolders, an automatic
`cl_rescan`, and a refusal on bot-hq-owned `_globals` system files
(`custom-instructions.md`, `custom-general-rules.md`, legacy `agents/`) so
an agent can't rewrite its own standing rules. The session-close learnings
delta (≤~5 non-obvious one-liners appended under `notes.md`'s
`## Learnings` with the full replacement body) rides this path, and a
`cl_write_file` lifts the close-out nudge exactly like `cl_rescan`. (A
human-review-queue predecessor — migrations 0025→0035 — was removed
2026-07-21: in practice its approvals were rubber-stamped, so the
friction bought nothing. The user still edits any CL file in the
Context Library tab.)

The close-out staleness sweep uses **explicit retirement declarations**, not
words removed by a rewrite. It recognizes line-leading ``Retired `name` ``,
``Deleted `name` ``, ``Renamed `name` to ...``, and `` `name` was retired `` forms
(also list items); quoted/fenced examples, questions and negated/planned
statements do not seed it. Appending a new declaration counts; copying an
existing one does not. Retraction in its source cancels it. Physical library
file deletions/renames are still detected through git. History files and lines
explaining retirements are excluded from hits. Reports remain bounded and
advisory; the separate `cl_stale_refs` tool checks repository references.

**The library is one git repository, and each write is its own commit**
(`bridge/cl_write.rs`, feedback #64 #65 #86). One process-wide lock,
`LIBRARY_GIT_LOCK`, covers a write from its first read of the file to its last
commit, so two writes never interleave and the guard below sees the file as it
is written. Under it, in order:
- the target's content git never saw (a bare `Write`) is committed alone;
- every OTHER pending change is committed apart, as `cl: N file(s) changed
  outside an agent write` (a fresh library is `git init`-ed here);
- the replace guard;
- the write;
- the write's own commit, of that path only, with the subject
  `cl: <project>/<file> (<agent>, <session>, <kind>)`, kind being `create`,
  `replace`, `append` or `edit`.

The reply names that commit's sha as the rollback point and which lines it
changed (`lines 40–52 (+14 −3)`, or `no change`). **The replace guard**
(`dropped_lines`) refuses a replace that would drop lines another writer added
since this session's last FULL write of the file (a `create` or `replace`; an
append or an edit never showed the session the whole file), or, when it never
made one, since the session started. It reads `git blame` from that base and
the writers from the commit subjects, and lists the lines; `confirm_overwrite`
lets the drop through. Edits made outside the CL tools count like any writer's,
except the library's first commit (what was on disk before it was a
repository). No repository, a git failure or no base means no guard. The app's own saves
(Library tab, a session's Context subtab) take the same lock and commit as
`cl: <path> (user)`.

**First-run init:** `templates/cl/` is baked into the binary. On first
start (no `version.txt` in the data dir), bot-hq seeds the templates
under `<data_dir>/library/`. A pre-`library/` install (root-level CL, no
`version.txt`) is migrated once into the new layout by `Paths::init`.
(There is no "initialize default" button for a missing file — that sentence
described a rebuild-era plan that was never built; the seeds are written by
`Paths::init` and a deleted file stays deleted until the user recreates it.)

---

## Data locations

Defaults (env-overridable via `BOT_HQ_DATA_DIR`):

- **Data-home schema marker:** `<data_dir>/version.txt`
- **Context Library (cloud-syncable):** `<data_dir>/library/`
- **Installed plugins:** `<data_dir>/plugins/`
- **Machine policy/config (`config/` since v1.1):** `<data_dir>/config/general-policy.yaml`,
  `<data_dir>/config/tool-gate.json`, `<data_dir>/config/claude-overrides.json`
- **DB file:** `<data_dir>/.local/bot-hq.db`
- **Single-instance lock:** `<data_dir>/.local/lock`
- **Violations log:** `<data_dir>/.local/violations.jsonl`
- **Policy-hash cache:** `<data_dir>/.local/.policy-hashes.json`
- **Screenshots:** `<data_dir>/.local/screenshots/`
- **Session policy snapshot:** `<data_dir>/.local/session-policies/<sid>.yaml`
- **Rendered handoff docs:** `<data_dir>/.local/handoffs/<sid>/<participant>.md`

Top-level dirs are **sync boundaries**: `library/` = user content
(cloud-syncable), `config/` = portable machine policy, `.local/` =
host-only runtime + secrets + logs (never synced). The binary itself ships
in a platform bundle (`/Applications/bot-hq.app` on macOS; `/usr/bin` or
AppImage on Linux; `Program Files` on Windows), NOT under `<data_dir>`.
Pre-`library/` and pre-`config/` installs are migrated once on launch (the
`config/` split landed in v1.1, 2026-06-09).

**Dev** runs against the same default `~/.bot-hq/`. `BOT_HQ_DATA_DIR` keeps a
source build separate from an installed release when you run both.

---

## Future: auth-token keychain migration

v1 stores auth tokens plaintext in sqlite. v2 plan: migrate to OS
keychain via `keyring-core`. Per-platform stores: macOS Keychain
Services, Windows Credential Manager, Linux Secret Service (dbus). The
migration logic: on v2-first-launch, read each non-NULL token from
`agent_configs`, `Entry::set_password` it under
`("bot-hq", format!("{project}:{agent}:{provider}"))`, then NULL out
the column. Bump a `schema_version` row so it runs once.

Fall back to plaintext-sqlite mode on keychain failure (headless CI,
Linux without Secret Service daemon) with a startup warning. See
[`docs/rebuild-archive/decisions.md`](docs/rebuild-archive/decisions.md#auth-storage)
for the original Phase 0 research.

---

## Plugins

The plugin runtime v1 is live (see "Plugin runtime" under Tauri +
React UI): consent-gated install, `bhq-plugin://` serving, postMessage
RPC through the Rust-side catalog proxy, heartbeat/crash recovery,
panel tabs. The author contract is [`docs/PLUGINS.md`](docs/PLUGINS.md);
`examples/hello-plugin/` is the template + integration-test fixture.

Deferred plugin TIERS (extension points documented in PLUGINS.md):
plugin-contributed MCP tools (agent↔plugin), manifest-declared agents,
child-webview surface (real Browser tab), background execution,
zip/signed URL installs, inline `slot_name` slots, host-event relay.
(Per-plugin CSP extras SHIPPED — consent-gated `csp.extra_origins`,
`manifest.rs::validate_csp_extra_origins` / `serve.rs::build_plugin_csp`;
PLUGINS.md §Extra CSP origins.)

Concrete plugin ideas building on the runtime (each needs its own
design doc): **Cognotify** (human-comprehension deck over sessions +
CL), **Discord bridge**, **Clive** (legacy bot port), **CL cloud
sync**, **GitHub tab**.

---

## Eval harness — REMOVED (2026-08-17)

`bench/` held two Python harnesses: `swebench/`, a SWE-bench rollout harness
that drove sessions over real GitHub issues and scored the patches, and
`cl_poison/`, an obey-vs-verify eval measuring whether agents follow a poisoned
CL atom or check it against the source. Neither was compiled into the binary or
run at startup.

Both drove bot-hq through the **external MCP server, removed earlier the same
day**, and `cl_poison` imported `swebench`'s client directly
(`sys.path.insert(...)` + `from bothq_client import …`), so the two failed as a
unit. They were first bannered NON-FUNCTIONAL and kept, on the reasoning that
the rollout/scoring logic is transport-independent; round 6 deleted them at the
user's direction rather than carry ~1,900 lines of frozen Python that must be
rewritten whenever the driver returns as a plugin.

**The design record is the code itself:** `git show b604c67:bench/swebench/run_rollout.py`
and its siblings. That is the same convention this file uses for the driver's own
removal above.

---

## Glossary

- **Participant:** one claude-code subprocess in a session, playing a ROLE and
  displayed as `ROLE · Model`. A session runs N of them (dialog default 1, cap
  4; backend cap 8), each holding its role's capability snapshot.
- **Role:** a user-owned template — capabilities, instruction prose,
  participation mode, default model — edited in Settings → Roles. The seeded
  pair is HANDS (executes) and EYES (reviews); they are the user's config, not
  bot-hq's furniture.
- **IPAV:** Investigate → Plan → Apply → Verify. Discipline framework
  agents follow within a session.
- **CL (Context Library):** filesystem space at `<data_dir>/library/`
  holding agent custom instructions, rules, per-project conventions/
  notes. Indexed in SQLite for description-aware search.
- **Session:** a scope-keyed work container, holding N participants +
  chat history.
- **Emma:** removed (former solo helper agent; planned to return as the
  first bot-hq plugin — TBD).
- **claude-code:** the upstream CLI tool that wraps a language model.
  One subprocess per **CLI-backed** agent.
- **Native loop:** *removed.* bot-hq's own in-process Rust agent loop,
  a second backend opted into per saved model, EYES-only. Deleted by rc3
  D9 (2026-08-12) so the claude CLI is the default model connector (the Codex CLI joined it 2026-10-21, per model row — D9 amended); it
  returns later as a plugin, starting from `git show
  c7bba28:src/agents/native/`. Its two data files
  (`.local/native-accounting.jsonl`, `.local/native-history/`), the
  `models.native` checkbox, `AgentRole`, `may_run_native` and the
  high-context notice went with it.
- **stream-json:** claude-code's `--output-format stream-json` mode.
  One JSON event per line on stdout. See
  [`docs/stream-json-events.md`](docs/stream-json-events.md).
- **MCP (Model Context Protocol):** the protocol claude-code uses for
  external tool servers. Bot-hq runs ONE in-process — the internal
  agent↔UI signaling server. The second, the external driver, was removed
  on 2026-08-17 and demoted to a future plugin.
- **Policy:** machine-readable subset of CL rules — `general-policy.yaml`
  + project overlay. Drives forbidden-word grep, push gate, force-push
  gate.
- **Session policy snapshot:** the resolved general → project → session
  policy frozen per-session at spawn (`session_policy.rs`), editable in
  the Session Settings panel (the gear button in the session header). Push/force-push are pure toggles — no agent-side grants.
- **Awaiting flag:** per-session `Arc<AtomicBool>` set by the halt-declaring
  tools (`mark_awaiting_user` / `halt`, `request_phase_advance`) and by
  the host's gates (`action_gate`, the pre-push gate, the override) — NOT by
  `ask_user_choice` or an agent's `request_approval`, which park and stop
  nothing (rc3 D35; round 12). It
  drives the input-lock activity state; the ring stop itself is
  `SequencerCommand::HaltDeclared` / `GateOpened`, and a halt stops the ring
  where it stands.
- **Violations log:** append-only `.local/violations.jsonl` under the data
  dir recording policy enforcement events (denied tool calls, post-
  commit greps that fired, policy file mutations).
- **Tray (`session_tray`):** durable per-session record of awaiting-input
  items — `ask_user_choice` / `request_approval` / gated commands — so
  they survive app restart. Renamed from `session_questions` (migration
  0010).
