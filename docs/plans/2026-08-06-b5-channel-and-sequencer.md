# B5 — Channel Transport + Turn Sequencer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace `core/router.rs`'s bilateral peer-forwarding with a single
session channel plus a serial turn sequencer, so every wire into a participant
is a persisted, visible row.

**Architecture:** One append-only channel per session (the existing `messages`
table, already participant-aware after 0044). Participants read it through
`participant_cursors`; delivery is a cursor advance plus a
`participant_deliveries` row. A single turn sequencer loop replaces N reactive
tasks: exactly one participant holds the turn, so when a turn ends the sequencer
picks the next active participant, hands it every unread row, and waits.
Termination is consensus — the cycle runs until every active participant votes
done, or until any participant parks a question, which halts immediately.

**Tech Stack:** Rust, tokio (mpsc + `select!`), sqlx/SQLite, existing
`storage::participants` API (shipped in B3a/B3b), `SessionAgent` /
`ActivityTracker` (shipped in B4b).

---

## Read before starting

| doc | why |
|---|---|
| `docs/plans/2026-08-06-session-focused-redesign-design.md` §1, §1b, §3 | the turn model, consensus halt, and channel semantics this implements |
| `docs/plans/2026-08-06-router-behaviour-inventory.md` | **the acceptance criterion.** 20 behaviours, each PRESERVED / DISSOLVED / DROPPED |
| `docs/plans/2026-08-06-session-focused-redesign-implementation.md` §B5 | the batch's stated order and invariant |

**The rule that governs this whole batch, from the inventory:** *no line of
`router.rs` may be deleted until every row in that table is either green in the
new model (PRESERVED) or has a written reason (DROPPED).* Deletion is Task 14,
last, and it is gated on a checklist — not on "the tests pass".

**The one accepted behaviour change:** serialisation. Today's duo is concurrent
(`broadcast_user_message` wakes both agents and marks both busy). A turn cycle is
serial by definition. The user accepted this explicitly — *"the staleness was the
bug, not the speed."* Everything else is parity: capabilities, gates, tools,
prompts, surfaces unchanged. Record it, never call it a regression.

### What already exists (do not rebuild)

`src/storage/participants.rs` — `post_to_channel`, `channel_after`,
`unread_for_participant`, `cursor_for`, `advance_cursor`, `record_delivery`,
`withheld_for_participant`, `next_active_participant`, `set_done_vote`,
`clear_done_votes`, `all_active_voted_done`, `participants_for_session`,
`participant_by_slug`, `participant_by_id`, `ensure_session_roster`.

`src/core/session.rs` — `SessionAgent { participant_id, slug, turn_position,
handle }`, `SessionHandle::{agents, agents_mut, by_slug, hands, agent_count}`,
`roster_row`.

`src/core/activity.rs` — `set_busy_slug` / `is_busy_slug`, per-participant maps.

`src/core/duo.rs` — `DuoConfig.participant_id` (populated at spawn, currently
unread — B5 is its first consumer).

### Gates — run bare after every task, in this order

```bash
cargo test
cd frontend && npm test && npm run lint && npm run build
cd .. && cargo build --release
```

Never pipe a gate through `tail`/`grep` to read its status — the pipeline's exit
code is the last command's and always 0. Redirect to a log, `echo $?` separately.
Never run `cargo fmt`.

### Test-shape rule, learned in B4b

Any test that awaits an event **must** use a timeout helper, not a bare
`rx.recv().await`. In B4b a regression injection *hung for 7 minutes* instead of
failing, because the deleted emit meant the recv never returned. Copy
`next_event` from `src/core/activity.rs`'s test module. The consensus-halt tests
below have exactly that shape — a wake that should arrive and might not.

---

## Task 1: `PersistedMessage` — make "wire without a row" a compile error

