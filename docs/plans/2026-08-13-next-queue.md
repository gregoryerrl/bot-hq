# Next queue — what a bot-hq session should pick up

Mention this file to a session and it can start. Written 2026-08-13, after a day
that shipped rc3 D16–D24 and found three defects by running real sessions rather
than by reading code.

**Read first:** `ARCHITECTURE.md`, then `docs/plans/2026-08-11-rc3-decisions.md`
(D16 onward is the recent arc), then `PROGRESS.md`'s newest entries.

---

## The theme

Everything here came out of a live session. That is the pattern worth keeping:
the day's three real bugs — a halt that made peers unreachable, a wire with no
author, an epoch bound by a straggler — were all invisible to the test suite and
obvious in `participant_deliveries` and the log. **Run a session, then read what
it left behind.**

---

## 1. The reviewer should REPRODUCE, not review

The highest-leverage item, and it is prose rather than architecture. From the
project CL's own `improvements-2026-08-12-visibility-and-verification.md`, P3:

> On 2026-08-12 every real defect was found by an agent told to *apply the
> mutation, run the test, watch it go red, revert* — and none by an agent reading
> a diff.

EYES' role prose should ask for: reproduce the defect before accepting the fix;
delete the call site and confirm the suite goes red; re-run the claimed
verification rather than accepting the claim.

**Definition of done:** the seeded `eyes` role prose asks for reproduction, and a
session's review turn shows a command run rather than a paragraph about the diff.

**Where:** the role's instruction prose, edited in Settings → Roles. It is user
config, not code — so this is a change the user makes, or approves.

---

## 2. Decide whether a turn's backlog should be ONE stdin write

`deliver_backlog` (`src/core/sequencer.rs`) calls `input.deliver(&receipt)` once
per row. A four-row backlog is four separate stdin writes, and claude-code begins
the turn on the first — so rows 2..N land *inside* the turn row 1 opened and read
as interruptions rather than as the prompt.

This is what HANDS reported in `s-534b8761` as messages "injected alongside a
tool result", and it misattributed the cause (it thought they arrived from
outside its turn; they were its own opening backlog).

**rc3 D23 removed most of the harm** by putting `[speaker]` on every row, so what
used to be four anonymous strings is now four labelled ones. Whether coalescing
is still worth it is a MEASUREMENT, not a guess:

- run one N=3 session on the current build,
- read what a participant says about its own input,
- if it still reads as interruptions, coalesce; if not, close this.

**If you do coalesce**, the thing to be careful about is partial delivery:
`deliver_backlog` currently commits only the prefix that landed, and a single
write makes it all-or-nothing. That is arguably better, but it changes what a
full stdin buffer does mid-drain, and that path has tests worth re-reading first.

---

## 3. D20's second half — the user-set label

The ordinal shipped (`f3f4809`): a second participant of a role reads `EYES-2`.
What remains is the label the USER sets, which overrides it.

- a `label` column on `session_participants`, editable where the participant is
  chosen (New Session dialog),
- empty falls back to the ordinal, which is what ships today,
- shown wherever the participant is named — and it should become the
  `[speaker]` D23 puts on the wire, so the name peers read and the name the user
  reads are one string.

**Definition of done:** a user can name a participant "SECURITY" in the dialog and
that string appears in the roster, the chat byline, the turn-status line, and the
wire its peers read.

---

## 4. D21 — the parallel BOOT phase

Spec'd in the decisions doc. The trap is named there and is worth repeating: **no
participant may send a completion during boot.** There is no holder, so there is
no epoch, and a completion carrying `0` is discarded forever — which is exactly
the class of bug D24 fixed. Boot needs an explicit turn-start signal rather than
the pump inferring one from its own first event.

Boot is ORIENTATION, not work: participants read in parallel, none acts, and boot
output is persisted and shown to the user but not delivered to peers (it rides
D19a's `kind` filter).

---

## 5. Two smaller things, both real

- **`sessions.round_number` has no writer.** Same shape
  `current_turn_participant_id` had before D19b: a column that exists, is read by
  nothing useful, and reports 0 forever. Either write it from the ring's lap
  counter or delete it.
- **The tail of the ring starves.** Every user message resets the rotation to the
  front, so at N=3 with an active driver slot 2 gets roughly half the turns of
  slot 0 — measured 2-vs-6 in `s-534b8761` and again in `s-206e8921`. This is a
  consequence of a documented rule, not a bug, and it needs a DECISION rather
  than a fix. `@mention` (D17) is the current workaround.

---

## How to work

- **Small chunks, `cargo test` after each.** The suite is ~1077 lib + 58
  integration + 324 frontend and runs in about 20 seconds.
- **Mutation-verify anything you claim to have fixed.** Break the fix, watch the
  test go red, restore. Three tests written today passed with their own fix
  deleted until this was done — including one where `send` only queues, so the
  race the test existed to catch never happened.
- **A test that changes subject is fine; a test rewritten to match observed
  output is not.** Four frontend fixtures asserted the D20 collision as correct.
  Their expectations were the bug. Know which kind you are doing.
- **Do not rewrite append-only history.** `PROGRESS.md`, `decisions.md` and
  `issues.md` describe the tree as it was on their date.

## What NOT to do

- Do not relitigate the ring, the two participation modes, or the summons —
  D17/D18/D19/D22 are settled and recorded with their evidence.
- Do not add a second path that can put a participant on a turn. Every serious
  bug this month was two paths into one stdin, only one of which the ring could
  reason about.
- Do not push. `push_gate` is the user's.
