# Dogfood queue — bot-hq working on bot-hq

Point a session at this file. Written 2026-08-13 evening, **against verified
state rather than against the older plan docs**, because two items in the
previous queue were already done and nobody had noticed.

**Read first:** `ARCHITECTURE.md`, then `docs/plans/2026-08-11-rc3-decisions.md`
from **D16 onward** (that is today's arc), then `PROGRESS.md`'s newest entry.

---

## Verified DONE — do not redo these

Checked in the database and the source on 2026-08-13, not taken from a doc:

- **The reviewer already reproduces rather than reads.** The `eyes` role prose
  (12,943 chars) asks for it explicitly. The older queue listed this as the
  top item; the user had already rewritten the prose. **Read the role before
  proposing a prompt change.**
- **D16** `close_session` gates on the role's tick; **D17** `@mention` summons;
  **D18** two participation modes; **D19a/b** prose-only delivery + the turn
  holder recorded; **D20** the ordinal, the eight-colour palette, rotation and
  the per-participant pick; **D22** a park finishes the lap; **D23** every wire
  carries `[speaker]`; **D24** a straggler cannot bind a retired epoch; **D25** a
  turn carries at most one pass.
- **The Context Library pushes.** `~/.bot-hq/library` has its remote and is in
  sync. What may remain of the old P6 is the **pre-push secret check** — confirm
  whether one exists before building one.

All of the above were verified live in `s-d8773b42` and `s-991f7416`.

---

## 1. Coalesce a turn's backlog into ONE stdin write

**The biggest open item, and the best-evidenced.**

`deliver_backlog` (`src/core/sequencer.rs`) calls `input.deliver(&receipt)` once
per row, so a nine-row backlog is nine separate stdin writes. claude-code opens
the turn on the first and the rest arrive *during* it as interruptions.

Measured, not inferred:

- **The user's message arrived somewhere other than the front of the batch 37
  times out of 44** across four sessions — including row 9 of 9 and row 8 of 8.
  Still 3-in-4 on the current build.
- One session's reviewer spent its turn reviewing a peer's test run while the
  user's actual instruction ("prepare to close") sat unread at row 9. The user:
  *"why does it feel like its not addressing my current message?"*
- `handover → first output` has been **~9–10s in every build measured**, before
  and after every fix this week. It is prefill, and it is the only latency
  component nothing has moved.

**D23 made the user's row identifiable (`[user]`); only this makes it last.**

**Care required — this is a delicate path:**

- `deliver_backlog` currently commits only the PREFIX that landed, so a full
  stdin buffer mid-drain leaves the remainder past the cursor. One write makes
  that all-or-nothing. That is arguably better, but it changes what a slow child
  does and there are tests on it — read them first.
- The drain's `Stop::Superseded` / `Stop::Parked` / `Stop::Paused` arms cut a
  drain short between rows. With one write there is nothing to cut between; the
  command still has to be deferred so the loop can act on it.
- Keep the `kind` filter (D19a). Coalescing tool rows back in would undo it.

**Definition of done:** a turn's backlog reaches the participant as one message;
a user message posted before that turn reads as the last line of it; the
`REPLY` and `GAP → start` figures from `scripts/turn-latency.py` are compared
before and after on a real session.

---

## 2. D21 — the parallel BOOT phase

Spec'd in the decisions doc. Two things from it are load-bearing and easy to lose:

- **Boot is ORIENTATION, not work.** Participants may read in parallel; none may
  act. Boot output is persisted and shown to the user but NOT delivered to peers
  (it rides D19a's `kind` filter).
- **No participant may send a completion during boot.** There is no holder, so
  there is no epoch, and a completion carrying `0` is discarded forever — the
  exact class D24 just fixed. Boot needs an explicit turn-start signal rather
  than the pump inferring one from its own first event.

Related evidence from `s-a4e9a1b4`, worth designing against: with three
participants, all three independently posted the same four-bullet CL summary at
session open. The executor's own words — *"that's the trio version of a protocol
written for a duo"*. Cheap on a test session, noisy on real work where all three
would re-report one finding.

---

## 3. D20's remaining half — the user-set LABEL

The ordinal (`EYES-2`) and the whole colour story shipped. What did not:

- a `label` column on `session_participants`, editable where the participant is
  chosen, overriding the ordinal;
- empty falls back to the ordinal, which is what ships today;
- it should become the `[speaker]` D23 puts on the wire, so the name peers read
  and the name the user reads are one string.

Migration 0052 added `color` the same way — copy its shape, including the
roster-parity tripwire it had to satisfy.

---

## 4. Two real defects, both small

- **The close epilogue is inconsistent.** It landed in `s-991f7416` and not in
  `s-d8773b42`. In the pre-fix `s-a4e9a1b4` it arrived nine minutes after close
  and produced nothing. `run_close_epilogue` broadcasts through the ring, so a
  slow or wedged ring delays it — D24 may have fixed it, and the evidence is
  currently one-for-two. Reproduce before changing anything.
- **`sessions.round_number` has no writer.** `MAX(round_number)` is 0 across
  every session ever recorded. Same shape `current_turn_participant_id` had
  before D19b. Either write it from the ring's lap counter or delete the column.

---

## 5. Needs a DECISION, not a fix

**The tail of the ring starves.** Every user message resets the rotation to the
front, so at N=3 with an active driver slot 2 gets roughly half the turns of slot
0 — measured 2-vs-6 in `s-534b8761` and again in `s-206e8921`. This is a
consequence of a documented rule (D1: "a user message resets the cycle to
participant 1"), not a bug. `@mention` is the current manual workaround.

The options, for whoever raises it with the user: keep it and rely on mentions;
resume from where the ring was rather than the front; or make the reset target
configurable per session. Do not just change it — the predictability is the
reason the rule exists.

---

## How to work

- **Small chunks, `cargo test` after each.** ~1079 lib + 58 integration + 333
  frontend, about 20 seconds.
- **Mutation-verify anything you claim to have fixed.** Break the fix, watch the
  test go red, restore. Several tests written this week passed with their own fix
  deleted until this was done — including one where `send` only queues, so the
  race the test existed to catch never happened.
- **A green suite is not a working app.** A circular import shipped a blank
  window this week with `tsc` clean, `vite build` exit 0 and 333 tests passing:
  Vitest resolves a module graph per test file, so a cycle that only bites when
  two modules share one bundled chunk never appears there. `./start` now checks
  for cycles and stylesheet size before launching — **if you add a class of
  failure the suites cannot see, add its guard there.**
- **Know which kind of test edit you are making.** A test that changes SUBJECT
  because the behaviour deliberately changed is correct. A test rewritten to
  match observed output is how a wrong answer gets frozen. Four frontend fixtures
  asserted the colour collision as correct.
- **Distrust a claim that was correct under an old constraint.** The two-hue
  palette carried a comment calling a repeat "a shared colour, not a wrong one" —
  true at two participants, false once the cap moved to four, and it read exactly
  like a claim that was still true.
- **Measure with `scripts/turn-latency.py`**, not with a fresh query. Asking "are
  turns slower" three different ways gave 8.2s, 11.1s and 30.1s, all correct and
  all measuring something different.

## What NOT to do

- Do not relitigate the ring, the two participation modes, the summons, or the
  lap-before-halt. D17/D18/D19/D22 are settled and recorded with their evidence.
- Do not add a second path that can put a participant on a turn. Every serious
  bug this month was two paths into one stdin, only one of which the ring could
  reason about.
- Do not edit an applied migration. Changing any byte of `migrations/*.sql` that
  has run breaks boot on a checksum mismatch.
- Do not push. `push_gate` is the user's.