**Files:**
- Modify: `src/storage/participants.rs` (add the newtype + return it from `post_to_channel`)
- Test: same file, `mod tests`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn a_persisted_message_carries_the_row_it_came_from() {
    let s = storage_with_0044().await;
    s.create_session("s1", "t", None).await.unwrap();
    s.ensure_session_roster("s1").await.unwrap();
    let pm = s
        .post_to_channel("s1", "brian", None, "text", "work", None)
        .await
        .unwrap();
    assert!(pm.message_id() > 0, "a PersistedMessage is proof of a row");
    assert_eq!(pm.body(), "work");
    let rows = s.channel_after("s1", 0).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, pm.message_id());
}
```

**Step 2: Run it and watch it fail**

Run: `cargo test --lib storage::participants::tests::a_persisted_message -- --exact`
Expected: FAIL — `post_to_channel` currently returns `i64`, so `pm.message_id()`
does not resolve.

**Step 3: Implement**

```rust
/// Proof that a row exists. The constructor is private to this module and is
/// called ONLY by the insert paths, so a value of this type cannot be
/// fabricated — which is what makes "wire without a row" a compile error rather
/// than a discipline.
#[derive(Debug, Clone)]
pub struct PersistedMessage {
    message_id: i64,
    body: String,
    envelope: Option<String>,
}

impl PersistedMessage {
    pub fn message_id(&self) -> i64 { self.message_id }
    pub fn body(&self) -> &str { &self.body }
    pub fn envelope(&self) -> Option<&str> { self.envelope.as_deref() }
}
```

Change `post_to_channel` to return `Result<PersistedMessage>`, constructing it
from the insert's `last_insert_rowid()`. Fix the existing call sites (the tests
that assert on the returned id now call `.message_id()`).

**Step 4: Verify it passes**

Run: `cargo test --lib storage::participants`
Expected: PASS, all module tests green.

**Step 5: Prove the invariant with a `compile_fail` doctest**

A passing test proves the happy path; it does not prove the type cannot be
forged. Add to the `PersistedMessage` doc comment:

````rust
/// ```compile_fail
/// # use bot_hq::storage::PersistedMessage;
/// // There is no public constructor: only a row insert produces one.
/// let forged = PersistedMessage { message_id: 1, body: "x".into(), envelope: None };
/// ```
````

Run: `cargo test --doc storage::participants`
Expected: PASS (the doctest passes *because* the snippet fails to compile).

**Step 6: Commit**

```bash
git add src/storage/participants.rs
git commit -m "feat: add the PersistedMessage newtype"
```

---

## Task 1b: Converge `insert_message` onto `post_to_channel`

**Added after Task 1's quality review. This is a PREREQUISITE for Task 2, not a
follow-up.**

**The problem the review surfaced:** there are two live insert paths into
`messages`. `post_to_channel` mints a receipt; `Storage::insert_message`
(`src/storage/messages.rs`) returns a bare `i64` and mints nothing — and it is
the path `duo.rs` uses on **every chunk**.

Today that split is coherent: `insert_message` records an agent's *output*, and
delivery is a separate act, so output rows need no receipt. **After B5 those
collapse** — Brian's output row *is* the row Rain reads through her cursor.
There is one row, and if delivery is receipt-gated then that row must be
receipt-bearing. Leaving the split forces Task 2 into one of two bad shapes:

- a **second write** — two rows for one logical message, corrupting the very
  channel this redesign exists to make trustworthy; or
- a **re-read** of the row to synthesise a receipt — which re-opens
  forgery-by-reconstruction *and* costs a SELECT per chunk.

**Order matters, and step 1 must come first:**

1. **Move `post_to_channel`'s participant resolution to the inline subquery**
   that `insert_message` already uses. `post_to_channel` currently does a
   separate awaited `participant_by_slug` SELECT before its INSERT;
   `storage/messages.rs:22` explains why the other path refuses that — *"this
   runs on every text/tool_use/tool_result chunk, and an extra round trip per
   chunk is a cost worth not paying."* Do this **before** Task 2 flips the send
   path, or per-chunk traffic silently gains a round trip and Task 13b's perf
   criterion fails at the point where it is hardest to attribute.
2. Make `insert_message` a thin wrapper: map `Author` → `(origin,
   participant_slug)`, pass `envelope: None`, delegate, return a
   `PersistedMessage`. Keep an `-> i64` shim for the call sites that genuinely
   only want the id (`notify_message_persisted` in `state.rs`, `watchdog.rs`,
   `tray.rs`).
3. Delete the open-question note left at `participants.rs:457-460`.

