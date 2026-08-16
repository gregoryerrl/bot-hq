# rc3 audit — round 4 (2026-08-17)

Two mandates from the user: *"find messes, stalenesses, redundancies,
misimplementations and look for potential refactors AND/OR optimizations"*, and
*"this can also be a live evidence session of what round 3 audit session did
(since this is a new binary and their implementations are in this
instance/build)."*

Baseline at `90840a5`, tree clean: `cargo test` **1240 / 0 / 1 ignored** (every
`test result:` line summed, doc-tests included — `PROGRESS.md`'s 1239 counts
differently; the method is stated so the figure is reproducible), `cargo clippy
--all-targets` **4**, `npx vitest run` **392 in 45 files**, `npx tsc --noEmit`
clean.

---

## 1. Live evidence — round 3, observed rather than re-read

### R5 (migration 0059) — CONFIRMED, and still true under traffic

| table | rows | RFC3339-Z | zone-less |
|---|---|---|---|
| `participant_cursors.updated_at` | 94 | **94** | 0 |
| `participant_deliveries.delivered_at` | 4 647 | **4 647** | 0 |

Round 3 measured 85 and 4 011. **9 cursor rows and 636 delivery rows have been
written since**, by live traffic on the fixed code, every one well-formed. Not
"the code looks right" — the rows the fix governs kept coming out right.

### Migration 0060 — APPLIED AND LIVE

`_sqlx_migrations` highest applied = 60 (`retire the agent names`, success = 1).
`pragma table_info(sessions)` returns `slot0_model_at_spawn` /
`slot1_model_at_spawn` and none of the nine dropped columns.

### F13 — NOT observable, and that is the verdict

Zero rows matching `[PHASE REQUEST%` exist anywhere in the database. The DB was
reset 2026-08-12 and no agent has called `request_phase_advance` since, so the
defect left no artifact and the fix has produced none either. **F13's proof
remains its test, not the record.** It is not confirmed live and is not reported
as if it were.

### A near-miss, recorded so round 5 does not re-flag it

All 114 `phase_change` rows are `origin=user, author=user`. That reads like F13
residue and is not: `storage/messages.rs:122` documents `phase_change` /
`system_notice` as deliberately synthetic host rows, and `:127` excludes exactly
those from the user-engagement count. F13 was about `MessageKind::Text` receipts,
a different writer.

---

## 2. Findings

### F1 · The frontend `ParticipantView` was never reconciled, and rc3 D20's per-participant Name was dead in the UI · HIGH · FIXED

`frontend/src/lib/participants.ts` carried its own merge instruction — *"When the
two units merge: delete `ParticipantView` from this file, import it from
`../lib/bindings` instead"* — from a two-unit split that landed long ago. The
reconcile never ran, and the shapes diverged: Rust `ParticipantView` has **11**
fields, the hand mirror had **8**. Missing `label`, `effort`, `ultracode`;
`color` optional where the backend makes it required-nullable.

**The consequence was a user-visible feature that did not work.**
`participant_display_name` (`storage/participants.rs:399`) takes four inputs and
its own comment states the rule: *"The label replaces the role-and-ordinal half,
and only that half (rc3 D20, migration 0053)."* The frontend's
`participantLabel` took three and had no label branch, because its type had no
label field. Every UI surface resolves through it — chat bylines, the turn-status
line, tray cards, the Quickview, the enforcement log, the session header roster,
the Dashboard tile byline, and (`SessionView.tsx:751`) the `@mention` menu.

Sharper than "not rendered", and this is the reviewer's framing:
`participant_display_name` has exactly two production consumers —
`display_name_of` → `resolve_roster_facts` (`core/session.rs:1504-1516`), which
builds **the agent's own system-prompt roster**, and `session_docs.rs:173`. Both
pass the label. So for a labelled row **the agent was told it is `Driver · Claude
Opus 5` while every user-facing surface said `HANDS · Claude Opus 5`** — two
surfaces asserting different identities for one row.

