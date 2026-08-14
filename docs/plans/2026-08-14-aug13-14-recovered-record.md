# The Aug 13–14 record, recovered from the session transcript

Written 2026-08-14 after this session's third context compaction, by reading the
session's own JSONL back. The plan docs had gone stale twice in the same window,
so this is assembled from the transcript and the database — the two things that
do not drift. It exists so the next compaction cannot lose the arc again.

Scope: 2026-08-13 00:00 → 2026-08-14. Everything earlier lives in
`2026-08-11-rc3-decisions.md` (D1–D15) and the archived overnight-run doc.

---

## Session ledger, in order

| session | what it was | the finding |
|---|---|---|
| `s-81057bde` | test 5 | Verified D19: 8/8/9 deliveries, 0 of 65 tool rows leaked, no epoch-0 carrier. Also showed a first-turn park makes reviewers look mute (ring restarts at front). |
| `s-e8a20797` | test | **4 / 0 / 0 deliveries.** A parked question halted the ring unilaterally; peers spawned, alive, never handed a turn. Pre-rc3 the router forwarded regardless of halts. → **D22** (park finishes the lap). |
| `s-534b8761` | test 6 | D22 fired twice correctly (10/9/8, everyone spoke). Found the **N-stdin-writes defect**: a 4-row backlog is 4 writes; claude-code opens the turn on write 1, rows 2..N read as interruptions. Also: EYES-2 starved 2-vs-4 turns (front-reset), `round_number` dead. |
| `s-206e8921` | test 7 | **The D24 wedge.** User typed mid-turn (deliberately, as evidence); the reset discarded EYES' completion; the straggler bound retired epoch 9; every later completion carried 9 against live 11 → 19 minutes dead. Recovery was Stop → SIGKILL → stale → full respawn, **not** the ring reset I predicted. EYES did 6 minutes of real, persisted review work while wedged. |
| `s-a4e9a1b4` | ad-manager, 1h45m | Real work shipped (PRs #512/#513, red-first tests, a rebutted blocking finding — the adversarial loop working). Cost: **209 passes in 9 bursts = 45.4% of all tool calls** (→ D25); **18 discards**, same epoch carried twice on 4 occasions (D24 constant, not occasional); EYES 22 turns vs HANDS 12 (discards eat HANDS' turn-ends); REPLY p90 **101 s**; close epilogue delivered 9 min after close, produced nothing. |
| `s-d8773b42` | test 8 | Post-D24 build: GAP "ending" half fell to 1.0 s. |
| `s-991f7416` | test 9 | First run with everything on. **D25 refusal has teeth in production** — a model stops after 1–2 refusals; 5 passes (8.6%) vs 209 (45.4%). D24 clean. Colours distinct (user confirmed on screen). User message still buried 3 of 4 times — position is coalescing's job, not the label's. |
| `s-9cda64fd` | "stalled" session | **Not wedged — one model call took 565 s at 167k tokens.** Zero events for 9m25s, then it finished on its own; nothing external moved it (the idle nudge was queued, undelivered). Lesson: *"thinking on a big context" and "wedged" are the same observation from outside* → D26 logging. Context readings: 75k on fast turns → 167k before the slow one → 190k after. |
| `s-43d5e78e` | dogfood, 6h44m, 15 commits | Cleared the whole queue: coalescing `7060d97` (one write per **page**), D21 boot `584f06f`, label `24da3d3`+`4e531c8` (migration 0053), `round_number` `1984e61`, epilogue `7d49a34`. Found D19b's `set_current_turn` completely unpinned (my code, claimed verified). Declined the tail-starvation change — options 2/3 are "a second path into the turn"; the user's planned **turn-picker** is the real fix. Staged explicit paths instead of `add -A` to avoid sweeping my concurrent edits. Shipped the input lock `c13fcdb` as a declared band-aid. **None of it was live until the 16:03 relaunch** — measurements in-session were on the pre-coalescing build. |
| `s-8ac0d2d0` | quick test, force-closed | **The boot-loop trap:** lock closes the box → only way to speak is Stop → SIGKILL makes the session stale → next message respawns → boot re-runs (~60k tokens × N; 192k→251k across three boots; agents said "bearings already loaded"). Three `errored turn`s — SIGKILL severs in-flight provider calls, so every Stop costs a turn's API call. D25 refused passes 2–4 in production. → **D29** (boot yields; no boot on respawn) + eventually **D33**. |

Aug 14's own sessions: `s-382d3d18` (found D31, the stuck busy flag, and D32, the
banner claiming HALT over a working session) and `s-c8e411a5` (verified D33 live:
gate cleared on Approve in 36 s, questions stayed parkable, zero overlaps).