**Why not leave them split:** that is defensible only if something *enforces*
the "logged but not delivered" distinction. Nothing would. Two writers to one
table with divergent attribution logic is how the invisible-wire problem was
created in the first place.

---

## Task 2: The private input sender

**Files:**
- Modify: `src/core/session.rs` (`SessionAgent`)

`SessionAgent.handle.input_tx` is currently reachable by any caller. Make the
send path take a `PersistedMessage`, so the type from Task 1 becomes load-bearing
instead of decorative.

**Step 1: Write the failing test** — assert the only send path persists first:

```rust
#[tokio::test]
async fn sending_to_a_participant_requires_a_persisted_row() {
    // The wire body must equal the row body: the rendered envelope is
    // metadata around it, never a rewrite of it. Today six injection points
    // mutate the string after persistence; this is what closes them.
}
```

**Step 2–4:** add `SessionAgent::deliver(&self, msg: &PersistedMessage)` which
renders `envelope` + `body` and writes to the (now private) `input_tx`; convert
`SessionHandle::send_to_all` to take a `PersistedMessage`.

**Step 5: Commit** — `refactor: route participant sends through PersistedMessage`

> **Expect fallout.** `state.rs` (broadcast, tray answer, held wakes),
> `session.rs` (CL-opener nudge), and `duo.rs` all send strings today. Each
> becomes a `post_to_channel` followed by a deliver. That conversion IS Task 3.

---

## Task 3: The `system` participant — the six invisible wires become rows

> **SUBSUMED by Task 2 — verified by sweep, not assumed.** Two things
> collapsed it. First, the `system` **participant row** was dropped as a design
> decision: migration 0044 already models system as an *origin* with
> `participant_id = NULL`, and inventing a roster entry that never takes a turn
> and never votes would have contradicted the schema to match this heading.
> Second, making `input_tx` private in Task 2 broke every string-to-stdin site
> at once, so converting them was forced rather than optional.
>
> Closing sweep: `send_unrouted` has exactly **one** call site
> (`broadcast.rs:166`), there is **zero** raw `input_tx.send` outside
> `ParticipantInput`, and the host injections post `origin='system'` rows. The
> two remaining escape hatches are the known residuals — `send_unrouted` on the
> peer-forward path (the sequencer inherits it) and the module-private `relay`
> in the supervisor.

**Files:**
- Modify: `migrations/` — **NO.** Reuse the existing roles table; add a
  `system` row via `ensure_session_roster`, never by editing an applied
  migration (`0044` is immutable; the pre-commit hook blocks M/D/R on
  `migrations/*.sql`).
- Modify: `src/storage/participants.rs` (`ensure_session_roster`)
- Modify: `src/core/state.rs`, `src/core/session.rs` (the six injection points)

The six: peer prefix, phase envelope, Apply-entry nudge, reconcile directive,
idle nudge, spawn prompt.

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn every_host_injection_lands_as_a_system_row() {
    // origin='system', participant_id = the system participant, and the row is
    // visible in channel_after — no string reaches stdin without one.
}
```

**Step 2–4:** seed a `system` participant (`participation_mode = 'observer'`, so
the ring skips it and it never votes — see design §1b: observers do not vote);
convert each injection to `post_to_channel(.., "system", ..)` then deliver.

**Step 5: Commit** — `feat: post host injections as system channel rows`

---

## Task 4: Sequencer skeleton — advance the ring, exit on session end

**Files:**
- Create: `src/core/sequencer.rs`
- Modify: `src/core/mod.rs` (export)

Preserves inventory **#20** (`dropping_router_control_aborts_the_task`).

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn the_sequencer_exits_when_its_control_channel_closes() {
    let (tx, rx) = mpsc::channel(8);
    let task = tokio::spawn(run_sequencer(deps, rx));
    drop(tx);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), task).await.is_ok(),
        "sequencer must exit on session end, not linger"
    );
}
```

**Step 2:** Run — FAIL, `run_sequencer` does not exist.

**Step 3:** Implement the loop: `while let Some(cmd) = rx.recv().await`, with
`SequencerCommand::{TurnComplete, UserMessage, Pause, Resume}`.

**Step 4:** PASS. **Step 5:** commit `feat: add the turn sequencer skeleton`.

---