Six live `session_participants` rows carry user-typed labels across `s-8ac0d2d0`
and `s-382d3d18`. The write side worked (`Dashboard.tsx:430`); the read side could
not see the column.

**Why no gate caught it.** `tauri_cmd/sessions.rs` asserted the Rust joiner over
the view's halves and commented *"The frontend joins them the same way; this is
the shared implementation"* — with `label: None` in every fixture row. A fixture
that cannot separate "honoured" from "ignored" is conventions.md's test-fixture
shape rule, and `participants.test.ts` had no label case at all.

**Fixed:** the mirror keeps its hand-written form (see F1b), gains `label` and a
required `color`, and `participantLabel` gains the branch — the label replaces
the role-and-ordinal half, the model suffix survives, blank and whitespace fall
back to the ordinal. Five vitest cases mirror the Rust table one-for-one,
including all four blank forms, because the divergence the fix could re-create is
a `"  "` label rendering `"  · Claude Opus 5"` on screen. Verified by mutation:
removing the branch fails exactly the four label tests. The Rust fixture now sets
a label on one row and leaves the other unlabelled — a **difference between
rows**, not a constant a view could invent.

### F1b · The merge instruction was itself the stale artifact · FIXED

Executing it would have made things worse. `frontend/src/lib/bindings.ts:1` is
`// @ts-nocheck` and regenerates only at app launch, so importing the contract
from there removes the frontend's only *checked* declaration and leaves a fresh
clone type-checking against whatever was committed. `notes.md` records the
hand-mirror convention outright. The block now says stay-in-step, in the shape
`slugOrdinal` already used.

### F2 · Three comments named `brian_*` / `rain_*` as the CURRENT field names · MED · FIXED

Round 3 renamed the busy/health wire to `slot0_*` / `slot1_*` and updated some of
the prose about it. Three sites still asserted the old names in the present tense:
`tauri_cmd/sessions.rs:856-862` (*"The field NAMES are frozen wire … `brian_*` is
the participant at turn position 0"*, sitting above `pub slot0_busy`, and copied
verbatim into generated `bindings.ts:2002` because it is a `specta` doc),
`frontend/src/stores/runtime.ts:8`, and `runtime.test.ts:88`.

`Providers.tsx:175-181` shows the correct form from the same pass — *"They **were**
`brian_busy` / `rain_busy` until the D10 hard retirement"*. One rename pass fixed
one site and missed three: a fresh instance of round 3's own corollary, *changing
a parameter does not prompt a re-read of the comment above it.* Also fixed here:
`tauri_events/types.rs:169` called it *"a session's **duo** activity"*.

### F3 · Round 4's named task, answered — there is no second inert assertion · MEASUREMENT

Round 3 left: *"272 test-fixture occurrences remain unswept, at least one of which
was an inert assertion. How many of the other 271 are inert is unmeasured. That is
round 4's most concrete task."*

Re-measured with round 3's recorded command: **522 matching lines, 292
non-comment** — reproduces its figures exactly. Inertness needs an assertion whose
expected value the fixture never produces, so the searchable shape is *a test where
the slug appears only inside `assert*` and nowhere in the setup*. Eight tests have
it; all eight hand-read:

| site | verdict |
|---|---|
| `jsonrpc.rs:1405`, `:2300` | REAL — `caller()` (`:1304`) sets `agent: "brian"`; genuine round-trips |
| `policy/mod.rs:731-755` (5) | REAL — `"rain"` is the literal needle for `contains_word`'s boundary semantics |
| `capability_prompt.rs:553` | REAL — a deliberate ban-guard, reason in the comment beside it |
| `server.rs:496-497` | REAL — an arbitrary path segment in a reject case |
| `prompts.rs:368-369` | REAL — pins that a retired slug draws no builtin prose |
| `participants.rs:4527` | live, but its **message** was wrong — fixed |