---

## The numbers that anchor decisions

**Latency, named so the same question gets the same measurement**
(`scripts/turn-latency.py`: REPLY / GAP / SPLIT / PACE):

- Pre-rc3 REPLY (user types → first participant row): median **4.5 s**, p90 **14.9 s** (5 sessions, n=115).
- rc3 before the fixes: median **11.1 s**, p90 **50.2 s** (n=21); worst session p90 101 s.
- The first "8.2 s handoff" figure was wrong — it counted any author change,
  including wedge interjections. Withdrawn on the user's challenge.
- GAP decomposes as **ending** (turn end → handover) + **start** (handover →
  first output). D24 halved *ending* (9.5 → 1.0–5.0 s). *Start* is **~9–10 s in
  every build measured** — it is prefill, it scales with accumulated context
  (75k fast / 167k → 565 s), and no ring fix touches it.
- **37 of 44 user messages arrived somewhere other than the front of the batch**
  across four sessions (including row 9 of 9: "prepare to close" behind six rows
  of peer narration and two idle nudges). D23's `[speaker]` made the row
  identifiable; coalescing (`7060d97`) made position right.

**Pass economics:** 209 passes / 45.4% of tool calls in `s-a4e9a1b4`, recurring
in 9 bursts including twice during close. Post-D25: 5 passes / 8.6%, refusals
obeyed after 1–2 attempts. The round cap could never catch it — it counts laps,
and a participant looping inside one turn moves no laps.

**Deliveries as the ground truth:** the triple `4/0/0` → `10/9/8` → `8/8/9` is
what "the ring works" means in this project. Dissections read
`participant_deliveries` + the log, not the chat.

---

## Ideas raised in the window, not yet built

- **Context meter as a wait estimate.** The reading is collected after each
  turn; at 190k "the next turn will be slow" is predictable before it happens.
  Surface it next to the turn-status line. (Raised in the `s-9cda64fd`
  post-mortem; user never dispositioned it.)
- **Context growth is the real session-length ceiling.** 74k → 167k in ~30
  minutes on real work, post-D19a — the growth is own output + peer prose now.
  Named "coalescing's neighbour"; nothing scheduled.
- **Close epilogue inconsistency.** Landed in `s-991f74`, not in `s-d8773b`;
  arrived 9 min late and produced nothing in pre-fix `s-a4e9a1`. `7d49a34`
  touched it; evidence since is one-for-two. Reproduce before touching.
- **The turn-picker.** Referenced by the dogfood session as the user's planned
  feature and the real fix for tail starvation. Not spec'd anywhere in the
  repo; it exists only as that reference.
- **`duo.rs` → `core::pump`.** 2,226 lines, exports `DuoConfig` + `pump_agent`,
  nothing two-agent about it; module doc still claims it "fans text chunks out
  to the peer," which D19 ended. ~26 refs across 4 files. Was deferred only to
  avoid colliding with the then-open dogfood session — **that session is
  closed, so the rename is unblocked.** The false doc line is the part that
  actively misleads.

## Recommendations that were later overturned — do not resurrect