## Task 4b: Fix the storage helpers before the sequencer uses them

**Inserted after Task 4, which found these as their first consumer — two
verified by probe, not by reading. Prerequisite for Task 5.**

**This supersedes the claim below** that `next_active_participant` is "already
built and tested". It is tested for the cases B3a imagined; the sequencer is the
first code to actually drive it, and it exposed five problems.

1. **Consensus can become unreachable — the serious one.** `turn_position` is
   `INTEGER NOT NULL DEFAULT 0` behind a **non-unique** index
   (`0044:113`), and 0044's GUARD 5 enforces one-per-session-at-0 only at
   *migration* time; `insert_participant` accepts any position unchecked.
   `next_active_participant` advances on `p.turn_position > pos` — strictly
   greater — so with actives at A(0), B(0), C(1) the ring runs `a, c, a, c` and
   **B is never scheduled**. Consensus requires *every* active participant to
   vote, so `all_active_voted_done` then never returns true. Two participants
   sharing a position does not degrade the cycle; it makes termination
   impossible.

   **This is a schema gap, not just a query gap** — so a `(turn_position, id)`
   tiebreak in `next_active_participant` is not sufficient on its own. It would
   restore scheduling, but the database would still be able to *represent* the
   broken roster, and the next query written against `turn_position` inherits
   the same trap. 0044 is immutable, so a constraint means a **new migration** —
   additive (`CREATE UNIQUE INDEX` over enabled+active rows), no table rebuild,
   none of the 2×-disk risk that made 0044 heavy. Today's rosters are 0 and 1
   per session, so existing data satisfies it. Decide: index, tiebreak, or both.
   No task in this plan was scoped to touch `migrations/`; this one now is.
2. **`unread_for_participant` returns the participant's own rows** — it is
   `channel_after(session, cursor)` with no author filter. Verified: a row
   posted by `brian` comes back to `brian` as unread. Decide the filter at the
   storage layer so all four consuming tasks agree.
3. **Empty active roster has two shapes.** `next_active_participant` → `None`,
   `all_active_voted_done` → `false`. Neither means "done", so the sequencer
   must branch explicitly or spin.
4. **`record_delivery` + `advance_cursor` share no transaction.** A crash
   between them advances the cursor with no delivery rows — which undercuts the
   module's own claim to answer "what did participant X receive?".
5. **`channel_after` has no LIMIT.** A participant that has never read gets the
   entire session history in one `Vec`, then onto one wire.

---

## Task 5: Wake the next active participant (O(1) per turn)

Uses `next_active_participant(session_id, current_position)` — **after Task 4b
has fixed it.** Do not start this before 4b lands: the ring and the consensus
tally both sit on helpers 4b repairs, and building on them first means meeting
the deadlock as a hanging test rather than as a schema constraint.