**The 272 are stale fixture NAMES, not inert assertions.** Round 3 found the only
inert one. `participants.rs:4527` asserted over `participant_by_slug(…, "hands")`
while its failure message said *"two 'brian' rows"*, sending a reader hunting for
rows no roster has; the locals and the message now say `hands`. Two more of the
same class fixed in `bridge/tray.rs:1589` / `:2393` (*"so the duo resumes"*).

### F4 · Round 3's completeness claim is still false, and the INSTRUMENT is why · HIGH · FIXED

Round 3's restated claim: *"No production identifier names them as a current
thing. Every one of the 20 production occurrences is in a category that exists to
name a retired thing."* Two live identifiers falsify it:

| site | what it is |
|---|---|
| `agents/spawn.rs:1122` | `fn build_rain_disallowed_tools()`, called at `:1345` |
| `core/state.rs:1600` | `fn should_peer_ack_nudge(…, has_rain: bool)`, live via `state.rs:1537` |

Neither is a frozen template, a frozen wire, a gate transcription or a ban-guard
word list.

**Why three rounds of sweeps could not see them.** The recorded command is `rg -w
-e brian -e rain …`. `-w` requires a non-word character on both sides, and **`_` is
a word character**, so a compound identifier can never match. Measured, not
reasoned: `rg -w` returns **zero** for both sites; an unbounded search returns
seven lines. The instrument that produced 522 / 292 / 20 is **structurally blind to
exactly the class where a live name survives** — a bare `rain` is prose or a string
literal; a live name is almost always compound. Every round measured the safe half
with confidence.

`state.rs:1596-1601` was stale three ways at once: the parameter name, a doc saying
*"in a duo session … the peer-ack nudge to Brian"*, and the gate itself — the caller
passes `handle.agent_count() > 1`, any roster above one, not a duo.

**Fixed:** `build_rain_disallowed_tools` → `build_read_only_disallowed_tools`
(selected by `!grants(Capability::EditFiles)`, never by slug — the name read as if
a deleted slug still gated enforcement); `has_rain` → `has_peer` with a doc that
states the real gate. Five test names and five locals renamed with them.

### F5 · The guard that ends the recurrence · NEW · `tests/retired_identifier_test.rs`

The recurrence has a mechanism behind it. The frontend **cannot** regress —
`framing.ts` + `framing.test.ts` sweep it. Rust's only sweep
(`protocol.rs:~950-990`) covers MCP tool `description` prose. **Nothing swept Rust
source identifiers**, which is why F4 exists and why round 5 would have found the
next one.

The guard splits each identifier on `_` and compares **segments**,
case-insensitively — what `-w` should have been:

| identifier | verdict |
|---|---|
| `has_rain`, `build_rain_disallowed_tools` | flagged |
| `the_drain_rather_than_finishing_it` | clean — `drain` ≠ `rain` |
| `brian` (bare) | clean — a string literal is legacy DATA |

Substring matching flags all three; `-w` flags none; segments flag exactly the
right one. Comments are exempt for the reason `framing.ts` gives — *a comment
describing a 2026-05-28 incident in that day's vocabulary is a RECORD.*

**The exemption tax was the open question and it is two files**, both already
documented as frozen: `external_jsonrpc.rs` (the published driver wire) and
`paths.rs` (`LEGACY_*_CUSTOM_INSTRUCTION`, byte-frozen against
`body.trim() != legacy_template.trim()`). A second test asserts each exemption
still carries a retired identifier, so a stale carve-out cannot quietly become the
hole the next live name enters through.

Verified by mutation: reverting `has_peer` to `has_rain` fails the guard naming
both lines, while the past-tense comment mentioning `has_rain` one line above is
correctly ignored.

---

## 3. Declined, with the reasons recorded so round 5 does not re-derive them

