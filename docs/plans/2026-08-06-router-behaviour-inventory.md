# Router behaviour inventory — a verdict per encoded behaviour

**Batch B0.3** of the session-focused redesign. `src/core/router.rs` is 1454
lines: **632 code, 822 tests**. Batch B5 deletes it. Those 822 lines are the
*specification* of what it does; deleting them without a verdict on each is how
a subsystem gets lost rather than replaced.

**Verdicts:**
- **PRESERVED** — the behaviour must exist after B5, as a channel policy. A
  green test in the new model is required before the old one is deleted.
- **DISSOLVED** — the behaviour cannot occur in the new model; the failure it
  guards against is structurally impossible. No replacement needed.
- **DROPPED** — the behaviour could still occur but we consciously accept its
  loss. Requires a stated reason.

**Rule for B5:** no line of `router.rs` may be deleted until every row below is
either green-in-the-new-model (PRESERVED) or has a written reason (DROPPED).

---

## The 20 encoded behaviours

| # | test | behaviour it pins | verdict |
|---|---|---|---|
| 1 | `hard_cap_breaks_after_cap` | after N consecutive peer-forwards with no user message, break the volley | **DISSOLVED** — the cap counts *emergent* ping-pong. A fixed ring cannot ping-pong: turn order is assigned, not contested. Round termination is consensus (design §1b), with a round cap as a crude backstop. |
| 2 | `single_stream_cross_agent_same_phrase_breaks_fast` | two agents repeating the same phrase break quickly | **PRESERVED** → spin detection. Reframed: a *single participant* repeating itself across rounds. Cross-agent echo is impossible in a ring, but self-repetition is not. |
| 3 | `varied_substantive_cross_agent_never_breaks` | genuinely different content never trips the breaker | **PRESERVED** — the false-positive guard on #2. This is the test that stops spin detection eating productive work. |
| 4 | `awaiting_suppresses_forward` | while the session awaits the user, forwards are not delivered | **PRESERVED** → cursors do not advance while awaiting. Same behaviour, visible instead of hidden. |
| 5 | `a_broken_volley_records_which_message_it_dropped` | a suppressed forward is recorded with a preview | **PRESERVED, upgraded** → `participant_deliveries.withheld_reason`. Policies gate delivery, never persistence, so the message itself is now a visible row rather than a preview in a side table. |
| 6 | `a_delivered_forward_records_nothing` | the delivery path never touches storage | **DROPPED — deliberately, and this is the point of the redesign.** The invisibility this test *enforces* is the defect the user reported. Its replacement asserts the opposite: every delivery leaves a row. **Perf must be measured** (see §Perf below) — the test exists because someone cared about the hot path. |
| 7 | `awaiting_holds_the_forward_and_delivers_it_after_the_user_replies` | held forwards flush on the user's reply | **DISSOLVED** — there is no hold queue. A cursor either advanced or did not; on wake, the participant reads everything after it. This is what retires issue #26 (`held_late` ×12/day). |
| 8 | `peer_ack_suppresses_and_doesnt_count` | `peer_ack` suppresses the forward and doesn't count toward the cap | **PRESERVED** → a participant declaring *done* posts a vote instead of waking the next. Same intent, expressed in the consensus model. |
| 9 | `peer_ack_on_substantive_turn_forwards_anyway` | a >200-byte acked turn forwards anyway, tagged | **PRESERVED, upgraded** — the length proxy stays as the floor, and the tag becomes `envelope` metadata the user can see. Origin note: this guard exists because four full reviews were destroyed by acked-but-substantive turns. |
| 10 | `peer_ack_final_suppresses_a_substantive_turn` | `final: true` suppresses regardless of length | **PRESERVED** → an explicit done-vote. |
| 11 | `substantive_turn_without_final_still_forwards` | the inverse of #10 | **PRESERVED** — same pair as #9. |
| 12 | `convergence_reset_clears_stale_streak` | a user message resets the repetition streak | **PRESERVED** → a user message resets the cycle to participant 1 AND clears done-votes and spin state. |
| 13 | `convergence_reset_survives_a_suppressed_forward` | the reset flag isn't consumed by a suppressed forward | **PRESERVED** — subtle and worth carrying: a reset must survive a turn that produced nothing, or a stale streak silences the first real post after a user message. |
| 14 | `counters_track_per_direction_on_delivery` | per-direction forward counters | **DROPPED** — "direction" is meaningless in a ring with N participants. Replaced by per-participant delivery counts, which the roster surfaces anyway. Reason: the diagnostic exists to spot a one-sided break, which cursor lag shows directly. |
| 15 | `the_hard_cap_holds_the_forward_and_delivers_it_after_the_user_speaks` | cap-held forwards flush on the user's turn | **DISSOLVED** — with #1 and #7. |
| 16 | `a_runaway_keeps_only_the_newest_capped_forward_per_agent` | at most one held forward per agent is retained | **DISSOLVED** — no hold queue means no eviction policy. |
| 17 | `a_forward_still_held_when_the_session_ends_is_recorded_as_lost` | a forward held at session end is recorded as a loss | **DISSOLVED** — nothing is ever "held"; an unread cursor at close is visible in the roster, not a loss to be recorded after the fact. |
| 18 | `a_promptly_flushed_hold_is_not_recorded_as_a_loss` | the false-positive guard on #17 | **DISSOLVED** with #17. |
| 19 | `paused_holds_forwards_and_flush_delivers_exactly_once` | pause holds, resume delivers exactly once | **PRESERVED** — exactly-once delivery on resume becomes a cursor advance, which is idempotent by construction. The *pause* semantic must survive: a paused session must not wake the next participant. |
| 20 | `dropping_router_control_aborts_the_task` | the task exits when its channel closes | **PRESERVED** → the turn sequencer must exit on session end. Same lifecycle property, new owner. |

### Pure helpers (not behaviours, but they carry logic)

| helper | verdict |
|---|---|
| `jaccard_similarity_normalizes_and_handles_edges` | **PRESERVED** — spin detection reuses it verbatim. Move with its test. |
| `peer_of_is_bilateral` | **DISSOLVED** — the ring replaces the bijection. This single 2-line function is the clearest expression of the whole agent-focused assumption. |

---

## Tally

- **PRESERVED: 11** (2, 3, 4, 5, 8, 9, 10, 11, 12, 13, 19, 20 + jaccard) — each
  needs a green test in the new model **before** the old one is deleted.
- **DISSOLVED: 8** (1, 7, 15, 16, 17, 18 + `peer_of`) — structurally impossible;
  no replacement.
- **DROPPED: 2** (6, 14) — both with stated reasons above.

## Perf note carried from behaviour #6

`a_delivered_forward_records_nothing` exists because the delivery path was
deliberately kept off storage — `open_blocking` is a lock-free cache built for
exactly that reason. The channel model puts a write back on that path. A message
row is already written per turn, so it is the same order of magnitude, but
**this must be benchmarked in B5, not assumed.** If the cursor advance turns out
hot, batch it the way `BatchEmitter` already batches emission.

## What this inventory changed about the plan

Two behaviours (#9, #13) are subtle enough that they would plausibly have been
lost in a rewrite — an acked-but-substantive turn forwarding anyway (which
exists because four real reviews were destroyed), and a convergence reset
surviving a suppressed forward. Neither is obvious from reading the *code*; both
are only visible in the tests. That is the argument for this document.