**Test:** `a_completed_turn_wakes_exactly_one_participant` — assert one deliver,
to the participant at the next position, with every **unread** row (not "every
row after its cursor": that phrasing describes defect 2 above).

**Test:** `an_observer_is_skipped_not_given_a_no_op_turn`.

**Also decide here, do not defer again:** whether `TurnComplete` carries a
participant id. Task 4 left it payload-free with the hazard named — Pause →
agent finishes → Resume + `UserMessage` resets the ring to position 0 → the
stale `TurnComplete` advances from the wrong position. Zero send sites exist
today, so adding the field is still mechanical.

Commit: `feat: advance the turn ring on turn completion`

---

## Task 6: Consensus halt

Design §1b. Storage helpers exist (`set_done_vote`, `clear_done_votes`,
`all_active_voted_done`).

**Tests (each its own task-sized step):**

1. `the_cycle_halts_when_every_active_participant_votes_done`
2. `substantive_output_resets_the_tally` — a stale done cannot accumulate into a
   false arrival (this is inventory **#12**'s convergence-reset, generalised)
3. `observers_do_not_vote` — otherwise 1 active + 3 observers needs 4 yields

**Use the timeout helper.** "The session halts" means *no further wake arrives*,
which a bare recv cannot distinguish from a hang.

Commit: `feat: halt the cycle on consensus`

---

## Task 7: A parked question halts immediately

Design §1b: a yield, not a vote — one participant blocking on the user stops the
cycle regardless of the others. Preserves inventory **#4**
(`awaiting_suppresses_forward`), reframed: **the cursor does not advance while
awaiting**, so the behaviour is visible instead of hidden.

**Tests:** `a_parked_question_halts_the_cycle_unilaterally`;
`cursors_do_not_advance_while_awaiting`.

Commit: `feat: halt the cycle on a parked question`

---

## Task 8: A user message resets the cycle

Preserves inventory **#12** and the subtle **#13**.

**Tests:**
1. `a_user_message_resets_the_cycle_to_the_first_participant`
2. `a_user_message_clears_done_votes_and_spin_state`
3. `the_reset_survives_a_turn_that_produced_nothing` — **#13 verbatim in the new
   model.** A reset consumed by an empty turn silences the first real post after
   a user message. Not visible in the old code, only in its test.

Commit: `feat: reset the cycle on a user message`

---

## Task 9: Pause holds wakes

Preserves inventory **#19** (`paused_holds_forwards_and_flush_delivers_exactly_once`).
Exactly-once is free here — a cursor advance is idempotent — but *the pause
semantic must survive*: a paused session must not wake the next participant.

**Tests:** `a_paused_session_does_not_wake_the_next_participant`;
`resuming_delivers_each_unread_row_exactly_once`.

~~`ActivityTracker::holds_wakes()` already answers "cancelling or paused"; reuse
it rather than re-deriving.~~ **Superseded — the sequencer re-derived, and the
deviation was reviewed and accepted.** Two reasons, both in the module doc:
`holds_wakes()` is a *level* and this loop needs an *edge* (it sits in
`recv().await` between turns, so a latch flipped elsewhere is observed wherever
the loop happens to look, with no defined order against neighbouring commands —
and that ordering IS the pause semantic); and it answers `cancelling || paused`,
where a settling cancel is a state the sequencer has no concept of. The two
notions of "paused" are meant to agree, and nothing enforces that — which is the
mechanism behind the release conflict below.

**Also settled here, against what this plan originally implied:** a
`UserMessage` is EXEMPT from the pause gate and clears the latch. The host had
already decided this in three places — `state.rs:737` ("a user message is the
steer") calling `set_paused(false)`, `activity.rs:217` ("Cleared by Resume, a
user Send (steer), or a supersede"), and decisively `resume_session`, which is
implemented as `broadcast(RESUME_NOTICE)`, so **the Resume button IS a user
message today**. Holding it would have left `SequencerCommand::Resume` with no
producer and the pause unreleasable, while `ActivityTracker` read unpaused and
the UI dropped the only Resume affordance. This is faithful to #19 besides: the
old router's pause held *peer forwards*, and a user Send was always its release.

The release fires where the message is READ, not where it is dispatched — the
steer is re-queued behind the commands the pause held, and by dispatch time a
second `Pause` may have arrived, which releasing again would silently cancel.

Commit: `feat: hold wakes while the session is paused`

---

## Task 10: Move Jaccard verbatim

Inventory says: **PRESERVED — spin detection reuses it verbatim. Move with its
test.** `token_set` / `jaccard_from_sets` / `jaccard_similarity` from
`router.rs:604-640`, plus `jaccard_similarity_normalizes_and_handles_edges`.

Pure move, no behaviour change. Commit: `refactor: move jaccard into the sequencer`

---

## Task 11: Spin detection over ONE participant across rounds

Preserves inventory **#2** and its false-positive guard **#3**. Reframed:
cross-agent echo is impossible in a ring, but self-repetition is not.

**Tests:**
1. `a_participant_repeating_itself_across_rounds_is_flagged` (**#2**)
2. `varied_substantive_output_never_trips_the_detector` (**#3**) — the test that
   stops spin detection eating productive work. **Do not skip it.**

Commit: `feat: detect a participant spinning across rounds`

---

## Task 12: Done-votes from `peer_ack`, with the substantive override

Preserves inventory **#8, #9, #10, #11** as one coherent group.

- **#8** `peer_ack` → a done vote instead of waking the next participant.
- **#9** an acked-but-**substantive** turn (>200 bytes) still posts, tagged —
  **this guard exists because four full reviews were destroyed by acked
  substantive turns.** The length proxy stays as the floor; the tag becomes
  `envelope` metadata the user can see.
- **#10** `final: true` suppresses regardless of length.
- **#11** the inverse of #10.

**Tests:** one per behaviour, named after the originals so the inventory maps
1:1.

Commit: `feat: carry peer_ack semantics into done-votes`

---

## Task 13: Withheld deliveries are visible rows + the perf benchmark

**13a** — Preserves inventory **#5**, upgraded: a suppressed delivery is a
`participant_deliveries` row with `withheld_reason`, not a preview in a side
table. Policies gate delivery, never persistence.

Test: `a_withheld_delivery_records_its_reason_and_the_message_survives`.

**13b — the perf measurement the inventory demands.** Inventory **#6**
(`a_delivered_forward_records_nothing`) is DROPPED *deliberately* — the
invisibility it enforced is the defect the user reported. But it existed because
someone cared about the hot path (`open_blocking` is a lock-free cache built for
exactly this reason), so:

> **this must be benchmarked in B5, not assumed.**

Measure the per-delivery cursor advance + delivery row. A message row is already
written per turn, so the same order of magnitude is expected — **prove it**. If
the cursor advance is hot, batch it the way `BatchEmitter` already batches
emission (50 ms / N=20).

Record the number in the Apply doc. A missing benchmark here is an unmet
acceptance criterion, not a nice-to-have.

Commit: `feat: record withheld deliveries` + `perf: measure the channel delivery path`

---

## Task 14: Delete `router.rs` — gated on the inventory, not on green tests

**Do not start this task until the checklist below is complete.**

**Step 1: Walk the inventory table row by row.** For each of the 20:

- PRESERVED (12) → name the new test that covers it and confirm it is green.
- DISSOLVED (6) → confirm the structure that makes it impossible actually exists
  (no hold queue; a fixed ring; no bilateral `peer_of`).
- DROPPED (2) → confirm the written reason is still true. #6's reason depends on
  Task 13b's benchmark.

**Step 2: Write the verdict table into the Apply doc** — new test name or reason
per row. This is the artifact that proves the subsystem was replaced rather than
lost.

**Step 3: Delete** `src/core/router.rs`, the `RouterCommand` / `RouterControl` /
`RouterDeps` exports in `src/core/mod.rs`, `DuoConfig.router_tx`,
`SessionHandle.router`, and the `flush_held` family in `state.rs`.

**Step 4: Gates**, then the live smoke below.

**Step 5: Commit** — `refactor: delete the peer-forward router`

---

## Task 15: The live smoke — parity is not "it compiles"

Constraint 0's verification clause: *"the full gate suite green, plus a live
smoke comparing a default session against today's behaviour — not just 'it
compiles'."*

Run a real default session and confirm:

| check | expectation |
|---|---|
| tools + capabilities | HANDS/EYES sets unchanged |
| commit gate | reviewer files → executor dispositions → commit blocked until resolved |
| IPAV, tray, push/Tool gates, CL, terminal, worktrees | unchanged |
| turn order | HANDS acts, then EYES — visible in the roster |
| **serialisation** | `activity_events` shows one busy participant at a time, **not** `busy \| 1 \| 1` |

That last row is the recorded exception. Today's rows show `busy | 1 | 1`; after
B5 they must not. Assert it against the table, not by eye.

Commit: `test: pin the post-B5 serialisation contract`

---

## Definition of done

1. Every inventory row green-in-the-new-model or reason-confirmed (Task 14 Step 2).
2. `router.rs` deleted; no `RouterCommand` / `peer_of` / hold-queue references remain.
3. The delivery-path benchmark recorded (Task 13b).
4. B0's parity oracle (`src/signaling/parity.rs`) green and **unchanged** — it
   pins authorization, not concurrency, so B5 must not touch it.
5. Five gates green on every commit.
6. The live smoke passed, with serialisation asserted from `activity_events`.
7. Frontend diff still empty — the roster UI is **B8**, not this batch.

## Explicitly NOT in B5

Capability-gated tool authorization (B6), spawn derivation from capabilities
(B7), the roster + Roles tab UI (B8), and dropping `messages.author` /
`sessions.{brian,rain}_*` (a follow-up migration, gated on the grep audit).
