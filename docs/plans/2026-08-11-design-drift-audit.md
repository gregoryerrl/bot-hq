# rc3 drift audit — validated decisions vs. what exists

**Run 2026-08-11**, after the user asked whether the redesign had lost its
direction. Method: every `✅ VALIDATED` decision in
[`2026-08-06-session-focused-redesign-design.md`](2026-08-06-session-focused-redesign-design.md)
checked against the schema, the code, and
[`…-implementation.md`](2026-08-06-session-focused-redesign-implementation.md).

**Headline: the direction is intact.** Section 2 (Roles) is recorded exactly as
the user restated it, the `roles` table exists with the right shape, and HANDS /
EYES are seeded as editable rows. Nothing was lost or redirected.

**But three validated decisions exist in neither the code nor any batch.** They
did not slip a schedule; they fell out of one. That is the failure this document
exists to stop repeating.

---

## A. Missing and UNSCHEDULED

### A1. Round cap — the backstop task 14 assumes ⚠ HIGHEST SEVERITY

Design §1b lists two backstops: spin detection (primary) and a **round cap**,
"high enough to be invisible in normal use; visible and user-overridable per
session".

Router inventory row **#1** marks the L2 hard cap **DISSOLVED** *on the grounds
that* "round termination is consensus (design §1b), with a round cap as a crude
backstop."

**Neither half of that backstop exists.** `sessions.round_number` is a column no
Rust code reads. `participant_deliveries.withheld_reason` reserves `'round_cap'`
and nothing writes it. Grep for `round_cap|ROUND_CAP|round_number` across `src/`
returns nothing.

**Why this blocks task 14 specifically.** That task's gate requires every
DISSOLVED row to have "the structure that makes it impossible to actually
exist". Row #1's structure is the fixed ring — which does prevent *emergent
ping-pong* — but the row's own justification promises a replacement net, and the
net is absent. Delete `router.rs` today and a cycle whose consensus never
arrives (one participant that never votes done, e.g. an agent looping on a tool
error) has **no bound at all**. The hard cap is currently that bound.

Fix: implement the round cap before task 14, or amend inventory row #1 to say
the ring alone is the whole justification and consensus is the only stop — a
decision the user should make, not an implementer.

### A2. PASS — a participant declining a turn

Design §1: *"A participant may **PASS** rather than burn a turn. The pass is
recorded in the channel so it is visible."*

No implementation, no plan task, no test. `TurnComplete { done: true }` is a
**done vote**, which is a different thing: done means "nothing left to do" and
feeds consensus; a pass means "not me, this round" and should not. Today a
participant with nothing to say must either vote done — inflating consensus
toward a false arrival — or emit filler, which is what the archive study found
destroying reviews.

### A3. `on_demand` is unreachable

Design §1 defines three participation modes. `active` and `observer` both work
(`an_observer_is_skipped_not_given_a_no_op_turn` pins the second). `on_demand`
is *"not in rotation; reads; posts only when addressed."*

The rotation half is right — `next_active_participant` skips it. The **addressed
half does not exist**, and the same section deletes "per-message addressing" as
part of what the ring replaces. So an `on_demand` role can be created and can
never act: never rotated, never addressable. The mode is currently a way to
build a participant that cannot participate.

Fix: either give the ring an "address participant X" command, or drop the mode
from the design until there is a mechanism.

## B. Missing but SCHEDULED — no action needed, listed so they are not re-found

| decision | where it lands |
|---|---|
| `description_prompt` composed at spawn; hardcoded prompts deleted | **B7** (spawn derivation) |
| Roles tab: add/edit roles, per-role prose | **B8** (UI) |
| Session creation asks how many agents, **default 1** | **B8**; today `ensure_session_roster` hardcodes two from the per-agent `sessions` columns — see [`2026-08-11-agent-name-removal.md`](2026-08-11-agent-name-removal.md) |
| `roles` seeded prose migrated out of `prompts.rs` | **B7**; the rows exist, `description_prompt` is NULL for both |

## C. Found by the first live run (2026-08-11)

### C1. A native turn was one message, not one turn ✅ FIXED `d874d33`

> **AMENDED 2026-08-11.** The original entry said "the native loop self-drives".
> **That was wrong**, and the correction is kept visible rather than overwritten
> because how the diagnosis moved is the useful part.

With `BOT_HQ_SEQUENCER=1` on session `s-156543b6`, the ring delivered, cursors
advanced, and turns alternated — the model works. But Rain (native,
`deepseek-v4-pro`) took a fresh turn every ~5s while Brian held the turn, all
correctly discarded by the epoch guard (`completion does not name the turn in
flight`), so the ring was never corrupted — she just burned tokens out of turn.

**The mechanism was not self-driving.** `run_loop` blocks on `input_rx.recv()`
exactly as the subprocess blocks on stdin. What actually happened: the ring
writes **one stdin message per channel row**, and this runtime answered **each
one with its own API request**, while claude-code absorbs a queued burst into a
single turn. That breaks design §1's *"a turn is one participant's entire turn
(many tool calls), not one message"* — token cost is the consequence, not the
contract.

**The burst shape is the evidence; the aggregate is not.** 87 rows in → 82 turns
out is equally consistent with a self-driving loop, and the first reading drew
exactly that wrong conclusion from it. A single drain of **59 rows answered
one-per-request**, with 84 of the session's 135 calls landing *after* the last
row arrived, is not. Measured 2026-08-11: 87 rows in three drains (27 / 59 / 1),
135 API calls, **7,523,266 prompt tokens**.

**A second wrong fix was caught before it shipped.** The obvious cure — hoist
completion emission and re-fold *after* a turn — would have frozen the ring. The
pump snapshots the epoch on a turn's first event and clears it at
`TurnComplete`, so a turn spanning two folds reports the stale epoch, the guard
discards it, and nothing further arrives. One-turn-per-message was *accidentally*
the liveness mechanism that re-snapshots the epoch.

**Shipped fix:** fold only what is already queued **at the wake**, before the
turn starts; completion cardinality is unchanged at one per turn. ~15 lines in
`src/agents/native/agent.rs`, pinned by
`a_burst_of_queued_inputs_becomes_one_request` (3 queued rows → 1 request;
deleting the fold gives 3, capping it at 1 gives 2).

**Residual, not closed:** a row landing between the last `try_recv` and the
request still starts a turn of its own. The bound moves from one turn per row to
at most one stale turn per wake — a bound, not a proof.

Not catchable by any of the 1,101 tests then in the suite: every one used a fake
seat that sits still until fed.

---

## Why these three were lost, and the cheap guard

All three are **prose-only decisions**: they appear in the design's paragraphs,
never in the router inventory's 20 rows. The implementation plan was built from
the inventory, and the inventory is the acceptance criterion — so a decision that
never became a row was never scheduled, and task 14's gate cannot notice its
absence because it only walks rows.

Guard: **the inventory is the contract, so every validated design decision must
become a row in it** — PRESERVED, DISSOLVED, DROPPED, or NEW — before a batch is
planned from it. The three above should be added as NEW rows with owners.