- **Buffering** (hold the user's message, deliver at the next boundary) was my
  recommendation in three places, including the `s-8ac0d2d0` post-mortem and
  the dogfood follow-up. The user overturned it 2026-08-14: *"users are never
  allowed to type while agents are working, no halt = no type (except for pause
  button which is the real interrupt)"* → **D33**. The lock is the design;
  Pause is the one interrupt.
- **"Approvals halt the session"** — proposed by the user, split by agreement:
  the gate takes the input slot (UI), the ring is untouched (`halt_ring =
  !blocking` stands; freezing peers would undo D22's review lap).

---

## The halt / question-tray / input-box conversation, in full

The design conversation that produced D27–D33, reconstructed from the
transcript (it straddled the third compaction — the first half was nearly
lost). This is the contract the surfaces implement; treat it as binding
unless the user reopens it.

### The proposal, in the user's words

> *"instead of parkable in the tray, let's put it on top of the input box. in
> this way there can never be 2 halts parked anymore. and halt can serve as a
> recap for what happened in the session and what the agent is waiting for
> from the user. And sending a message will clear the halt … HALT — [short
> recap here], waiting output from these command `php tinker execute…`. I run
> the commands on laravel cloud and paste the output on the input box. That
> clears the halt at the same time I give them what they're waiting for."*

And the amendment: *"and ofcourse, answering questions from the tray will also
clear the halt (since those count as user's response)."*

### What was agreed around it

- **The recap is the strong argument, not dedup.** Every "why is it stopped?"
  had cost six queries across two tables and a log; the agent knew the answer
  at the moment it stopped and the knowledge went nowhere. (Dedup was the weak
  argument — and my "two halts never co-pend" check was later proven wrong:
  asked correctly, **52 overlaps**, worst one row under six more for 53
  minutes.)
- **Keep the durable row; the banner is a VIEW of it.** It must survive a
  restart; `list_session_tray` stays the source.
- **One banner, N lines** when several participants are blocked (reachable
  since D22) — "never two halts" as a display invariant, not a false claim
  about state.
- **The tray keeps structured picks**, and the division of labour is a rule,
  stated because without it agents use the two interchangeably: **the banner
  is the session's state, always present while halted; a tray choice is an
  optional pick attached to it.**
- **Clearing is ONE event.** The audit found it half-wired: a typed message
  released the ring *and* cleared the rows; a tray answer only released. Fix
  was not "patch the third caller" but a single `user_responded()` every
  entry point goes through (typed message, tray answer, phase advance) —
  **D28**. The stated risk: it sits on the release path, where a half-made
  change once broke every session for two hours.
- **The recap's known weakness:** it is the agent's claim about its own state,
  untestable by the suite. (Resolved in practice: agents already write real
  recaps unprompted — the gap was bounding them, hence the 3-line clamp.)

### The two screenshots, and what each overturned

1. **"halted while hands is still working — is this by design?"** → not
   design: a refused handover left its busy flag set (**D31**), and the
   watchdog had already turned the lie into a `stalled` verdict.
2. **"happened again, this time they are really working. I suspect parking a
   question in tray toggles the halt (it should not), its asynchronous."** →
   correct, and deeper than wording: no halt row existed at all; the banner
   said HALT whenever *anything* was pending. **A halt is a claim about the
   SESSION, not the tray** (**D32**). `ask_user_choice` is non-blocking by
   design; only `halt`/`mark_awaiting_user` is a participant saying it
   stopped.

### The Send idea, the gate pivot, and the decree

With the second screenshot came a proposal: *"remove the send button on tray
items. On Halt, sending a message will also send all of the answers on all
tray items."* Agreed **for choices and not for approvals** ("responding should
be one event" — but a gated approval is synchronously blocked and cannot wait
for a Send).

Then the pivot: *"or how about this: approvals are not parkable anymore …
instead of input box, it will show the approval gate."* Taken wholesale for
the UI; pushed back on "they halt a session" — `halt_ring = !blocking` is
deliberate (the asker is already stopped inside its tool call; freezing peers
would undo D22's review lap). The ring stays untouched.

And the destination, verbatim: *"yes, i want to build towards → Pause button
is the only real interrupt"*, then the clarification that killed buffering:
*"users are never allowed to type while agents are working, no halt = no type
(except for pause button which is the real interrupt)"* → **D33**.

### The settled contract, compact

| thing | behaviour |
|---|---|
| halt (`halt`/`mark_awaiting_user`) | banner above the box: `⏸ HALT`, who + clamped recap; box is open (nobody working); any user response clears row + releases ring via `user_responded` |
| parked question (`ask_user_choice`) | tray card; session keeps working; banner shows `◆ FOR YOU`, never HALT; answer whenever; answering also goes through `user_responded` |
| approval (`Approve`/`Reject` options) | not parkable: gate replaces the input box, answered on the spot, Reject is the explicit no; tray shows a count pointing at the gate; ring untouched |
| input box | locked ⟺ any participant busy (the map, not the enum); `paused` open; no halt = no type |
| Pause | the only interrupt; parks the session; Resume continues where the ring left off |

### The one thread left open by the pivot

**Batched choices** — recovered as ambiguous, put to the user, decided, and
**built the same day as rc3 D34** (`7e1e04d`): picks stage while the box is
open, Send delivers message + answers as one response, and the
`tray-answer-preempt` interrupt — which reset the ring and threw away the
holder's in-flight turn on every mid-work tray answer — is deleted and pinned
deleted.