- **`participant_views` is an N+1** (`tauri_cmd/sessions.rs:485-514`: 1 roster
  query + `role_by_id` + `get_model` per participant, called per Dashboard tile).
  Measured: **1** dashboard-visible session × 2 participants = 5 queries, against
  `roles` = 3 rows and `models` = 5. Real shape, no cost at this scale. A JOIN is
  the fix if it ever matters, and it must preserve the reason `:495-498` gives for
  reading both halves live rather than off the frozen `display_name`.
- **`large_enum_variant` on `Handover`** (`core/sequencer.rs:2874`, 304 bytes) —
  a turn-path return constructed once per handover; boxing trades a stack copy for
  a heap allocation on that path.
- **The three remaining `too_many_arguments`** (`cl_write.rs:22`,
  `storage/plugins.rs:21` / `:84`) — round 3's stated bound, unchanged.
- **~24 over-exported frontend symbols** (`RETIRED_FRAMING`,
  `LIST_PARTICIPANTS_CMD`, `isSpawnable`, `matchMentionables`, several in
  `pluginBridge.ts`) — referenced only inside their own file. Surface, no behaviour.

## 4. Refuted during review, recorded so they are not reopened

- **The read-only deny list is slug-gated, so a reviewer now spawns unrestricted.**
  The highest-stakes reading of F4 — a function named `build_rain_*` selecting
  enforcement by a deleted slug. **Refuted:** `spawn.rs:1305` is
  `if !cfg.capabilities.grants(Capability::EditFiles)`. The stale name was
  cosmetic; the enforcement was correct throughout.
- **`wire::SPAWN_MODEL_FIELDS` still names 0060-renamed columns.** **Refuted:**
  the emit sites read `s.slot0_model_at_spawn` / `s.slot1_model_at_spawn`
  (`external_jsonrpc.rs:461-462`, `:787-788`). A documented frozen-wire exemption,
  pinned by its own test.
- **`has_rain` → `has_peer` papers over an N-participant defect.** **Refuted:**
  the gate is `agent_count() > 1` and the roster search is
  `find(|a| a.edits_files())` — capability-based. The rename is cosmetic; the false
  part was the doc, which F4 fixed.
- **"duo" survives in live Rust.** **Refuted by measurement:** 131 whole-word hits,
  and the non-comment set is the guard word-lists themselves (`protocol.rs:975`,
  `general_rules.rs:506`), test fixtures, and one documented exemption
  (`plugin_api.rs:139` `args.get("duo")`, a legacy wire alias retained because
  dropping a wire key breaks installed plugins).

## 5. Parked for the user

`effort` / `ultracode` have **zero** frontend consumers — `rg 'effort|ultracode'
app/SessionView.tsx` returns one hit, the word "best-effort" in a comment. The
backend half is pinned by `participant_views_carry_the_rows_effort_and_ultracode`;
the consumer does not exist; the joining line is tested by nothing. That is
conventions.md's wire rule, instance six. Adding the fields to the mirror renders
nothing and rebuilds the dead-wire shape one layer up, so it is **deliberately not
in this batch**: the Rust doc's stated purpose — *"the session view had no way to
show what a running participant was actually spawned with"* — needs a SessionView
surface, which is a feature decision.

## 6. Stated bounds

- **The frontend is now partly cleared, not cleared.** The participants / runtime /
  activity seam was read in full — that is where F1 and F2 live. `ClaudeConfig.tsx`
  (1 314 lines), `ContextLibrary*` and `PluginManager.tsx` were not read.
- **F1's "no frontend surface reads `label`" rests on a grep plus a read** of every
  consumer of `participantLabel`, not on an exhaustive render audit.
- **No live multi-participant turn was run against the new label branch.** Its
  proof is the mutation test plus the vitest table, not a session that rendered a
  labelled participant on screen.
- The guard covers `src/**/*.rs` only — not `tests/`, not `examples/`, not
  migrations (immutable) and not the frontend (already guarded).
