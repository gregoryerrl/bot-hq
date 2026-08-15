//! Turn sequencer — the single loop that will replace today's reactive
//! per-agent tasks and `core::router`'s bilateral peer-forwarding.
//!
//! Exactly one participant holds the turn. When that turn ends the sequencer
//! picks the next active participant ([`Storage::next_active_participant`]),
//! hands it everything it has not read ([`Storage::unread_for_participant`]),
//! and waits. The cycle ends by consensus — every active participant has voted
//! done ([`Storage::all_active_voted_done`]) — or immediately, when a
//! participant parks a question for the user.
//!
//! **What is implemented is the ring advance, the delivery, both halts —
//! consensus, and a parked question — and the pause.** Spin detection is a later
//! task and is NOT here. Nothing spawns this yet, so no session behaves
//! differently because it exists.
//!
//! ## What the storage helpers guarantee
//!
//! The skeleton was the first consumer of the `storage::participants` helpers
//! and found five defects in them. All five are fixed; what follows is the
//! contract this loop relies on, not a list of hazards it works around:
//!
//! - `unread_for_participant` excludes the participant's OWN rows, so a
//!   backlog is input rather than an echo. It is BOUNDED at
//!   [`UNREAD_BATCH_LIMIT`](crate::storage::UNREAD_BATCH_LIMIT) rows and
//!   returns a [`ChannelPage`](crate::storage::ChannelPage); see "how far a
//!   turn reads" below for what this loop does with `more`.
//! - `next_active_participant` takes the participant that HELD the turn, not
//!   its position, and steps by place in the rotation. Migration 0045 makes two
//!   active participants sharing a `turn_position` unrepresentable besides, so
//!   the ring reaches everyone and consensus is reachable.
//! - An empty rotation is DONE — `all_active_voted_done` is vacuously `true`
//!   there. The implication runs ONE WAY: no actives ⟹ done. The converse is
//!   false — with every active participant voted done, `all_active_voted_done`
//!   is `true` while `next_active_participant` still returns `Some`. So
//!   `is_none()` is not a consensus test, and this file does not use it as one:
//!   [`hand_over`] treats `None` as "nobody to wake", which is all it means.
//! - `commit_delivery` records the batch and moves the cursor in one
//!   transaction, so there is no way to advance a cursor past rows with no
//!   record of what was handed over.
//!
//! ## Delivery does not route around the session-scope check
//!
//! A receipt carries the session its row belongs to, and delivering it into
//! another session's agent wires one session's text into another's process. The
//! compare used to live on
//! [`SessionHandle::send_to_all`](crate::core::SessionHandle::send_to_all),
//! which was the only caller holding both ids — and that left two
//! receipt-carrying routes past it: `SessionAgent::deliver` and the three-hop
//! `agent.handle.input().deliver(&receipt)`. Receipt-gated is not scope-gated,
//! and those two were receipt-gated only.
//!
//! Both routes END at
//! [`ParticipantInput::deliver`](crate::agents::ParticipantInput::deliver), so
//! the compare moved down to that one point: the input now carries its session
//! id and checks every receipt against it. This loop holds
//! [`ParticipantInput`] clones and therefore inherits the check rather than
//! restating it — which is also why [`SequencerDeps`] carries inputs and not a
//! [`SessionHandle`](crate::core::SessionHandle). Handles live in
//! `AppState::sessions` behind a mutex and the sequencer's control side is
//! expected to sit ON a handle, so the task cannot own the thing that owns it.
//! Cloned stdin is what the router and the idle watchdog already hold for the
//! same reason.
//!
//! ## The forward ladder does not survive onto the turn path
//!
//! `router::route_forward` can drop a forward (convergence) or hold one (the
//! hard cap) AFTER its row is written, so today's chat can show a row that no
//! peer ever read. **This loop does not inherit that ladder.** Every row past a
//! participant's cursor is offered to it when its turn comes, and
//! [`Storage::commit_delivery`] records each one with no withheld reason.
//!
//! That is a consequence of the model, not a policy bolted onto it. The ladder
//! is a property of PUSHING: a forward is an interruption, so suppressing one
//! is how a volley is kept from running away. A turn is a PULL — a participant
//! reads the channel when the ring reaches it, and there is no volley to damp
//! because nobody speaks out of turn. Dropping a row there would buy nothing
//! and lose the reason the cursor exists.
//!
//! `withheld_reason` stays in `commit_delivery`'s signature because a POLICY
//! may withhold a row later; that is what the column is for. This path writes
//! `None` for every row, and nothing on it withholds today.
//!
//! ### What is recorded is the ENQUEUE, not the read
//!
//! An earlier draft of this section said "recorded but not delivered" ENDS
//! here. It does not, and the overstatement is worth naming because four later
//! tasks read this file as spec.
//! [`ParticipantInput::deliver`](crate::agents::ParticipantInput::deliver)
//! returns `true` once the row is in the participant's stdin channel — a
//! 64-slot buffer in front of the process — and this loop commits the delivery
//! on that. Three gaps follow, none of them closed here:
//!
//! - a row still sitting in that buffer when `agents::spawn::supervise` tears
//!   its incarnation down is discarded, and the cursor is already past it.
//!   Cursors do not rewind, so those rows are gone;
//! - a participant with no stdin at all gets nothing. This one IS handled:
//!   [`deliver_backlog`] commits only the prefix that actually reached the
//!   channel, so the cursor never moves past a row the agent did not get;
//! - a failed [`Storage::commit_delivery`] leaves the cursor behind rows that
//!   already went out, so the next turn hands them over a second time. The
//!   storage layer expects this — the delivery INSERT is `OR IGNORE` on
//!   `(participant_id, message_id)`, so the record is idempotent — but the
//!   participant reads the rows twice.
//!
//! So the claim this path supports is: it withholds nothing, and it records
//! what it handed to the transport. "The agent read it" is a stronger statement
//! than anything here establishes.
//!
//! ## A participant with no stdin freezes the cycle
//!
//! [`SequencerDeps::inputs`] can be missing a participant that is in the ring —
//! `SessionAgent::participant_id` is `None` whenever the roster read failed at
//! spawn. Handing that participant the turn delivers nothing, and no
//! [`SequencerCommand::TurnComplete`] can come back from a process that was
//! given no input, so **the cycle stops there**. Nothing in this file recovers
//! from that on its own: auto-advancing past an unreachable participant is a
//! recovery policy and belongs with spin detection.
//!
//! What is here is the way OUT of it that does not need a policy —
//! [`SequencerCommand::ParticipantJoined`] supplies the missing stdin, and if
//! the turn is already sitting on that participant its backlog is delivered
//! immediately. That is also the only way a participant invited AFTER the task
//! spawned can ever be reached, since [`run_sequencer`] owns its
//! [`SequencerDeps`] and nothing else can write to the map.
//!
//! Consensus needs every active participant to vote done, and a participant
//! that never receives input never votes — so a ring member with no stdin has
//! to become reachable, not merely be stepped over. That is now a live cost
//! rather than a future one: an unreachable ring member cannot vote, so the
//! tally can never complete, so the session can never halt by consensus.
//!
//! ## The halt is a yield, not a stop
//!
//! Every accepted completion carries how it ended —
//! [`TurnComplete`](SequencerCommand::TurnComplete)'s [`TurnEnding`] — and
//! [`halted_on_consensus`] records it and asks
//! [`Storage::all_active_voted_done`] BEFORE the ring is stepped. Arriving
//! means waking nobody, so a step taken first would have to be taken back.
//! **`all_active_voted_done` is the halt test and the only one**; the ring's
//! own `None` is "nobody to wake" and is not a substitute, as above.
//!
//! **One of the three endings carries no vote at all.**
//! [`TurnEnding::Passed`] — the design's PASS — steps the ring, sets nothing
//! and clears nothing but the passer's own stale vote, so a ring in which every
//! participant passes can never halt by consensus. That is the intended
//! reading, not an oversight: a pass says "not me this round", and a session
//! where nobody has anything to say has not arrived anywhere.
//!
//! **What bounds such a ring is the round cap, and nothing else in this file
//! does** — the list of the alternatives is short and worth being exact about,
//! because the obvious candidate is NOT on it. A
//! [`UserMessage`](SequencerCommand::UserMessage) does not end an all-pass ring
//! — it resets the cycle to the front and hands out another turn, which is a
//! redirect, not a stop. What else stops it is
//! [`Pause`](SequencerCommand::Pause), a
//! [`HaltDeclared`](SequencerCommand::HaltDeclared), the command channel
//! closing (the session going away), or a participant that stops completing
//! turns at all — a dead process leaves the turn in flight for ever, which ends
//! the spend by wedging rather than by deciding. Spin detection is deliberately
//! not among them: [`TurnEnding::Passed`] skips it, for the false positive
//! named at that call site.
//!
//! The cap counts LAPS of the ring in the current uninterrupted stretch, halts
//! the cycle when it is reached and posts a row saying so. It is design §1b's
//! CRUDE second backstop — subordinate to consensus and to spin detection, and
//! set high enough (500 laps by default, `0` = off) that it should never be
//! what ends a legitimate run. See [`DEFAULT_ROUND_CAP_LAPS`] for the unit, the
//! measurement behind the number, and the per-session override.
//!
//! That paragraph is about the CONSENSUS halt, which is a tally. The other halt
//! reason — a parked question — is not, and has its own section below.
//!
//! Two things reset the tally, both of them substantive output: a completion
//! ending [`TurnEnding::Spoke`], and a
//! [`UserMessage`](SequencerCommand::UserMessage).
//! The reset is session-wide, not per-participant, because a vote cast before
//! someone else spoke was a statement about a session that no longer exists —
//! left standing, one stale done and one fresh one add up to an arrival nobody
//! voted for, and the session halts with a participant never having read what
//! the other said.
//!
//! The second of those resets is bound to the RESTART, not to the command:
//! [`advance_turn`] empties the tally whenever it steps to the front of the
//! rotation. A user message is still the only way to the front, so the two are
//! the same event and no test can tell the SHAPES apart — but the parked-question
//! halt below made the binding earn its keep anyway. That halt leaves votes
//! standing (it touches none), so the cycle its release restarts is the one case
//! where the tally arriving at the front is non-empty for a reason other than
//! "the user spoke over a turn". `a_parked_question_finishes_the_lap_then_halts`
//! is the test: it parks with one `done` standing and then requires the first
//! [`TurnEnding::Done`] of the restarted cycle to STEP the ring rather than
//! complete a tally of two.
//!
//! The reset's own two halves are pinned separately —
//! `a_user_message_resets_the_cycle_to_the_first_participant` for the ring, on a
//! ring of three so a rewind is distinguishable from a step, and
//! `a_user_message_over_a_live_turn_clears_the_tally` for the votes. Plenty of
//! tests here reach the clear with a holder still in flight — that is the one
//! condition under which `current.is_none()` and `holder.is_none()` differ — but
//! they arrive with an EMPTY tally, so the narrowing is invisible to them. The
//! second test above is the one that gets there with a vote standing, and can
//! therefore see it. `the_reset_survives_a_turn_that_produced_nothing`
//! then carries the router's #13: after the restart, a turn that produces no
//! output must not be able to complete a tally holding a pre-message vote, or
//! the first real post after the user spoke is silenced by a participant that is
//! never woken.
//!
//! **Mechanically the halt is: no holder, no live epoch, and the loop goes back
//! to `recv`.** It emits nothing and marks nothing extra. Three consequences,
//! and the third is a gap:
//!
//! - the DURABLE record already exists — the votes are in
//!   `session_participants.done_vote`, so a host that wants to know whether a
//!   session arrived asks `all_active_voted_done` rather than watching for an
//!   event. Adding a second marker here would be a second copy of a fact that
//!   is already stored;
//! - the halt SURVIVES a repeat. `holder` is `None` and the epoch has moved, so
//!   a completion arriving afterwards cannot name the turn in flight — there is
//!   not one — and the cycle restarts only on a user message, which is what
//!   "yields to the user" means operationally.
//!   `the_cycle_halts_when_every_active_participant_votes_done` pins this with a
//!   late SUBSTANTIVE completion, because repeating the halting VOTE pins
//!   nothing: a loop that wrongly accepted that would record the same vote,
//!   find the same consensus and fall silent for the same reason. **The two
//!   clears are belt and braces, and the test is honest about it** — delete
//!   either one alone and the suite stays green, since the other half of the
//!   guard rejects unaided; delete both and that test fails;
//! - **nothing NOTIFIES the user.** A yield the user is not told about is a
//!   session that has gone quiet, and telling them needs a sink
//!   [`SequencerDeps`] does not have — it carries storage and stdins, no event
//!   emitter. Which sink that is (a `SessionActivity::AwaitingUser` transition,
//!   a tray row) is the host's contract, and guessing it here would be inventing
//!   the interface the task that spawns this loop has to define. That task owns
//!   it, along with the epoch round trip.
//!
//! ### The second halt reason: a parked question
//!
//! [`HaltDeclared`](SequencerCommand::HaltDeclared) halts the same way and
//! on different grounds. Consensus is an ARRIVAL — every active participant
//! agreed there is nothing left to do — so it is a tally and needs all of them.
//! A parked question is a YIELD by one, and **the ring finishes its lap before
//! the yield takes effect** (rc3 D22): the asker's turn ends, everyone waiting on
//! nothing still gets a turn, and the cycle halts when the rotation comes back
//! around to somebody who is blocked. It is still not counted,
//! not guarded and not voted on. `a_parked_question_finishes_the_lap_then_halts`
//! pins the difference — the ring stops with one participant's `done` standing
//! and the other's never cast, which no consensus test would ever produce.
//!
//! Mechanically it is the same two lines, in the same place: [`halt`]. So the
//! two reasons cannot drift apart, and everything the bullets above say holds
//! here too — no event, no marker, both halves of `TurnComplete`'s guard
//! rejecting afterwards, and nothing notifying the user.
//!
//! **Nothing releases it that does not also release a consensus halt**, and the
//! bridge supports that better than the obvious worry suggests. The worry is
//! that "the user answered" and "the user posted a row" might be different
//! events, since `resolve_choice` has an in-band branch that returns the pick
//! through the agent's own MCP call and writes no row. Traced rather than
//! assumed, they are the same event: every path that clears the awaiting flag
//! also writes an `origin = "user"` row.
//!
//! - every agent-facing park — `ask_user_choice`, `supersede_question`,
//!   `request_approval_parked`, `action_gate` — reaches
//!   `ask_user_choice_inner` with `blocking = false`, and that path DROPS the
//!   oneshot receiver before returning. So `resolve_choice`'s `tx.send` cannot
//!   succeed, and the answer always falls through to `deliver_oob`, which posts
//!   the row. The in-band branch is live code that no question can reach: its
//!   only `rx.await` belongs to the BLOCKING `request_approval`, whose sole
//!   caller is the pre-push git hook — host-internal, never a participant in a
//!   cycle;
//! - `mark_awaiting_user` parks with no `messages` row at all (`emit_halt_row`
//!   writes only `session_tray`, and halts never populate the pending map), but
//!   its releases still post one: a user broadcast, or `advance_phase`, which
//!   writes its transition notice as `Author::User` — and `insert_message` maps
//!   that author to `origin = "user"`.
//!
//! So a second command for "the user answered" would fire on exactly the
//! occasions `UserMessage` already does, and would be handled identically —
//! reset to the front, clear the tally, hand out a turn. Two commands the loop
//! cannot tell apart are one command. The row is also what makes the restart
//! worth taking at all: it wakes the front of the ring, and that wake earns its
//! keep only if something sits past that participant's cursor.
//!
//! What none of this establishes is that one row means one release. It does not,
//! and that gap is real — see
//! [`HaltDeclared`](SequencerCommand::HaltDeclared) for the case where two
//! parks share a flag that only ever counts to one.
//!
//! ### A row can arrive with no command behind it
//!
//! One more misfit, and it belongs to the command set rather than to consensus:
//! **nothing says "a row arrived"**, so a halt is broken only by a
//! `UserMessage`. That costs two things, and the second contradicts the design
//! rather than merely limiting it:
//!
//! - a row written by anything else — a host note, a tool result landing late —
//!   WAKES nobody;
//! - it resets NO VOTE either. "Any substantive output resets the tally" is
//!   therefore true only of output this loop is told about, and the gap is not
//!   theoretical: **a session can arrive at consensus with rows no participant
//!   has read.**
//!
//! `the_cycle_halts_when_every_active_participant_votes_done` demonstrates that
//! rather than merely permitting it. It posts `host note`, both participants
//! vote done, and the cycle halts — at which instant neither cursor has moved
//! past `go`. The row reaches A only because the test then sends a user
//! message; without one it would sit unread behind an arrival that declared
//! there was nothing left to do.
//!
//! Production writers on that path exist today. Three are `origin = "system"`
//! host injections and nothing else: `session`'s first-spawn phase nudge
//! (`session.rs:908`), `state`'s per-agent phase instruction (`state.rs:920`)
//! and `duo`'s two adherence nudges (`duo.rs:310`, `duo.rs:459`).
//!
//! **`watchdog`'s idle nudge is not one of them, and an earlier draft listed it
//! as one.** `deliver_idle_nudge` writes TWO rows for the two things it says.
//! NOTICE — the one-line summary the user reads in the chat — goes through
//! `insert_message(.., Author::User, ..)` (`watchdog.rs:364`), which
//! `storage/messages.rs:60` maps to `origin = "user"`. Only NUDGE, the
//! instruction Brian reads, is posted as `"system"` (`watchdog.rs:379`).
//!
//! It is not alone: `AppState::advance_phase` writes its transition notice as
//! `Author::User` too (`state.rs:878-882`), and `request_phase_advance` FALLS
//! BACK to that author when an agent slug will not parse
//! (`bridge/tray.rs:916`). So **rows land on this path under the user's origin
//! with no human behind them**, which matters more for the pause than it does
//! here — see "what releases a pause" below for the obligation that puts on
//! whoever mints [`UserMessage`](SequencerCommand::UserMessage).
//!
//! (`broadcast` and `tray` write `origin = "user"` too — that is the one origin
//! a command already covers, and theirs really is the user, so they are not on
//! this list.)
//!
//! Closing it needs a fifth command and is out of scope here. Read it with the
//! notification gap above rather than apart from it: the session yields to a
//! user who is not told it has yielded, possibly over rows nobody read.
//!
//! ## How far a turn reads
//!
//! A participant 500 rows behind gets 200 from one `unread_for_participant`.
//! **This loop drains before handing the turn over** rather than leaving the
//! rest for next time.
//!
//! Deferring would start a turn on stale context: the batch limit is a
//! transport bound — its own doc says so, and says a token budget belongs where
//! the model is known — so honouring it as a context policy would mean a
//! participant acting on rows 1–200 while the user's newest instruction sat in
//! 201–500, arriving a full lap of the ring later. Draining keeps the transport
//! bounded and the semantics simple: when your turn comes you have read
//! everything you had not read.
//!
//! The loop terminates on its own: a non-empty batch advances the cursor past
//! at least one row, and an empty batch reports `more == false`.
//! [`MAX_TURN_BATCHES`] is a liveness bound for the other case — a writer
//! appending faster than the drain — not the termination argument.
//!
//! ## One turn, one write
//!
//! A page goes out as a SINGLE stdin write —
//! [`ParticipantInput::deliver_batch`](crate::agents::ParticipantInput::deliver_batch),
//! joining each row's wire with
//! [`WIRE_JOIN`](crate::storage::WIRE_JOIN) — not one write per row.
//!
//! This is a correctness property, not a saving. One outgoing message is one
//! stream-json line, and claude-code opens a TURN on the first line it reads:
//! delivering nine rows one at a time handed the participant ONE row and then
//! interrupted it eight times, mid-turn. Measured across four sessions
//! (2026-08-13), the user's own message arrived somewhere other than the front
//! of the batch **37 times out of 44**, including row 9 of 9 — and rc3 D23's
//! `[speaker]` prefix made that visible without making it stop. The ring already
//! orders a backlog by ascending id, so the user's newest instruction is its
//! LAST row; coalescing is what makes the participant read it in that order
//! rather than race it.
//!
//! Two consequences, both deliberate:
//!
//! - **the commit is all-or-nothing per page.** It was a PREFIX while a command
//!   could cut between two rows; there is nothing to cut between now. A stopped
//!   drain leaves the page wholly past the cursor, so the next turn that reaches
//!   it reads the backlog entire rather than its tail;
//! - **the page is the unit, not the turn.** A drain of `n` pages is still `n`
//!   writes. Every realistic backlog is one page — the measured ones were nine
//!   rows against a 200-row page — while a cold `on_demand` wake stays bounded
//!   by the page rather than becoming one multi-megabyte line. Nothing here caps
//!   a line by BYTES: the token cost is identical however the rows are split,
//!   and the page bound is a measured one where a byte budget would be a number
//!   with nothing behind it.
//!
//! What does NOT change is the `kind` filter (rc3 D19a). It runs inside
//! `unread_for_participant`, upstream of this, so coalescing cannot fold tool
//! rows back in.
//!
//! ## The drain does not hold the command channel shut
//!
//! Draining is the longest thing this loop does: up to [`MAX_TURN_BATCHES`]
//! writes into a 64-slot stdin channel that PARKS when full. Awaited plainly, a
//! participant whose process has stopped reading would wedge the whole session's
//! sequencer inside one `deliver_batch` with no way to reach it — session
//! teardown included.
//!
//! So each page is written under a `select!` against the command channel, and
//! four things end a drain early:
//!
//! - the control channel CLOSING, which is session end. A teardown must not
//!   wait on a wedged agent's stdin;
//! - a [`SequencerCommand::UserMessage`], which resets the ring and therefore
//!   supersedes the turn being fed. This is the user's way out of a wedged
//!   participant, and it costs nothing correctness-wise: the rows that did not
//!   land stay past the cursor and are offered again when the ring returns;
//! - a [`SequencerCommand::HaltDeclared`] **naming the participant being
//!   fed**, whose turn is over, so there is no longer a turn to feed. One naming
//!   anyone else does not stop the drain (rc3 D22) — that turn is still live;
//! - a [`SequencerCommand::Pause`], which stops the cycle where it stands. Same
//!   ledger as the park, traced rather than inherited from it: the prefix that
//!   landed is committed, the remainder stays past the cursor, and
//!   [`SequencerCommand::Resume`] re-drains from there.
//!   `a_pause_stops_the_drain_rather_than_finishing_it` asserts both the cursor
//!   and the remainder.
//!
//! **The park stops the drain rather than letting it finish, and both were
//! available.** Stopping costs the rows already read but not delivered — which
//! is not a loss, because [`Storage::commit_delivery`] records only the prefix
//! that landed, so the remainder stays past the cursor exactly as it does for a
//! user message. Finishing costs the word "immediately": the participant being
//! fed is normally the one that just blocked on a human, so every further row
//! goes into a 64-slot buffer in front of a process that has stopped reading —
//! and `deliver` PARKS when that buffer fills, for as long as the human takes to
//! answer. A halt that waits out a human before taking effect is not a halt.
//! `a_parked_question_stops_the_drain_rather_than_finishing_it` pins it.
//!
//! **"Immediately" is about the drain in progress, not about the loop.** The
//! park is deferred, so anything that arrived AHEAD of it on the channel is
//! handled first — and a `TurnComplete` that got there first will step the ring
//! and start a fresh drain for the next participant before the halt lands. So a
//! park can be one full delivery away from taking effect, and that delivery goes
//! to somebody who is not the parker. It is bounded (one turn, not a human's
//! thinking time) and it is the price of handling commands in arrival order,
//! which is deliberate everywhere else in this loop. Worth knowing before
//! reading "halts immediately and unilaterally" as "no wire can follow a park".
//!
//! Every OTHER command is taken off the channel and deferred, so a sender never
//! parks behind a drain, and then handled in arrival order once the drain
//! finishes. Deferring rather than acting is what keeps "when your turn comes
//! you have read everything you had not read" true — acting on a `TurnComplete`
//! mid-drain would hand the turn over with rows undelivered, which is the
//! deferral this section rejects. The ones that END a drain are deferred as
//! well, not acted on here: the drain sets `stop` and returns, and the loop
//! applies the reset, the halt or the pause itself.
//!
//! **Arrival order has exactly one exception, and it is the pause.** A
//! [`SequencerCommand::Pause`] goes on the FRONT of the deferred queue; every
//! other deferral goes on the back. The case is a `TurnComplete` that the drain
//! set aside a moment before reading the pause: dispatched in arrival order it
//! steps the ring and starts a fresh turn in a session the user has just
//! stopped. For the park, the section above prices the same shape at one extra
//! wake and accepts it, because a park is a fact the loop is TOLD about — a
//! human is already blocking, and their answer is what ends both the wake and
//! the halt. A pause is an INSTRUCTION, and nothing ends the extra turn but a
//! Resume the user has not sent yet. `a_pause_stops_the_drain_rather_than_finishing_it`
//! pins the queue order and
//! `a_completion_deferred_ahead_of_a_pause_hands_out_no_turn` pins what it buys.
//!
//! ## A pause holds wakes; it does not end a turn
//!
//! [`SequencerCommand::Pause`] carries router-inventory #19
//! (`paused_holds_forwards_and_flush_delivers_exactly_once`) onto the turn path,
//! and the two halves land very differently. The router held a LIST of forwards
//! and its flush had to hand each one over exactly once; this loop holds a FLAG
//! and reads a CURSOR, so exactly-once needs no bookkeeping at all —
//! [`Storage::commit_delivery`] moves the cursor to the highest id in the prefix
//! that landed and never rewinds, so re-reading it on resume offers each unread
//! row once and no more. What has to be BUILT is the other half, the one #19
//! names as the property that must survive: **a paused session must not wake the
//! next participant.**
//!
//! **A pause is not a [`halt`], and the two must not share that helper.** `halt`
//! is `*holder = None; *epoch += 1;` — it ENDS the turn, and four things break
//! if a pause is written that way:
//!
//! - `*holder = None` ends the turn, so the paused participant's own completion
//!   fails [`TurnComplete`](SequencerCommand::TurnComplete)'s identity compare
//!   and its work is DISCARDED rather than held;
//! - `*epoch += 1` fails the same guard by the other half;
//! - `None` is what the ring reads as "reset to the front", so a resume would
//!   REWIND instead of resuming where it stood;
//! - [`advance_turn`] keys its tally clear on `current.is_none()`, which is
//!   equivalent to `reset` today only because every `None` holder means a
//!   genuine restart. A halt-shaped pause breaks that equivalence — on resume
//!   `current.is_none()` would be true with `reset` false — so every resume
//!   would silently empty the tally. The one defensive line in this file that
//!   costs nothing would become a live bug.
//!
//! So what a pause touches is a `paused` flag local to [`run_sequencer`];
//! `holder` and `epoch` are left exactly as they stand. The fourth bullet is
//! therefore still hypothetical after this task: `advance_turn` is reached with
//! `reset = false` only from inside `if live`, which requires a holder, so
//! `current.is_none()` and `reset` remain the same condition — re-measured, and
//! see the comment on the clear.
//!
//! While the flag is set every command except the three below is HELD: kept in
//! arrival order, neither acted on nor discarded. Holding rather than dropping
//! is what makes "a pause keeps the turn in flight" mean something — the
//! completion of a turn that ended during the pause is exactly the thing that
//! must not be thrown away — and
//! `a_paused_session_does_not_wake_the_next_participant` asserts both, the
//! silence and then the release that hands the same completion back to the loop.
//! **The order they come back in is load-bearing**, not tidiness: see
//! [`release_held`] for the pair whose misordering hands out a turn with a human
//! still blocking.
//!
//! ### What releases a pause: a user message, and also `Resume`
//!
//! **The steer is the release, and it is the one the app already ships.** Three
//! shipped sites say so and this loop follows them rather than inventing a
//! fourth: `state`'s user-message path calls `set_paused(false)` under the
//! comment "a user message is the steer";
//! [`ActivityTracker::set_paused`](crate::core::activity::ActivityTracker::set_paused)
//! documents the latch as cleared by "Resume, a user Send (steer), or a
//! supersede"; and `AppState::resume_session` — the Paused bar's Resume button,
//! the only resume affordance the UI has — is implemented as a broadcast of
//! `RESUME_NOTICE`. So on today's wiring the Resume BUTTON arrives here as a
//! [`UserMessage`](SequencerCommand::UserMessage) and nothing mints
//! [`Resume`](SequencerCommand::Resume) at all: hold the user message and the
//! pause is unreleasable, with the bar that offers the only way out gone from
//! the UI the moment `ActivityTracker` reads unpaused. Both role prompts also
//! tell the agents that "the bridge halts the duo until the next user message";
//! holding it would make that promise false.
//!
//! That is not a hole in inventory #19 either. The router's pause held PEER
//! FORWARDS, and a user Send was always its release — "a paused session must not
//! wake the next participant" is about the turn path, not about a human
//! steering. `a_user_message_releases_a_pause_and_wakes_the_ring` pins the
//! release; `a_paused_session_does_not_wake_the_next_participant` pins the turn
//! path it does not weaken.
//!
//! **So there are two sources of truth for "paused" now** — this flag and
//! `ActivityTracker`'s latch — and they are meant to agree, on the same three
//! events. Keeping them in step is the wiring task's obligation; nothing here
//! can check it.
//!
//! ### What a `UserMessage` producer must be, now that it releases a pause
//!
//! An earlier draft of the paragraph above finished "whoever mints these
//! commands is the code that already flips the latch". **That is false, and the
//! writer list in the command-set section names the counter-examples.** It
//! holds for `state`'s user-message path, which calls `set_paused(false)` under
//! the comment "a user message is the steer" (`state.rs:737-740`) — and
//! therefore for the Resume button, which is a broadcast and routes through it.
//! It does not hold for:
//!
//! - `AppState::advance_phase`, which writes its transition notice as
//!   `Author::User` (`state.rs:878-882`) → `origin = "user"`
//!   (`storage/messages.rs:60`), and calls `clear_awaiting` but **never**
//!   `set_paused(false)`. `state.rs:743-744` says why in as many words: "a phase
//!   self-advance is not a user message";
//! - `watchdog`'s idle-nudge NOTICE (`watchdog.rs:364`), same mapping, same
//!   absence.
//!
//! The first one is reachable while stopped. An agent self-advances on its own
//! initiative — `general_rules.rs:166` tells it to, "no user click needed" — and
//! **a pause holds WAKES, not the holder**: the participant whose turn was in
//! flight keeps working by design, so it can reach that tool mid-pause. Mint a
//! [`UserMessage`](SequencerCommand::UserMessage) off that row and this loop
//! un-pauses, resets the ring to the front and clears the tally, while
//! `ActivityTracker` still reads Paused and the UI still shows the bar. The two
//! sources of truth disagree, and the one the user can see is the one that is
//! wrong.
//!
//! So the obligation belongs to the PRODUCER and is worth stating as a rule
//! rather than an assumption: **a producer of
//! [`UserMessage`](SequencerCommand::UserMessage) must either flip the pause
//! latch or not be minted from a non-human writer.** `origin = "user"` does not
//! decide the second half — host-authored rows already carry that origin — so
//! the wiring cannot be a query over it.
//!
//! **This claim was TRUE when it was written, and a decision elsewhere
//! falsified it.** `UserMessage` did not release a pause until this task made it
//! one; before that, a producer that never touched the latch was harmless, and
//! re-checking the claim the day it was written would have confirmed it. That is
//! a different failure from the usual stale citation and it wants a different
//! habit: you cannot re-verify your way out of it. When a command GAINS a
//! meaning, go back and ask what its existing producers were allowed to assume
//! under the old one.
//!
//! [`Resume`](SequencerCommand::Resume) is KEPT even though nothing mints it
//! today. It is the explicit release for a resume that carries no message, which
//! is what the wiring task needs if `resume_session` is ever split off
//! `broadcast` — and it is the only release that also finishes the delivery the
//! pause cut short, since a user message resets the ring instead. Neither
//! release hands out a turn of its own: a halted cycle has no holder, so it
//! stays halted and a user message restarts it exactly as it always did. That is
//! the third state a pause can arrive in, after "a turn in flight" and
//! "mid-drain", and `a_pause_over_a_halted_cycle_hands_out_no_turn_on_resume`
//! locks it.
//!
//! Both releases take effect where the command is READ, not where it is
//! dispatched — the same rule as the drain's pause deferral. A user message off
//! the DEFERRED queue was read earlier, so it cannot release a pause that
//! arrived after it; `a_pause_behind_a_user_message_still_holds_the_cycle` is
//! that case, and without the distinction a Stop pressed after a message is
//! silently cancelled.
//!
//! ### Where the one mutation of the replay hides, and what finds it
//!
//! [`release_held`] splices the held queue AHEAD of whatever is already
//! deferred. Splicing it BEHIND instead is a real defect rather than an
//! equivalent form, and it hides from almost everything here.
//!
//! It hides because the two agree whenever `deferred` is empty, which is every
//! ordinary state. `held` is non-empty only while paused; the loop drains
//! `deferred` completely before it calls `recv` again; and a drain only ever
//! runs unpaused. So the release almost always concatenates with nothing.
//!
//! The exception needs a drain to set commands aside and THEN stop on a pause,
//! with a [`Resume`](SequencerCommand::Resume) among the ones set aside — wire
//! order `X, Resume, Y, Pause`, which the drain turns into
//! `[Pause, X, Resume, Y]`. The pause latches, `X` is held, and the resume then
//! releases with `held = [X]` and `deferred = [Y]`. `X` was read before `Y`, so
//! `X` must go first; spliced behind, the two swap.
//!
//! **`X` and `Y` are two COMPLETIONS naming the same live turn, and it was the
//! missing entry in an enumeration that had this filed as untestable for a
//! while.** They have to be commands a drain merely SETS ASIDE — so not a park,
//! a user message or a pause, all of which stop it — which leaves a completion,
//! a [`ParticipantJoined`](SequencerCommand::ParticipantJoined), and
//! [`Resume`](SequencerCommand::Resume) itself, which is also only ever deferred
//! by a drain. The earlier enumeration was one short there, and it looked for a
//! pair of DIFFERENT commands besides: a join is observed through a backlog the
//! resume's own delivery has already drained, which is where it stopped. The
//! pair that works is a completion twice over, differing only in its
//! [`TurnEnding`] — one completes a tally the other clears — so exactly one of
//! them is ever dispatched, and which one decides whether a participant is
//! woken.
//! `the_replay_is_dispatched_ahead_of_what_the_drain_had_already_deferred`
//! builds it: `X = TurnComplete{A, 3, Done}` halts the cycle on a vote B
//! already cast, `Y = TurnComplete{A, 3, Spoke}` clears that tally and
//! hands B a turn. Behind-spliced it fails with `expected no wire, got "row 0"`,
//! and it is the only test in this file that does.
//!
//! Nothing mints [`Resume`](SequencerCommand::Resume) today, so the state is
//! still unreachable in production — untested is what it no longer is.
//!
//! ### Why not `ActivityTracker::holds_wakes`
//!
//! The host already has this notion of paused:
//! [`ActivityTracker::holds_wakes`](crate::core::activity::ActivityTracker::holds_wakes)
//! answers "cancelling or paused", and `core::router` reads it lock-free on
//! every forward. It is not reused here, for the reason
//! [`HaltDeclared`](SequencerCommand::HaltDeclared) already gives about the
//! awaiting flag: **a flag is a LEVEL and this loop needs an EDGE.** The
//! sequencer sits in `recv().await` between turns, so a latch flipped elsewhere
//! is only ever observed wherever the loop happens to look, with no defined order
//! against [`Resume`](SequencerCommand::Resume) or
//! [`UserMessage`](SequencerCommand::UserMessage) — and the ordering of a pause
//! against the commands around it is the whole of this section. It also answers
//! a question this loop does not ask: `holds_wakes` is `cancelling || paused`,
//! and a cancel settling is a state the sequencer has no concept of.
//!
//! The NOTION is the same one, though, and the two are meant to agree: whoever
//! mints [`Pause`](SequencerCommand::Pause) MUST be the code that calls
//! `set_paused(true)`, and that wiring belongs to the task that spawns this
//! loop, alongside the epoch round trip. An obligation on the producer, stated
//! the same way and for the same reason as the one on
//! [`UserMessage`](SequencerCommand::UserMessage)'s producers above — nothing
//! today mints this command, so there is no existing site to read it off.
//!
//! The latch shape is borrowed deliberately — `set_paused` is a plain store
//! rather than a counter, and so is this.

use crate::agents::ParticipantInput;
use crate::signaling::SignalingBridge;
use crate::storage::{MessageKind, Participant, PersistedMessage, Storage};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// How many [`UNREAD_BATCH_LIMIT`](crate::storage::UNREAD_BATCH_LIMIT) batches
/// one turn will drain before handing over anyway.
///
/// Not the termination argument — see the module doc. This bounds the case
/// termination does not cover: a writer appending rows faster than the drain
/// consumes them would otherwise hold the turn open indefinitely.
///
/// 32 × 200 = 6,400 rows. **Measured 2026-08-07** against the live database:
/// the largest single channel holds 4,719 rows, which is 74% of this cap — not
/// the comfortable margin the figure this doc carried first (3,585, measured
/// some weeks earlier) implied. Re-measure before treating the headroom as
/// real; the number moves, and four later tasks read this file as spec:
///
/// ```sql
/// SELECT MAX(c) FROM (SELECT COUNT(*) c FROM messages GROUP BY session_id);
/// ```
///
/// Reaching the cap is therefore no longer only "something is producing during
/// a turn". It is one long-lived session away, and what happens there is: the
/// drain stops, the turn is handed over, and the remainder arrives on this
/// participant's next turn — the deferral the module doc rejects as a policy,
/// used here only as a backstop, with a `warn!` to say it happened.
///
/// `the_batch_cap_hands_over_with_the_remainder_still_past_the_cursor` pins
/// that, on a small `max_batches` rather than a 6,401-row fixture — see
/// [`deliver_backlog`]'s parameter.
const MAX_TURN_BATCHES: usize = 32;

// --- the round cap ---------------------------------------------------------
// Design §1b's SECOND backstop, and the crude one. Spin detection is primary
// and this is subordinate to it and to the consensus halt: it is a safety net,
// not a checkpoint, and §1b's rejection of a round BUDGET — "the car pulling
// over every N miles to ask permission to continue" — is why it has to be set
// high enough never to fire on legitimate work.

/// Laps before the cycle halts itself, when no tier of the policy sets one.
///
/// **The unit is a LAP: one full pass of the ring over the ACTIVE
/// participants** (rc3 decisions D2 left the unit open and flagged it; this is
/// the resolution). Not messages, not turns, not rounds-per-participant. At
/// N=2 one lap is two turns; at N=1 a lap is a single turn.
///
/// **What was measured, in the unit it was measured in.** Across **3,561**
/// uninterrupted stretches in the existing corpus (D2, 2026-08-11) the largest
/// was **294 agent text messages**, which at N=2 participants is roughly **147
/// laps**. 500 laps is therefore about **3.4× the largest observed real run
/// AT N=2** — which is what design §1b means by "high enough to be invisible in
/// normal use", at that N.
///
/// **The 3.4× is an N=2 number and does not carry to other rosters.** The
/// corpus is counted in MESSAGES, this cap is counted in LAPS, and the
/// conversion divides by N — so the same 294-message stretch is ~147 laps at
/// N=2 but ~294 laps at N=1. The same 500 is therefore about **1.7×** on a solo
/// ring, half the headroom, and the margin scales roughly as `500·N / 294`.
/// That is the number to quote as rc3 moves toward a one-participant default;
/// quoting 3.4× for a solo session overstates the margin by 2×.
///
/// **That is a messages-to-laps CONVERSION of one stretch, not a corpus-wide
/// organic maximum in laps, and it must not be quoted as one.** The available
/// proxies for a per-lap count are rain-only and undercount laps badly in the
/// tail, so the only honest statement about the corpus is the one above: the
/// biggest stretch anyone has actually run, converted at the N it ran at.
///
/// **The default is not being changed to compensate.** 500 laps with `0` = off
/// is the user's settled decision; what is corrected here is a claim that read
/// as if the margin were a property of the constant rather than of the roster.
///
/// `0` means the cap is OFF — a deliberate unattended run. Per-session
/// override: `round_cap` in [`crate::policy::Policy`], inherited
/// general → project → session and editable in the gear tab.
pub const DEFAULT_ROUND_CAP_LAPS: u32 = 500;

/// The cap in force for this session RIGHT NOW, in laps.
///
/// Re-read at each lap boundary rather than snapshotted into
/// [`SequencerDeps`], which is the `push_gate` shape: the session-policy
/// snapshot is seeded at spawn and then LIVE — the gear tab writes it and the
/// pre-push hook re-reads it per push — so a cap frozen at spawn would be the
/// one policy value in that file the user could not actually change. A lap is
/// N agent turns, so this is one small YAML read per several minutes of work.
///
/// **Every failure resolves to [`DEFAULT_ROUND_CAP_LAPS`], never to `0`.** No
/// data dir (unit tests), no snapshot yet, an unreadable or malformed one:
/// each leaves the backstop ARMED at its default. Resolving a broken file to
/// "off" would silently disarm the net on exactly the sessions whose state is
/// already suspect, and the cost of the other lean is a halt the user releases
/// with one message.
fn round_cap_laps(deps: &SequencerDeps) -> u32 {
    let Some(data_dir) = deps.data_dir.as_ref() else {
        return DEFAULT_ROUND_CAP_LAPS;
    };
    match crate::policy::session_policy::read_session_policy(data_dir, &deps.session_id) {
        Ok(Some(sp)) => sp.policy.round_cap.unwrap_or(DEFAULT_ROUND_CAP_LAPS),
        Ok(None) => DEFAULT_ROUND_CAP_LAPS,
        Err(e) => {
            warn!(
                session = %deps.session_id,
                error = %e,
                "sequencer: the session policy could not be read; the round cap stays at its \
                 default"
            );
            DEFAULT_ROUND_CAP_LAPS
        }
    }
}

/// The row a capped halt posts, so the halt is VISIBLE (rc3 decision D7: a
/// silent halt is indistinguishable from a hang).
///
/// `system_notice` under `origin = 'system'` with a NULL participant, unlike
/// the PASS row, which is prose under `origin = 'participant'`. The two differ
/// because their authors do: a pass is a participant's own line and this is the
/// host saying it stopped handing out turns. D7 accepted the cost of one more
/// injection in this lane explicitly.
///
/// One line, per that lane's sizing, and it names both ways out: say something
/// (which restarts the cycle and the lap count with it), or raise the cap.
fn round_cap_notice(laps: u32) -> String {
    format!(
        "[System: round cap reached — {laps} laps of the turn cycle without every participant \
         agreeing it was done. The cycle is halted and yields to you. Send a message to \
         continue, or change `round_cap` in Session Settings (0 turns the cap off).]"
    )
}

/// What the sequencer task needs, cloned from the session's own state at spawn
/// — the same arrangement [`RouterDeps`](crate::core::RouterDeps) uses.
pub struct SequencerDeps {
    /// The session whose turn cycle this task runs. Every ring, cursor and
    /// consensus query is scoped by it.
    pub session_id: Arc<str>,
    /// The channel is the transport: rows, cursors, done votes and the roster
    /// all live in storage, so this is where a turn's context comes from.
    pub storage: Storage,
    /// Each live participant's stdin, keyed by `session_participants.id`.
    ///
    /// A clone per participant rather than a [`SessionHandle`] — see the
    /// scope-check section of the module doc for why the task cannot hold the
    /// handle, and why holding stdin still gets the session-scope compare.
    ///
    /// **The key is not checked against the input.** The scope compare inside
    /// `deliver` is on the SESSION, so filing participant A's stdin under
    /// participant B's id inside one session is silent: B's turn would be read
    /// by A, and every row would pass the check. Nothing here can catch that —
    /// a `ParticipantInput` carries a session id and no participant id — so it
    /// is a build-time obligation on whoever assembles this map.
    ///
    /// A participant in the ring but ABSENT from the map has no live process
    /// behind it: `SessionAgent::participant_id` is `None` whenever the roster
    /// read failed at spawn. [`deliver_backlog`] warns and delivers nothing,
    /// and it does not skip ahead in the ring — see "a participant with no
    /// stdin freezes the cycle" in the module doc for what that costs and for
    /// the way out, which is [`SequencerCommand::ParticipantJoined`].
    pub inputs: HashMap<i64, ParticipantInput>,
    /// Where each participant's pump reads the epoch of the turn it is holding,
    /// keyed like [`Self::inputs`].
    ///
    /// Written here at handover and read by the pump on its turn's first event,
    /// which is the round trip [`SequencerCommand::TurnComplete`]'s doc calls
    /// unsolved — this is the solution. The cell exists because there is no
    /// channel from this loop to a pump: the turn travels as bytes on stdin, and
    /// the process reading them is not the task that reports the completion.
    ///
    /// A participant missing from this map still gets turns; its completions
    /// arrive with epoch 0 and are discarded by the guard once the ring has
    /// stepped once, so the cycle stalls on it rather than mis-stepping. Empty
    /// is therefore the safe default, and it is what the unit tests use — they
    /// are their own sender and mint epochs directly.
    pub epochs: HashMap<i64, Arc<std::sync::atomic::AtomicU64>>,
    /// Where `.local/session-policies/<sid>.yaml` lives, so the round cap can
    /// be re-read per lap — see [`round_cap_laps`], which is the only reader.
    ///
    /// `None` is not "no cap": it resolves to [`DEFAULT_ROUND_CAP_LAPS`], so a
    /// caller with no data dir (the unit tests) still runs the backstop.
    pub data_dir: Option<PathBuf>,
    /// Where the capped halt's row is announced, so the UI refreshes on it.
    ///
    /// Optional for the same reason [`crate::core::duo::DuoConfig`]'s is: the
    /// unit tests have no bridge, and a missed notification costs a row the
    /// user sees on their next refetch rather than immediately. The ROW is
    /// posted either way — that is the half D7 requires, and it does not depend
    /// on this field.
    pub bridge: Option<Arc<SignalingBridge>>,
    /// Where a participant is marked WORKING when the ring hands it a turn.
    ///
    /// **The ring is the only thing that knows a turn started**, and until this
    /// field existed it could not say so. `SessionActivity::derive` locks the
    /// chat input while any participant is busy, and busy was set in exactly two
    /// places — `AppState::broadcast` (every agent, when the user types) and
    /// `SessionHandle::send_to_all` — while the PUMP cleared each participant's
    /// own flag at its own turn end. So a user message locked the input, each
    /// participant unlocked its share as its turn finished, and after ONE lap
    /// every flag was clear: the input re-opened while the ring was still
    /// cycling (D22's lap, the consensus tally, the round cap's 500 laps).
    ///
    /// The user reported it from the outside — *"I can type while agents are
    /// working, it might legitimately interrupt your turns"* — and they are
    /// right about the cost: a message typed mid-lap supersedes the in-flight
    /// turn, and when the reset target is the participant already holding it
    /// (the front of the rotation, the common case) its new backlog is written
    /// to a stdin whose turn is still running.
    ///
    /// The guarantee that used to cover this was the ROUTER's ordering —
    /// peer-busy set before sender-idle, so `derive` never saw both idle
    /// mid-handoff — and it was deleted with `core/router.rs` in rc3. This is
    /// its replacement, and it is stronger: the router only closed the gap
    /// between two agents, where this holds the lock for the whole cycle.
    ///
    /// `Option` for the same reason [`Self::bridge`] is: the unit rings
    /// construct deps directly and most of them are not about the input lock.
    pub activity: Option<Arc<crate::core::activity::ActivityTracker>>,
}

/// A wake for the sequencer.
///
/// **Bodies stay out of these.** A command says that something happened; WHAT
/// happened is a row in `messages`, and the sequencer reads rows from cursors.
/// Carrying the text in the command instead would put a second copy of it in
/// flight with no row identity behind it — the thing this batch's receipt work
/// removed.
///
/// [`ParticipantJoined`](Self::ParticipantJoined) carries a
/// [`ParticipantInput`] and is not an exception to that: an stdin is the
/// capability to write to a process, not a message, and it carries no text.
#[derive(Debug)]
pub enum SequencerCommand {
    /// The turn identified by `epoch` finished — advance the ring.
    ///
    /// **`ending` carries the consensus vote**, and it is a field on this
    /// command rather than a command of its own. A turn ends exactly once, so
    /// what it meant is a property of the ending, not a second event. Split
    /// into `TurnComplete` + `Done`, both would mean "my turn ended", both would
    /// need this same two-field guard, and both would have to step the ring;
    /// a sender that emitted the pair would then step it twice and put two
    /// participants on a turn at once, which is the one invariant this loop
    /// exists to keep. One field keeps "one accepted completion, one ring step,
    /// one vote" true by construction.
    ///
    /// **The pass (design §1) widened this field rather than adding a fourth
    /// command**, for exactly that reason: a `TurnPassed` alongside this one
    /// would be a second thing meaning "my turn ended", needing the same guard
    /// and taking the same ring step. [`TurnEnding`] is still ONE field, and it
    /// is still the derived MEANING rather than the signals behind it — a
    /// sender handed `(peer_ack, peer_ack_final, passed, body)` would have four
    /// fields to disagree about instead of one, and the decision would move
    /// into this loop. [`turn_ending`] is where it is made.
    ///
    /// [`TurnEnding::Spoke`] is substantive output and RESETS the tally for the
    /// whole session; [`TurnEnding::Passed`] resets nothing and counts as
    /// nothing — see [`halted_on_consensus`]. The vote is recorded only for a
    /// completion that passes the guard below, for the same reason the ring is
    /// only stepped for one.
    ///
    /// **Both fields are the guard, and the epoch is the load-bearing one.**
    /// The sequencer stamps a fresh `epoch` on every handover
    /// ([`hand_over`]) and accepts a completion only when both fields match the
    /// turn in flight.
    ///
    /// `participant_id` alone is not enough, and the case it misses is the
    /// commonest one there is. A user message resets the ring to its first
    /// place while the previous holder is still mid-turn. When that holder IS
    /// the first place in the ring — the ordinary "user interjects while the
    /// first agent works" — the reset re-wakes the same participant, so the
    /// stale completion names the current holder, passes an identity check, and
    /// steps the ring off a participant that was woken half a second ago. Two
    /// agents on a turn at once, which is the one invariant this loop exists to
    /// keep. `a_completion_from_a_turn_the_user_restarted_is_discarded` is that
    /// case; with the epoch compare removed it fails.
    ///
    /// **There is no case running the other way**, so do not read "both fields"
    /// as a symmetry. For a sender that returns the epoch it was handed, the
    /// identity compare is redundant — the epoch already names one turn and one
    /// holder — and disabling it leaves every test here green. It is kept as
    /// defence against a malformed sender; see the comment on the guard itself.
    ///
    /// A user message is the reachable producer today, and a supervisor that
    /// respawns an agent mid-turn would be a second. **An earlier draft named
    /// Pause/Resume as one "once it is implemented"; it is implemented now and it
    /// is not one.** A pause leaves `holder` and `epoch` alone, and a completion
    /// that arrives during one is held and replayed at the epoch it was minted
    /// with — so it passes this guard rather than failing it.
    ///
    /// One case does discard a held completion, and it is not the pause doing
    /// it: a [`HaltDeclared`](Self::HaltDeclared) held AHEAD of it halts the
    /// cycle on replay, which takes the holder and moves the epoch, so the
    /// completion behind it is discarded exactly as it would have been live.
    /// That is the park's semantics surviving the pause intact rather than a
    /// producer of its own, and `the_pause_replays_what_it_held_in_arrival_order`
    /// is where it is observed. (An earlier draft of this paragraph named a
    /// replayed `UserMessage` instead. That is no longer reachable — a user
    /// message is a RELEASE and is never held.)
    ///
    /// **Where a sender gets the epoch is not solved here.** The sequencer
    /// mints it at handover, and it has to travel out with the turn and come
    /// back on the completion. Nothing spawns this loop yet, so nothing carries
    /// it yet; the tests below are the only senders, and wiring the round trip
    /// belongs with the task that spawns the loop. `done` now rides the same
    /// unsolved round trip, so the task that carries the epoch OUT is the one
    /// that has to carry the vote back. A participant that arrives by
    /// [`ParticipantJoined`](Self::ParticipantJoined) is the sharpest case:
    /// that command hands the loop an stdin and gets nothing back, so a late
    /// joiner holding the turn has no way to learn which epoch to complete
    /// with — and therefore no way to cast a vote that passes the guard.
    TurnComplete {
        participant_id: i64,
        epoch: u64,
        ending: TurnEnding,
    },
    /// The user posted to the channel. Resets the cycle to the first active
    /// participant and hands it the turn.
    ///
    /// Also one of the commands that cuts a drain short — see "the drain does
    /// not hold the command channel shut" in the module doc.
    ///
    /// **It is also the release for a pause**, and on today's wiring the only
    /// one: `AppState::resume_session` is a broadcast, so the Paused bar's
    /// Resume button arrives as this command. See "what releases a pause" in the
    /// module doc for the three shipped sites that decide it — and the section
    /// after it for what releasing a pause demands of whoever MINTS this
    /// command, which is not satisfied by "the row said `origin = 'user'`".
    ///
    /// The release is applied where this command is READ off the wire, and the
    /// commands the pause held are replayed AHEAD of it, so the message still
    /// lands in arrival order behind them.
    ///
    /// **`mentions` is the participants the user NAMED**, in the order written
    /// (rc3 D17). Empty is the ordinary case and the one every paragraph above
    /// describes: reset to the front. Non-empty changes the target and nothing
    /// else — each named participant takes one turn, in order, and then the
    /// rotation carries on **from where it was**, because a mention is an
    /// insertion rather than a reset. Summoning someone must not silently
    /// restart the cycle at participant 1.
    ///
    /// Resolved to ids by the producer, not here: this loop holds no roster and
    /// an id it cannot find is a state it should not have to reason about. The
    /// parse itself is `core::mentions`, which runs on exactly one path — the
    /// user's own message — so a participant cannot summon anyone.
    UserMessage { mentions: Vec<i64> },
    /// The user STAGED a response while the ring runs (the Stage toggle,
    /// 2026-08-15): the content sits in `AppState`; this is only the flag.
    /// At the next turn boundary the ring parks instead of dealing and emits
    /// [`SignalingEvent::StagedDeliveryDue`] — the delivery then arrives as
    /// an ordinary [`UserMessage`](Self::UserMessage), which is what makes a
    /// staged send land exactly like a typed one, never mid-turn. Pause is
    /// still the only interrupt; staging changes WHEN the user may compose,
    /// not when a message may land.
    MessageStaged,
    /// The user un-toggled Stage to edit: clear the flag. The content was
    /// already removed from `AppState` by the caller, so a boundary that
    /// races this command finds nothing to deliver and simply yields — an
    /// idle ring under an open box, which is what an editing user wants.
    MessageUnstaged,
    /// A participant's stdin, arriving after the task was spawned.
    ///
    /// The map in [`SequencerDeps`] is owned by [`run_sequencer`], so this is
    /// the only way to add to it: a participant invited mid-session, or one
    /// whose roster read failed at spawn and left it unreachable. Replaces any
    /// existing entry for the id, which is what a respawn needs.
    ///
    /// If the turn is already sitting on this participant, its backlog goes out
    /// on arrival — the turn was handed to it and could not be delivered, and
    /// this is when it becomes deliverable. The ring does NOT move: no turn
    /// ended.
    ParticipantJoined {
        participant_id: i64,
        input: ParticipantInput,
    },
    /// A participant parked a question for the user: halt the cycle now.
    ///
    /// **A yield, not a vote.** A completion is an opinion about one turn and is
    /// guarded by [`TurnComplete`](Self::TurnComplete)'s two fields. This is a
    /// fact about the SESSION — a human is being waited on — so it carries no
    /// participant id and no epoch, and nothing counts it. One participant
    /// blocking on the user stops the cycle regardless of what the others would
    /// have done; that is what "unilaterally" means, and it is why there is
    /// nothing here to guard.
    ///
    /// The asymmetry is deliberate and it costs something. A duplicate or
    /// late-arriving park halts a cycle a user message has already restarted —
    /// one wasted wake, recoverable by another user message. A guard that
    /// rejected a park for not naming the holder would be wrong in the worse
    /// direction: the parker need NOT be the holder (an on-demand participant,
    /// or one whose turn a reset superseded while its process kept running), and
    /// refusing its park leaves the ring handing out turns while a human is
    /// blocking — the state this command exists to end.
    ///
    /// **The error in the other direction is the likelier one, and nothing here
    /// can fix it.** The surplus halt above is the safe side; UNDER-halting is
    /// the unsafe one, and it is reachable through the RELEASE. The awaiting
    /// flag this command stands for is a bare `bool` —
    /// `set_session_awaiting`/`clear_session_awaiting` are a plain
    /// `store(true)`/`store(false)` with no refcount — and `resolve_choice`
    /// clears it on the FIRST answer. Several questions parked at once is
    /// normal, not pathological; `list_my_pending_questions` exists to dedupe
    /// them. So two participants parking gives two halts, the user answering one
    /// gives one row and therefore one `UserMessage`, and the cycle restarts
    /// with a human still blocking on the second question — the exact state this
    /// command exists to end, arrived at through its release rather than despite
    /// it.
    ///
    /// Counting parks in this loop would not close it: the loop would have to
    /// know which answer released which park, and nothing carries that. What has
    /// to change is the flag becoming a count, which is the bridge's to fix and
    /// not this file's.
    ///
    /// **A command rather than a flag read.** The bridge already keeps a
    /// per-session `Arc<AtomicBool>`
    /// (`SignalingBridge::register_session_awaiting`), and `core::router` reads
    /// exactly that, lock-free, per forward — so a flag in [`SequencerDeps`]
    /// would need no sender at all. It was not taken, because a flag is a LEVEL
    /// and this loop needs an EDGE: it sits in `recv().await` between turns, so
    /// a flag is only ever seen wherever the loop happens to look, and it has no
    /// defined order against [`UserMessage`](Self::UserMessage) — the one pair
    /// whose ordering decides whether the release restarts the cycle or a `true`
    /// that has not been cleared yet re-halts it on the spot. "Nothing mints
    /// commands yet" does not tell the two apart either: nothing mints
    /// `TurnComplete` or `UserMessage` yet either, and the same task owes all
    /// three.
    ///
    /// **Released by [`UserMessage`](Self::UserMessage)**, like the consensus
    /// halt — see "the second halt reason" in the module doc for why there is no
    /// release command of its own and what that leaves unwired.
    ///
    /// # A halt is a halt (rc3 D35)
    ///
    /// **The ring stops where it stands.** No lap, no per-participant blocked
    /// set — a latch that parks dealing until the user's next message. The
    /// user, after watching D22's courtesy lap put peers to work under a ⏸
    /// HALT banner: *"HALT doesn't halt the agents... A halt is a halt. Still
    /// working means still working."*
    ///
    /// This variant spent a day as `QuestionParked` with D22's lap semantics
    /// (end the asker's turn, finish the rotation, halt on reaching the
    /// blocked). That existed because ordinary QUESTIONS used to send it, and
    /// a first-turn question halting the ring made peers unreachable
    /// (`s-e8a20797`: 4/0/0 deliveries). D35 removed the cause instead of
    /// softening the effect: **a question no longer reaches the ring at all**
    /// — only `mark_awaiting_user` / `request_phase_advance` mint this, and
    /// they mean stop.
    ///
    /// # `participant_id` — whose turn ends
    ///
    /// The holder declaring the halt ends its turn; a halt declared by a
    /// NON-holder (a tool call still live after its turn was superseded) sets
    /// the latch and leaves the live turn alone — Pause is the only interrupt,
    /// and the latch stops the next deal, which is the halt taking effect at
    /// the boundary. `None` (unresolvable declarer) behaves like a non-holder
    /// with nothing in flight: halt outright.
    HaltDeclared { participant_id: Option<i64> },
    /// An approval gate parked: a command is synchronously blocked on the
    /// user's yes/no (rc3 **D35**). **The session halts.** While any gate is
    /// open the ring deals no turns — the user's decree, overturning the D22-era
    /// split ("the asker is blocked; peers keep working"): *"Approval gate
    /// halts the session, stop overcomplicating things like halting just for
    /// the agent that asked."*
    ///
    /// The turn in flight stays in flight: the asker is usually mid-turn,
    /// blocked inside its own tool call, and cutting that would kill the very
    /// command awaiting approval. The gate is a LATCH consulted where turns are
    /// dealt, not an interrupt.
    GateOpened,
    /// An approval gate resolved (approved, rejected, or discarded). Decrements
    /// the latch and **never deals a turn itself** — the wake rides the
    /// existing release (`user_responded` → [`UserMessage`]) or the asker's own
    /// completion, so there is no second path onto a turn.
    GateResolved,
    /// Stop: hold the cycle where it stands, hand out no further turns.
    ///
    /// **The turn in flight stays in flight.** This is not [`halt`] and must not
    /// be written as one — see "a pause holds wakes" in the module doc for the
    /// four things sharing that helper breaks, the fourth of which turns a free
    /// defensive line into a live bug. It sets a flag; `holder` and `epoch` are
    /// untouched, so the paused participant's completion still names the turn it
    /// was handed and is HELD rather than discarded.
    ///
    /// Everything except this command, [`Resume`](Self::Resume) and a
    /// freshly-read [`UserMessage`](Self::UserMessage) — the steer, which is the
    /// release the app already ships — is held while the flag is set, in arrival
    /// order. The pause also cuts a drain short, on the FRONT of the deferred
    /// queue, which is the one place this loop departs from arrival order. All
    /// three are in the module doc.
    ///
    /// **A latch, not a counter**, and the asymmetry with
    /// [`HaltDeclared`](Self::HaltDeclared)'s refcount problem is worth
    /// being plain about, because it is the same hazard with the sign flipped.
    /// Two pauses and one resume runs the cycle again while one of the two
    /// pausers still means it to be stopped. There is no second pauser today —
    /// the user's stop button is one control — and the shape matches
    /// `ActivityTracker::set_paused`, which is a plain store. If a second one
    /// ever appears this becomes a count, in the same place and for the same
    /// reason the awaiting flag will.
    Pause,
    /// Release a [`Pause`](Self::Pause) and continue the cycle.
    ///
    /// **Nothing mints this today** — `AppState::resume_session` broadcasts, so
    /// the Resume button arrives as a [`UserMessage`](Self::UserMessage). It is
    /// kept because it is the release for a resume that carries no message,
    /// which is what the wiring task needs if `resume_session` is ever split off
    /// `broadcast`, and because it is the only release that finishes the
    /// delivery the pause cut short rather than resetting the ring.
    ///
    /// Two things, and it hands out no turn doing either. It finishes that
    /// delivery — the same holder, the same epoch, re-read from the cursor — and
    /// it replays what the pause held, in arrival order. A halted cycle has
    /// neither, so a resume leaves it halted; a user message is still the only
    /// release for that.
    ///
    /// **Idempotent in the sense that matters, which is narrower than "a no-op".**
    /// The delivery half re-reads a cursor [`Storage::commit_delivery`] has
    /// already moved, so a repeat re-offers nothing it has already delivered. It
    /// is not inert: rows written since the last drain go to the current holder,
    /// outside a hand-over, because the read is of the cursor and not of a
    /// snapshot taken at the pause. That is the same property
    /// `resuming_delivers_each_unread_row_exactly_once` relies on to see anything
    /// at all. That test releases twice to lock router-inventory #19's "a flush
    /// racing the unpause must not double-deliver" — as a regression lock, not as
    /// a guard: see it for why the repeat kills no mutation of its own.
    ///
    /// **The delivery is not gated on the flag, and nothing pins that.** Wrap
    /// this arm in `if paused` and the suite stays green, because a resume with
    /// no pause outstanding and a resume that finds an empty backlog are the same
    /// silence. It is left ungated so there is one path rather than two, and
    /// because "the holder has everything it has not read" is true either way;
    /// read it as a choice, not as a pinned behaviour. Gating on the HELD QUEUE
    /// instead — flushing only when the pause caught something — is a different
    /// matter and is wrong: rows written while paused are held by no queue, and
    /// that mutation is caught by the test above.
    Resume,
}

/// Run the turn sequencer for one session.
///
/// Returns when `rx` closes — i.e. when the last sender is dropped, which is
/// session end. That is the same exit `run_router` has, and it is how
/// router-inventory behaviour #20 survives the handover: a session teardown
/// must end the task, not leave it holding a session's state alive.
///
/// #20's other half — `RouterControl::drop` aborting the task outright — has no
/// counterpart here yet, because nothing holds a sequencer handle to drop. It
/// belongs with the control struct that wires this into a session.
pub async fn run_sequencer(mut deps: SequencerDeps, mut rx: mpsc::Receiver<SequencerCommand>) {
    debug!(session = %deps.session_id, "sequencer: started");
    // The turn in flight. `None` is "the cycle has not started", which is also
    // what `next_active_participant` reads as "reset to the front".
    let mut holder: Option<Participant> = None;
    // Which turn that is. Bumped by every ring step, so a completion minted
    // before the step cannot be mistaken for one minted after it — including
    // when the step lands on the same participant. See `TurnComplete`.
    let mut epoch: u64 = 0;
    // Commands a drain took off `rx` without acting on them. Drained BEFORE
    // `recv`, so arrival order is preserved end to end.
    let mut deferred: VecDeque<SequencerCommand> = VecDeque::new();
    // Is the cycle paused? A LATCH, not a counter — see `SequencerCommand::Pause`
    // for what that costs. Deliberately NOT `holder`/`epoch`: a pause holds the
    // turn in flight, it does not end one.
    let mut paused = false;
    // Commands that arrived while paused, in arrival order. A second queue
    // rather than `deferred`, which is popped before `recv` and would therefore
    // spin: pop, re-hold, pop. Replayed by `Resume`.
    let mut held: VecDeque<SequencerCommand> = VecDeque::new();
    // Per-participant repetition state for spin detection. In the loop's frame
    // rather than storage: it describes THIS cycle, and a cycle that ends should
    // not leave a streak behind for the next one to inherit. Cleared outright by
    // a user message, which is the router's convergence reset in the ring model.
    let mut spin: HashMap<i64, SpinState> = HashMap::new();
    // Completed laps of the ring in the CURRENT uninterrupted stretch — the
    // round cap's counter. In the loop's frame for the same reason `spin` is,
    // and reset the same way: [`advance_turn`] zeroes it whenever it steps to
    // the front of the rotation, which is where a user message lands.
    //
    // **Per stretch, not per session, and that is load-bearing rather than a
    // convenience.** It is the unit D2's measurement is in — 3,561 UNINTERRUPTED
    // stretches — so a lifetime counter would be capping something nobody
    // measured. It is also what keeps the halt releasable: the cap's own halt
    // yields to the user, and a counter that survived their reply would re-fire
    // on the very next lap and wedge the session shut instead of backstopping
    // it. `a_user_message_starts_the_lap_count_over` is that case.
    let mut laps: u32 = 0;
    // Who the user has summoned, and where the rotation was when they did
    // (rc3 D17). In the loop's frame for the same reason `spin` and `laps` are:
    // it describes THIS stretch, and a queue that outlived one would hand out a
    // turn nobody asked for.
    let mut summons = Summons::default();
    // Has any participant done anything but PASS since the current lap began
    // (rc3 **D27**)? A lap of nothing but passes is a lap in which nobody had
    // anything to say, and dealing another asks the same question again at the
    // price of a full-context model call per participant.
    //
    // A `Done` counts as something: it is a participant declaring it is
    // finished, which the consensus tally acts on. Only a pass is the absence of
    // an answer.
    //
    // Measured in `s-8ac0d2d0`: after boot completed with no task yet given, the
    // ring dealt passes for 77 seconds — 23 provider calls carrying ~240 KB
    // each — to produce the string "(passed — nothing to add this round)". The
    // only floor was the 500-lap round cap, which at ~13s a turn is over five
    // hours. The user stopped it by hand, which is the one thing a backstop is
    // supposed to make unnecessary.
    let mut spoke_this_lap = false;
    // The Stage toggle's flag (2026-08-15): a user response is staged in
    // AppState and delivery is owed at the next turn boundary. The boundary
    // PARKS instead of dealing and emits `StagedDeliveryDue`; the delivery
    // then arrives as an ordinary `UserMessage` milliseconds later, so a
    // staged send lands exactly like a typed one — never mid-turn, never
    // superseding the holder.
    let mut staged_pending = false;
    // Participants that have parked a question and cannot proceed until the user
    // answers (rc3 D22). The ring skips no-one on account of this — it HALTS when
    // it reaches one, which is what bounds the extra work at one lap. Cleared by
    // a user message, which is the answer.
    // rc3 **D35**: "a halt is a halt." One latch, not a set — a declared halt
    // stops the ring where it stands, whoever declared it. Cleared by the
    // user's next message, like the blocked-set it replaces.
    let mut halted_pending_user = false;
    // Open approval gates. While nonzero the ring deals no turns. Seeded from
    // the durable rows so a respawned ring cannot deal turns under a gate that
    // parked before the restart.
    let mut open_gates: usize = deps
        .storage
        .count_pending_gates(&deps.session_id)
        .await
        .unwrap_or(0);
    if open_gates > 0 {
        tracing::info!(
            session = %deps.session_id,
            open_gates,
            "sequencer: started with approval gate(s) already pending; dealing no turns until they resolve"
        );
    }
    loop {
        let cmd = match deferred.pop_front() {
            Some(cmd) => cmd,
            None => match rx.recv().await {
                // **The steer releases the pause, and it is the release the rest
                // of the app already ships.** `state`'s user-message path calls
                // `set_paused(false)` with the comment "a user message is the
                // steer"; `ActivityTracker::set_paused` documents the latch as
                // cleared by "Resume, a user Send (steer), or a supersede"; and
                // `AppState::resume_session` — the Paused bar's Resume button —
                // IS a broadcast of `RESUME_NOTICE`, so on today's wiring the
                // button arrives here as a `UserMessage` and nothing mints
                // `Resume` at all. Holding this command would make the pause
                // unreleasable by the only affordance the UI offers. Both role
                // prompts also promise the agents that "the bridge halts the duo
                // until the next user message"; holding it would make that false.
                //
                // **At READ time, not at dispatch time** — the same rule the
                // drain's pause deferral follows. A `UserMessage` reaching this
                // loop off the DEFERRED queue was read earlier and must not
                // release a pause that arrived after it: it falls through to the
                // gate below and is held like anything else.
                Some(cmd @ SequencerCommand::UserMessage { .. }) if paused => {
                    paused = false;
                    // The steer takes its place at the END of the held queue, so
                    // everything the pause caught ahead of it — a park, a
                    // completion, a join — is applied first and in arrival order.
                    // Dropping the queue instead would under-halt (a held park
                    // would never take effect), and applying the message first
                    // would restart a cycle the park is about to stop.
                    //
                    // **The command itself is re-queued, not a fresh one.** It
                    // carries the user's mentions (D17), and minting a
                    // replacement here would release the pause while silently
                    // dropping who they summoned.
                    held.push_back(cmd);
                    let replayed = release_held(&mut held, &mut deferred);
                    debug!(
                        session = %deps.session_id,
                        replayed,
                        "sequencer: a user message released the pause"
                    );
                    continue;
                }
                Some(cmd) => cmd,
                None => break,
            },
        };
        // **The pause gate: while paused, nothing that could wake a participant
        // is acted on, and nothing is thrown away either.** Both halves are the
        // point. Discarding would lose a completion's work — the state the
        // module doc's "a pause is not a halt" section exists to keep — and
        // acting would hand out the turn the pause is there to withhold.
        //
        // `Resume` is the exemption because it is the release; `Pause` because
        // holding it would make the latch a counter (see the variant doc); a
        // freshly-read `UserMessage` because it is the release the app already
        // ships, and it is exempted above rather than here so a deferred one is
        // not mistaken for a fresh arrival. Every other command waits, including
        // `ParticipantJoined`: its map insert is harmless on its own, but it is
        // followed by a DELIVERY, and splitting the two would reorder a join
        // against a completion that arrived behind it. Nothing reads `inputs`
        // while paused, so waiting costs nothing.
        if paused && !matches!(cmd, SequencerCommand::Pause | SequencerCommand::Resume) {
            debug!(
                session = %deps.session_id,
                held = held.len() + 1,
                "sequencer: the cycle is paused; holding this command for the resume"
            );
            held.push_back(cmd);
            continue;
        }
        match cmd {
            SequencerCommand::TurnComplete {
                participant_id,
                epoch: completed,
                ending,
            } => {
                // The completion has to name the turn in flight: the same
                // participant AND the same turn.
                //
                // **The two halves are not symmetric.** The epoch is what
                // separates two turns — see the variant doc for the case that
                // `participant_id` alone lets through, and
                // `a_completion_from_a_turn_the_user_restarted_is_discarded`,
                // which fails without it. There is no mirror case: an epoch
                // names exactly one turn and a turn has exactly one holder, so
                // for a sender that returns the epoch it was handed the identity
                // compare is redundant — mutating it to `true` leaves the whole
                // suite green, and no test here pins it.
                //
                // It stays as defence against a MALFORMED sender: one that
                // echoes a live epoch back under the wrong participant id (a
                // crossed round trip, a copied field). Nothing in this file mints
                // the epochs senders will carry, so "well-formed" is an
                // assumption about code that is not written yet, and this is the
                // cheap half of the guard to keep.
                let live = completed == epoch
                    && holder.as_ref().is_some_and(|h| h.id == participant_id);
                // A DISCARDED completion was silent, which made a stalled ring
                // indistinguishable from an idle one from the outside: the cycle
                // simply stops and nothing anywhere says why. Discarding is
                // usually correct (a superseded turn, a restarted cycle), so this
                // is `debug` rather than a warning — but it must be SAYABLE, or
                // the only way to diagnose a ring that stopped is to infer it
                // from delivery rows after the fact.
                if !live {
                    debug!(
                        session = %deps.session_id,
                        participant_id,
                        carried_epoch = completed,
                        live_epoch = epoch,
                        holder = ?holder.as_ref().map(|h| h.id),
                        "sequencer: completion discarded — the ring did NOT step"
                    );
                }
                if live {
                    // A substantive turn is what makes a lap worth dealing
                    // (rc3 D27). Recorded before the consensus check so a turn
                    // that BOTH speaks and halts still counts as speech.
                    // Only a lap of NOTHING BUT passes yields (rc3 D27). A
                    // `Done` is a participant saying it is finished — that is
                    // information, and the consensus tally is what acts on it.
                    // Treating "no substantive output" as the trigger would also
                    // catch a converging session and pre-empt the arrival the
                    // tally exists to reach.
                    if !matches!(ending, TurnEnding::Passed) {
                        spoke_this_lap = true;
                    }
                    // The vote is recorded and consensus asked BEFORE the ring
                    // is stepped: arriving means waking nobody, so a step taken
                    // first would have to be taken back. Both are inside the
                    // guard, because a superseded turn's vote is an opinion
                    // about a turn that no longer exists — counting it would
                    // let a discarded completion do the one thing discarding it
                    // was meant to prevent.
                    if halted_on_consensus(&deps, &mut holder, &mut epoch, participant_id, ending)
                        .await
                    {
                        // Consensus is a stop, and every stop is a HALT
                        // (2026-08-15): fill the slot so the arrival has a
                        // banner even before any close-ask lands in the tray.
                        if let Some(bridge) = deps.bridge.as_ref() {
                            let _ = bridge
                                .mark_awaiting_user(
                                    deps.session_id.to_string(),
                                    "system".to_string(),
                                    "Every participant voted done — the task \
                                     looks complete. Answer any close-ask in \
                                     the tray, or send your next direction."
                                        .to_string(),
                                )
                                .await;
                        }
                        // The cycle just yielded on consensus with a staged
                        // response pending: deliver it now — it is the
                        // user's queued next message, and a yielded ring is
                        // exactly the boundary it was waiting for.
                        if staged_pending {
                            staged_pending = false;
                            if let Some(bridge) = deps.bridge.as_ref() {
                                bridge.notify_staged_delivery_due(&deps.session_id);
                            }
                        }
                    } else {
                        // Spin is a property of SUBSTANTIVE output. A `done`
                        // vote is a participant saying it has nothing left to
                        // add — the cycle converging rather than failing to —
                        // and judging that as repetition would halt on exactly
                        // the ending the consensus tally exists to reach. The
                        // read is skipped with it, so prose from a done-voting
                        // turn folds into the next substantive comparison
                        // instead of being dropped.
                        //
                        // **A pass is skipped for the same reason, and the
                        // false positive it avoids is the sharper one.** A pass
                        // is the sanctioned way to stay quiet, so its rows are
                        // near-identical BY DESIGN — the one shape the Jaccard
                        // test cannot tell from a participant that is stuck. A
                        // reviewer passing while an executor works productively
                        // would trip the streak within three rounds and halt a
                        // healthy cycle, which is the "punishes long-but-
                        // productive work as a false positive" the design names
                        // when it rejects a round cap as the wrong instrument.
                        // `a_participant_that_passes_every_round_never_trips_spin_detection`
                        // is that case.
                        if ending.is_substantive()
                            && spinning(&deps, &mut spin, participant_id).await
                        {
                            // The router BROKE THE VOLLEY and unlocked input. In
                            // a ring that is a halt — the same yield the parked
                            // question takes: stop handing out turns and let the
                            // user back in. Nothing is suppressed, which is the
                            // whole difference from the path this replaces: the
                            // message that tripped it is already a row, visible,
                            // and stays that way.
                            //
                            // **Structurally identical to the parked-question
                            // halt**, which is what keeps mutation M7 alive: this
                            // leaves no holder, so no later completion can be
                            // `live`, so `advance_turn(reset = false)` stays
                            // unreachable with a `None` holder. Re-measured after
                            // this landed — see the note in `advance_turn`.
                            //
                            // `warn` rather than a `system_notice` row: the
                            // notice lane already carries five host injections
                            // and is sized for one line (task 16). The
                            // on-screen reason rides the HALT SLOT instead —
                            // declared just below via the bridge, closing the
                            // "halted cycle with no on-screen reason" gap this
                            // comment used to end on.
                            let streak = spin.get(&participant_id).map_or(0, |s| s.streak);
                            halt(&deps, &mut holder, &mut epoch).await;
                            warn!(
                                session = %deps.session_id,
                                participant_id,
                                streak,
                                "sequencer: participant repeating itself across rounds; cycle halted"
                            );
                            // The halted cycle gets an ON-SCREEN reason (the
                            // gap the comment above used to end on). Same
                            // route as the provider-limit and error-streak
                            // halts: the session's halt slot + the banner,
                            // via `mark_awaiting_user` — which also latches
                            // the ring and interrupts the spinning
                            // participant's generation if one is in flight.
                            // s-f6a441ff sat "just quiet" for exactly this.
                            if let Some(bridge) = deps.bridge.as_ref() {
                                let slug = deps
                                    .storage
                                    .participant_by_id(participant_id)
                                    .await
                                    .ok()
                                    .flatten()
                                    .map(|p| p.slug)
                                    .unwrap_or_else(|| format!("participant {participant_id}"));
                                let _ = bridge
                                    .mark_awaiting_user(
                                        deps.session_id.to_string(),
                                        slug.clone(),
                                        format!(
                                            "⚠ {slug} is repeating itself across rounds \
                                             (streak {streak}) — the cycle halted so you \
                                             can steer. Its last messages were near-\
                                             identical; send a message to redirect, or \
                                             close the session if the task is done."
                                        ),
                                    )
                                    .await;
                            }
                        } else if staged_pending {
                            // The boundary the Stage toggle was waiting for:
                            // PARK instead of dealing (the same yield a halt
                            // takes) and hand delivery to the app layer. The
                            // delivery arrives as an ordinary UserMessage
                            // milliseconds later and deals to the front — so
                            // the staged send lands between turns, exactly
                            // like a typed one, and no holder's work is ever
                            // superseded by it.
                            staged_pending = false;
                            halt(&deps, &mut holder, &mut epoch).await;
                            debug!(
                                session = %deps.session_id,
                                "sequencer: staged response pending at the boundary; \
                                 parking and handing delivery to the app"
                            );
                            if let Some(bridge) = deps.bridge.as_ref() {
                                bridge.notify_staged_delivery_due(&deps.session_id);
                            }
                        } else {
                            advance_turn(
                                &deps,
                                &mut rx,
                                &mut holder,
                                &mut epoch,
                                &mut deferred,
                                &mut laps,
                                &mut summons,
                                halted_pending_user,
                                open_gates,
                                &mut spoke_this_lap,
                                false,
                            )
                            .await;
                        }
                    }
                } else {
                    debug!(
                        session = %deps.session_id,
                        participant_id,
                        completed,
                        epoch,
                        holder = ?holder.as_ref().map(|h| h.id),
                        "sequencer: completion does not name the turn in flight; discarded"
                    );
                }
            }
            SequencerCommand::UserMessage { mentions } => {
                // **Whoever the user named takes the next turn, in the order
                // they were named** (rc3 D17). Appended rather than assigned:
                // two messages in quick succession queue behind each other, the
                // same way two mentions in one message do.
                //
                // The queue is drained one entry per turn by `advance_turn`, and
                // it pre-empts the ring step rather than replacing the rotation
                // — see `Summons` for why the anchor is what makes that an
                // insertion.
                summons.queue.extend(mentions);
                // The user spoke, so nobody is waiting on them any more (rc3
                // D22). Cleared BEFORE the ring is stepped, or the restart would
                // land on a participant this set still calls blocked and halt on
                // the spot — the release re-halting itself.
                halted_pending_user = false;
                // The user's own output is substantive, so it resets the tally —
                // but the reset is NOT written here. It rides the restart itself,
                // in `advance_turn`; see the comment there for why this arm is
                // the wrong place to own it.
                //
                // The user speaking with nobody named resets the cycle to the
                // front of the rotation, whoever held the turn — `None` is what
                // `next_active_participant` reads as "reset". The previous
                // holder's turn is not cancelled; nothing here can stop it. What
                // happens instead is that the epoch moves, so its completion is
                // discarded when it arrives.
                //
                // Spin state IS cleared here, unlike the tally. Router inventory
                // #12 names the pair — a user message clears done-votes AND the
                // repetition streak — and the streak has to go for a reason the
                // tally does not share: it is what a halt was decided on, so a
                // streak surviving the message that released the halt would let
                // the first turn of the new cycle be judged against prose from
                // before the user spoke, and halt again on it.
                //
                // **Placed on the call site rather than the mechanism, which is
                // the opposite of the tally's placement**, and the argument in
                // `advance_turn` for binding to the mechanism applies here too.
                // It is here because every restart-to-the-front reachable today
                // IS a user message — a halt leaves no holder, so no completion
                // can be live, so nothing else reaches the front — and moving it
                // would mean threading this map through `advance_turn` to cover
                // a path that does not exist yet. If a second restart path ever
                // lands, this belongs next to the tally clear, not here.
                spin.clear();
                advance_turn(
                    &deps,
                    &mut rx,
                    &mut holder,
                    &mut epoch,
                    &mut deferred,
                    &mut laps,
                    &mut summons,
                    halted_pending_user,
                    open_gates,
                    &mut spoke_this_lap,
                    true,
                )
                .await;
            }
            SequencerCommand::MessageStaged => {
                if holder.is_none() {
                    // No turn in flight — the ring is parked, yielded, or
                    // between deals. There is no boundary to wait for:
                    // deliver now, exactly as the Send an open box offers.
                    debug!(
                        session = %deps.session_id,
                        "sequencer: message staged with no turn in flight; delivering now"
                    );
                    if let Some(bridge) = deps.bridge.as_ref() {
                        bridge.notify_staged_delivery_due(&deps.session_id);
                    }
                } else {
                    staged_pending = true;
                    debug!(
                        session = %deps.session_id,
                        "sequencer: message staged; delivers at the next turn boundary"
                    );
                }
            }
            SequencerCommand::MessageUnstaged => {
                staged_pending = false;
            }
            SequencerCommand::ParticipantJoined {
                participant_id,
                input,
            } => {
                let replaced = deps.inputs.insert(participant_id, input).is_some();
                debug!(
                    session = %deps.session_id,
                    participant_id,
                    replaced,
                    "sequencer: participant stdin registered"
                );
                // The turn may already be sitting on this participant, unable to
                // be delivered — that is the frozen cycle the module doc
                // describes. It is deliverable now. The ring does not move and
                // the epoch does not change: no turn ended, one finally started.
                let dealt = match holder.as_ref().filter(|h| h.id == participant_id) {
                    Some(to) => {
                        deliver_backlog(&deps, to, &mut rx, MAX_TURN_BATCHES, &mut deferred).await
                    }
                    None => Dealt::Live,
                };
                if let Dealt::CannotComplete(reason) = dealt {
                    unwind_wedged_turn(&deps, &mut holder, &mut epoch, reason).await;
                }
            }
            SequencerCommand::HaltDeclared { participant_id } => {
                // Unguarded and uncounted, unlike a completion — see the variant
                // doc. No vote is touched, so the tally standing when the
                // question was parked is exactly the tally the release has to
                // clear, and `advance_turn` is where that happens.
                // **rc3 D35: a halt is a halt — the ring stops NOW.** The
                // D22-era behaviour recorded the asker as blocked and finished
                // the lap so peers could review first; the user watched those
                // peers work under a ⏸ HALT banner and overruled it: *"HALT
                // doesn't halt the agents... A halt is a halt. Still working
                // means still working."* (D22's original defect — a first-turn
                // park making peers unreachable — cannot come back this way:
                // an ordinary QUESTION no longer sends this command at all.)
                halted_pending_user = true;
                // Only the HOLDER parking ends the turn in flight. A halt
                // declared by a non-holder — a tool call still live after its
                // turn was superseded — must not cut the holder's turn (Pause
                // is the only interrupt); the latch stops the NEXT deal, which
                // is the halt taking effect at the boundary.
                let ends_a_turn = holder
                    .as_ref()
                    .is_some_and(|h| Some(h.id) == participant_id);
                debug!(
                    session = %deps.session_id,
                    ?participant_id,
                    ends_a_turn,
                    "sequencer: a halt was declared; the ring stops where it stands"
                );
                if ends_a_turn {
                    advance_turn(
                        &deps,
                        &mut rx,
                        &mut holder,
                        &mut epoch,
                        &mut deferred,
                        &mut laps,
                        &mut summons,
                        halted_pending_user,
                        open_gates,
                        &mut spoke_this_lap,
                        false,
                    )
                    .await;
                } else if holder.is_none() {
                    // Nothing in flight: halt outright so the epoch moves and a
                    // straggler cannot bind the retired turn.
                    halt(&deps, &mut holder, &mut epoch).await;
                }
                // A staged response delivers AS THE RELEASE: the halt asked
                // for the user's next message and one is already queued. The
                // delivery clears the halt exactly as a typed answer would.
                if staged_pending {
                    staged_pending = false;
                    if let Some(bridge) = deps.bridge.as_ref() {
                        bridge.notify_staged_delivery_due(&deps.session_id);
                    }
                }
            }
            SequencerCommand::GateOpened => {
                open_gates += 1;
                debug!(
                    session = %deps.session_id,
                    open_gates,
                    "sequencer: an approval gate opened; the session halts until it resolves"
                );
                // The asker is usually mid-turn, blocked inside the gated tool
                // call — its turn stays live. With nothing in flight, move the
                // epoch now so a straggler cannot bind the retired turn while
                // the gate holds the ring.
                if holder.is_none() {
                    halt(&deps, &mut holder, &mut epoch).await;
                }
            }
            SequencerCommand::GateResolved => {
                open_gates = open_gates.saturating_sub(1);
                debug!(
                    session = %deps.session_id,
                    open_gates,
                    "sequencer: an approval gate resolved"
                );
                // Deliberately deals nothing. The wake is the asker's own
                // completion (it was mid-turn, blocked on the tool result) or
                // the release the resolve path already fires
                // (`user_responded` → `UserMessage`). Dealing here would be a
                // second path onto a turn.
            }
            SequencerCommand::Pause => {
                // **Not [`halt`], and the difference is the whole command.**
                // `holder` and `epoch` are left exactly as they are, so the turn
                // in flight is still the turn in flight: its completion still
                // names it, the ring still knows where it stands, and the resume
                // carries on rather than rewinding. See the variant doc for the
                // four things sharing `halt` would break.
                //
                // Idempotent. A second pause latches an already-latched flag,
                // which is what makes `Resume` a release rather than a decrement.
                paused = true;
                debug!(
                    session = %deps.session_id,
                    holder = ?holder.as_ref().map(|h| h.id),
                    "sequencer: the cycle is paused; no further turns are handed out"
                );
            }
            SequencerCommand::Resume => {
                paused = false;
                // **The replay goes on FIRST, ahead of anything the delivery
                // below defers.** That delivery reads commands — it runs under
                // the same select every drain does — so a pause arriving during
                // it lands on the FRONT of this queue and has to sit ahead of the
                // replayed commands, or the completion this pause held would step
                // the ring during the next one.
                // `a_pause_racing_a_resume_re_holds_what_the_resume_replayed`
                // is that case, and it is the only test here that fails if these
                // two blocks swap.
                //
                // Popped from the back onto the front, which restores arrival
                // order.
                let replayed = release_held(&mut held, &mut deferred);
                // Finish the delivery the pause cut short. Not a new turn — the
                // ring does not move and the epoch does not change; this is the
                // turn the pause interrupted, being handed the rest of what its
                // holder had not read. `Resume` never hands a turn OUT: a halted
                // cycle has no holder, so it stays halted, and a user message is
                // still its only release.
                //
                // Ahead of the replay in EXECUTION for the reason the
                // drain-before-handover rule gives — acting on a held completion
                // first would step the ring with rows undelivered, which is the
                // deferral the module doc rejects.
                //
                // Re-read from the cursor rather than replayed from a list, so
                // what goes out is everything unread AS OF THE RESUME, and each
                // row exactly once: `commit_delivery` moved the cursor past the
                // prefix that landed and it never rewinds.
                let dealt = match holder.as_ref() {
                    Some(to) => {
                        deliver_backlog(&deps, to, &mut rx, MAX_TURN_BATCHES, &mut deferred).await
                    }
                    None => Dealt::Live,
                };
                if let Dealt::CannotComplete(reason) = dealt {
                    unwind_wedged_turn(&deps, &mut holder, &mut epoch, reason).await;
                }
                debug!(
                    session = %deps.session_id,
                    holder = ?holder.as_ref().map(|h| h.id),
                    replayed,
                    "sequencer: the cycle resumes"
                );
            }
        }
    }
    debug!(session = %deps.session_id, "sequencer: control channel closed; exiting");
}

/// Move everything a pause held onto the FRONT of the deferred queue, in
/// arrival order, and say how many. Shared by the two releases —
/// [`SequencerCommand::Resume`] and a freshly-read
/// [`SequencerCommand::UserMessage`] — so they cannot disagree about the order
/// things come back in.
///
/// **Arrival order is the whole of it, and the misordering is unsafe rather
/// than untidy.** A park held ahead of a completion halts the cycle, so the
/// completion behind it then steps the ring INTO that block, so the cycle yields
/// and nobody is woken. Replay the two the other way round and the completion
/// steps the ring onto the blocked participant before anything knows it is
/// blocked — a turn handed to somebody who is waiting on a human and can do
/// nothing with it.
/// `the_pause_replays_what_it_held_in_arrival_order` is the pin; a reversal is
/// the mutation it exists for.
///
/// **A splice rather than a pop/push loop, deliberately.** Written as
/// `pop_back` onto `push_front` it produces the same queue, and a one-token slip
/// to `pop_front`/`push_front` reverses it — which is the unsafe direction
/// above, arrived at by a typo. Concatenating the two queues has no direction to
/// slip: what is left to get wrong is which side goes first, and that is a whole
/// operand swap rather than a token.
///
/// The held queue goes AHEAD of whatever is already deferred, and that is the
/// arrival order rather than a preference. Both releases are reached with an
/// empty `deferred` in every ordinary state — the loop drains `deferred` before
/// it ever calls `recv` again — so the concatenation is usually with nothing.
/// Where it is not, everything in `deferred` was read AFTER everything in
/// `held`: the only route to a non-empty one is a drain that set commands aside
/// and then stopped on a [`SequencerCommand::Pause`] it pushed to the front, and
/// a drain reads in wire order.
/// `the_replay_is_dispatched_ahead_of_what_the_drain_had_already_deferred`
/// builds exactly that state and is the only test here that fails on the other
/// operand order; the module doc's mutation note has the wire sequence and why
/// the pair has to be two completions.
fn release_held(
    held: &mut VecDeque<SequencerCommand>,
    deferred: &mut VecDeque<SequencerCommand>,
) -> usize {
    let replayed = held.len();
    // `held ++ deferred`, then hand that back as the deferred queue.
    held.append(deferred);
    std::mem::swap(held, deferred);
    replayed
}

/// Step the ring, stamp the new turn, and deliver its backlog — emptying the
/// tally first if this step RESTARTS the cycle rather than continuing it, and
/// counting the lap if it WRAPS one.
///
/// `reset` is a user message: the ring goes back to its first place instead of
/// one past the current holder. It is the only restart today, which is exactly
/// why the tally clear lives in here and not at its call site; see the comment
/// on the clear. The lap counter is reset by the same test for the same reason.
///
/// Takes `reset` rather than the current participant because `holder` is behind
/// a `&mut` here — the caller cannot lend it out and have it written back in
/// the same call.
async fn advance_turn(
    deps: &SequencerDeps,
    rx: &mut mpsc::Receiver<SequencerCommand>,
    holder: &mut Option<Participant>,
    epoch: &mut u64,
    deferred: &mut VecDeque<SequencerCommand>,
    laps: &mut u32,
    summons: &mut Summons,
    halted_pending_user: bool,
    open_gates: usize,
    spoke_this_lap: &mut bool,
    user_spoke: bool,
) {
    // **rc3 D35: nothing is dealt while the session is halted or gated.**
    // Checked before ANY handover is minted, so no busy flag is ever set for a
    // turn that will be refused — the whole D31 take-back becomes unreachable
    // instead of carefully handled. `halted_pending_user` is cleared by the
    // user's message BEFORE this runs (the release path), so a release deals
    // normally; an open gate holds even through a user message — the answer to
    // the gate is what lifts it, and the message waits in the channel.
    if halted_pending_user || open_gates > 0 {
        debug!(
            session = %deps.session_id,
            halted_pending_user,
            open_gates,
            "sequencer: dealing is parked (halt declared or approval pending); the cycle yields"
        );
        halt(deps, holder, epoch).await;
        return;
    }
    // **A user message restarts the cycle at the front — UNLESS they named
    // someone** (rc3 D17 #4). A mention is an insertion: it changes who takes
    // the next turn and leaves the rotation where it is, because summoning an
    // advisor must not silently send the ring back to participant 1.
    //
    // The two halves of "reset" come apart here, and only the ring half is
    // conditional. The BOOKKEEPING half — the tally and the lap count — is
    // cleared by the user speaking either way; see below.
    let restart = user_spoke && summons.queue.is_empty();
    // **The ring steps from the ANCHOR, not from the holder** (rc3 D17). For an
    // ordinary turn the two are the same participant — the anchor is set to
    // whoever the ring hands to — so this changes nothing on the common path.
    // It differs after a summons, where the holder is somebody the user pulled
    // in out of band and the rotation must resume from where it actually was.
    let current = if restart { None } else { summons.anchor.as_ref() };
    // The ring's own ordering key for whoever is handing the turn on, copied
    // out before `hand_over` so the borrow of `*holder` ends here and the
    // assignment below can take it mutably. `(turn_position, id)` is exactly
    // the order `next_in_ring` steps through — the `wrapped` comparison below
    // is the only reader, and `the_round_cap_counts_laps_of_the_ring_not_turns`
    // is what pins it.
    let current_key = current.map(|c| (c.turn_position, c.id));
    // A cycle starting at the front of the ring starts with an empty tally.
    //
    // **Bound to the mechanism, not to a call site.** This lived in the
    // `UserMessage` arm, where it was correct and invisible: a user message is
    // the only restart there is, so "the user spoke" and "a cycle restarts" were
    // the same event and nothing said which one the clear belonged to. The
    // parked-question halt is what separates them. It touches no vote, so the
    // cycle its release restarts is the one that arrives here with a tally left
    // over from BEFORE the halt — and left standing, the first `done: true` of
    // the new cycle completes it and halts again with the rest of the ring never
    // having taken a turn. That is the false arrival this file exists to
    // prevent, and `a_parked_question_finishes_the_lap_then_halts` is the test
    // that would go red: delete this clear and its last `expect(3)` times out.
    //
    // The test is `current.is_none()` rather than `restart` because those are
    // the two ways to the front of the rotation IN PRINCIPLE: an explicit
    // restart, and a `None` holder, which is what a halt leaves behind.
    //
    // **`user_spoke` is now the other half of the condition, and rc3 D17 is why
    // the two are no longer the same question.** A user message that names
    // someone does NOT go to the front — the mention is an insertion — so
    // `current` stays `Some` and this block would be skipped. That would leave
    // the tally from before the user spoke standing: the summoned participant
    // takes its turn, PASSES (a pass records no vote and clears nothing), and
    // the stale votes are still there for the next completion to arrive on,
    // halting a cycle in which the actives never read the message that started
    // it. Precisely the false arrival the paragraph above is about, reached down
    // a second path. The user speaking clears the tally whether or not they
    // named anyone.
    //
    // **Today they are the same condition and no test can tell them apart.**
    // `reset = false` is reached from exactly one place — inside `if live`,
    // which requires `holder.is_some()` — so `current` is always `Some` there,
    // and `current.is_none()` is true exactly when `reset` is. Swap the
    // condition for `reset` and the suite stays green — re-measured against the
    // reset tests below, which did not change it. This is kept as the defensive
    // shape, not because it covers a case that exists: it is the right form for
    // a second restart path, and it costs nothing now. Do not read it as pinning
    // a reachable behaviour, and do not read the task numbers an earlier draft
    // gave here as a schedule — task 8 owned the user-message reset and added no
    // second path.
    //
    // **The pause (task 9) has landed and added none either, which was not a
    // given.** A pause written as `halt()` — the obvious sharing, since a halt is
    // also "stop handing out turns" — leaves `holder` as `None` with the cycle
    // still live, so the resume's step would arrive here with `current.is_none()`
    // true and `reset` false, and every resume would empty the tally. That is why
    // the pause sets a flag instead and leaves `holder` alone. Re-measured after
    // it: swap this condition for `reset` and all 35 tests still pass.
    //
    // **Spin detection (task 11) has landed and is NOT the second path** — an
    // earlier draft of this paragraph guessed it might be. It halts, so it does
    // reach `None`, but it reaches it the way the parked question does: `halt()`
    // clears the holder, and every later completion then fails the `live` guard,
    // which is what calls this with `reset = false`. A `None` holder and a
    // `reset = false` call therefore still cannot co-occur. A recovery that
    // STEPPED PAST a stuck participant instead of halting would be the second
    // path, and that is the shape to re-measure against, not spin detection.
    //
    // That count is 37 as of 2026-08-11 — re-measured with task 11's three
    // tests, not carried forward — and it moves every time a test lands here; an
    // earlier draft carried 30 through two additions. Re-run the swap rather
    // than trusting the figure; what the figure is FOR is "no test tells them
    // apart", which is the part that has to be re-measured anyway.
    //
    // What IS pinned is the other narrowing. `holder.is_none()` looks equivalent
    // and is not: a user message resets a LIVE cycle too, where the holder is
    // `Some` and only `current` is `None`. Both halt tests reset from a halted
    // cycle, so both stay green under that swap. THREE fail (re-measured
    // 2026-08-11 — the list said two, having been edited when the third landed
    // rather than re-run): `a_user_message_over_a_live_turn_clears_the_tally`,
    // `the_reset_survives_a_turn_that_produced_nothing` and
    // `a_failed_reset_clears_the_tally_but_leaves_the_turn_in_flight`.
    //
    // A failure warns and continues, like the other storage faults on this
    // path. It is the one that leans the wrong way — stale votes can only make
    // an arrival come EARLY — but the alternative is refusing to hand out a
    // turn because a write failed, which strands the session outright.
    if user_spoke || current.is_none() {
        // The lap count belongs to the stretch, and this is where a stretch
        // begins — see the counter's declaration for why it is not a lifetime
        // total, and why a cap that could not be released would be worse than
        // no cap at all.
        *laps = 0;
        // The column follows the counter, here and at the increment below, and
        // nowhere else — so `sessions.round_number` cannot drift from the number
        // the round cap is actually measuring.
        deps.storage.set_round_number(&deps.session_id, *laps).await;
        if let Err(e) = deps.storage.clear_done_votes(&deps.session_id).await {
            warn!(
                session = %deps.session_id,
                error = %e,
                "sequencer: the tally was not cleared for a restart; the next arrival may \
                 count votes cast before it"
            );
        }
    }
    // **A summons pre-empts the ring step, and takes the turn INSTEAD of it.**
    // The step it displaces is not owed to anyone afterwards: the ring resumes
    // from the anchor, which this turn does not move, so nobody's place is lost
    // — the rotation is simply paused for one turn.
    if let Some(to) = hand_to_summoned(deps, &mut summons.queue).await {
        // No lap counting: a summoned turn is not a step through the ring, so
        // it cannot wrap one. No anchor update either — that IS the mechanism.
        *holder = Some(to);
        *epoch += 1;
        if let Dealt::CannotComplete(reason) = start_turn(deps, holder, epoch, rx, deferred).await
        {
            unwind_wedged_turn(deps, holder, epoch, reason).await;
        }
        return;
    }
    match hand_over(deps, current).await {
        // The ring could not be read. Keeping the holder AND the epoch is what
        // makes the retry in `hand_over`'s comment real: the same holder's
        // completion still matches, so it re-attempts the step. Overwriting
        // `holder` with `None` here instead would strand the cycle — every
        // later completion would fail the guard above, and nothing but another
        // user message would ever move it again.
        //
        // **On the reset path this leaves a user message half-applied, and that
        // is a decision rather than an oversight.** The tally is emptied above,
        // BEFORE the ring is read, so a failed read gives a cycle whose votes are
        // cleared but whose ring never went back to the front — and the previous
        // holder's completion is still live, so the retry it triggers is a STEP
        // from where the turn already was, not a second attempt at the reset.
        //
        // **A lap can be lost, and the way it is lost is this file's own hazard
        // reached down a second path.** An earlier draft priced this as a
        // misordering — every cursor is still behind the user's row, so whoever
        // is woken next reads it — and that is not the whole cost. On a
        // SUCCESSFUL reset the epoch moves, so the previous holder's completion
        // is discarded vote and all. On a held one it does not, so that
        // completion is accepted and its `done: true` — an opinion about a turn
        // that ended before the user spoke — is recorded into the tally the
        // reset just emptied. One more done vote and the cycle arrives, with the
        // participant that was mid-turn never woken for the message that was
        // supposed to restart it. `advance_turn` can empty a tally; it cannot
        // un-live a turn in flight.
        //
        // So a failed reset is NOT a reset. It is deliberately the safe half of
        // one, and both alternatives are still worse. Clearing only after a
        // successful hand-over leaves stale votes standing across a user message
        // on EVERY held reset rather than one, which is the false arrival this
        // file exists to prevent. Halting instead — the two lines of [`halt`] —
        // parks the session on a transient storage error with nothing to tell
        // the user it has yielded (see the notification gap in the module doc),
        // where this shape costs one lap of a ring that keeps moving.
        //
        // **Pinned**, by `a_failed_reset_clears_the_tally_but_leaves_the_turn_in_flight`
        // — both halves, and the lost lap. An earlier draft called this
        // unreachable from a test on the grounds that `Storage`'s pool is
        // private; the field is, but `Storage::pool` hands it out. See that test
        // for how the ring read is broken without breaking the clear.
        Handover::Held => {}
        Handover::To(next) => {
            // The D22-era "rotation reached a blocked participant" check lived
            // here, with the D31 busy-flag take-back it forced. Both are gone
            // (rc3 D35): a halt now parks dealing at the TOP of this function,
            // before any handover is minted — so no flag is ever set for a turn
            // that will be refused, and there is nothing to take back.
            // **The ring wrapped iff the participant taking the turn does not
            // sort strictly AFTER the one handing it on.** `next_in_ring` steps
            // by position through a ring ordered by `(turn_position, id)`, so
            // `ring[i] -> ring[i+1]` always moves that key forward and the only
            // step that does not is `ring[len-1] -> ring[0]`. A one-participant
            // ring steps to itself, where the key is EQUAL — hence `<=`, and
            // hence every turn of a solo ring is a whole lap, which is what "one
            // full pass over the active participants" means at N=1.
            //
            // Two steps deliberately do not count. A reset has no `current_key`:
            // it STARTS a stretch rather than closing a lap, and the counter was
            // just zeroed above. A `Handover::Held` never reaches this arm at
            // all, so a ring read that failed does not spend a lap.
            //
            // **One roster mutation under-counts by a lap, and it is left
            // under-counting on purpose.** If the holder is disabled mid-turn
            // and sat BEFORE `ring[0]` in the order, `next_in_ring` falls
            // through to its "first member sorting after `current`" arm, lands
            // on `ring[0]`, and the comparison below reads that as forward
            // motion rather than as the wrap it also is. The
            // alternative is tracking who has already held the turn this lap —
            // more state, for a backstop whose whole design is to sit well
            // above real work (~3.4× at N=2, ~1.7× at N=1; see
            // [`DEFAULT_ROUND_CAP_LAPS`] for why the margin depends on N).
            // Erring one lap LATE is the safe direction for a net that must
            // never fire on legitimate work.
            let wrapped = match (current_key, next.as_ref()) {
                (Some(cur), Some(n)) => (n.turn_position, n.id) <= cur,
                _ => false,
            };
            if wrapped {
                *laps += 1;
                deps.storage.set_round_number(&deps.session_id, *laps).await;
                // **A lap in which nobody spoke is a lap that answered nothing**
                // (rc3 D27). Every active participant declined its turn, so the
                // session has nothing to do and the only party who can change
                // that is the user. Yield to them rather than asking the same
                // question again — each repetition is one full-context model
                // call per participant, and the round cap is five hours away.
                //
                // Checked BEFORE the cap because it is the more specific reason
                // and the one worth reporting: "everyone passed" tells the user
                // what to do next, where "500 laps" tells them something ran
                // away.
                //
                // NOT a consensus arrival, and the difference is the whole
                // design: consensus is every participant saying it is FINISHED,
                // which ends the work. This says nobody has anything to add
                // right now, which ends the LAP. A pass still casts no vote and
                // still clears nothing.
                if !*spoke_this_lap {
                    halt(deps, holder, epoch).await;
                    announce_all_passed(deps).await;
                    // Every stop is a HALT (2026-08-15: "HALT means the floor
                    // is the user's") — the yield fills the session's halt
                    // slot so even the laziest stop has a banner. Agents are
                    // taught to pre-empt this generic reason with their own
                    // recap; this is the backstop for when nobody did.
                    if let Some(bridge) = deps.bridge.as_ref() {
                        let _ = bridge
                            .mark_awaiting_user(
                                deps.session_id.to_string(),
                                "system".to_string(),
                                "Every participant passed a full lap — nothing \
                                 to add without you. Send a message to resume."
                                    .to_string(),
                            )
                            .await;
                    }
                    debug!(
                        session = %deps.session_id,
                        laps = *laps,
                        "sequencer: a full lap of passes; the cycle yields to the user"
                    );
                    return;
                }
                // A new lap begins: whether anyone speaks in THIS one is a
                // fresh question.
                *spoke_this_lap = false;
                let cap = round_cap_laps(deps);
                // `0` is off, and it is tested BEFORE the comparison rather than
                // folded into it: `*laps >= 0` is true on the very first lap, so
                // a cap of zero written as a plain comparison would halt the
                // session instantly — the exact inverse of what it means.
                if cap != 0 && *laps >= cap {
                    // The same yield the parked question and the spin halt take:
                    // clear the holder, bump the epoch, hand out no turn. The
                    // ring step that landed here is dropped with `next` — the
                    // cap fires INSTEAD of the turn it was about to start, not
                    // after it — and `halt` supplies the epoch bump that step
                    // would have made, so the numbering does not skip.
                    halt(deps, holder, epoch).await;
                    // D7: a visible row, not just a log line. Posted after the
                    // halt so the cycle is already yielded if the write fails —
                    // a session that halted with no row is a notification gap,
                    // where one that kept running because a row failed would be
                    // the backstop not backstopping.
                    announce_round_cap(deps, *laps).await;
                    // And the halt slot (2026-08-15): the cap's stop gets the
                    // same banner every other stop gets.
                    if let Some(bridge) = deps.bridge.as_ref() {
                        let _ = bridge
                            .mark_awaiting_user(
                                deps.session_id.to_string(),
                                "system".to_string(),
                                format!(
                                    "The round cap ({laps} laps) was reached — \
                                     something may be running away. Send a \
                                     message to steer, or raise `round_cap` in \
                                     Session Settings.",
                                    laps = *laps
                                ),
                            )
                            .await;
                    }
                    warn!(
                        session = %deps.session_id,
                        laps = *laps,
                        cap,
                        "sequencer: the round cap was reached; the cycle halts and yields to \
                         the user"
                    );
                    return;
                }
            }
            *holder = next;
            // **The ring moved, so the anchor moves with it — and this is the
            // only place it is written.** Every step through the rotation comes
            // through this arm; a summons deliberately does not, which is the
            // whole of D17's "resumes where it was".
            summons.anchor = holder.clone();
            // Every step, including a restart that lands on the same
            // participant. That case is exactly why the epoch exists.
            *epoch += 1;
            if let Dealt::CannotComplete(reason) =
                start_turn(deps, holder, epoch, rx, deferred).await
            {
                unwind_wedged_turn(deps, holder, epoch, reason).await;
            }
        }
    }
}

/// Publish the epoch for the turn just handed out, then hand over the backlog.
///
/// Extracted so the ring step and a summons cannot start a turn DIFFERENTLY.
/// They already share the invariant that matters — publish the epoch before the
/// rows go out — and two copies of it are two things that can drift, with the
/// drift costing a participant every completion it ever sends.
async fn start_turn(
    deps: &SequencerDeps,
    holder: &Option<Participant>,
    epoch: &u64,
    rx: &mut mpsc::Receiver<SequencerCommand>,
    deferred: &mut VecDeque<SequencerCommand>,
) -> Dealt {
    // The DEAL — the holder column + the busy mark — happens HERE, strictly
    // after every check that can refuse the turn (the halt latch, open gates,
    // the all-pass yield, the round cap). It lived in `hand_over`, which
    // marked the participant BEFORE those checks ran: a wrap that yielded
    // orphaned a busy flag for a turn no pump would ever run or clear, and
    // the input stayed locked under the yield notice (s-f6a441ff — the user
    // force-paused out of it, twice). A `None` holder still writes the
    // column — clearing it, nobody is active — and marks nobody. This also
    // means a SUMMONED turn is now recorded and marked like any other, which
    // the eager placement never did.
    hand_turn_to(deps, holder.as_ref()).await;
    let Some(to) = holder.as_ref() else {
        return Dealt::Live;
    };
    // A new turn, so a new pass is legitimate again (rc3 D25). Cleared HERE
    // rather than when the participant passes, because the count has to survive
    // a turn that never ends — which is precisely the state it exists to bound.
    if let Some(bridge) = deps.bridge.as_ref() {
        bridge.clear_passes(&deps.session_id, &to.slug);
    }
    // Publish BEFORE the rows go out. The pump snapshots on its turn's first
    // event, and that event cannot happen until the agent has read something —
    // so writing first is what makes the snapshot see this turn's epoch rather
    // than the previous one. Release-ordered against the pump's Acquire load.
    //
    // **A holder with stdin but no epoch cell freezes the cycle**, and silently,
    // so it is worth a loud line. The pump reads its cell with `unwrap_or(0)`,
    // so such a participant completes at epoch 0, fails the `live` guard against
    // any epoch past the first, and is discarded — for ever. The ring stops on
    // it with nothing in the log to say why.
    //
    // This is a build-time obligation on whoever assembles [`SequencerDeps`],
    // exactly like the "file A's stdin under B's id" hazard `inputs` already
    // documents: the two maps must be keyed identically and nothing in the type
    // system says so. `session.rs` populates them in one pass, so this is
    // unreachable today — the warning exists because the next assembler is the
    // one that will not.
    match deps.epochs.get(&to.id) {
        Some(cell) => cell.store(*epoch, std::sync::atomic::Ordering::Release),
        None => warn!(
            session = %deps.session_id,
            participant_id = to.id,
            slug = %to.slug,
            epoch = *epoch,
            "sequencer: the participant taking the turn has no epoch cell; its \
             completions will carry 0, fail the guard, and the cycle will stop here"
        ),
    }
    deliver_backlog(deps, to, rx, MAX_TURN_BATCHES, deferred).await
}

/// A turn was dealt and can never complete — unwind it, and DECLARE.
///
/// Every stop is a HALT (2026-08-15), and this is the stop nobody chose. Before
/// this existed the three ways to reach it — a participant whose stdin closed
/// under the deal, a page its own input refused as out-of-session, a backlog
/// that could not be read — each warned into the log and returned, leaving the
/// holder set, its busy flag up, the halt slot empty and the input locked. The
/// health dot did flip for a dead pump (`watchdog.rs`), and that is all: the
/// idle nudge cannot cover a wedge that reads Busy, so the session sat working
/// on a turn that had already failed, and the only way out was Pause plus a
/// SIGKILL.
///
/// Order is deliberate: clear the flag, unwind the ring, NULL the column, then
/// declare. Everything before the declaration is local, so a failed write leaves
/// a session that is merely stopped rather than one that is stopped and lying.
async fn unwind_wedged_turn(
    deps: &SequencerDeps,
    holder: &mut Option<Participant>,
    epoch: &mut u64,
    reason: String,
) {
    let Some(stuck) = holder.clone() else {
        return;
    };
    if let Some(activity) = &deps.activity {
        activity.set_busy_slug(&stuck.slug, false);
    }
    // Clears the holder, bumps the epoch, and NULLs the column — leaving a dead
    // participant named there is how a wedged session reads as working.
    halt(deps, holder, epoch).await;
    warn!(
        session = %deps.session_id,
        participant_id = stuck.id,
        slug = %stuck.slug,
        reason = %reason,
        "sequencer: a dealt turn cannot complete; unwinding and declaring the halt"
    );
    // A session already closed is not one to declare a halt on — the agents are
    // being killed on purpose and their stdin closing IS the teardown. Read
    // rather than assumed, and fail-open: an unreadable row declares, because a
    // spurious banner on a dying session costs less than a wedge with none.
    let closed = matches!(
        deps.storage.get_session(&deps.session_id).await,
        Ok(Some(s)) if s.closed_at.is_some()
    );
    if closed {
        return;
    }
    if let Some(bridge) = deps.bridge.as_ref() {
        let _ = bridge
            .mark_awaiting_user(deps.session_id.to_string(), "system".to_string(), reason)
            .await;
    }
}

/// Post [`round_cap_notice`] into the channel and tell the UI it landed.
///
/// **The post is the contract; the notification is the nicety.** D7 asks for a
/// row, and a row is what a reopened session, a scrollback and an agent's next
/// backlog all read. [`SequencerDeps::bridge`] only decides whether the chat
/// updates without waiting for the next refetch, so its absence is not a
/// failure and is not logged as one.
///
/// A failed WRITE is logged, and loudly: the cycle is already halted by then,
/// so what is lost is the only on-screen account of why — the notification gap
/// the module doc names, which is exactly what D7 exists to close.
/// Post the all-passed notice and tell the UI it landed (rc3 **D27**).
///
/// Mirrors [`announce_round_cap`], and for the same reason: a cycle that yields
/// with nothing on screen is a session the user reads as hung. The difference is
/// what it SAYS — a round cap reports something that ran away, this reports that
/// the participants are waiting on them, which is an instruction rather than an
/// alarm.
async fn announce_all_passed(deps: &SequencerDeps) {
    match deps
        .storage
        .post_to_channel(
            Arc::clone(&deps.session_id),
            "system",
            None,
            MessageKind::SystemNotice.as_str(),
            "Every participant passed this round — nobody has anything to add \
             without you. The cycle has yielded; send a message to resume.",
            None,
        )
        .await
    {
        Ok(row) => {
            if let Some(bridge) = deps.bridge.as_ref() {
                bridge.notify_message_persisted(Arc::clone(&deps.session_id), row.message_id());
            }
        }
        Err(e) => warn!(
            session = %deps.session_id,
            error = %e,
            "sequencer: the cycle yielded on an all-pass lap but its notice was not \
             posted; the session has stopped with nothing on screen to say so"
        ),
    }
}

async fn announce_round_cap(deps: &SequencerDeps, laps: u32) {
    match deps
        .storage
        .post_to_channel(
            Arc::clone(&deps.session_id),
            // Host-authored, so `origin = 'system'` with a NULL participant
            // (0044) — the halt is nobody's turn output, and there is no
            // `system` roster row to attribute it to.
            "system",
            None,
            MessageKind::SystemNotice.as_str(),
            round_cap_notice(laps),
            None,
        )
        .await
    {
        Ok(row) => {
            if let Some(bridge) = deps.bridge.as_ref() {
                bridge.notify_message_persisted(Arc::clone(&deps.session_id), row.message_id());
            }
        }
        Err(e) => warn!(
            session = %deps.session_id,
            laps,
            error = %e,
            "sequencer: the round cap halted the cycle but its notice was not posted; the \
             session has yielded with nothing on screen to say so"
        ),
    }
}

/// Record how a turn ended, then answer: has the cycle arrived?
///
/// `true` means every active participant has declared done and the caller must
/// NOT step the ring — the turn is already cleared here. See "the halt is a
/// yield, not a stop" in the module doc for what that leaves observable.
///
/// **A [`TurnEnding::Passed`] can only ever answer `false` while the passing
/// participant is still in the active rotation**, because the write above
/// leaves that participant's vote at 0 and the query below wants every active
/// vote set. The one state where a pass returns `true` is a passer that is no
/// longer active — disabled mid-turn — with every remaining active voted done,
/// and that is a genuine arrival among the participants who are left. The query
/// is asked for all three endings rather than skipped for the pass so that case
/// has an answer at all.
///
/// **A storage failure answers `false`.** Neither error is a vote, and of the
/// two ways to be wrong the cycle continuing is the recoverable one: a spurious
/// extra lap costs a turn and the participant votes again, whereas a halt
/// nobody voted for parks the session waiting on a user who was never told they
/// are being waited on. Same instinct as [`Handover::Held`] — do not invent a
/// state out of a failure.
async fn halted_on_consensus(
    deps: &SequencerDeps,
    holder: &mut Option<Participant>,
    epoch: &mut u64,
    participant_id: i64,
    ending: TurnEnding,
) -> bool {
    let recorded = match ending {
        TurnEnding::Done => deps.storage.set_done_vote(participant_id, true).await,
        // Substantive output resets the tally for the WHOLE session, not just
        // for this participant. A done cast before this turn was a statement
        // about a session that no longer exists — see
        // `substantive_output_resets_the_tally` for the arithmetic that lets
        // one stale vote and one fresh one add up to an arrival nobody voted
        // for.
        TurnEnding::Spoke { .. } => deps.storage.clear_done_votes(&deps.session_id).await,
        // **A pass casts no vote, and RETRACTS its own.**
        //
        // Neither half is decoration. Casting nothing is what keeps a pass from
        // completing the tally, which is the whole point of the ending. The
        // retraction is what keeps a pass from completing it by ACCIDENT: the
        // vote column is per-participant and survives across rounds, so a
        // participant that voted done last round and passes this one would
        // still be counted as done, and the tally would complete on a turn
        // whose whole meaning was "not me". That path is reachable, not
        // theoretical — `a_pass_retracts_the_passers_own_stale_done_vote`
        // builds it out of two done votes and two passes.
        //
        // It retracts ONLY its own. Clearing the session (what `Spoke` does)
        // would make a pass behaviourally identical to the filler turn it
        // exists to replace: a participant with nothing to say would still be
        // wiping everyone else's converged votes, which is the second of the
        // two bad endings the design names.
        TurnEnding::Passed => deps.storage.set_done_vote(participant_id, false).await,
    };
    if let Err(e) = recorded {
        warn!(
            session = %deps.session_id,
            participant_id,
            ?ending,
            error = %e,
            "sequencer: done vote not recorded; continuing the cycle"
        );
        return false;
    }
    match deps.storage.all_active_voted_done(&deps.session_id).await {
        Ok(true) => {
            halt(deps, holder, epoch).await;
            debug!(
                session = %deps.session_id,
                participant_id,
                "sequencer: every active participant voted done; the cycle yields to the user"
            );
            true
        }
        Ok(false) => false,
        Err(e) => {
            warn!(
                session = %deps.session_id,
                error = %e,
                "sequencer: consensus read failed; continuing the cycle"
            );
            false
        }
    }
}

/// Take the turn out of flight. **This is the whole of a halt** — no event, no
/// marker, no vote touched.
///
/// One function for both reasons the cycle can yield ([`halted_on_consensus`]
/// and [`SequencerCommand::HaltDeclared`]) so they cannot drift apart. They
/// differ in what leads here, not in what a halted cycle IS.
///
/// Both lines are load-bearing, but for DIFFERENT reasons — remove either and a
/// test fails, and it is worth saying which, because only one of the two catches
/// is about the guard:
///
/// - **the holder clear shuts the other delivery door.**
///   [`SequencerCommand::ParticipantJoined`] delivers on arrival whenever the
///   holder is the participant that joined, so a halt that moved only the epoch
///   would still feed a respawn arriving mid-halt.
///   `cursors_do_not_advance_once_the_cycle_has_yielded` probes exactly that with a join;
///   drop `*holder = None` and it fails on the wire. `None` is also what
///   `next_active_participant` reads as "reset to the front", so the user
///   message that ends the halt starts the next cycle where a fresh session
///   would, tally clear included;
/// - **the epoch bump is belt and braces on the discard path.** The module doc
///   says so of the consensus halt and it is no different here: with the holder
///   gone, `TurnComplete`'s identity compare already rejects every later
///   completion unaided. What actually goes red without the bump is the epoch
///   NUMBERING — `a_parked_question_finishes_the_lap_then_halts` names the
///   epochs it completes, and a halt that skipped the bump mints one fewer, so
///   the test's last completion names a turn that was never handed out. A real
///   failure, but an arithmetic one; do not read it as the guard being pinned
///   from both sides.
async fn halt(deps: &SequencerDeps, holder: &mut Option<Participant>, epoch: &mut u64) {
    *holder = None;
    *epoch += 1;
    // **And the COLUMN, which is what everything outside this task reads.**
    // `halt` cleared the holder this task carries and nothing else, so after a
    // consensus / all-pass / cap / parked-question halt
    // `sessions.current_turn_participant_id` still named the last holder: a
    // YIELDED session read as one still working on that participant's turn.
    // Third instance of one pattern (2026-08-13: the capped-halt row, the close
    // epilogue's silent skip, this) — bot-hq keeps producing ending states
    // indistinguishable from a different one, and the column is the cheapest of
    // them to close. Best-effort by construction: `set_current_turn` warns and
    // moves on, so a failed write costs a UI hint, never a turn.
    deps.storage.set_current_turn(&deps.session_id, None).await;
}

/// Where a ring step landed.
enum Handover {
    /// The turn moved. `None` inside means nobody is active — "nobody to wake",
    /// and NOT a consensus test; see the module doc.
    To(Option<Participant>),
    /// The rotation could not be read, so the turn stays exactly where it was —
    /// holder and epoch both. A separate variant rather than echoing the
    /// current holder back, because the caller cannot tell those apart on the
    /// reset path: a user message passes `None` as the current holder, so
    /// "unchanged" and "reset to nobody" would be the same value.
    Held,
}

/// Who the user summoned, and where the rotation sits (rc3 D17).
///
/// **`anchor` is the last participant to hold a RING turn, and a summoned turn
/// does not move it.** That one rule is what makes a mention an insertion rather
/// than a reset: the ring always steps from the anchor, so after the summoned
/// participant has spoken the rotation carries on from exactly where it was
/// interrupted. Stepping from the HOLDER instead would work identically for
/// ordinary turns and silently reorder the ring around every summons — and for
/// an `on_mention` participant, which is not in the rotation at all, "one place
/// along from here" has no answer.
#[derive(Default)]
struct Summons {
    /// Participant ids the user named, in the order written. One turn each,
    /// popped as the ring hands them out.
    queue: VecDeque<i64>,
    /// The last ring turn's holder. `None` is the front of the rotation.
    anchor: Option<Participant>,
}

/// Hand the turn to the next participant the user summoned, if there is one.
///
/// Pops until it finds one that can actually take a turn, so a participant that
/// was disabled or removed between the mention and the turn is skipped rather
/// than freezing the cycle on an id with no process behind it. Returns `None`
/// when the queue is empty or holds nothing live, which is the caller's signal
/// to step the ring normally.
///
/// **A read failure drops the summons rather than holding the turn**, which is
/// the opposite of [`hand_over`]'s choice and deliberately so: `hand_over`
/// holding means "retry the step", and a retry is reachable there because the
/// same holder's next completion re-attempts it. Here there is nothing to
/// retry from — the queue entry is already popped — and holding the turn for a
/// summons that cannot be read would strand the session on a transient error.
/// The ring still moves; the user's message still lands, one turn later, on
/// whoever the rotation reaches.
async fn hand_to_summoned(deps: &SequencerDeps, queue: &mut VecDeque<i64>) -> Option<Participant> {
    while let Some(id) = queue.pop_front() {
        match deps.storage.participant_by_id(id).await {
            Ok(Some(p)) if *p.session_id == *deps.session_id && p.enabled => {
                debug!(
                    session = %deps.session_id,
                    participant_id = p.id,
                    slug = %p.slug,
                    queued = queue.len(),
                    "sequencer: the user summoned this participant; it takes the next turn"
                );
                hand_turn_to(deps, Some(&p)).await;
                return Some(p);
            }
            // Every remaining case is "the summons cannot be honoured": no such
            // row, another session's row (which delivering into would wire one
            // session's text to another's process), a disabled row, or a read
            // that failed. None of them is worth stopping the session over.
            other => {
                warn!(
                    session = %deps.session_id,
                    participant_id = id,
                    reason = match other {
                        Ok(None) => "no such participant",
                        Ok(Some(_)) => "not a live participant of this session",
                        Err(_) => "the roster could not be read",
                    },
                    "sequencer: a summons was dropped"
                );
            }
        }
    }
    None
}

/// Step the ring past `current`. Delivery is the caller's next move, not this
/// function's, so a failed step cannot half-deliver.
///
/// `current == None` resets to the front of the rotation, which is what a user
/// message with nobody named does.
async fn hand_over(deps: &SequencerDeps, current: Option<&Participant>) -> Handover {
    let next = match deps
        .storage
        .next_active_participant(&deps.session_id, current)
        .await
    {
        Ok(next) => next,
        // The ring is a roster read, so a failure here is a storage problem,
        // not an empty rotation. Holding the turn where it is keeps the two
        // apart: a later completion from the same holder retries the step,
        // whereas reporting "nobody is active" would reset the cycle on the
        // next user message. `Held` is what makes that retry reachable — see
        // `advance_turn`, which leaves the epoch alone for it.
        Err(e) => {
            warn!(
                session = %deps.session_id,
                error = %e,
                "sequencer: ring read failed; holding the turn where it is"
            );
            return Handover::Held;
        }
    };
    if next.is_none() {
        // Nobody active. NOT a consensus test — see the module doc; consensus
        // is `all_active_voted_done`, asked in `halted_on_consensus` before the
        // caller ever gets here.
        debug!(
            session = %deps.session_id,
            "sequencer: no active participant to hand the turn to"
        );
    }
    // COMPUTE only — the deal (`hand_turn_to`: the holder column + the busy
    // mark) moved to `start_turn` (s-f6a441ff). Dealing here meant the mark
    // landed BEFORE the caller's wrap checks: an all-pass yield or a round cap
    // then refused the turn it had already marked, and the flag — set for a
    // turn no pump would ever run, so no pump would ever clear — locked the
    // input under a "send a message to resume" notice until the user
    // force-paused. `an_all_pass_yield_leaves_the_input_open` reproduces it.
    Handover::To(next)
}

/// Hand the turn to `next` — record WHO holds it, and mark it WORKING.
///
/// **One function because it is one event.** Recording the holder (rc3 D19b) and
/// marking it busy are the same fact stated to two readers: the UI's roster, and
/// the chat input's lock. They were separate once and only one of them existed —
/// `set_current_turn` was written and the busy flag was not, which is the whole
/// of the input-unlocks-mid-cycle defect. The CL's remedy for a two-halves join
/// is to extract it somewhere a test can call, and this is that place: both call
/// sites (the summons, the ring step) are now one line each, so a future edit
/// cannot move one half without the other.
///
/// `None` marks nobody, deliberately — but **be exact about when it is
/// reached**, because an earlier draft of this comment was not. It said `None`
/// is "a HALT — consensus, a parked question, the round cap", and that is
/// FALSE: [`halt`] only clears the local holder and bumps the epoch, it never
/// calls this. The one reachable `None` is [`hand_over`]'s nobody-active arm.
///
/// The consequence is a live gap this function does not close: after a real
/// halt `sessions.current_turn_participant_id` still names the last holder, so
/// a yielded session reads as working in the UI. Same shape as D7's capped-halt
/// row and item 4A's silent skip arms — an ending state indistinguishable from
/// a different one. Closing it means threading `deps` into `halt` and calling
/// this with `None` there; noted rather than done, because it is the same
/// "a yield must say it yielded" change the idle-nudge work needs.
///
/// The INPUT LOCK is unaffected either way, and that is worth separating: the
/// pump clears each participant's busy flag at its own turn end, so after a halt
/// every flag is clear, the session derives `Idle`, and the input unlocks. That
/// is the unlock condition and it needs no code of its own —
/// `a_halt_leaves_nobody_busy` pins it. (True only because this function is the
/// ONE busy-true writer: `AppState::broadcast` used to pre-mark every agent, and
/// a pre-mark on a participant a halt stopped the ring before reaching had no
/// turn end to clear it — s-ff729daa, the input locked under the HALT banner.
/// `broadcast_marks_nobody_busy` in core::state pins that loop deleted.)
///
/// **This closes a wedge, not just a cosmetic lock.** A message typed while a
/// turn is in flight lands on the holder's stdin mid-turn; the pump binds its
/// epoch at turn-OPEN, so the completion that follows carries the pre-reset
/// epoch and is discarded — and the discard arm does not step the ring. The
/// pump has cleared its state, the cursor is already past the message, and the
/// loop waits in `rx.recv()` with no timeout, so nothing can produce the epoch
/// the ring is now waiting on. The only exit is another user message, which is
/// the same action that caused it. Holding the lock for the whole cycle makes
/// that landing unreachable by construction.
///
/// What this does NOT do is clear the PREVIOUS holder. The pump owns that, at
/// its own turn end, and it stays there: moving the clear here would close the
/// sub-second gap between a completion and the next handover, and buy a wedge
/// for it — a completion that never arrives would leave the flag set and the
/// input locked forever, which `SessionHandle::send_to_all`'s doc records as a
/// hazard already paid for once. That gap CAN be typed into, and harmlessly: no
/// turn is in flight there, so the message takes the designed fresh-turn path
/// rather than landing mid-turn. A permanently locked input is worse than a
/// window whose worst case is the intended behaviour.
async fn hand_turn_to(deps: &SequencerDeps, next: Option<&Participant>) {
    // Say who holds it. The ring has always known; the column that exists to
    // report it was never written, so the UI could not tell a participant
    // waiting its turn from one that had died.
    deps.storage
        .set_current_turn(&deps.session_id, next.map(|p| p.id))
        .await;
    if let (Some(activity), Some(p)) = (&deps.activity, next) {
        activity.set_busy_slug(&p.slug, true);
    }
    // A deal used to be SILENT in the log — the s-f6a441ff dissection spent an
    // hour proving a busy flag near a yield could not have come from here,
    // because nothing recorded whether it had. Every deal says so now.
    if let Some(p) = next {
        debug!(
            session = %deps.session_id,
            participant_id = p.id,
            slug = %p.slug,
            "sequencer: turn dealt"
        );
    }
}

/// What a deal left behind: whether the turn it fed can still end.
///
/// The ring steps on a completion, and a completion can only come from a
/// participant that received something. So every way a drain can end without
/// rows going out is a turn that is dealt, marked busy, and unable to complete —
/// and until this type existed each of those paths `warn!`ed and returned,
/// leaving the holder set, the busy flag up, no halt slot filled and nothing to
/// clear either. The only exit was a Pause and a SIGKILL.
#[derive(Debug, PartialEq, Eq)]
enum Dealt {
    /// Rows went out, or the drain stopped for a reason that owns the next step
    /// itself (a supersede, a declared halt, a pause, session end).
    Live,
    /// The turn can never complete. The string is the halt reason the user
    /// reads — every stop is a HALT, including the stop nobody chose.
    CannotComplete(String),
}

/// Why a drain stopped before the end of its page.
enum Stop {
    /// A user message arrived: the ring is about to reset, so the turn being
    /// fed is superseded. Already pushed onto the deferred queue.
    Superseded,
    /// A halt was declared: the cycle is about to halt, so there is no turn
    /// left to feed. Already pushed onto the deferred queue.
    Halted,
    /// The cycle was paused: the participant being fed is the one the user just
    /// stopped, so there is no-one left to feed either. Pushed onto the FRONT of
    /// the deferred queue rather than the back — see the drain's own arm.
    Paused,
    /// The control channel closed — session end.
    SessionEnd,
    /// `deliver_batch` returned `false`.
    Unreachable,
}

/// Hand `to` everything it has not read, and record what it got.
///
/// Drains rather than delivering one page — see "how far a turn reads" in the
/// module doc.
/// `max_batches` is [`MAX_TURN_BATCHES`] on every production path; it is a
/// parameter so the cap's own behaviour can be exercised without a 6,401-row
/// fixture. The caller that does that is
/// `the_batch_cap_hands_over_with_the_remainder_still_past_the_cursor`, which
/// calls this function directly — the loop above has no way to pass anything
/// but the constant.
///
/// **Each page is ONE write.** See "one turn, one write" in the module doc for
/// why, and for what that costs the `Stop` arms below.
///
/// Commands that arrive mid-drain go onto `deferred` — see "the drain does not
/// hold the command channel shut" in the module doc for which of them also END
/// the drain and which are merely set aside.
async fn deliver_backlog(
    deps: &SequencerDeps,
    to: &Participant,
    rx: &mut mpsc::Receiver<SequencerCommand>,
    max_batches: usize,
    deferred: &mut VecDeque<SequencerCommand>,
) -> Dealt {
    let Some(input) = deps.inputs.get(&to.id) else {
        warn!(
            session = %deps.session_id,
            participant_id = to.id,
            slug = %to.slug,
            "sequencer: the participant holding the turn has no stdin; delivering nothing \
             and the cycle stops here until one arrives"
        );
        // Deliberately NOT a wedge: this participant has no live process yet,
        // and `SequencerCommand::ParticipantJoined` is the documented way the
        // cycle un-freezes when one arrives. Halting here would turn a
        // recoverable wait into the user's problem.
        return Dealt::Live;
    };
    for _ in 0..max_batches {
        let page = match deps.storage.unread_for_participant(to.id).await {
            Ok(page) => page,
            Err(e) => {
                warn!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    error = %e,
                    "sequencer: backlog read failed"
                );
                // Same wedge class as an unreachable stdin, one layer down: the
                // turn is dealt and marked, and nothing it could complete on
                // ever reached it.
                return Dealt::CannotComplete(format!(
                    "{}'s turn could not be read from the channel ({e}). The turn was \
                     dealt but nothing reached them, so nothing can end it. Send a \
                     message to deal a fresh turn.",
                    to.slug
                ));
            }
        };
        if page.rows.is_empty() {
            return Dealt::Live;
        }
        let total = page.rows.len();
        // `from_row` is what makes a row READ BACK deliverable: receipts are
        // otherwise minted only by the INSERT, and every row written before a
        // restart is only ever available this way.
        //
        // Built for the WHOLE page up front, because the page is what goes out
        // — see "one turn, one write" in the module doc. That holds a page of
        // cloned bodies alongside `page.rows` rather than one at a time; the
        // total clone count is what it always was, and the page is already
        // bounded at `UNREAD_BATCH_LIMIT`.
        let receipts: Vec<PersistedMessage> =
            page.rows.iter().map(PersistedMessage::from_row).collect();
        let mut landed: Vec<(i64, Option<&str>)> = Vec::with_capacity(total);
        let mut stop: Option<Stop> = None;
        // ONE write for the whole page, retried until it lands or a command
        // takes precedence — see "one turn, one write" in the module doc.
        loop {
            tokio::select! {
                // Commands first. Both futures here are cancel-safe —
                // `recv` by documentation, and a dropped `Sender::send`
                // enqueues nothing — so the losing branch costs at most a
                // re-attempt of the same page, never a half-written one.
                // Biased so a command already waiting always wins: the
                // whole point is that a full stdin cannot hide it.
                biased;
                cmd = rx.recv() => match cmd {
                    Some(cmd @ SequencerCommand::UserMessage { .. }) => {
                        deferred.push_back(cmd);
                        stop = Some(Stop::Superseded);
                        break;
                    }
                    // Deferred like the user message, and for the same
                    // reason: the ACT is the loop's, not this function's.
                    // Ending the drain here is what makes the yield
                    // immediate; see the module doc for what stopping costs
                    // against what finishing would.
                    //
                    // **Only when it is THIS participant that parked** (rc3
                    // D22). A park used to stop the cycle wherever it stood,
                    // so cutting any drain short was right. It now ends only
                    // the ASKER's turn — so a park naming somebody else
                    // leaves this turn live, and stopping its drain would
                    // hand it a partial backlog for no reason. The command is
                    // still deferred either way; the loop decides.
                    Some(cmd @ SequencerCommand::HaltDeclared { .. }) => {
                        let mine = matches!(
                            cmd,
                            SequencerCommand::HaltDeclared { participant_id }
                                if participant_id == Some(to.id)
                        );
                        deferred.push_back(cmd);
                        if mine {
                            stop = Some(Stop::Halted);
                            break;
                        }
                    }
                    // Ends the drain like the two above, and for the same
                    // reason the park does: the participant being fed is the
                    // one the user just stopped, so every further row goes
                    // into a buffer in front of a process that is not
                    // reading, and `deliver` PARKS when it fills — until the
                    // user unpauses. A pause that waits out the user before
                    // taking effect is not a pause.
                    //
                    // **`push_front`, unlike every other deferral here, and
                    // that asymmetry is the decision.** Arrival order is the
                    // rule everywhere else in this loop, and it costs a
                    // parked question at most one extra wake (the module doc
                    // prices it). It costs a pause the whole semantic: a
                    // `TurnComplete` set aside a moment ago would be
                    // dispatched first, step the ring, and start a fresh turn
                    // in a session the user has stopped — and unlike the
                    // park's extra wake there is no user answer coming to end
                    // it, only a Resume the user has not sent yet. So the
                    // pause takes effect where it was READ rather than where
                    // it would have been dispatched, and everything the drain
                    // had merely set aside waits behind it.
                    Some(cmd @ SequencerCommand::Pause) => {
                        deferred.push_front(cmd);
                        stop = Some(Stop::Paused);
                        break;
                    }
                    // Set aside and re-attempt this page. Deferring rather
                    // than acting is what keeps the drain-before-handover
                    // rule true.
                    Some(cmd) => deferred.push_back(cmd),
                    None => {
                        stop = Some(Stop::SessionEnd);
                        break;
                    }
                },
                landed_ok = input.deliver_batch(&receipts) => {
                    if !landed_ok {
                        stop = Some(Stop::Unreachable);
                        break;
                    }
                    // `None` = delivered. Nothing on the turn path
                    // withholds; see the module doc.
                    landed.extend(receipts.iter().map(|m| (m.message_id(), None)));
                    break;
                }
            }
        }
        // The page either landed whole or not at all, and this commits whichever
        // it was. It moves the cursor to the highest id passed here and never
        // rewinds, so recording a row the transport did not take would lose it
        // forever — which is why nothing is recorded when the write is skipped.
        //
        // **This used to be a PREFIX**, because delivery was a row at a time and
        // a command could cut between two of them. One write per page makes it
        // all-or-nothing, and that is the intended trade: a stopped drain leaves
        // the page wholly past the cursor instead of half-read, so the turn that
        // picks it up gets the backlog entire.
        if let Err(e) = deps.storage.commit_delivery(to.id, &landed).await {
            warn!(
                session = %deps.session_id,
                participant_id = to.id,
                error = %e,
                "sequencer: delivery not recorded; the batch will be re-offered"
            );
            // The rows DID go out — the agent has them and will report — so the
            // turn can still end. Only the bookkeeping failed.
            return Dealt::Live;
        }
        match stop {
            None => {}
            Some(Stop::Superseded) => {
                debug!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    delivered = landed.len(),
                    of = total,
                    "sequencer: a user message superseded this turn mid-drain"
                );
                return Dealt::Live;
            }
            Some(Stop::Halted) => {
                debug!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    delivered = landed.len(),
                    of = total,
                    "sequencer: a parked question halted this turn mid-drain"
                );
                return Dealt::Live;
            }
            Some(Stop::Paused) => {
                // Costs nothing but the rows read and not delivered, exactly as
                // the two above: the commit just made recorded nothing this page,
                // so the whole of it is still past this participant's cursor and
                // `Resume` re-drains from there.
                debug!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    delivered = landed.len(),
                    of = total,
                    "sequencer: a pause stopped this turn's delivery mid-drain"
                );
                return Dealt::Live;
            }
            Some(Stop::SessionEnd) => {
                debug!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    delivered = landed.len(),
                    of = total,
                    "sequencer: session ended mid-drain"
                );
                return Dealt::Live;
            }
            Some(Stop::Unreachable) => {
                // `deliver_batch` returns `false` for two unrelated reasons —
                // a dead input pump, and a receipt from another session — and
                // this warning named only the first for a while, so a routing
                // bug would have read as a dead pipe. `is_closed` separates
                // them.
                //
                // It is a second look, not the same observation: the channel
                // can close between the refusal and this check, which would
                // report a scope refusal as a closed pipe. That direction is
                // harmless; the reverse cannot happen, because a closed sender
                // never re-opens.
                //
                // Either way the page did not go out, so the turn this drain
                // was feeding cannot end: the participant has nothing to answer
                // and will never report. The caller unwinds and declares.
                if input.is_closed() {
                    warn!(
                        session = %deps.session_id,
                        participant_id = to.id,
                        slug = %to.slug,
                        delivered = landed.len(),
                        of = total,
                        "sequencer: stdin closed before this page went out; it stays past the cursor"
                    );
                    return Dealt::CannotComplete(format!(
                        "{} is unreachable — their process is gone, so the turn they \
                         were dealt can never end. Send a message to respawn them.",
                        to.slug
                    ));
                }
                warn!(
                    session = %deps.session_id,
                    participant_id = to.id,
                    slug = %to.slug,
                    delivered = landed.len(),
                    of = total,
                    "sequencer: a page was refused with stdin still open — a \
                     receipt in it is out of this participant's session scope"
                );
                return Dealt::CannotComplete(format!(
                    "{}'s turn was refused by their own input as out-of-session — a \
                     routing fault, not a stopped agent. Nothing was delivered, so \
                     nothing can end the turn. Send a message to deal a fresh one.",
                    to.slug
                ));
            }
        }
        if !page.more {
            return Dealt::Live;
        }
    }
    warn!(
        session = %deps.session_id,
        participant_id = to.id,
        batches = max_batches,
        "sequencer: backlog still not drained at the batch cap; the rest waits for the next turn"
    );
    // Rows went out — the cap is a hand-over, not a wedge.
    Dealt::Live
}

// ---------------------------------------------------------------------------
// Jaccard helpers — moved VERBATIM from `core::router` (2026-08-10).
//
// The router inventory marks these PRESERVED: spin detection (a later task in
// this file) reuses them unchanged, so they land here ahead of the caller
// rather than being rewritten next to it. `core::router` still owns the
// convergence breaker that calls them and imports them from here until that
// path is deleted; nothing about their behaviour changed in the move.
// ---------------------------------------------------------------------------

/// Tokenize a forward body for convergence comparison: split on whitespace, trim
/// each token of leading/trailing non-alphanumerics, lowercase, drop empties — so
/// "OK.", "OK", "ok" all reduce to {ok}.
pub(super) fn token_set(s: &str) -> HashSet<String> {
    s.split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Token-set Jaccard similarity — the shape-based convergence signal (no length
/// threshold, no keyword/prefix list). Edge: BOTH sets empty (pure punctuation /
/// emoji like "." or "🤝", the canonical s-e4fc25 volley) → 1.0, so convergence
/// catches it fast rather than deferring to the hard-cap. One empty, one not →
/// 0.0. Two DISTINCT substantive messages always carry alphanumeric tokens, so
/// they can never collide at 1.0 via the both-empty path.
pub(super) fn jaccard_from_sets(sa: &HashSet<String>, sb: &HashSet<String>) -> f64 {
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(sb).count();
    let union = sa.union(sb).count();
    if union == 0 {
        1.0
    } else {
        inter as f64 / union as f64
    }
}

/// String-level convenience wrapper (tokenizes BOTH sides). Test-only: the hot
/// path keeps the previous forward's token set and calls `jaccard_from_sets`
/// directly, so it never re-tokenizes the previous body.
#[cfg(test)]
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    jaccard_from_sets(&token_set(a), &token_set(b))
}

// --- spin detection --------------------------------------------------------
// Router inventory #2 (`single_stream_cross_agent_same_phrase_breaks_fast`),
// guarded by #3 (`varied_substantive_cross_agent_never_breaks`). Reframed for
// the ring: cross-agent echo is impossible when exactly one participant holds
// the turn, but a participant repeating ITSELF across rounds is not.

/// Jaccard bar at or above which a turn counts as a repeat of the same
/// participant's previous one.
///
/// **Was deliberately split from the router's convergence threshold**, which
/// task 14 deleted along with `core::router`. They measured different things:
/// that one compared consecutive forwards across a single interleaved stream of
/// two agents, this one compares one participant against its own last turn.
/// They started equal because the number
/// was calibrated on the same material, not because they are one knob.
const SPIN_SIMILARITY_THRESHOLD: f64 = 0.85;

/// Consecutive self-similar turns before a participant counts as spinning.
///
/// Mirrors the router's `VOLLEY_SIMILAR_BREAK = 2`: a streak of 2 is three
/// similar turns in a row, because the first reading has nothing to compare
/// against and sets a baseline without scoring.
const SPIN_BREAK_STREAK: u32 = 2;

/// Prose rows read per completion. `insert_message` fires per chunk, so one turn
/// is many rows; this bounds the read without bounding the session.
const SPIN_TEXT_ROWS: i64 = 64;

/// One participant's repetition state.
#[derive(Default)]
struct SpinState {
    /// Token set of the prose read at the last comparison. `None` until the
    /// first reading, which is NOT the same as an empty set: [`jaccard_from_sets`]
    /// scores two empty sets as 1.0 — the punctuation-only volley it exists to
    /// catch — so a first turn of "." compared against an empty baseline would
    /// score a perfect match against nothing.
    last: Option<HashSet<String>>,
    /// Consecutive self-similar turns. **Not reset when it trips**, matching the
    /// router: sustained repetition stays tripped until the content changes or a
    /// user message clears the state.
    streak: u32,
    /// Highest message id already folded into `last`.
    watermark: i64,
}

/// Has this participant just repeated itself? Records the reading either way.
///
/// Reads storage because [`SequencerCommand::TurnComplete`] carries no text —
/// only who finished and which turn. The prose is already a row by then (task
/// 2's guarantee), so the canonical record is both the cheaper source and the
/// one a malformed sender cannot lie about.
///
/// **A turn that produced no prose leaves the streak standing** rather than
/// resetting it — router inventory #13, subtle enough to have earned its own
/// row: a reset consumed by a turn that produced nothing would silence the
/// detector on the turn after it.
///
/// **A failed read answers `false` and does not advance the watermark**, so the
/// next comparison re-reads those rows alongside the new ones. Same instinct as
/// every other storage fault on this path: a missed detection costs a lap, an
/// invented one halts a session on a transient error.
async fn spinning(
    deps: &SequencerDeps,
    spin: &mut HashMap<i64, SpinState>,
    participant_id: i64,
) -> bool {
    let state = spin.entry(participant_id).or_default();
    let read = deps
        .storage
        .participant_text_since(participant_id, state.watermark, SPIN_TEXT_ROWS)
        .await;
    let (text, newest) = match read {
        Ok(Some(found)) => found,
        Ok(None) => return false,
        Err(e) => {
            warn!(
                session = %deps.session_id,
                participant_id,
                error = %e,
                "sequencer: a participant's prose could not be read; spin not evaluated this turn"
            );
            return false;
        }
    };
    state.watermark = newest;
    let tokens = token_set(&text);
    match state.last.as_ref() {
        Some(prev) if jaccard_from_sets(prev, &tokens) >= SPIN_SIMILARITY_THRESHOLD => {
            state.streak += 1;
        }
        Some(_) => state.streak = 0,
        // First reading: a baseline, not a score.
        None => {}
    }
    state.last = Some(tokens);
    state.streak >= SPIN_BREAK_STREAK
}

// --- what a completed turn means -------------------------------------------
// Router inventory #8, #9, #10 and #11 as one group. In the router these were
// four branches over "do I forward this?"; in a ring there is nothing to
// forward — the turn's text is already a row — so the same four branches decide
// the VOTE instead: does this ending mean "I have nothing left to add"?

/// Longest acked turn still read as a bare acknowledgement.
///
/// **Mints its own rather than importing `router::PEER_ACK_MAX_SUPPRESSED_LEN`,**
/// which is private to a module task 14 deletes — same reasoning as
/// [`SPIN_SIMILARITY_THRESHOLD`]. The VALUE is the router's on purpose: the
/// inventory says the length proxy stays as the floor, so this is a move, not a
/// retune.
const PEER_ACK_MAX_SUPPRESSED_LEN: usize = 200;

/// What a completed turn means — the three ways a turn can end.
///
/// **Three variants rather than two booleans, because two of the three are
/// mutually exclusive claims about the tally** and a pair of flags would let a
/// sender assert both. `done` says "I have nothing left to do" and COUNTS
/// toward the consensus halt; `Passed` says "not me this round" and counts
/// toward nothing. A struct carrying both could be handed to the loop with each
/// set, and the loop would have to invent a precedence rule at the point where
/// it is least visible. Here the precedence is decided once, in
/// [`turn_ending`], and the result cannot express the contradiction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnding {
    /// The turn carried substantive text. Steps the ring and RESETS the tally
    /// for the whole session — see [`halted_on_consensus`].
    Spoke {
        /// A `peer_ack` was overridden because the turn carried substantive
        /// text. The row records it via `Envelope::with_peer_ack_override`, so
        /// the override is something the user can see rather than a sentence
        /// spliced onto the body. **No ring behaviour reads it** — it decorates
        /// the row and nothing else, which is why it rides the variant that is
        /// otherwise indistinguishable from a plain substantive ending.
        peer_ack_override: bool,
    },
    /// Nothing left to do — the consensus vote.
    Done,
    /// **PASS: this participant declines the turn** (design §1, "a participant
    /// may PASS rather than burn a turn").
    ///
    /// Not [`Done`](Self::Done), and the difference is the whole reason the
    /// variant exists. Before it, a participant with nothing to say had two
    /// endings available and both were wrong: vote done — which feeds the tally
    /// and walks the session toward an arrival nobody actually reached — or
    /// emit filler. The filler cost is on the record in `PROGRESS.md`
    /// (2026-08-04): a reviewer woken with nothing attached answered `"Old plan
    /// — holding for Brian's plan"`, 40 chars, and each such turn burned a slot
    /// of the volley budget that was already being exhausted before substantive
    /// reviews could get through. A pass is the third ending: it steps the
    /// ring, casts no vote, and cannot complete consensus.
    ///
    /// Not `Spoke` either: a pass is by construction not substantive, so it
    /// must not reset a tally the way real output does. Making it reset would
    /// leave it behaviourally identical to the filler it replaces.
    Passed,
}

impl TurnEnding {
    /// The ordinary ending — substantive output with no ack to override.
    ///
    /// A named constant because it is what an errored turn ends as and what
    /// nearly every test sends, and `Spoke { peer_ack_override: false }` puts
    /// the one field no ring behaviour reads in front of the one it does.
    pub const SPOKE: TurnEnding = TurnEnding::Spoke { peer_ack_override: false };

    /// Did this turn produce substantive output?
    ///
    /// **One caller — spin detection** — so this is naming, not sharing. The
    /// tally reset in [`halted_on_consensus`] answers the same question but
    /// does it by matching all three arms, because it has a different action
    /// for each; folding it into this predicate would leave the pass and the
    /// done vote sharing a branch they do not share.
    ///
    /// A method rather than an inline `matches!` because the call site reads
    /// `if ending.is_substantive() && spinning(..)`, and the whole point of
    /// that line is WHICH endings are judged for repetition. Written inline it
    /// says which variant is excluded, which is the same fact with the reason
    /// removed.
    fn is_substantive(self) -> bool {
        matches!(self, TurnEnding::Spoke { .. })
    }
}

/// Derive a turn's ending from the `peer_ack` signals it carried.
///
/// **Here rather than on the command, and called by the sender.**
/// [`SequencerCommand::TurnComplete`] argues its own case for carrying exactly
/// `done` and not the signals behind it; widening it to `(peer_ack,
/// peer_ack_final, body)` would move this decision into the loop and give a
/// malformed sender three fields to disagree about instead of one. So the
/// semantics live in this module — which owns what a turn means — and the pump
/// that carries the epoch out calls this on the way back in. That wiring is
/// task 14/15's, the same unsolved round trip `done` already rides.
///
/// The four inventory rows, in the order the router evaluated them:
///
/// - **#8** a bare `peer_ack` is a done vote. In the router it suppressed the
///   forward and skipped the counters; here it declines to wake the next
///   participant by voting, which is the same intent expressed as consensus.
/// - **#9** an acked turn over [`PEER_ACK_MAX_SUPPRESSED_LEN`] is NOT a vote —
///   it posts, tagged. **This guard exists because four full reviews were
///   destroyed** by an agent posting its verdict and calling `peer_ack` in the
///   same turn: the tool name reads as "acknowledge my peer", the effect was
///   "throw my turn away".
/// - **#10** `final: true` votes regardless of length — the agent ASSERTING
///   this is its closing turn outranks the length proxy.
/// - **#11** the inverse: substantive and not final still posts.
///
/// Safe in a way the router's version had to argue for: suppression there
/// skipped the wake but never the record, and here there is no suppression at
/// all. The text is a row before this is ever consulted.
///
/// # The pass (design §1)
///
/// `passed` is the `pass_turn` tool, observed by the pump exactly as `peer_ack`
/// is. It is folded in HERE rather than decided in the pump for the reason the
/// four rows above are: a turn ends one way, and the one place that says which
/// way is this function.
///
/// Two rules, and both are decisions rather than mechanics:
///
/// - **A pass over the length floor is OVERRIDDEN**, the same way an ack is
///   (#9). The failure it prevents is arithmetic, not editorial: `Passed` is
///   the one ending that does NOT reset the tally, so a substantive turn read
///   as a pass would carry a done vote cast before it straight over the top of
///   real output — the exact "one stale vote and one fresh one add up to an
///   arrival nobody voted for" that `substantive_output_resets_the_tally`
///   exists to stop. **The overridden pass carries no tag** (rc3 decisions,
///   locked): #9's ack tag says "your ack did not land"; there is no equivalent
///   claim to make here, because the row IS the turn's own text.
/// - **A pass outranks an ack when a turn calls both.** They disagree — the ack
///   casts a done vote, the pass casts nothing — so one has to win, and the
///   pass does. Of the two ways to be wrong, an extra lap costs a turn and the
///   participant votes again, whereas a halt nobody voted for parks the session
///   waiting on a user who was never told they are being waited on. Same
///   instinct as [`halted_on_consensus`]'s storage-failure arm.
///
/// The pass gate is a PREFIX guarded by `passed`, so with `passed == false`
/// this function is byte-for-byte the ladder it was: #8-#11 are untouched, not
/// merely re-verified.
pub fn turn_ending(peer_ack: bool, peer_ack_final: bool, passed: bool, body: &str) -> TurnEnding {
    // Trimmed before measuring, like the router — whitespace must not push a
    // content-free turn over the floor. One reading, shared by both ladders, so
    // a pass and an ack cannot disagree about what "content-free" means.
    let content_free = body.trim().len() <= PEER_ACK_MAX_SUPPRESSED_LEN;
    if passed && content_free {
        return TurnEnding::Passed;
    }
    if !peer_ack {
        return TurnEnding::SPOKE;
    }
    if peer_ack_final || content_free {
        return TurnEnding::Done;
    }
    TurnEnding::Spoke { peer_ack_override: true }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::OutgoingUserMessage;
    use crate::policy::session_policy::write_session_policy;
    use crate::policy::{Policy, SessionPolicy};
    use crate::storage::UNREAD_BATCH_LIMIT;
    use std::time::Duration;
    use tempfile::{tempdir, TempDir};
    use tokio::task::JoinHandle;

    /// Every await in this file carries this deadline.
    ///
    /// Not stylistic. A bare `.await` on an event that stopped arriving turns a
    /// deleted emit into a hung test instead of a failing one — earlier in this
    /// batch exactly that shape cost seven minutes per run before anything was
    /// printed. Two seconds is orders of magnitude above what an in-process
    /// `recv()` needs.
    const DEADLINE: Duration = Duration::from_secs(2);

    /// How long a negative assertion waits before believing the silence.
    /// Short — it is paid on every `quiet()` call — but well past the
    /// in-process delivery these tests measure in microseconds.
    const QUIET: Duration = Duration::from_millis(250);

    /// The three endings, named for the command literals below.
    ///
    /// Aliases rather than the paths themselves because a `TurnComplete` is
    /// already three fields on one line in ~50 tests, and
    /// `TurnEnding::Spoke { peer_ack_override: false }` would put the field NO
    /// ring behaviour reads (it decorates the row) in front of the one every
    /// one of those tests is about. `PASSED` is spelled out at its own call
    /// sites for the same reason in reverse — it is the subject there.
    const SPOKE: TurnEnding = TurnEnding::SPOKE;
    const DONE: TurnEnding = TurnEnding::Done;
    const PASSED: TurnEnding = TurnEnding::Passed;

    /// The default stubbed stdin buffer.
    ///
    /// Sized above the largest backlog these tests post (201 rows) so that a
    /// test about ring order is not also a test about back-pressure. That is a
    /// convenience, not an assumption the loop makes: production stdin is 64
    /// slots, and a drain that outruns it PARKS inside `deliver`.
    ///
    /// [`ring_sized`] is how a test opts out of the convenience.
    /// `a_backlog_larger_than_the_stdin_buffer_lands_in_full` is the one that
    /// does, and it is the only coverage of the parking path: every other test
    /// here runs with more slots than it posts rows, so none of them would
    /// notice a drain that dropped a row instead of waiting for a slot.
    const STDIN_CAPACITY: usize = 512;

    /// One stubbed participant: its roster id and the stdin a test reads.
    struct Seat {
        id: i64,
        rx: mpsc::Receiver<OutgoingUserMessage>,
    }

    /// A wire with its `[speaker]` prefix removed.
    ///
    /// **Every assertion in this file is about ROUTING** — who was handed which
    /// rows, in what order — and names the rows by content only to identify
    /// them. The speaker (rc3 D23) is a property of the WIRE FORMAT, and
    /// threading it through forty-five routing assertions would state it forty-
    /// five times while testing it nowhere: a run that dropped the prefix from
    /// exactly one path would still redden all of them, and a run that put the
    /// WRONG name on would redden none, because the expectation would have been
    /// written from the observed output.
    ///
    /// So it is stripped here and pinned where it is the subject:
    /// `a_delivered_row_says_who_wrote_it` below (end to end, including a peer's
    /// slug), `the_wire_is_the_row_plus_its_envelope` in `agents::spawn` (what
    /// reaches stdin), and `render_wire`'s own tests in `storage` (the format).
    ///
    /// Only a LEADING `[word] ` goes; `[PHASE: Apply]` survives, because the
    /// speaker is always first and a phase tag has a space inside the brackets.
    fn unlabelled(wire: String) -> String {
        let Some(rest) = wire.strip_prefix('[') else {
            return wire;
        };
        match rest.split_once("] ") {
            // A speaker is one word. Anything else is envelope decoration that
            // happens to start with a bracket, and must be left alone.
            Some((speaker, body)) if !speaker.contains(' ') => body.to_string(),
            _ => wire,
        }
    }

    /// The ROWS inside one wire.
    ///
    /// A turn's page reaches stdin as a single write with
    /// [`WIRE_JOIN`](crate::storage::WIRE_JOIN) between the rows, so a wire and
    /// a row stopped being the same thing. Every assertion in this file is about
    /// ROUTING — who was handed which rows, in what order — and none of them is
    /// about how many writes that took, so the seat helpers below count rows and
    /// this is where the two are separated. Exactly the treatment
    /// [`unlabelled`] gives D23's speaker prefix, and for the same reason:
    /// restating the delivery shape in forty-five routing assertions would test
    /// it in none of them.
    ///
    /// The shape itself is pinned where it IS the subject —
    /// `a_turns_backlog_arrives_as_one_message` and
    /// `a_page_boundary_is_the_only_thing_that_splits_a_backlog` below, both of
    /// which read raw wires and never come through here.
    ///
    /// **Splitting on a blank line is exact for these fixtures and not in
    /// general.** Every row posted in this file is single-line, so each part is
    /// one row; a body containing a blank line would split into two. That fails
    /// LOUDLY rather than silently — the second part carries no `[speaker]`, and
    /// the assert below names it — so a future fixture that grows one is told
    /// what happened instead of quietly asserting against fragments.
    fn rows_of(wire: String) -> Vec<String> {
        wire.split(crate::storage::WIRE_JOIN)
            .map(|part| {
                assert!(
                    part.starts_with('['),
                    "a delivered row must lead with its [speaker]; got {part:?}. If this \
                     fired on a multi-line fixture body, that body was split at its blank \
                     line — see `rows_of`."
                );
                unlabelled(part.to_string())
            })
            .collect()
    }

    impl Seat {
        /// Everything on this stdin right now.
        ///
        /// `try_recv`, not `recv().await`: a test only calls this once the
        /// sequencer task has exited, so anything not already queued is
        /// something that was never sent. Awaiting for it would hang where
        /// this fails.
        fn drain(&mut self) -> Vec<String> {
            let mut out = Vec::new();
            while let Ok(m) = self.rx.try_recv() {
                out.extend(rows_of(m.message.content));
            }
            out
        }

        /// [`expect`](Self::expect) with the `[speaker]` prefix left ON, and
        /// counting WIRES rather than rows — for the three tests whose subject
        /// IS the delivery shape: `a_delivered_row_says_who_wrote_it`,
        /// `a_turns_backlog_arrives_as_one_message` and
        /// `a_page_boundary_is_the_only_thing_that_splits_a_backlog`. Everything
        /// else goes through [`rows_of`] and asserts routing.
        ///
        /// Carries no quiescence window of its own — a caller that needs the
        /// negative half ("and NOTHING else was written") follows it with
        /// [`quiet`](Self::quiet), which is exactly what makes those three
        /// tests able to say a page was one write and not three.
        ///
        /// Deliberately NOT a `try_recv` drain: the rows have to be waited for,
        /// and a drain that read too early would return an empty vec and assert
        /// nothing while looking like it passed.
        async fn expect_raw(&mut self, n: usize) -> Vec<String> {
            let mut out = Vec::new();
            for i in 0..n {
                let m = tokio::time::timeout(DEADLINE, self.rx.recv())
                    .await
                    .unwrap_or_else(|_| panic!("raw wire {} of {n} never arrived", i + 1))
                    .expect("the sequencer dropped this participant's stdin");
                out.push(m.message.content);
            }
            out
        }

        /// One [`QUIET`] window of silence, or the wire that broke it.
        ///
        /// The shared body of [`quiet`](Self::quiet) and the tail of
        /// [`expect`](Self::expect) — the two negative assertions in this file
        /// differ only in what they say when they fail.
        async fn extra_wire(&mut self) -> Option<Vec<String>> {
            match tokio::time::timeout(QUIET, self.rx.recv()).await {
                Ok(Some(m)) => Some(rows_of(m.message.content)),
                // Elapsed, or the sender was dropped. Both are silence.
                _ => None,
            }
        }

        /// Assert nothing arrives within a bounded window, **with the control
        /// channel still open**.
        ///
        /// `drain()` cannot express this and the difference is load-bearing.
        /// `drain` is only valid after the task exits, and closing the control
        /// channel is itself what aborts an in-flight drain — the select is
        /// biased on commands, so a closed `rx` wins every iteration. A test
        /// that drops `tx` and then finds an empty seat therefore cannot tell
        /// "the sequencer refused to wake this participant" from "the wake was
        /// cut short by the close", and a guard asserted that way passes with
        /// the guard removed. Waiting while the channel is open is what makes
        /// the silence mean something.
        async fn quiet(&mut self) {
            if let Some(w) = self.extra_wire().await {
                panic!("expected no wire, got {w:?}");
            }
        }

        /// Wait for exactly `n` wires, then assert nothing else is queued.
        ///
        /// A synchronisation point: it returns only once the sequencer has
        /// finished the delivery, which is what lets a test post a NEW row
        /// between two commands and know which turn will read it.
        ///
        /// **Both halves are asserted here, and the second one is why this is
        /// not just a bounded `recv` loop.** For a while it was: the body read
        /// `n` wires and returned, so `expect(n)` caught UNDER-delivery only
        /// while the doc claimed both. That is not a hypothetical gap — four
        /// tests below were moved off `drain()` (which compares whole contents)
        /// onto `expect(n)`, and appending one duplicate wire to the end of
        /// every drain left all four green:
        /// `a_completed_turn_wakes_exactly_one_participant`,
        /// `a_summonable_participant_is_skipped_not_given_a_no_op_turn`,
        /// `a_participant_with_no_stdin_holds_the_turn_rather_than_losing_its_rows`
        /// and `a_backlog_past_the_batch_limit_is_drained_before_the_turn_is_handed_over`.
        /// With the [`QUIET`] window below, that same duplicate fails all four.
        ///
        /// The cost is one `QUIET` per call, paid on the happy path. That is
        /// the price of a negative assertion — [`quiet`](Self::quiet) pays it
        /// too — and it buys the half of this contract that was being asserted
        /// nowhere.
        /// Wait for a wake, then take whatever else lands in one [`QUIET`]
        /// window.
        ///
        /// The positive counterpart of [`quiet`](Self::quiet), and deliberately
        /// NOT [`expect`](Self::expect): it pins that the ring woke this seat
        /// and leaves the COUNT to the caller. A seat's backlog is however far
        /// its cursor was behind, which the round-cap tests do not control and
        /// are not about — they drive laps, and a wire count asserted there
        /// would be a delivery assertion wearing a scheduling test's name.
        async fn woken(&mut self) -> Vec<String> {
            let first = tokio::time::timeout(DEADLINE, self.rx.recv())
                .await
                .expect("the ring never woke this participant")
                .expect("the sequencer dropped this participant's stdin");
            let mut out = rows_of(first.message.content);
            while let Some(w) = self.extra_wire().await {
                out.extend(w);
            }
            out
        }

        async fn expect(&mut self, n: usize) -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            while out.len() < n {
                let m = tokio::time::timeout(DEADLINE, self.rx.recv())
                    .await
                    .unwrap_or_else(|_| {
                        panic!("row {} of {n} never arrived", out.len() + 1)
                    })
                    .expect("the sequencer dropped this participant's stdin");
                out.extend(rows_of(m.message.content));
            }
            // Over-delivery inside the LAST write, which the quiescence window
            // below cannot see: those rows arrived in a message this call
            // already consumed.
            assert!(
                out.len() <= n,
                "expected exactly {n} rows, got {}: {out:?}",
                out.len()
            );
            if let Some(w) = self.extra_wire().await {
                panic!("expected exactly {n} rows, then {w:?} arrived as well");
            }
            out
        }
    }

    /// A session whose rotation is `roster` — `(slug, participation_mode)` in
    /// turn-position order — with one [`STDIN_CAPACITY`]-slot stdin per
    /// participant.
    ///
    /// Returns the deps (moved into the task), a clone of the storage the test
    /// posts and asserts with, and one [`Seat`] per roster entry.
    async fn ring(roster: &[(&str, &str)]) -> (SequencerDeps, Storage, Vec<Seat>) {
        ring_sized(roster, STDIN_CAPACITY).await
    }

    /// [`ring`], with the session's round cap written to a real policy snapshot
    /// in a temp dir.
    ///
    /// Goes through [`write_session_policy`] and the file the gear tab edits
    /// rather than poking a number into the deps, because the number is not
    /// what these tests are about — the RESOLUTION is: `None` inherits the
    /// default, `Some(0)` is off, `Some(n)` caps at n. A `Fixed(u32)` field
    /// would let all three of those pass with the policy plumbing deleted.
    ///
    /// The [`TempDir`] is returned so the caller keeps it alive; drop it and
    /// the snapshot vanishes mid-test and every cap resolves to the default.
    async fn capped_ring(
        roster: &[(&str, &str)],
        round_cap: Option<u32>,
    ) -> (SequencerDeps, Storage, Vec<Seat>, TempDir) {
        let (mut deps, storage, seats) = ring(roster).await;
        let dir = tempdir().unwrap();
        write_session_policy(
            dir.path(),
            "s1",
            &SessionPolicy {
                policy: Policy {
                    round_cap,
                    ..Policy::default()
                },
                tool_gate: Vec::new(),
            },
        )
        .unwrap();
        deps.data_dir = Some(dir.path().to_path_buf());
        (deps, storage, seats, dir)
    }

    /// Every `system_notice` row in the session, oldest first.
    async fn notices(storage: &Storage) -> Vec<String> {
        storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.kind == MessageKind::SystemNotice.as_str())
            .map(|m| m.content)
            .collect()
    }

    /// [`ring`], with the stdin buffer named rather than defaulted.
    ///
    /// For the tests that want a drain to RUN OUT of buffer: pass a capacity
    /// below the number of rows the test posts and `deliver` parks mid-drain,
    /// the way production's 64 slots do behind a slow child.
    async fn ring_sized(
        roster: &[(&str, &str)],
        stdin_capacity: usize,
    ) -> (SequencerDeps, Storage, Vec<Seat>) {
        let storage = Storage::memory().await.unwrap();
        storage.create_session("s1", "t", None).await.unwrap();
        let mut inputs = HashMap::new();
        let mut seats = Vec::new();
        for (position, (slug, mode)) in roster.iter().enumerate() {
            let id = storage
                .insert_participant("s1", slug, slug, None, None, "[]", mode, position as i64)
                .await
                .unwrap();
            let (tx, rx) = mpsc::channel(stdin_capacity);
            inputs.insert(id, ParticipantInput::new("s1", tx));
            seats.push(Seat { id, rx });
        }
        let deps = SequencerDeps {
            session_id: "s1".into(),
            storage: storage.clone(),
            inputs,
            epochs: HashMap::new(),
            // No snapshot to read, so the cap resolves to
            // `DEFAULT_ROUND_CAP_LAPS` — 500 laps, which is far past what any
            // test here drives. `capped_ring` is how a test opts in to a cap it
            // can actually reach.
            data_dir: None,
            bridge: None,
            activity: None,
        };
        (deps, storage, seats)
    }

    /// [`ring`], wired to a real [`ActivityTracker`] so the busy flag — the one
    /// the input lock and the stall watchdog both read — is observable.
    ///
    /// The default `ring` passes `activity: None`, which is why a whole class of
    /// defect was invisible to this file: the ring can mark a participant busy
    /// and every test still passes, because no test had a tracker to look at.
    async fn ring_with_activity(
        roster: &[(&str, &str)],
    ) -> (
        SequencerDeps,
        Storage,
        Vec<Seat>,
        Arc<crate::core::ActivityTracker>,
    ) {
        let (mut deps, storage, seats) = ring(roster).await;
        let bridge = crate::signaling::SignalingBridge::new();
        let activity = crate::core::ActivityTracker::new(
            "s1".to_string(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            bridge,
            roster.iter().map(|(slug, _)| slug.to_string()).collect(),
        );
        deps.activity = Some(Arc::clone(&activity));
        (deps, storage, seats, activity)
    }

    /// A stdin that arrives AFTER the task was spawned, as
    /// [`SequencerCommand::ParticipantJoined`] carries one: the input to send
    /// and the seat that reads it.
    ///
    /// Deliberately not built by [`ring`] — the point of these cases is a
    /// participant the deps map does not have (or no longer has), so the
    /// channel has to be made outside it.
    fn late_stdin(id: i64) -> (ParticipantInput, Seat) {
        let (tx, rx) = mpsc::channel(STDIN_CAPACITY);
        (ParticipantInput::new("s1", tx), Seat { id, rx })
    }

    async fn post(storage: &Storage, origin: &str, slug: Option<&str>, body: &str) {
        storage
            .post_to_channel("s1", origin, slug, MessageKind::Text.as_str(), body, None)
            .await
            .unwrap();
    }

    /// The user spoke and named nobody — the ordinary message, and what every
    /// test here meant before mentions existed (rc3 D17).
    ///
    /// A helper rather than the literal at 60 call sites, so that adding a field
    /// to the command does not turn into a mechanical edit across the file where
    /// the ONE site that should have carried a value is indistinguishable from
    /// the 59 that should not. [`summoning`] is the other half.
    fn user_message() -> SequencerCommand {
        SequencerCommand::UserMessage {
            mentions: Vec::new(),
        }
    }

    /// Park an Approve/Reject gate row for `session` — the durable shape both
    /// gate kinds write, and the one `count_pending_gates` seeds the latch from.
    async fn park_gate(storage: &Storage, choice_id: &str) -> i64 {
        storage
            .insert_tray_entry(
                "s1",
                choice_id,
                "hands",
                crate::storage::QuestionKind::Choice,
                "Run gated command?",
                Some(&["Approve".to_string(), "Reject".to_string()]),
                None,
                Some("git push"),
            )
            .await
            .expect("gate row parks")
    }

    /// `participant_id` declared a halt (rc3 D35: the ring stops NOW).
    fn halt_by(participant_id: i64) -> SequencerCommand {
        SequencerCommand::HaltDeclared {
            participant_id: Some(participant_id),
        }
    }

    /// A park whose asker could not be resolved — the fallback that halts the
    /// whole cycle where it stands, as every park used to.
    fn parked_by_nobody() -> SequencerCommand {
        SequencerCommand::HaltDeclared {
            participant_id: None,
        }
    }

    /// The user spoke and named these participants, in this order.
    fn summoning(mentions: &[i64]) -> SequencerCommand {
        SequencerCommand::UserMessage {
            mentions: mentions.to_vec(),
        }
    }

    /// `send` with a deadline. A bounded channel's `send` parks once the buffer
    /// fills, so a sequencer that stopped draining would hang the caller rather
    /// than fail it — the same trap [`exited`] avoids on the other end.
    async fn send(tx: &mpsc::Sender<SequencerCommand>, cmd: SequencerCommand) {
        tokio::time::timeout(DEADLINE, tx.send(cmd))
            .await
            .expect("sequencer stopped draining its command channel")
            .expect("sequencer dropped its receiver while a sender was live");
    }

    /// Did the task END within [`DEADLINE`]? A panic inside the loop also ends
    /// the task, so the join result is unwrapped rather than counted as an exit
    /// — an unwritten `match` arm must fail here, not read as a clean shutdown.
    async fn exited(task: JoinHandle<()>) -> bool {
        match tokio::time::timeout(DEADLINE, task).await {
            Ok(joined) => {
                joined.expect("sequencer task panicked");
                true
            }
            Err(_) => false,
        }
    }

    fn nothing() -> Vec<String> {
        Vec::new()
    }

    #[tokio::test]
    async fn the_sequencer_exits_when_its_control_channel_closes() {
        // Router-inventory #20, carried forward: the task must end when its
        // last sender goes (session end), not linger holding a session's worth
        // of state alive.
        let (deps, _storage, _seats) = ring(&[("a", "active")]).await;
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        drop(tx);
        assert!(
            exited(task).await,
            "sequencer must exit on session end, not linger"
        );
    }

    #[tokio::test]
    async fn a_completed_turn_wakes_exactly_one_participant() {
        let (deps, storage, mut seats) =
            ring(&[("a", "active"), ("b", "active"), ("c", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "user one").await;
        post(&storage, "system", None, "host note").await;
        post(&storage, "participant", Some("a"), "a's last turn").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // The user message resets the ring to its first place, so A takes the
        // turn; then A finishes it. Each wake is awaited before the next command
        // goes in: the drain selects commands first and biased, so a closed
        // control channel wins every iteration and would stop the drain before a
        // row landed.
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(2).await,
            vec!["user one", "host note"],
            "A read the channel, but not its own last turn back as fresh input"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(
            seats[1].expect(3).await,
            vec!["user one", "host note", "a's last turn"],
            "the turn steps to the next PLACE in the rotation, carrying every unread row"
        );
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[2].drain(),
            nothing(),
            "one completed turn wakes ONE participant, not the rotation"
        );
    }

    /// **The ring must reach its THIRD place.** Written after a live N=3 session
    /// (`s-a0416b2a`, 2026-08-13) in which the third participant produced output
    /// but received ZERO deliveries and never moved its cursor: the ring went
    /// A → B and stopped there for the rest of the session.
    ///
    /// The gap that let it through is in the test above, not in the ring —
    /// `a_completed_turn_wakes_exactly_one_participant` drives exactly ONE
    /// completion and then asserts the third seat is silent, which is correct at
    /// that instant and says nothing about whether the rotation can ever arrive.
    /// Two seats were tested; the step between the second and the third was
    /// tested by nothing.
    #[tokio::test]
    async fn the_rotation_reaches_every_place_not_just_the_first_two() {
        let (deps, storage, mut seats) =
            ring(&[("a", "active"), ("b", "active"), ("c", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));

        send(&tx, user_message()).await; // epoch 1 → A
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        post(&storage, "participant", Some("a"), "from a").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[1].expect(2).await, vec!["go", "from a"], "epoch 2 → B");

        // The step nothing covered.
        post(&storage, "participant", Some("b"), "from b").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: SPOKE },
        )
        .await;
        assert_eq!(
            seats[2].expect(3).await,
            vec!["go", "from a", "from b"],
            "epoch 3 → C: a three-place ring must reach its third place, carrying \
             every unread row. A ring that stops at two is two agents and a spectator."
        );

        drop(tx);
        assert!(exited(task).await);
    }

    /// rc3 **D23**: a delivered row says who wrote it.
    ///
    /// The one test here whose subject IS the wire format, which is why it reads
    /// raw — everything else in this file strips the prefix so its routing
    /// assertions stay about routing. See [`unlabelled`].
    ///
    /// **The wire carried no author at all before this.** A participant handed
    /// four rows received four anonymous strings and had to infer which was the
    /// user's task, which was a peer's aside, and which was the host talking.
    /// `s-81057bde` is what that costs: a reviewer reported "no task from the
    /// user and no HANDS output" while `participant_deliveries` recorded eight
    /// rows handed to it. Both statements were true — it had read them and could
    /// not tell what they were.
    #[tokio::test]
    async fn a_delivered_row_says_who_wrote_it() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        // One of each authority a participant can be handed, because telling
        // them apart is the whole point: the user is who it works for, a peer is
        // who it argues with, and the host is neither — an agent that reads a
        // system notice as the user has been handed a fabricated instruction.
        post(&storage, "user", None, "the task").await;
        post(&storage, "participant", Some("b"), "a peer's opinion").await;
        post(&storage, "system", None, "a host notice").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        // ONE raw wire, because a turn's page is one write — so the three
        // speakers are asserted where they actually appear, inside it.
        assert_eq!(
            seats[0].expect_raw(1).await,
            vec![[
                "[user] the task",
                // The PEER's slug, not "participant" — and the same string
                // `@mention` parses, so what a participant reads is what the
                // user would type to summon it.
                "[b] a peer's opinion",
                "[system] a host notice",
            ]
            .join(crate::storage::WIRE_JOIN)]
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// rc3 **D20**'s other half (migration 0053): the name the user gave a
    /// participant is the name its PEERS read.
    ///
    /// The point of the label was that `EYES-2` still says nothing about which
    /// reviewer this is. Shipping it into the roster and leaving the wire on the
    /// slug would fix that for the user and leave every participant reading the
    /// numbers — which is the same complaint, one layer down.
    ///
    /// Driven through the ring rather than by calling `speaker_of`, because the
    /// claim is that the label reaches the WIRE: it is resolved at read time by
    /// a LEFT JOIN in `channel_page`, and a test on the function alone passes
    /// with that join deleted.
    #[tokio::test]
    async fn a_labelled_participant_says_its_name_on_the_wire() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let b = seats[1].id;
        sqlx::query("UPDATE session_participants SET label = ? WHERE id = ?")
            .bind("Skeptic")
            .bind(b)
            .execute(storage.pool())
            .await
            .unwrap();
        // One row from the labelled peer, one from the user, one from the host —
        // so the test also says what a label must NOT rename.
        post(&storage, "participant", Some("b"), "b's opinion").await;
        post(&storage, "user", None, "the task").await;
        post(&storage, "system", None, "a host notice").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect_raw(1).await,
            vec![[
                // The user's name for it, not `b`.
                "[Skeptic] b's opinion",
                // Neither of these is a participant, so neither can be renamed:
                // an agent that reads a host notice as the user has been handed
                // a fabricated instruction (D23).
                "[user] the task",
                "[system] a host notice",
            ]
            .join(crate::storage::WIRE_JOIN)]
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// A blank label is not a name — on the wire, exactly as in the roster.
    #[tokio::test]
    async fn a_blank_label_leaves_the_slug_on_the_wire() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let b = seats[1].id;
        sqlx::query("UPDATE session_participants SET label = ? WHERE id = ?")
            .bind("   ")
            .bind(b)
            .execute(storage.pool())
            .await
            .unwrap();
        post(&storage, "participant", Some("b"), "b's opinion").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect_raw(1).await,
            vec!["[b] b's opinion"],
            "whitespace must not put an empty [] on the wire"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// A tracker wired to a real bridge, for the input-lock tests.
    ///
    /// `awaiting` is its own flag and stays false here: it OUTRANKS busy in
    /// `derive`, so a test that let it float would pass on the wrong reason.
    async fn tracker(slugs: &[&str]) -> Arc<crate::core::activity::ActivityTracker> {
        crate::core::activity::ActivityTracker::new(
            "s1",
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            SignalingBridge::new(),
            slugs.iter().map(|s| s.to_string()).collect(),
        )
    }

    /// **A halt clears the column too — a yielded session must not read as a
    /// working one.**
    ///
    /// `halt` cleared the holder the ring task carries and bumped the epoch, and
    /// that was all: `sessions.current_turn_participant_id` kept naming whoever
    /// held the turn when the halt landed. Every halt path was affected — the
    /// parked question here, consensus, the all-pass yield, the round cap — so a
    /// session whose floor was the user's still reported a participant working
    /// on a turn that had been retired.
    ///
    /// Pinned on the parked-question path because it is the one a user meets
    /// daily; the column write lives in `halt` itself, so the other four paths
    /// cannot diverge from it.
    #[tokio::test]
    async fn a_halt_stops_the_column_naming_a_holder() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        async fn column(s: &Storage) -> Option<i64> {
            let row: Option<(Option<i64>,)> =
                sqlx::query_as("SELECT current_turn_participant_id FROM sessions WHERE id = ?")
                    .bind("s1")
                    .fetch_optional(s.pool())
                    .await
                    .unwrap();
            row.and_then(|(id,)| id)
        }
        assert_eq!(column(&storage).await, Some(a), "A holds the turn");

        // A parks a question: the ring halts where it stands (rc3 D35).
        send(&tx, SequencerCommand::HaltDeclared { participant_id: Some(a) }).await;
        // The halt is processed before anything else this channel carries, so a
        // second command that must observe it is enough of a barrier.
        for _ in 0..200 {
            if column(&storage).await.is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            column(&storage).await,
            None,
            "the session halted and the column still names A — a yielded session \
             reporting itself as working is the state the halt exists to end"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// **A dealt turn that can never complete DECLARES.**
    ///
    /// The wedge: the ring deals a turn, marks the holder busy, and the page
    /// cannot go out — here because the participant's stdin is gone. Before the
    /// `Dealt` unwind this warned into the log and returned, leaving the holder
    /// set, the busy flag up, the input locked and the halt slot empty. The
    /// health dot flipped for a dead pump and nothing else did; the idle nudge
    /// cannot cover a wedge that reads Busy. The session's only exit was a Pause
    /// and a SIGKILL.
    ///
    /// Three assertions, because the bug had three halves and fixing one is
    /// what a partial fix looks like: the flag comes down, the column stops
    /// naming a dead holder, and the user is TOLD.
    #[tokio::test]
    async fn a_turn_dealt_to_an_unreachable_participant_unwinds_and_declares() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let act = tracker(&["a", "b"]).await;
        deps.activity = Some(Arc::clone(&act));
        let bridge = SignalingBridge::new();
        let mut events = bridge.subscribe();
        deps.bridge = Some(Arc::clone(&bridge));
        post(&storage, "user", None, "go").await;

        // A's process is gone: its stdin receiver is dropped before the ring
        // ever reaches it, which is what a dead subprocess looks like from here.
        let seat_a = seats.remove(0);
        drop(seat_a);

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;

        let reason = tokio::time::timeout(DEADLINE, async {
            loop {
                match events.recv().await {
                    Ok(crate::signaling::SignalingEvent::AwaitingUser { reason, .. }) => {
                        return reason
                    }
                    Ok(_) => continue,
                    Err(e) => panic!("the event channel closed before any halt: {e}"),
                }
            }
        })
        .await
        .expect("a turn was dealt to an unreachable participant and nothing was declared");
        assert!(
            reason.contains('a') && reason.contains("unreachable"),
            "the halt names who and why: {reason}"
        );
        assert!(
            !act.is_busy_slug("a"),
            "the busy flag outlived the turn it was set for — the input stays locked \
             on a participant that cannot answer"
        );
        let column: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT current_turn_participant_id FROM sessions WHERE id = ?")
                .bind("s1")
                .fetch_optional(storage.pool())
                .await
                .unwrap();
        assert_eq!(
            column.and_then(|(id,)| id),
            None,
            "the column still names the dead holder, so the UI reads the wedge as work"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// **rc3 D19b: the ring records WHO holds the turn** — and until now nothing
    /// pinned it.
    ///
    /// Found while mutation-testing `hand_turn_to`: deleting the
    /// `set_current_turn` call outright left all 1100 tests green. The column
    /// exists so the UI can tell a participant waiting its turn from one that
    /// has died, and the write that fills it was silently deletable — the same
    /// unpinned-wire shape `set_current_turn` was itself introduced to fix, one
    /// level up.
    ///
    /// It is pinned HERE rather than in a test of its own because the extraction
    /// is what makes the pairing load-bearing: recording the holder and marking
    /// it busy are one event, and a test covering only the busy half would let
    /// the other be dropped by anyone tidying the function.
    #[tokio::test]
    async fn the_ring_records_who_holds_the_turn() {
        async fn holder(s: &Storage) -> Option<i64> {
            let row: Option<(Option<i64>,)> =
                sqlx::query_as("SELECT current_turn_participant_id FROM sessions WHERE id = ?")
                    .bind("s1")
                    .fetch_optional(s.pool())
                    .await
                    .unwrap();
            row.and_then(|(id,)| id)
        }

        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        assert_eq!(holder(&storage).await, Some(a), "A holds turn one");

        post(&storage, "participant", Some("a"), "a spoke").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        let _ = seats[1].expect(2).await;
        assert_eq!(
            holder(&storage).await,
            Some(b),
            "the ring stepped, so the column steps with it"
        );

        // **What this test does NOT pin, stated rather than implied.** An earlier
        // draft ended by driving a consensus halt and claiming the column was
        // cleared. It is not: `halt` never calls `hand_turn_to`, so after a real
        // halt the column still names the last holder — and the draft asserted
        // nothing about it anyway, so it read as coverage while testing nothing.
        // The gap is real and recorded in `hand_turn_to`'s doc; what is pinned
        // here is that the column FOLLOWS the ring, which is the half D19b
        // shipped with no test at all.
        drop(tx);
        assert!(exited(task).await);
    }

    /// **The ring marks the participant it hands the turn to.**
    ///
    /// Until this existed the ring could not say a turn had started: busy was
    /// set only by `AppState::broadcast` and `SessionHandle::send_to_all`, and
    /// cleared by each pump at its own turn end. `SequencerDeps` had no tracker
    /// at all, so the one component that knows a turn began had no way to
    /// report it.
    #[tokio::test]
    async fn the_ring_marks_the_participant_it_hands_the_turn_to() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let act = tracker(&["a", "b"]).await;
        deps.activity = Some(Arc::clone(&act));
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        assert!(act.is_busy_slug("a"), "the ring handed A the turn and did not mark it");
        assert_eq!(
            act.current(),
            crate::core::activity::SessionActivity::Busy,
            "a participant holding a turn means the session is working, so the input locks"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// **The regression the user reported, and the one that fails today.**
    ///
    /// *"I can type while agents are working, it might legitimately interrupt
    /// your turns, therefore corrupting the quality of work you provide."*
    ///
    /// When busy was set only at the broadcast layer — every participant
    /// pre-marked on the user's message, each clearing its OWN flag as its turn
    /// ended — one full lap cleared every flag and the session derived `Idle`:
    /// the input re-opened while the ring was still cycling (D22's lap, the
    /// consensus tally, the round cap's 500). The cure is the ring marking each
    /// holder as it deals — and since s-ff729daa that is the ONLY busy-true
    /// writer; the broadcast pre-mark is deleted, because a pre-mark on a
    /// participant the ring stops before reaching has no turn end to clear it.
    /// What made the lap gap expensive rather than cosmetic: a message typed
    /// mid-lap supersedes the in-flight turn, and when the reset target is the
    /// participant already holding it, its new backlog is written to a stdin
    /// whose turn is running.
    ///
    /// The assertion is on the state AFTER a completed lap, with the ring still
    /// live — which is exactly the window the user was typing into.
    #[tokio::test]
    async fn the_input_stays_locked_across_a_whole_lap() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        let act = tracker(&["a", "b"]).await;
        deps.activity = Some(Arc::clone(&act));
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // A finishes — the PUMP would clear A here, so simulate that faithfully
        // rather than leaving a flag the real system would have dropped.
        act.set_busy_slug("a", false);
        post(&storage, "participant", Some("a"), "a spoke").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        let _ = seats[1].expect(2).await;
        assert!(act.is_busy_slug("b"), "the ring handed B the turn");

        // B finishes, closing lap one. The ring wraps to A and hands it turn
        // three — and THAT is where the input used to re-open.
        act.set_busy_slug("b", false);
        post(&storage, "participant", Some("b"), "b spoke").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: SPOKE },
        )
        .await;
        let _ = seats[0].expect(1).await;

        assert!(
            act.is_busy_slug("a"),
            "lap two handed A a turn; without the ring marking it, every flag is \
             clear here and the user can type into a working session"
        );
        assert_eq!(
            act.current(),
            crate::core::activity::SessionActivity::Busy,
            "the input must stay locked for the whole cycle, not just the first lap"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// **And the fix must not lock the user out permanently**, which is the
    /// obvious way to over-correct.
    ///
    /// A halt hands `None` — no holder recorded, nobody marked. The pump has
    /// already cleared whoever finished, so the session falls to `Idle` and the
    /// input opens. That is the unlock condition, and it needs no code of its
    /// own; this pins that it actually holds.
    #[tokio::test]
    async fn a_halt_leaves_nobody_busy() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let act = tracker(&["a"]).await;
        deps.activity = Some(Arc::clone(&act));
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        assert!(act.is_busy_slug("a"));

        // The solo ring votes done: consensus halts the cycle, so no turn is
        // handed out and no participant is marked.
        act.set_busy_slug("a", false);
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: DONE },
        )
        .await;
        seats[0].quiet().await;

        assert!(
            !act.is_busy_slug("a"),
            "a halt hands nobody a turn, so nothing may be marked working"
        );
        assert_eq!(
            act.current(),
            crate::core::activity::SessionActivity::Idle,
            "the cycle yielded — this is when the user gets their input back"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_summonable_participant_is_skipped_not_given_a_no_op_turn() {
        // A wake nobody asked for is pure waste, so the ring filters `on_mention`
        // out rather than handing it a turn it ends immediately. The only thing
        // that reaches one is the user naming it (rc3 D17), and this is the
        // property that keeps a later change from putting it back in the
        // rotation "so the summons has somewhere to land".
        let (deps, storage, mut seats) = ring(&[
            ("a", "active"),
            ("watcher", "on_mention"),
            ("b", "active"),
        ])
        .await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // Await each wake before sending the next command. The drain selects
        // commands FIRST and biased, so a control channel that is already closed
        // wins every iteration and stops the drain before a row lands — firing
        // everything up front and dropping `tx` would assert against a delivery
        // the sequencer correctly refused to make.
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(
            seats[2].expect(1).await,
            vec!["go"],
            "the turn steps OVER the summonable one to the next active participant"
        );
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[1].drain(),
            nothing(),
            "the summonable one sits between A and B in ring order and must not be woken"
        );
    }

    /// rc3 **D17**, the whole of it: a mention hands the next turn to the named
    /// participant, and the rotation carries on from where it was.
    ///
    /// **Three active participants, not two, and that is what makes the second
    /// half falsifiable.** With A and B only, "resume where it was" and "reset
    /// to the front" name the same participant after B's turn, so a summons
    /// implemented as a reset would pass. With A, B and C the two answers differ
    /// — C if the rotation resumed, A if it restarted — and only one of them is
    /// D17 #4.
    #[tokio::test]
    async fn a_summons_takes_one_turn_and_the_rotation_resumes_where_it_was() {
        let (deps, storage, mut seats) = ring(&[
            ("a", "active"),
            ("b", "active"),
            ("c", "active"),
            ("adv", "on_mention"),
        ])
        .await;
        let (a, b, adv) = (seats[0].id, seats[1].id, seats[3].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"], "epoch 2 → B");

        // The user speaks mid-rotation and names the advisor. The turn is
        // B's — nothing cancels it, exactly as with any other user message —
        // but the NEXT one is the advisor's.
        post(&storage, "user", None, "@adv thoughts?").await;
        send(&tx, summoning(&[adv])).await;
        assert_eq!(
            seats[3].expect(2).await,
            vec!["go", "@adv thoughts?"],
            "the summoned participant takes the next turn, not the front of the ring — \
             and a first wake carries its WHOLE backlog, because a dormant \
             participant's cursor has never moved"
        );

        // …and exactly one. After it speaks the rotation resumes from B, which
        // is where it was interrupted.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: adv,
                epoch: 3,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(
            seats[2].expect(2).await,
            vec!["go", "@adv thoughts?"],
            "the ring resumes at C — the place after B — rather than restarting at A"
        );

        drop(tx);
        assert!(exited(task).await);
        assert_eq!(
            seats[3].drain(),
            nothing(),
            "one summons is one turn: the advisor drops back out until named again"
        );
        // B's completion never came, so it holds no second turn either. The
        // point is A: a summons must not have quietly reset the cycle.
        assert_eq!(seats[0].drain(), nothing(), "A was not woken a second time");
        let _ = b;
    }

    #[tokio::test]
    async fn mentions_take_their_turns_in_the_order_written() {
        // D17 #3. `@x @y` is two summonses, not a choice between them, and the
        // order is the user's.
        let (deps, storage, mut seats) = ring(&[
            ("a", "active"),
            ("x", "on_mention"),
            ("y", "on_mention"),
        ])
        .await;
        let (x, y) = (seats[1].id, seats[2].id);
        post(&storage, "user", None, "both of you").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, summoning(&[x, y])).await;
        assert_eq!(seats[1].expect(1).await, vec!["both of you"], "X first");
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: x,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(seats[2].expect(1).await, vec!["both of you"], "then Y");
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: y,
                epoch: 2,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["both of you"],
            "the queue empties and the ring takes over"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_summons_that_cannot_be_honoured_is_dropped_and_the_ring_still_moves() {
        // The race, not the ordinary case: an `@word` naming nobody never
        // reaches this loop, because `AppState::resolve_mentions` drops it and
        // the message arrives as a plain reset. What DOES reach here is a
        // participant that was live when the user typed and is gone by the time
        // the turn is handed out.
        //
        // The cost of getting this wrong is the worst failure the ring has: a
        // turn handed to a participant with no process behind it completes
        // never, and the cycle stops there for good.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        post(&storage, "user", None, "go").await;
        // An id no participant holds — the same state a deleted row leaves.
        let ghost = 9_999;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, summoning(&[ghost])).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "the summons is dropped and the ring hands the turn out as usual"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_summons_after_a_halt_clears_the_tally_the_halt_was_built_on() {
        // **The false arrival, reached down the mention path.** A user message
        // clears the done tally; a message that NAMES someone does not go to the
        // front of the ring, so the clear cannot ride the restart the way it
        // does for an ordinary one.
        //
        // Left unhandled the sequence is: the ring converges and halts, the user
        // summons an advisor, the advisor PASSES (a pass records no vote and
        // clears nothing), `all_active_voted_done` still sees the votes from
        // before the user spoke, and the session halts again — with the actives
        // never having read the message that restarted it.
        //
        // Delete the `user_spoke ||` in `advance_turn`'s clear and this test
        // times out on the last `expect`.
        let (deps, storage, mut seats) = ring(&[
            ("a", "active"),
            ("b", "active"),
            ("adv", "on_mention"),
        ])
        .await;
        let (a, b, adv) = (seats[0].id, seats[1].id, seats[2].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: TurnEnding::Done,
            },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: b,
                epoch: 2,
                ending: TurnEnding::Done,
            },
        )
        .await;
        // Both actives have voted done: the cycle has yielded to the user. The
        // halt wakes nobody, so there is no delivery to await — `quiet` is the
        // barrier, exactly as in `the_cycle_halts_when_every_active_participant_votes_done`.
        seats[0].quiet().await;
        seats[1].quiet().await;
        assert!(
            storage.all_active_voted_done("s1").await.unwrap(),
            "the tally is complete — this test is about what happens next"
        );

        post(&storage, "user", None, "@adv one more thing").await;
        send(&tx, summoning(&[adv])).await;
        assert_eq!(
            seats[2].expect(2).await,
            vec!["go", "@adv one more thing"],
            "the summons releases the halt and hands the advisor the turn"
        );
        assert!(
            !storage.all_active_voted_done("s1").await.unwrap(),
            "the user speaking un-converges the session, whether or not they named someone"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: adv,
                epoch: 4,
                ending: TurnEnding::Passed,
            },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["@adv one more thing"],
            "the ring carries on: A reads the message the halt would have swallowed"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_completion_from_a_superseded_turn_does_not_advance_the_ring() {
        // The hazard `TurnComplete`'s participant id exists for. A user message
        // resets the ring while the previous holder is still mid-turn; that
        // turn's completion then arrives BEHIND the reset. Payload-free it
        // would advance the ring off the participant the reset just woke, so
        // two agents would hold a turn at once.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "r1").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["r1"]);
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["r1"], "B now holds the turn");

        // The user speaks over B's turn. Waiting on A's wake is what makes the
        // next line's ordering a fact rather than a race.
        post(&storage, "user", None, "r2").await;
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["r2"], "the ring reset to A");

        // B's turn ends, late.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: SPOKE },
        )
        .await;
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[1].drain(),
            nothing(),
            "the superseded completion advanced nothing: B was not re-woken behind A"
        );
        assert_eq!(seats[0].drain(), nothing(), "and A was not woken twice");
    }

    #[tokio::test]
    async fn a_completion_from_a_turn_the_user_restarted_is_discarded() {
        // The case `participant_id` alone CANNOT catch, and the commonest one
        // there is: the user interjects while the FIRST participant is mid-turn,
        // so the reset re-wakes that same participant. The stale completion then
        // names the live holder and passes an identity check — stepping the ring
        // off a participant woken moments ago, which puts two agents on a turn at
        // once. Only the epoch separates the two turns.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "r1").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["r1"]);

        // The user speaks over A's own turn. The ring resets to its first place,
        // which IS A — same participant, new turn. Waiting on the wake is what
        // makes the ordering below a fact rather than a race.
        post(&storage, "user", None, "r2").await;
        send(&tx, user_message()).await; // epoch 2, A again
        assert_eq!(
            seats[0].expect(1).await,
            vec!["r2"],
            "the reset re-woke the SAME participant — the whole point of this case"
        );

        // A's first turn ends, late, carrying the epoch it was handed.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        // Proven with the channel still OPEN. Dropping `tx` here instead would
        // abort any delivery the guard wrongly allowed, and an empty seat would
        // then prove nothing — this test passed with the epoch compare removed
        // until it was written this way.
        seats[1].quiet().await;

        // And the live turn is untouched: completing THAT epoch does advance,
        // so the silence above was the guard working rather than the ring being
        // stuck.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 2,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(
            seats[1].expect(2).await,
            vec!["r1", "r2"],
            "the live completion steps the ring the stale one could not"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_user_message_resets_the_cycle_to_the_first_participant() {
        // Router-inventory #12's ring half. The reset is a REWIND, and a rewind
        // is only distinguishable from a step in the MIDDLE of a ring: this one
        // is three deep with the turn parked on B, so back-to-the-front lands on
        // A and one-more-place lands on C.
        //
        // Every other reset in this file runs on a ring of two, where the step
        // from B and the rewind to A are the same participant. Ignore `reset`
        // entirely and, of the tests that were here before this one, only
        // `a_completion_from_a_turn_the_user_restarted_is_discarded` notices —
        // and there it fails as a wake that never came, which reads as the epoch
        // guard rather than as the ring.
        //
        // The middle seat is what this test holds. A rewind that is correct from
        // the FRONT and merely steps from anywhere else passes every test that
        // was here before this one — including that one, whose reset lands while
        // A holds. `a_failed_reset_clears_the_tally_but_leaves_the_turn_in_flight`
        // notices it too, but as an uncleared tally: that narrowing takes
        // `current` away from the clear as well, so it fails on a vote. This is
        // the only test that fails on the RING.
        let (deps, storage, mut seats) =
            ring(&[("a", "active"), ("b", "active"), ("c", "active")]).await;
        let a = seats[0].id;
        let c = seats[2].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // Each wake is awaited before the next command goes in: the drain selects
        // commands first and biased, so anything sent ahead of a wake can cut
        // short the drain that would have produced it.
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(
            seats[1].expect(1).await,
            vec!["go"],
            "epoch 2 — the turn is now parked in the middle of the ring"
        );

        // The user speaks over B's turn. The row is posted BEFORE the command,
        // which is what makes the wire below a fact rather than a race.
        post(&storage, "user", None, "u2").await;
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["u2"],
            "the ring went back to its FIRST place, not on to the place after B"
        );

        // C is where a plain step would have landed, and both rows are still
        // past its cursor — so its silence is the rewind rather than an empty
        // backlog. With the control channel still OPEN: dropping `tx` first
        // would abort a delivery a stepping loop had already begun.
        assert!(
            !storage.unread_for_participant(c).await.unwrap().rows.is_empty(),
            "the silence has to be the rewind, not an empty backlog"
        );
        seats[2].quiet().await;
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn the_cycle_halts_when_every_active_participant_votes_done() {
        // The cycle runs until every ACTIVE participant agrees there is nothing
        // left to do. Two actives here, so it takes two done votes with no
        // substantive output between them.
        //
        // "Halts" means NO FURTHER WAKE ARRIVES, which a bare `recv().await`
        // cannot tell from a hang and a dropped `tx` cannot tell from a delivery
        // it aborted — the drain is biased on commands, so a closed control
        // channel stops a wake the halt was supposed to prevent, and a halt
        // asserted that way passes with the consensus check removed. `quiet()`
        // is the instrument: a bounded window with the channel still OPEN.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // A has nothing to add. One vote of two is not consensus, so the ring
        // steps — and this is also what says the halt below is the SECOND vote
        // arriving rather than the first one halting everything.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: DONE },
        )
        .await;
        assert_eq!(
            seats[1].expect(1).await,
            vec!["go"],
            "one done vote of two is not consensus — the cycle continues"
        );

        // An unread row for whoever would be woken next. Without it every
        // silence below would also be what a sequencer that stepped the ring
        // and found an empty backlog produces, and the test would prove nothing
        // — a ring step delivers no wire when there is nothing past the cursor.
        post(&storage, "system", None, "host note").await;
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence has to be the halt, not an empty backlog"
        );

        // B votes done too: every active participant has now declared done with
        // nothing substantive between the two votes, so the session yields.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: DONE },
        )
        .await;
        seats[0].quiet().await;
        seats[1].quiet().await;
        assert!(
            storage.all_active_voted_done("s1").await.unwrap(),
            "the arrival is durable in the roster, which is where a host reads it"
        );

        // Nothing can complete a turn that is not in flight. A late completion
        // for the turn that halted the cycle — a retry, a supervisor echoing
        // the last one back after a respawn — is discarded rather than
        // re-entering the cycle.
        //
        // **`done: false` is what makes this assertion mean anything.** Repeat
        // the halting vote instead and a loop that accepted it would record the
        // same vote, find the same consensus and fall silent for the same
        // reason: the silence proves nothing, and both `*holder = None` and
        // `*epoch += 1` can be deleted with the suite still green. A
        // SUBSTANTIVE late completion separates them — taken as live it clears
        // the tally and steps the ring, so the silence and the intact tally
        // below are two independent observations of the same discard.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: SPOKE },
        )
        .await;
        seats[0].quiet().await;
        assert!(
            storage.all_active_voted_done("s1").await.unwrap(),
            "a completion arriving after the halt did not reset the tally"
        );

        // Halted, not dead. The user speaking restarts the cycle at the front
        // of the ring — without this the silence above would also be what a
        // wedged loop looks like — and it clears the tally on the way in.
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["host note"],
            "the user's message restarts the cycle at the front of the ring"
        );
        assert!(
            !storage.all_active_voted_done("s1").await.unwrap(),
            "and resets the tally: a vote cast before the user spoke cannot count \
             toward the next arrival"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// **A pass cannot complete the tally**, which is the property the whole
    /// third ending exists for (design §1; rc3 decisions).
    ///
    /// The shape is the sharpest one available on a ring of two: A votes done,
    /// so the tally is one vote short, and B — the participant whose vote would
    /// complete it — passes instead. Read as a done vote that is consensus and
    /// the session halts; read as a pass it is a session where one participant
    /// is finished and the other simply had nothing this round, which has not
    /// arrived anywhere.
    ///
    /// The wake at the end is what makes the assertion positive rather than a
    /// silence that could mean anything.
    #[tokio::test]
    async fn a_pass_does_not_complete_the_consensus_tally() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // One vote of two. The tally now needs exactly B.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: DONE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"]);

        // The pass row the pump posts alongside the completion (`duo.rs`
        // PASS_NOTICE). Posted here because the sequencer writes no rows — and
        // it is also what gives A something unread to be woken WITH, so the
        // wake below is a real observation rather than an empty-backlog no-op.
        post(&storage, "participant", Some("b"), "(passed — nothing to add this round)").await;

        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: PASSED },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["(passed — nothing to add this round)"],
            "B passed rather than voting done, so the cycle continues to A"
        );
        assert!(
            !storage.all_active_voted_done("s1").await.unwrap(),
            "a pass casts NO vote — the tally is still one short in the roster, \
             which is where a host reads it"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    /// A pass **retracts the passer's own done vote**, and this is the case
    /// where that matters: without it, a vote cast two rounds ago is still
    /// standing when the other participants converge, and the session halts on
    /// an arrival its own passer had already withdrawn from.
    ///
    /// Built out of two passes and two done votes on a ring of two:
    ///
    /// | step | ending | votes after |
    /// |---|---|---|
    /// | A, epoch 1 | `Done` | A=1 B=0 |
    /// | B, epoch 2 | `Passed` | A=1 B=0 |
    /// | A, epoch 3 | `Passed` | **A=0** B=0 |
    /// | B, epoch 4 | `Done` | A=0 B=1 → no consensus |
    ///
    /// Drop the retraction and the last row reads A=1 B=1, so the cycle halts
    /// on epoch 4 and the final wake never arrives.
    #[tokio::test]
    async fn a_pass_retracts_the_passers_own_stale_done_vote() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: DONE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"]);

        post(&storage, "participant", Some("b"), "b passes").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: PASSED },
        )
        .await;
        assert_eq!(seats[0].expect(1).await, vec!["b passes"]);

        // A passes on top of its OWN done vote from epoch 1. That vote is what
        // this turn supersedes.
        post(&storage, "participant", Some("a"), "a passes").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, ending: PASSED },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["a passes"]);
        assert!(
            !storage.all_active_voted_done("s1").await.unwrap(),
            "A's epoch-1 vote is gone: A itself replaced it with a pass"
        );

        // B now votes done. With A's vote retracted this is one of two and the
        // ring steps; with it standing it would be the second of two and halt.
        post(&storage, "system", None, "host note").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 4, ending: DONE },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["host note"],
            "B's done vote is the FIRST of two, not the second — A withdrew its own"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    /// **A ring where everyone passes never halts by consensus, and nothing in
    /// this file bounds it.** Stated as a test rather than a comment because it
    /// is the shape a reader will most want reassurance about, and the honest
    /// answer is that the reassurance does not exist yet.
    ///
    /// Three full rounds of nothing but passes, and every one of them hands out
    /// the next turn. Note what does NOT stop it: a user message resets the
    /// cycle to the front and hands out another turn, so the steer redirects
    /// this ring rather than ending it. The module doc lists what actually
    /// does. The mechanical backstop is the round cap (rc3 decisions D2,
    /// default 500, `0` = off), which is a separate unshipped slice and
    /// deliberately not built here.
    ///
    /// Note what the rounds cost: each pass is a ROW, so the next participant
    /// always has something unread and is always woken. An all-pass ring is
    /// therefore not self-limiting through an empty backlog either — it is a
    /// real spend, which is precisely why the cap is owed.
    /// rc3 **D31**: a refused handover takes its busy flag back with it.
    ///
    /// `hand_over` runs `hand_turn_to`, which records the holder AND marks it
    /// busy — deliberately one event. The D22 blocked-check fires AFTER that, so
    /// the participant being refused a turn has already been marked as working,
    /// and nothing would ever clear it: the pump clears its own flag at ITS turn
    /// end, and this participant never receives a turn to end.
    ///
    /// Reported from `s-382d3d18` as a session that claimed both states at once
    /// — the halt banner reading "waiting on your answer" directly above "HANDS
    /// is working — the turn hasn't ended yet". Ninety seconds later the stall
    /// watchdog called HANDS `stalled`, because `busy` is a precondition of that
    /// verdict, so a cosmetic-looking lie became a health verdict.
    #[tokio::test]
    async fn a_declared_halt_parks_the_ring_before_any_handover_is_minted() {
        // **Changed subject at rc3 D35 — this was D31's take-back test.** Under
        // D22 a park kept dealing, so the ring could mint a handover (marking
        // the next participant busy) and then refuse it, and D31 existed to
        // take that flag back. The halt now parks dealing at the TOP of
        // `advance_turn`, before any handover exists — so the take-back is not
        // "handled", it is unreachable, and this pins the stronger property:
        // after a halt, no busy flag is ever set for a turn nobody received.
        let (deps, storage, mut seats, activity) =
            ring_with_activity(&[("a", "active"), ("b", "active")]).await;
        let (a, _b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        assert!(activity.is_busy_slug("a"), "the holder is marked working");

        // A declares a halt. The ring stops where it stands: B is never handed
        // a turn, so B is never marked working — there is nothing to take back.
        post(&storage, "system", None, "host note").await;
        send(&tx, halt_by(a)).await;
        seats[1].quiet().await;
        assert!(
            !activity.is_busy_slug("b"),
            "no handover was minted under the halt, so nobody NEW reads as working — \
             this is the session that used to say halted and working at once"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    /// rc3 **D27** — a full lap of passes yields to the user.
    ///
    /// The bug this closes cost real money in `s-8ac0d2d0`: boot finished before
    /// the user had given a task, so every participant passed, and the ring kept
    /// dealing turns. 23 provider calls in 77 seconds, each carrying ~240 KB, to
    /// produce "(passed — nothing to add this round)". The only floor was the
    /// 500-lap round cap — over five hours at that rate — so the user stopped it
    /// by hand, which is the one thing a backstop exists to make unnecessary.
    ///
    /// **Not consensus, and the sibling test below is why the distinction has to
    /// hold.** Consensus is every participant saying it is FINISHED, which ends
    /// the work and needs `done` votes. This says nobody has anything to add
    /// right now, which ends the LAP. A pass still casts no vote.
    #[tokio::test]
    async fn a_full_lap_of_passes_yields_to_the_user() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // A passes. One pass is not a lap — the ring must still reach B.
        send(&tx, SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: PASSED })
            .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"], "a single pass steps the ring");

        // An unread row for whoever would be woken next, so the silence below is
        // the yield rather than a step that found nothing to hand over.
        post(&storage, "user", None, "still here").await;
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence has to be the yield, not an empty backlog"
        );

        // B passes too: the lap wraps with nobody having spoken.
        send(&tx, SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: PASSED })
            .await;
        seats[0].quiet().await;
        seats[1].quiet().await;

        // And it SAYS so — a cycle that stops with nothing on screen reads as a
        // hang, which is what the user reported before this existed.
        let said = notices(&storage).await;
        assert!(
            said.iter().any(|n| n.contains("Every participant passed")),
            "the yield has to be visible: {said:?}"
        );

        // Halted, not dead: the user's message restarts it.
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(2).await,
            vec![
                "still here",
                "Every participant passed this round — nobody has anything to add \
                 without you. The cycle has yielded; send a message to resume.",
            ],
            "the restart hands over the backlog INCLUDING the notice — it is a row like \
             any other, and a participant reading why the cycle stopped is the point"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// One substantive turn is enough to earn the lap — the yield is for a lap
    /// where NOBODY spoke, not for one that contained a pass.
    #[tokio::test]
    async fn a_lap_with_one_substantive_turn_keeps_cycling() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // A speaks, B passes: the lap wraps having produced something.
        post(&storage, "participant", Some("a"), "from a").await;
        send(&tx, SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE })
            .await;
        assert_eq!(seats[1].expect(2).await, vec!["go", "from a"]);
        // Something for A to READ on its next turn — its own row is excluded from
        // its own backlog, so without this the silence below would be an empty
        // handover rather than a yield.
        post(&storage, "system", None, "host note").await;
        send(&tx, SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: PASSED })
            .await;

        // The ring carries on to A rather than yielding.
        assert_eq!(
            seats[0].expect(1).await,
            vec!["host note"],
            "a lap that produced work earns another one"
        );
        assert!(
            !notices(&storage).await.iter().any(|n| n.contains("Every participant passed")),
            "nothing to announce: somebody spoke"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_ring_where_everyone_passes_never_halts_by_consensus() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // **The subject is the TALLY, and it is unchanged** — a pass casts no
        // vote, so no number of them ever adds up to an arrival. What changed
        // under rc3 D27 is what happens at the end of a lap of nothing but
        // passes: the cycle yields to the user instead of dealing another. So
        // the ring turns for the rest of THIS lap, and the assertion that
        // matters — no consensus, ever — holds throughout.
        let mut epoch = 1u64;
        {
            for (holder, slug, seat) in [(a, "a", 1usize), (b, "b", 0usize)] {
                let round = 0;
                let row = format!("{slug} passes, round {round}");
                post(&storage, "participant", Some(slug), &row).await;
                send(
                    &tx,
                    SequencerCommand::TurnComplete {
                        participant_id: holder,
                        epoch,
                        ending: PASSED,
                    },
                )
                .await;
                epoch += 1;
                // B has never been woken before this first step, so its cursor
                // is still behind the user's message. Every later wake carries
                // one row, because a pass posts exactly one and a participant
                // never reads its own.
                let mut expected = Vec::new();
                if epoch == 2 {
                    expected.push("go".to_string());
                }
                expected.push(row);
                // A's pass steps the ring to B — a single pass is not a lap.
                // B's pass CLOSES the lap, and the yield's own notice is what
                // lands instead of a further turn.
                if seat == 1 {
                    let got = seats[seat].expect(expected.len()).await;
                    assert_eq!(got, expected, "a single pass steps the ring");
                }
                assert!(
                    !storage.all_active_voted_done("s1").await.unwrap(),
                    "round {round}: passes never accumulate into an arrival"
                );
            }
        }

        drop(tx);
        assert!(exited(task).await);
    }

    /// A participant that passes every round must NOT trip spin detection.
    ///
    /// Its rows are near-identical by design — that is what a pass IS — so the
    /// Jaccard test cannot tell it from a participant that is stuck, and the
    /// two mean opposite things. The case that would break is the ordinary one:
    /// a reviewer passing while an executor works productively. Three identical
    /// passes is one more than `SPIN_BREAK_STREAK` needs, so a detector fed by
    /// passes would have halted before the last wake below.
    ///
    /// `a_participant_repeating_itself_across_rounds_is_flagged` is the same
    /// shape with `SPOKE` in place of `PASSED`, and it DOES halt — which is
    /// what makes this a decision about the ending rather than a detector that
    /// stopped working.
    #[tokio::test]
    async fn a_participant_that_passes_every_round_never_trips_spin_detection() {
        const SAME: &str = "(passed — nothing to add this round)";

        // **Two participants, and B speaks every lap** (rc3 D27). A solo ring
        // wraps on EVERY turn, so at N=1 the first pass now closes an all-pass
        // lap and the cycle yields — ending this test before it can repeat
        // anything. The SUBJECT is untouched: A passes byte-identically, round
        // after round, and must never be judged as repeating itself.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        let mut epoch = 1u64;
        for (round, tick) in [(0, "tick one"), (1, "tick two"), (2, "tick three")] {
            post(&storage, "participant", Some("a"), SAME).await;
            send(
                &tx,
                SequencerCommand::TurnComplete { participant_id: a, epoch, ending: PASSED },
            )
            .await;
            epoch += 1;
            let got = seats[1].expect(if round == 0 { 2 } else { 1 }).await;
            assert_eq!(
                got.last().map(String::as_str),
                Some(SAME),
                "an identical pass is not a spin (round {round})"
            );

            // B says something real, so the lap is not all-pass and the ring
            // comes back round to A for another identical pass.
            post(&storage, "participant", Some("b"), tick).await;
            send(
                &tx,
                SequencerCommand::TurnComplete { participant_id: b, epoch, ending: SPOKE },
            )
            .await;
            epoch += 1;
            assert_eq!(
                seats[0].expect(1).await,
                vec![tick],
                "the ring returns to A (round {round})"
            );
        }

        drop(tx);
        assert!(exited(task).await, "no halt, so the loop is still draining");
    }

    #[tokio::test]
    async fn substantive_output_resets_the_tally() {
        // The stale-done case. A votes done, then B speaks — and a vote cast
        // before that output describes a session that no longer exists. Let it
        // stand and A's stale done plus B's eventual fresh one add up to an
        // arrival neither of them voted for, halting the session with A never
        // having seen what B said.
        //
        // The shape is chosen so the two worlds DIVERGE: A's turn after B's
        // output must itself end non-done, or A re-votes done and the stale
        // vote is indistinguishable from the fresh one.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // The vote that must not survive what follows.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: DONE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"]);

        // B produces substantive output. A row, because that is what
        // substantive MEANS — and posting it is also what makes A's next wake
        // observable at all.
        post(&storage, "participant", Some("b"), "b found something").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: SPOKE },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["b found something"],
            "the ring came back to A carrying B's output"
        );
        let roster = storage.participants_for_session("s1").await.unwrap();
        assert!(
            !roster.iter().find(|p| p.id == a).unwrap().done_vote,
            "B's output cleared A's done vote — asserted on the vote itself, because \
             consensus is `false` either way at this point and would not tell the two apart"
        );

        // A reads it and has nothing to add either, but that is not a vote — it
        // is output of its own, so the tally stays at zero.
        post(&storage, "participant", Some("a"), "a replied").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["a replied"]);

        // Unread by A, so A's wake below is a wire rather than a silent step.
        post(&storage, "system", None, "host note").await;
        // B now votes done — the vote that WOULD complete a tally still holding
        // A's stale done from three turns ago. It must not: A has spoken since,
        // so the ring comes back to A instead of the session halting.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 4, ending: DONE },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["host note"],
            "A's done was cleared by B's output, so B's vote is one of two and the \
             cycle continues"
        );

        // And the tally does still ARRIVE — the reset delays consensus, it does
        // not make it unreachable. A votes done on top of B's live vote.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 5, ending: DONE },
        )
        .await;
        seats[0].quiet().await;
        seats[1].quiet().await;
        assert!(
            storage.all_active_voted_done("s1").await.unwrap(),
            "two consecutive done votes, and B's unread `host note` proves the silence \
             is the halt rather than an empty backlog"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_user_message_over_a_live_turn_clears_the_tally() {
        // Router-inventory #12's vote half, in the state the existing coverage
        // cannot observe. The clear itself is pinned twice already —
        // `the_cycle_halts_when_every_active_participant_votes_done` and
        // `a_parked_question_finishes_the_lap_then_halts` both go red without
        // it — but both reset a HALTED cycle, where the holder is already gone.
        // So `current.is_none()` could be narrowed to `holder.is_none()` and
        // both stay green — measured — which makes "the user's message clears the
        // tally" true only of a user who waited for the session to fall silent.
        //
        // **Reaching a live holder is not the rare part; arriving with a vote
        // standing is.** Several tests here already reset over a holder still in
        // flight — `a_completion_from_a_turn_the_user_restarted_is_discarded`
        // and `a_completion_from_a_superseded_turn_does_not_advance_the_ring`
        // among them — and every one of them gets there with an EMPTY tally, so
        // the narrowing changes nothing they could see. Here the user speaks
        // over a live turn with B's done standing.
        //
        // The vote left standing is B's and the participant woken is A, so the
        // clear also has to be session-wide rather than a reset of whoever is
        // being woken.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"], "epoch 2, B holds");

        // A row for A to be woken by, and then B's done vote — the tally the
        // user's message has to clear.
        post(&storage, "user", None, "note for a").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: DONE },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["note for a"],
            "one done vote of two is not consensus — epoch 3, A holds"
        );
        let roster = storage.participants_for_session("s1").await.unwrap();
        assert!(
            roster.iter().find(|p| p.id == b).unwrap().done_vote,
            "B's vote is standing with A mid-turn — the premise of this test"
        );

        // The user speaks over A's LIVE turn. Awaiting the wake is also what
        // orders the read below: the tally is emptied before `hand_over`, so a
        // wire that has arrived is a clear that has committed.
        post(&storage, "user", None, "u2").await;
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["u2"],
            "epoch 4 — the cycle restarted at the front with A still holding"
        );

        // Asserted on the VOTE rather than on consensus: A has never voted, so
        // `all_active_voted_done` is `false` on both sides of this line and
        // would not tell a cleared tally from a standing one. What that costs is
        // the behavioural half — the arrival a stale vote buys too early — and
        // that is `the_reset_survives_a_turn_that_produced_nothing` below.
        let roster = storage.participants_for_session("s1").await.unwrap();
        assert!(
            !roster.iter().find(|p| p.id == b).unwrap().done_vote,
            "the user's message cleared a vote cast before it, with a turn still in flight"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn the_reset_survives_a_turn_that_produced_nothing() {
        // Router-inventory #13 (`convergence_reset_survives_a_suppressed_forward`)
        // carried onto the turn path. There the reset was a FLAG with a
        // consumption point, and the hazard was an event producing no output
        // burning it before the forward that needed it — which is why the router
        // consumes it at the convergence stage rather than at the top.
        //
        // This loop has no flag: `advance_turn` empties the tally at the restart
        // itself, so there is nothing for a later event to burn and the property
        // holds by construction. **Which makes this a regression lock rather than
        // a guard test, and it earns no mutation of its own.** Every mutation it
        // catches — the clear deleted, narrowed to `holder.is_none()`, made
        // per-participant, or DEFERRED to the next advance the way the router's
        // flag is — is caught by `a_user_message_over_a_live_turn_clears_the_tally`
        // as well, and all but the narrowing by the two halt tests too. Measured,
        // including the deferral, which is the router's own shape.
        //
        // What it adds is the failure MODE, and that is the whole of #13's point.
        // The test above catches a stale vote as a stored `done_vote`; the same
        // vote here is a participant that is never woken and a real post that
        // never gets out — which is what "a stale streak silences the first post
        // after a user message" meant when the router had to be careful about it.
        //
        // The shape: a done vote stands, the user speaks, and the FIRST turn of
        // the restarted cycle produces nothing at all — no row, and `done: true`.
        // With the pre-message vote still standing that empty turn completes a
        // tally of two, and the cycle halts with B never woken, so B's first real
        // post after the user's message never happens.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"], "epoch 2, B holds");

        // The vote the user's message has to make stale, and a row for A to be
        // woken by when the ring comes back round.
        post(&storage, "user", None, "note for a").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: DONE },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["note for a"],
            "one done vote of two is not consensus — epoch 3, A holds"
        );

        // The user speaks.
        post(&storage, "user", None, "u2").await;
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["u2"], "epoch 4, the cycle restarted");

        // A's turn produces NOTHING — it writes no row and ends `done: true`.
        // One vote of two, so the ring steps; with B's pre-message vote still
        // standing it is two of two and B is never woken at all.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 4, ending: DONE },
        )
        .await;
        assert_eq!(
            seats[1].expect(2).await,
            vec!["note for a", "u2"],
            "the reset outlived a turn that produced nothing — and the user's row reached \
             B undiminished by it"
        );

        // And the first real post after the user's message reaches its peer,
        // which is the thing the stale vote would have silenced. **Kept for the
        // shape, not for coverage, and worth being plain about:** it is an
        // ordinary ring step, already pinned by
        // `a_completed_turn_wakes_exactly_one_participant`, and under every
        // mutation this test catches the run dies at the `expect(2)` above, so
        // these lines never execute. They finish the sentence the name makes.
        post(&storage, "participant", Some("b"), "b's answer").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 5, ending: SPOKE },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["b's answer"],
            "epoch 6 — the first real post after the user spoke came back round to A"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_failed_reset_clears_the_tally_but_leaves_the_turn_in_flight() {
        // What a user message does when the ring read under it FAILS —
        // [`Handover::Held`] on the reset path. The clear runs before the read,
        // so the two halves of a reset come apart: the votes go, the ring does
        // not rewind, and the turn that was in flight stays in flight.
        //
        // This was documented as unreachable from a test. It is not: `Storage`
        // hands out its pool, and the two statements do not need the same
        // columns — `next_active_participant` selects `PARTICIPANT_COLUMNS`
        // while `clear_done_votes` is a bare `UPDATE ... SET done_vote = 0`, so
        // renaming a column only the SELECT names breaks the ring read alone.
        // Without this, moving the clear into the `Handover::To` arm below —
        // which is the shape that makes a failed reset clear nothing — passes
        // the whole suite.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        // A's done vote — the tally the user's message has to clear.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: DONE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"], "epoch 2, B holds");

        // Break the ring read, and only it.
        sqlx::query("ALTER TABLE session_participants RENAME COLUMN display_name TO dn_x")
            .execute(storage.pool())
            .await
            .unwrap();

        post(&storage, "user", None, "u2").await;
        send(&tx, user_message()).await;
        // Nothing was handed out, so there is no wire to synchronise on: the
        // silence IS the failed half. A has `u2` past its cursor, so a rewind
        // that had happened would have delivered it here.
        seats[0].quiet().await;
        // Read back with raw SQL — `participants_for_session` selects the column
        // this test renamed.
        let voted: (i64,) =
            sqlx::query_as("SELECT done_vote FROM session_participants WHERE id = ?")
                .bind(a)
                .fetch_one(storage.pool())
                .await
                .unwrap();
        assert_eq!(
            voted.0, 0,
            "the tally is emptied BEFORE the ring is read, so a read that failed still \
             cleared it"
        );

        // The failure was transient. What it left behind is not: the epoch never
        // moved, so B's pre-message turn is still the turn in flight.
        sqlx::query("ALTER TABLE session_participants RENAME COLUMN dn_x TO display_name")
            .execute(storage.pool())
            .await
            .unwrap();
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: DONE },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["u2"],
            "B's completion was ACCEPTED — a successful reset would have moved the epoch \
             and discarded it, vote and all"
        );

        // And here is what that costs, which is #13's hazard reached down the
        // other path: B's vote was cast about a turn that ended before the user
        // spoke, and it counts. A's done makes two of two.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, ending: DONE },
        )
        .await;
        seats[0].quiet().await;
        seats[1].quiet().await;
        assert!(
            storage.all_active_voted_done("s1").await.unwrap(),
            "the cycle arrived on a vote cast before the user's message"
        );
        assert!(
            !storage.unread_for_participant(b).await.unwrap().rows.is_empty(),
            "and B never read the message that was supposed to restart the cycle — one \
             lap lost, which is the price of holding the turn rather than halting"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_discarded_completions_vote_does_not_count_toward_the_tally() {
        // The guard gates the VOTE as well as the ring step. A completion that
        // does not name the turn in flight is an opinion about a turn that no
        // longer exists, and counting it would let a completion the loop
        // discarded do the one thing discarding it was meant to prevent —
        // arrive at a halt on a vote nobody currently holding a turn cast.
        //
        // Both halves of the guard reject the injected completion here (dead
        // epoch, and not the holder), so this pins the vote against the guard
        // as a whole rather than against either half.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // B is not holding the turn and epoch 0 is spent. Discarded.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 0, ending: DONE },
        )
        .await;
        // Unread by B, so B's wake below is a wire rather than a silent step.
        post(&storage, "system", None, "host note").await;

        // A votes done — the live half of a tally that would be COMPLETE if
        // B's discarded vote had been recorded.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: DONE },
        )
        .await;
        assert_eq!(
            seats[1].expect(2).await,
            vec!["go", "host note"],
            "one live vote of two: the ring steps to B, which has still not voted"
        );
        assert!(
            !storage.all_active_voted_done("s1").await.unwrap(),
            "the discarded completion left no vote behind"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn the_summonable_do_not_vote() {
        // Only the rotation votes. An `on_mention` participant is skipped in it,
        // so it never gets a ring turn, so it can never declare done — count them and one active plus three watchers would need four
        // yields to halt, which is a session that never halts at all.
        //
        // One active and two non-voters here: consensus has to arrive on A's
        // single done.
        let (deps, storage, mut seats) = ring(&[
            ("a", "active"),
            ("watcher", "on_mention"),
            ("helper", "on_mention"),
        ])
        .await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // Unread by all three when the vote lands. A ring of one WRAPS, so a
        // sequencer that did not halt would hand A the turn straight back and
        // deliver this row; without it A's silence would be ambiguous.
        post(&storage, "system", None, "host note").await;
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence has to be the halt, not an empty backlog"
        );

        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: DONE },
        )
        .await;
        seats[0].quiet().await;
        seats[1].quiet().await;
        seats[2].quiet().await;

        // The claim in the name, stated directly: the session has arrived while
        // two of its three participants have not voted, and cannot have.
        assert!(storage.all_active_voted_done("s1").await.unwrap());
        let roster = storage.participants_for_session("s1").await.unwrap();
        let silent: Vec<&str> = roster
            .iter()
            .filter(|p| !p.done_vote)
            .map(|p| p.slug.as_str())
            .collect();
        assert_eq!(
            silent,
            vec!["watcher", "helper"],
            "consensus arrived on the rotation's one vote, with neither non-voter having cast one"
        );

        // Halted, not wedged.
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["host note"]);
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_declared_halt_stops_the_ring_where_it_stands() {
        // **Changed subject at rc3 D35 — it used to be "finishes the lap, then
        // halts" (D22).** The user watched that lap put peers to work under a
        // ⏸ HALT banner and overruled it: *"HALT doesn't halt the agents...
        // A halt is a halt. Still working means still working."* D22's original
        // defect (a first-turn park making peers unreachable) cannot return
        // through this door: an ordinary QUESTION no longer reaches the ring at
        // all — only a declared halt does, and a halt means stop.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"], "epoch 2, B holds");

        // B votes done and hands back; A holds epoch 3. B's vote must SURVIVE
        // the halt below, because it is what the release has to clear.
        post(&storage, "user", None, "note for a").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: DONE },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["note for a"],
            "one done vote of two is not consensus — epoch 3, A holds"
        );

        // Unread by B when the halt lands, so B's silence below is the halt
        // rather than a ring step that found nothing to hand over.
        post(&storage, "user", None, "note for b").await;
        assert!(
            !storage.unread_for_participant(b).await.unwrap().rows.is_empty(),
            "the silence has to be the halt, not an empty backlog"
        );

        // A declares a halt. The ring stops HERE: B gets nothing, whatever is
        // sitting unread for it. (Under D22 this dealt B one more turn first.)
        send(&tx, halt_by(a)).await;
        seats[1].quiet().await;
        seats[0].quiet().await;
        assert!(
            !storage.all_active_voted_done("s1").await.unwrap(),
            "the cycle stopped one vote short — a halt, not consensus"
        );

        // The halt moved the epoch, so A's stale completion for the turn the
        // halt ended is discarded — taken as live it would clear the tally and
        // step the ring.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, ending: SPOKE },
        )
        .await;
        seats[1].quiet().await;
        seats[0].quiet().await;
        let roster = storage.participants_for_session("s1").await.unwrap();
        assert!(
            roster.iter().find(|p| p.id == b).unwrap().done_vote,
            "and the discarded completion cleared no vote"
        );

        // Halted, not dead: the user's message restarts the cycle at the front,
        // and the backlog the halt held is what the first turn reads.
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["note for b"],
            "the release deals from the front, backlog intact"
        );

        // The release CLEARED the tally: B's done was standing when the halt
        // landed. If it survived the restart, A's first `done: true` would
        // complete a tally of two and halt the cycle again with B never having
        // taken a turn after the release.
        post(&storage, "user", None, "note for b again").await;
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 5,
                ending: DONE,
            },
        )
        .await;
        assert_eq!(
            seats[1].expect(3).await,
            vec!["note for a", "note for b", "note for b again"],
            "one fresh done vote is not consensus — the stale vote was cleared. And \
             B's backlog is everything since ITS last turn: the halt held the rows, \
             it did not spend them on a lap the user never saw"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn cursors_do_not_advance_once_the_cycle_has_yielded() {
        // Router-inventory #4 (`awaiting_suppresses_forward`) carried onto the
        // turn path. There it was a forward the router declined to push; here
        // there is nothing to suppress, because a halted cycle hands out no
        // turns — so the behaviour shows up on the CURSORS, which is a durable
        // artefact rather than a wire that was not sent.
        //
        // **A SOLO ring, since rc3 D22.** A park no longer freezes the cycle
        // where it stands — it yields the asker's turn and the rotation finishes
        // its lap, which is a different property with its own test. What survives
        // unchanged is this one: once the rotation has come back to somebody
        // waiting on the user, nothing moves. At N=1 the ring steps from A to A,
        // so the lap is over the instant the park lands and the two properties do
        // not have to be untangled to observe either.
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "delivery is live in this run"
        );
        send(&tx, halt_by(a)).await;

        // Written while the session is awaiting. Nobody may be handed it.
        post(&storage, "user", None, "while awaiting").await;
        // Both doors to a delivery, tried in turn. A completion for the turn the
        // park took away: live, it would step the ring onto B and move B's
        // cursor over two rows.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        // And a respawn, which delivers on arrival whenever the holder is the
        // participant that joined. A halt leaves no holder, so it must not — an
        // implementation that halted by refusing to ADVANCE while keeping the
        // holder would deliver here.
        let (input, mut joined) = late_stdin(a);
        send(
            &tx,
            SequencerCommand::ParticipantJoined { participant_id: a, input },
        )
        .await;
        joined.quiet().await;

        assert_eq!(
            storage.cursor_for(a).await.unwrap(),
            1,
            "A's cursor sits where its pre-park turn left it"
        );

        // The user answers. Cursors move again — without this the frozen pair
        // above would also be what a wedged loop looks like. The wire lands on
        // the stdin that JOINED, which is how the insert above is shown to have
        // taken effect rather than been dropped.
        send(&tx, user_message()).await;
        assert_eq!(
            joined.expect(1).await,
            vec!["while awaiting"],
            "the release hands the front of the ring the row it could not have while awaiting"
        );
        assert_eq!(storage.cursor_for(a).await.unwrap(), 2);
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_parked_question_stops_the_drain_rather_than_finishing_it() {
        // A park arriving mid-drain ENDS it, joining the user message as the
        // second command that can. The participant being fed is the one that
        // just blocked on a human, so every further row goes into a buffer in
        // front of a process that has stopped reading — and `deliver` PARKS when
        // that buffer fills, for as long as the human takes.
        //
        // Driven through `deliver_backlog` directly, with the command already on
        // the channel when the drain reaches its first row. That is where the
        // biased select reads it; a park arriving at row 5 takes the same branch.
        //
        // The backlog spans TWO batches on purpose. `break 'rows` alone ends the
        // page, so on a single-page fixture the drain returns anyway at
        // `!page.more` and the `Stop::Halted` arm's own `return` is dead weight
        // no assertion could see. Past the batch limit the outer loop would come
        // back for a second page — with the park already consumed off the
        // channel, so nothing would stop it a second time.
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let overflow = UNREAD_BATCH_LIMIT as usize + 1;
        for i in 0..overflow {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        assert!(
            storage.unread_for_participant(a).await.unwrap().more,
            "the fixture has to outgrow one batch or the `return` below is untested"
        );
        let holder = storage
            .next_active_participant("s1", None)
            .await
            .unwrap()
            .expect("the fixture's one active participant");

        // `cmd_tx` is HELD, not dropped. A closed command channel is
        // `Stop::SessionEnd` inside the drain, which stops it for a reason that
        // has nothing to do with the park — and would leave this test green with
        // the park branch deleted.
        let (cmd_tx, mut rx) = mpsc::channel(8);
        send(&cmd_tx, halt_by(a)).await;
        let mut deferred = VecDeque::new();
        deliver_backlog(&deps, &holder, &mut rx, MAX_TURN_BATCHES, &mut deferred).await;

        seats[0].quiet().await;
        assert!(
            matches!(deferred.front(), Some(SequencerCommand::HaltDeclared { .. })),
            "the drain sets the park aside for the loop rather than swallowing it — \
             the halt itself is the main loop's arm, not this function's"
        );
        assert_eq!(deferred.len(), 1);
        // Stopping costs the rows read but not delivered, and that is not a
        // loss: the cursor never moved past them, so the whole backlog is
        // re-offered when the ring comes back.
        assert_eq!(storage.cursor_for(a).await.unwrap(), 0);
        let left = storage.unread_for_participant(a).await.unwrap();
        assert!(left.more);
        assert_eq!(left.rows.first().map(|r| r.content.as_str()), Some("row 0"));
    }

    #[tokio::test]
    async fn a_park_naming_somebody_else_does_not_cut_this_turn_s_drain_short() {
        // The other half of the arm above, and it only exists because rc3 D22
        // changed what a park means. While a park stopped the cycle where it
        // stood, cutting ANY drain short was right — there was no turn left to
        // feed. It now ends only the ASKER's turn, so a park naming somebody else
        // leaves this turn live, and stopping here would hand the holder a
        // partial backlog for no reason at all.
        //
        // The command is still DEFERRED either way: the act belongs to the loop,
        // which is where the block is recorded.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        for i in 0..3 {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        let holder = storage
            .participants_for_session("s1")
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.id == a)
            .unwrap();

        let (cmd_tx, mut rx) = mpsc::channel(8);
        send(&cmd_tx, halt_by(b)).await;
        let mut deferred = VecDeque::new();
        deliver_backlog(&deps, &holder, &mut rx, MAX_TURN_BATCHES, &mut deferred).await;

        assert_eq!(
            seats[0].drain(),
            vec!["row 0", "row 1", "row 2"],
            "the whole backlog goes out: this participant is not the one that parked"
        );
        assert!(
            matches!(deferred.front(), Some(SequencerCommand::HaltDeclared { .. })),
            "and the park is still handed to the loop, which is what records the block"
        );
        assert_eq!(
            storage.cursor_for(a).await.unwrap(),
            3,
            "a completed drain moves the cursor over every row it delivered"
        );
        let _ = &mut seats[1];
    }

    #[tokio::test]
    async fn a_backlog_past_the_batch_limit_is_drained_before_the_turn_is_handed_over() {
        // `ChannelPage::more` is a normal outcome, not an edge case — 268 of
        // 384 live sessions held more rows than one batch when this was measured
        // (2026-08-10). The figure drifts; the shape does not, and it only ever
        // drifts upward. Leaving the rest for next time would start a turn on
        // context the participant is already known to be missing, and the newest
        // rows would arrive a full lap of the ring later.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        let overflow = UNREAD_BATCH_LIMIT as usize + 1;
        for i in 0..overflow {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        assert!(
            storage.unread_for_participant(b).await.unwrap().more,
            "the fixture has to outgrow one batch or this test proves nothing"
        );

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // Each wake awaited before the next command: the drain selects commands
        // first and biased, so a closed control channel would stop it mid-batch.
        send(&tx, user_message()).await;
        let _ = seats[0].expect(overflow).await;
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        let got = seats[1].expect(overflow).await;
        drop(tx);
        assert!(exited(task).await);

        // The COUNT is asserted by `expect(overflow)` itself and is not restated
        // here: short of `overflow` it times out, past it the quiescence window
        // fires. An `assert_eq!(got.len(), overflow)` at this point held for a
        // while and was tautological — `expect(n)` returns exactly `n` by
        // construction — so it read as a check on the sequencer while testing
        // the helper's `Vec::push`. What is left is what `expect` does NOT pin:
        // the rows are the right ones, in order, spanning both batches.
        assert_eq!(got.first().map(String::as_str), Some("row 0"));
        assert_eq!(
            got.last().map(String::as_str),
            Some(format!("row {}", overflow - 1).as_str())
        );
        // And the delivery was RECORDED, not just written: the cursor sits on
        // the last row, so nothing is re-offered next turn.
        assert!(
            storage
                .unread_for_participant(b)
                .await
                .unwrap()
                .rows
                .is_empty(),
            "the cursor moved with the batches"
        );
        assert!(
            storage.withheld_for_participant(b).await.unwrap().is_empty(),
            "nothing on the turn path is withheld"
        );
    }

    /// **The turn's whole backlog is ONE stdin write.** rc3, 2026-08-13.
    ///
    /// The subject is the delivery SHAPE, so it reads raw wires and follows with
    /// a quiescence window; the routing helpers deliberately cannot see the
    /// difference (see [`rows_of`]).
    ///
    /// **What was wrong.** One outgoing message is one stream-json line, and
    /// claude-code opens a turn on the first line it reads. Delivering a backlog
    /// row-at-a-time therefore did not hand a participant its backlog — it
    /// handed over row 1 and then interrupted the turn with the rest. Measured
    /// across four sessions: the user's own message arrived somewhere other than
    /// the front of the batch 37 times out of 44, including row 9 of 9. One
    /// reviewer spent its turn on a peer's test run while the user's actual
    /// instruction sat unread at the end of the batch, and the user asked "why
    /// does it feel like its not addressing my current message?".
    ///
    /// So the fixture is that session's shape: peer chatter, a host notice, and
    /// the user's instruction posted LAST. The assertion is that all three reach
    /// stdin together, in id order, with the user's row at the end — which is
    /// what makes it the thing the participant is answering rather than an
    /// interruption it may already have talked past.
    #[tokio::test]
    async fn a_turns_backlog_arrives_as_one_message() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        post(&storage, "participant", Some("b"), "a peer's turn").await;
        post(&storage, "system", None, "a host notice").await;
        post(&storage, "user", None, "what I actually want").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;

        let wires = seats[0].expect_raw(1).await;
        assert_eq!(
            wires,
            vec![[
                "[b] a peer's turn",
                "[system] a host notice",
                "[user] what I actually want",
            ]
            .join(crate::storage::WIRE_JOIN)],
            "three rows, one write, in the order the channel holds them"
        );
        assert!(
            wires[0].ends_with("[user] what I actually want"),
            "the user's row is the LAST thing the participant reads: {:?}",
            wires[0]
        );
        // The negative half, and the half that fails if the coalescing is
        // removed: `expect_raw(1)` above would take row 1 and be satisfied.
        seats[0].quiet().await;

        drop(tx);
        assert!(exited(task).await);
    }

    /// A page boundary is the only thing that splits a backlog — the companion
    /// to the test above, and what stops "one write" being read as a promise the
    /// drain cannot keep.
    ///
    /// `unread_for_participant` is bounded at [`UNREAD_BATCH_LIMIT`], so a
    /// backlog past it takes more than one read and therefore more than one
    /// write. That bound is the ONLY splitter: nothing caps a write by bytes,
    /// deliberately (see "one turn, one write" in the module doc), so 201 rows
    /// must be exactly two wires of 200 and 1 — not three, and not 201.
    #[tokio::test]
    async fn a_page_boundary_is_the_only_thing_that_splits_a_backlog() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let per_page = UNREAD_BATCH_LIMIT as usize;
        for i in 0..per_page + 1 {
            post(&storage, "user", None, &format!("row {i}")).await;
        }

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;

        let wires = seats[0].expect_raw(2).await;
        assert_eq!(
            wires[0].split(crate::storage::WIRE_JOIN).count(),
            per_page,
            "the first write is a whole page"
        );
        assert_eq!(
            wires[1],
            format!("[user] row {per_page}"),
            "and the second is the remainder, not a re-send"
        );
        seats[0].quiet().await;

        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_participant_with_no_stdin_holds_the_turn_rather_than_losing_its_rows() {
        // A spawned agent whose roster read failed has no `participant_id`, so
        // it has no entry in `inputs`. Its cursor must not move: a cursor never
        // rewinds, so advancing it here would drop those rows permanently.
        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        deps.inputs.remove(&b);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // A's wake is awaited before the completion goes in — the drain selects
        // commands first and biased, so a closed control channel would stop the
        // drain and this test's "delivery is live" anchor would be the thing
        // that broke, not B's missing stdin.
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "delivery is live in this run"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        drop(tx);
        assert!(exited(task).await);

        // A's wire is asserted ABOVE, before the completion is sent, and it is
        // not decoration: without it this test passed for a whole session
        // against a sequencer that delivered NOTHING to anyone. B receiving
        // nothing has to mean "B has no stdin", and the only way to say that is
        // to show the same run delivering to a seat that has one.
        assert_eq!(seats[1].drain(), nothing());
        assert_eq!(
            storage.cursor_for(b).await.unwrap(),
            0,
            "an undelivered backlog stays undelivered — the cursor did not move"
        );
    }

    #[tokio::test]
    async fn a_backlog_larger_than_the_stdin_buffer_lands_in_full() {
        // The only test here that lets a drain run out of buffer. Production
        // stdin is 64 slots and `deliver_batch` PARKS when it fills, so a drain
        // deep enough to outrun it parks and resumes; every other test in this
        // file runs with more slots than it posts writes and would not notice a
        // drain that dropped a page rather than waiting for one.
        //
        // **The fixture had to grow when a page became one write.** It was eight
        // rows against two slots, which was three writes' worth of pressure when
        // a row was a write and is ONE write now — the buffer would never fill,
        // and this test would have gone on passing while covering nothing. What
        // fills a 2-slot stdin today is three PAGES, so that is what it posts:
        // the reader has to free space before the third can land.
        let (deps, storage, mut seats) =
            ring_sized(&[("a", "active"), ("b", "active")], 2).await;
        let (a, b) = (seats[0].id, seats[1].id);
        let rows = 2 * UNREAD_BATCH_LIMIT as usize + 1;
        for i in 0..rows {
            post(&storage, "user", None, &format!("row {i}")).await;
        }

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        // `expect` is the reader: it drains the seat as the sequencer fills it,
        // so the parking and the unparking both happen inside this call.
        let want: Vec<String> = (0..rows).map(|i| format!("row {i}")).collect();
        assert_eq!(
            seats[0].expect(rows).await,
            want,
            "a full stdin delays a page; it does not lose one"
        );

        // And the loop was not wedged by the parking — the turn still hands over.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(seats[1].expect(rows).await, want);
        drop(tx);
        assert!(exited(task).await);

        for id in [a, b] {
            assert!(
                storage.unread_for_participant(id).await.unwrap().rows.is_empty(),
                "the cursor moved with the rows, not ahead of the ones that parked"
            );
        }
    }

    #[tokio::test]
    async fn the_batch_cap_hands_over_with_the_remainder_still_past_the_cursor() {
        // What `MAX_TURN_BATCHES` actually does at the cap. It is the backstop
        // for a writer appending faster than the drain consumes, and what it
        // does there IS the deferral the module doc rejects as a policy — so it
        // has to be pinned rather than described.
        //
        // Driven through `deliver_backlog` directly with a small `max_batches`.
        // That parameter exists for exactly this caller and had none: both
        // production sites pass the constant, and reaching the real cap needs
        // 6,401 rows.
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let cap = 2usize;
        let per_batch = UNREAD_BATCH_LIMIT as usize;
        let capped = cap * per_batch;
        for i in 0..capped + 1 {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        let holder = storage
            .next_active_participant("s1", None)
            .await
            .unwrap()
            .expect("the fixture's one active participant");
        assert_eq!(holder.id, a);

        // Held, not dropped: a closed command channel is `Stop::SessionEnd`
        // inside the drain, which would end it before the cap ever bit.
        let (_cmd_tx, mut rx) = mpsc::channel(8);
        let mut deferred = VecDeque::new();
        deliver_backlog(&deps, &holder, &mut rx, cap, &mut deferred).await;

        // Exactly `cap` batches went out — `expect` times out below that and
        // fails its quiescence window above it.
        let got = seats[0].expect(capped).await;
        assert_eq!(got.first().map(String::as_str), Some("row 0"));
        assert_eq!(
            got.last().map(String::as_str),
            Some(format!("row {}", capped - 1).as_str()),
            "the drain stopped at the cap, mid-backlog"
        );
        assert!(deferred.is_empty(), "nothing arrived to defer");

        // The half that matters: the cap DEFERS the remainder, it does not drop
        // it. The cursor sits where delivery stopped, so the rest is offered
        // again when the ring comes back round.
        let left = storage.unread_for_participant(a).await.unwrap();
        assert_eq!(
            left.rows.iter().map(|r| r.content.as_str()).collect::<Vec<_>>(),
            vec![format!("row {capped}").as_str()],
            "the remainder is still past the cursor"
        );
    }

    #[tokio::test]
    async fn a_participant_that_joins_while_holding_the_turn_gets_its_backlog_at_once() {
        // The way OUT of the frozen cycle: A was handed the turn with no stdin,
        // so nothing could be delivered and no completion can ever come back.
        // The stdin arriving is when that turn becomes deliverable — and it must
        // go out WITHOUT the ring moving, because no turn ended.
        let (mut deps, storage, mut seats) =
            ring(&[("a", "active"), ("b", "active"), ("c", "active")]).await;
        let a = seats[0].id;
        deps.inputs.remove(&a);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        // No wire to await between these two: A has no stdin yet, so the frozen
        // state has nothing to observe. Ordering is the command channel's — the
        // join is handled after the reset because it was sent after it.
        send(&tx, user_message()).await; // epoch 1, A holds, undeliverable
        let (input, mut joined) = late_stdin(a);
        send(
            &tx,
            SequencerCommand::ParticipantJoined {
                participant_id: a,
                input,
            },
        )
        .await;
        assert_eq!(
            joined.expect(1).await,
            vec!["go"],
            "the stdin arriving delivered the turn A was already holding"
        );

        // The EPOCH did not change: the turn in flight is still the one minted
        // at the reset, so the completion carrying epoch 1 is the live one. Had
        // the join stamped a new turn, this would name a stale epoch and be
        // discarded — and the wake below would never come.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(
            seats[1].expect(1).await,
            vec!["go"],
            "the ring stepped one place, A→B, from where the join left it"
        );
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[2].drain(),
            nothing(),
            "and it did not step twice: C sits past B and was never woken"
        );
    }

    #[tokio::test]
    async fn a_participant_that_joins_without_the_turn_is_registered_but_not_woken() {
        // The other half of the arm's conditional. A join is a map insert, not a
        // wake: B has a backlog the whole time, and delivering it here would put
        // B and the holder on a turn at once.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "A holds the turn — and delivery is live in this run"
        );

        let (input, mut joined) = late_stdin(b);
        send(
            &tx,
            SequencerCommand::ParticipantJoined {
                participant_id: b,
                input,
            },
        )
        .await;
        // Asserted with the control channel still OPEN. Dropping `tx` first
        // would abort any delivery the arm wrongly made, and the empty seat
        // would then prove nothing.
        joined.quiet().await;

        // The insert DID take, though — B's turn, when it comes, is delivered on
        // the stdin that arrived. Without this the silence above would also be
        // what a dropped-on-the-floor join looks like.
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: SPOKE,
            },
        )
        .await;
        assert_eq!(joined.expect(1).await, vec!["go"]);
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_second_join_replaces_the_first_and_the_turn_follows_the_new_stdin() {
        // Replace semantics, which is what a RESPAWN needs: same participant id,
        // different process. An insert that kept the first entry would keep
        // writing into the dead incarnation's pipe — silently, since a dropped
        // receiver only shows up as `deliver` returning false much later.
        let (mut deps, storage, seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let a = seats[0].id;
        deps.inputs.remove(&a);
        post(&storage, "user", None, "first").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        let (first_input, mut first_seat) = late_stdin(a);
        send(
            &tx,
            SequencerCommand::ParticipantJoined {
                participant_id: a,
                input: first_input,
            },
        )
        .await;
        assert_eq!(first_seat.expect(1).await, vec!["first"]);

        // A respawns mid-turn. Awaiting the wire above puts this row strictly
        // after the first delivery, so which stdin reads it is a fact rather
        // than a race.
        post(&storage, "user", None, "second").await;
        let (second_input, mut second_seat) = late_stdin(a);
        send(
            &tx,
            SequencerCommand::ParticipantJoined {
                participant_id: a,
                input: second_input,
            },
        )
        .await;
        assert_eq!(
            second_seat.expect(1).await,
            vec!["second"],
            "the backlog followed the stdin that arrived LAST"
        );
        // And nothing more went to the first one. Its sender was dropped by the
        // replace, but a queued wire would still be read back here.
        first_seat.quiet().await;
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_paused_session_does_not_wake_the_next_participant() {
        // Router-inventory #19's HOLD half, **on the turn path specifically**,
        // which is the whole of what #19 preserves here: a pause stops the cycle
        // where it stands, so a `TurnComplete` arriving during one must not hand
        // the next participant a turn. It says nothing about a human steering —
        // a `UserMessage` releases the pause and wakes the ring, which is what
        // the old router did too (its pause held peer FORWARDS, and a user Send
        // was always the release). `a_user_message_releases_a_pause_and_wakes_the_ring`
        // is that side; the two are not in tension.
        //
        // **Held, not discarded** is the other half of the claim and it is
        // asserted here too, because the two come apart under the obvious wrong
        // implementation. A pause written as `halt()` clears the holder and moves
        // the epoch, so A's completion would fail `TurnComplete`'s guard and its
        // work would be thrown away rather than kept — and the silence below
        // would look identical. The resume is what tells them apart: B wakes on
        // the completion the pause held, at the epoch it was minted with.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "delivery is live in this run"
        );

        // Twice, because the latch is a latch and not a counter: the single
        // resume below has to release both. Hold a duplicate pause instead of
        // exempting it and it is replayed on resume, re-pausing on the spot —
        // B's wake never comes and this test is where that shows up.
        send(&tx, SequencerCommand::Pause).await;
        send(&tx, SequencerCommand::Pause).await;
        // Unread by B when the completion lands, so B's silence is the pause
        // rather than a ring step that found nothing to hand over.
        assert!(
            !storage.unread_for_participant(b).await.unwrap().rows.is_empty(),
            "the silence has to be the pause, not an empty backlog"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        // With the control channel still OPEN — dropping `tx` here would abort
        // the very delivery a pause that did not hold would have made, and the
        // empty seat would then prove nothing.
        seats[1].quiet().await;
        assert_eq!(
            storage.cursor_for(b).await.unwrap(),
            0,
            "and nothing moved on B's behalf either"
        );

        // The completion was HELD, not thrown away: the resume hands it back to
        // the loop and the ring steps exactly as it would have.
        send(&tx, SequencerCommand::Resume).await;
        assert_eq!(
            seats[1].expect(1).await,
            vec!["go"],
            "the turn the pause held was handed over on resume, not lost with it"
        );
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[0].drain(),
            nothing(),
            "and the resume re-fed A nothing: its cursor was already past every row"
        );
    }

    #[tokio::test]
    async fn resuming_delivers_each_unread_row_exactly_once() {
        // Router-inventory #19's FLUSH half. There the router held a LIST of
        // forwards and the flush had to hand each one over exactly once; here
        // there is no list. The pause holds a CURSOR still, and the resume hands
        // the holder whatever sits past it — so exactly-once is what
        // `commit_delivery` already guarantees, moving the cursor to the highest
        // id in the batch and never rewinding.
        //
        // The second release at the end carries #19's other clause — a flush
        // racing the unpause must not double-deliver — and it is worth being
        // exact about what that buys here, because the obvious claim ("it is what
        // reads the cursor's idempotence") is wrong. **It kills no mutation of
        // its own**, measured: every way of breaking the delivery breaks the
        // `expect(2)` above first, and gating the whole `Resume` arm on the flag
        // — so the second release runs no code at all — leaves this test green.
        // It is a regression lock on the shape the router had to work for.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "row 0").await;
        post(&storage, "user", None, "row 1").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(2).await, vec!["row 0", "row 1"]);

        send(&tx, SequencerCommand::Pause).await;
        // Written while the cycle is paused, and posted BEFORE the resume goes
        // in, which is what makes the wires below a fact rather than a race.
        //
        // **No silence is asserted between here and the resume**, because none
        // would discriminate: nothing tells this loop that a row arrived (see
        // the module doc), so a pause that did nothing at all is just as quiet
        // over these two posts. What the pause is on the hook for is the line
        // after the resume.
        post(&storage, "user", None, "row 2").await;
        post(&storage, "user", None, "row 3").await;

        send(&tx, SequencerCommand::Resume).await;
        assert_eq!(
            seats[0].expect(2).await,
            vec!["row 2", "row 3"],
            "the resume hands the holder everything past its cursor"
        );

        // Paused and released a SECOND time, with nothing left past the cursor.
        // A real release rather than a stray `Resume`: the second pause is what
        // makes the loop take the same path it took above, so the silence is a
        // release that found nothing rather than an arm that declined to run.
        send(&tx, SequencerCommand::Pause).await;
        send(&tx, SequencerCommand::Resume).await;
        seats[0].quiet().await;
        assert!(
            storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the delivery was RECORDED, not just written — which is the whole of \
             what makes a second release idempotent"
        );

        // B has every row past its cursor throughout, so its silence is the
        // absence of a hand-over rather than an empty backlog.
        assert!(
            !storage.unread_for_participant(b).await.unwrap().rows.is_empty(),
            "the silence below has to be the missing turn, not an empty backlog"
        );
        drop(tx);
        assert!(exited(task).await);

        assert_eq!(
            seats[1].drain(),
            nothing(),
            "a resume finishes the HOLDER's delivery; it hands out no turn"
        );
    }

    #[tokio::test]
    async fn a_pause_stops_the_drain_rather_than_finishing_it() {
        // A pause arriving mid-drain ENDS it, joining the user message and the
        // parked question as the commands that can. The reason is the park's,
        // reached independently rather than inherited: the participant being fed
        // is the one the user has just stopped, so every further row goes into a
        // 64-slot buffer in front of a process that is not reading, and
        // `deliver` PARKS when that buffer fills — for as long as the pause
        // lasts, which is until the user says otherwise. A pause that waits out
        // the user before taking effect is not a pause.
        //
        // **What stopping costs is traced here, not assumed.** `commit_delivery`
        // records only the prefix that landed and the cursor moves to the highest
        // id in it, so the remainder stays past the cursor — the last two
        // assertions — and `Resume` re-drains from there.
        //
        // Driven through `deliver_backlog` directly, with both commands already
        // on the channel when the drain reaches its first row. The completion is
        // there to pin the ORDER: it arrived FIRST and is merely set aside, and
        // the pause still has to be what the loop handles first. Handled in
        // arrival order that completion would step the ring and hand out a fresh
        // turn — for a park the module doc prices that at one extra wake, but a
        // pause exists to stop exactly it.
        //
        // The backlog spans TWO batches for the same reason the park's test
        // does: `break 'rows` alone ends the page, so on a single-page fixture
        // the `Stop::Paused` arm's own `return` is dead weight no assertion
        // could see.
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let overflow = UNREAD_BATCH_LIMIT as usize + 1;
        for i in 0..overflow {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        assert!(
            storage.unread_for_participant(a).await.unwrap().more,
            "the fixture has to outgrow one batch or the `return` below is untested"
        );
        let holder = storage
            .next_active_participant("s1", None)
            .await
            .unwrap()
            .expect("the fixture's one active participant");

        // `cmd_tx` is HELD, not dropped. A closed command channel is
        // `Stop::SessionEnd` inside the drain, which stops it for a reason that
        // has nothing to do with the pause — and would leave this test green
        // with the pause branch deleted.
        let (cmd_tx, mut rx) = mpsc::channel(8);
        send(
            &cmd_tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        send(&cmd_tx, SequencerCommand::Pause).await;
        let mut deferred = VecDeque::new();
        deliver_backlog(&deps, &holder, &mut rx, MAX_TURN_BATCHES, &mut deferred).await;

        seats[0].quiet().await;
        assert!(
            matches!(deferred.front(), Some(SequencerCommand::Pause)),
            "the pause is set aside for the loop AHEAD of the completion that arrived \
             before it"
        );
        assert!(
            matches!(deferred.get(1), Some(SequencerCommand::TurnComplete { .. })),
            "and that completion is kept, not swallowed — the pause holds work, it \
             does not discard it"
        );
        assert_eq!(deferred.len(), 2);
        assert_eq!(storage.cursor_for(a).await.unwrap(), 0);
        let left = storage.unread_for_participant(a).await.unwrap();
        assert!(left.more);
        assert_eq!(left.rows.first().map(|r| r.content.as_str()), Some("row 0"));
    }

    #[tokio::test]
    async fn a_completion_deferred_ahead_of_a_pause_hands_out_no_turn() {
        // The behavioural half of the ordering above, end to end. A completion
        // arrives mid-drain and is set aside; the pause arrives behind it and
        // stops the drain. Dispatched in arrival order the completion would step
        // the ring and wake B — an agent starting work after the user pressed
        // stop, which is the one thing this command exists to prevent.
        //
        // **How the timing is made a fact rather than a hope.** The three
        // commands go in with nothing awaited between them: `send` on a channel
        // with room completes without yielding, and `#[tokio::test]` runs a
        // CURRENT-THREAD runtime, so the spawned loop cannot be polled until this
        // test next parks. All three are therefore on the channel before the
        // drain reads its first row, where the biased select takes them in order.
        //
        // The 201-row fixture is slack against that argument being wrong
        // somewhere: a drain that had already started would still be hundreds of
        // sends and a second page read from finishing. The one arrangement this
        // test could not see is a drain that had FINISHED first — that is the
        // loop dispatching in plain arrival order, which is not what "deferred
        // ahead of" means and is not what is being pinned.
        //
        // Without the priority the seat below is not quiet: the completion is
        // dispatched first, steps the ring, and hands B a 201-row turn while the
        // session is stopped.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        for i in 0..UNREAD_BATCH_LIMIT as usize + 1 {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        assert!(
            !storage.unread_for_participant(b).await.unwrap().rows.is_empty(),
            "the silence has to be the pause, not an empty backlog"
        );

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        send(&tx, SequencerCommand::Pause).await;
        // With the control channel still OPEN, as everywhere else here.
        seats[1].quiet().await;
        assert_eq!(
            storage.cursor_for(b).await.unwrap(),
            0,
            "B was handed no turn, so its cursor never moved"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_pause_over_a_halted_cycle_hands_out_no_turn_on_resume() {
        // The third state a pause can arrive in, after "a turn in flight" and
        // "mid-drain": already halted. A halt is released by a user message and
        // by nothing else, and a resume must not quietly become a second release
        // — `Resume` finishes the HOLDER's delivery and replays what the pause
        // held, and a halted cycle has neither.
        //
        // **A regression lock, not a red-first guard, and worth being plain
        // about.** With Pause and Resume as no-ops this passes for the trivial
        // reason that a halted cycle is quiet anyway.
        //
        // Its EXCLUSIVE catch is narrower than "a resume written as an advance":
        // a resume that advances while a holder exists is caught by
        // `a_paused_session_does_not_wake_the_next_participant` too, since there
        // the resume would wake B twice over. What only this test sees is a
        // resume that advances **when there is no holder at all** — the halted
        // case, where `advance_turn` would read `None` as "reset to the front"
        // and wake the ring with the user never having answered the question that
        // halted it.
        // A SOLO ring, since rc3 D22: a park yields the asker's turn and the
        // rotation finishes its lap, so at N=2 the cycle is not halted straight
        // after the park — B still has a turn coming. At N=1 the ring steps from
        // A to A, which is already blocked, so the halt is immediate and this
        // test can be about the pause rather than about the lap.
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "delivery is live in this run"
        );
        send(&tx, halt_by(a)).await;

        // Unread, so any silence below is the halt holding rather than a ring
        // step that found nothing to hand over.
        post(&storage, "user", None, "while halted").await;
        send(&tx, SequencerCommand::Pause).await;
        send(&tx, SequencerCommand::Resume).await;
        seats[0].quiet().await;
        assert_eq!(
            storage.cursor_for(a).await.unwrap(),
            1,
            "A's cursor sits where its pre-halt turn left it"
        );

        // Halted, not dead — and it is still the USER's message that releases
        // it, which is what says the silence above was the halt rather than a
        // wedged loop.
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["while halted"]);
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_pause_racing_a_resume_re_holds_what_the_resume_replayed() {
        // Why the replay goes onto the deferred queue BEFORE the resume's own
        // delivery rather than after it. The delivery can read commands — that
        // is the whole point of the select it runs under — so a user pressing
        // stop again while the resume is still feeding lands a fresh `Pause` on
        // the FRONT of the queue. Replayed first, the held commands sit behind
        // that pause and are held again. Replayed afterwards they would sit
        // AHEAD of it, and the completion the first pause held would step the
        // ring during the second one.
        //
        // Timed the same way as
        // `a_completion_deferred_ahead_of_a_pause_hands_out_no_turn`: nothing is
        // awaited between the sends, so on the current-thread test runtime all
        // five are on the channel before the loop is polled at all. Every drain
        // here therefore meets a command at its first row, which is where the
        // biased select reads it — including the resume's own.
        //
        // **Green before this task's change, for the trivial reason that a
        // no-op pause never replays anything.** What it is here for is the one
        // mutation the other five miss: replay the held queue AFTER the resume's
        // delivery instead of before it and this seat wakes.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        for i in 0..UNREAD_BATCH_LIMIT as usize + 1 {
            post(&storage, "user", None, &format!("row {i}")).await;
        }

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        send(&tx, SequencerCommand::Pause).await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        // The resume replays that completion — and the second pause reaches the
        // resume's own delivery before it has handed A a single row.
        send(&tx, SequencerCommand::Resume).await;
        send(&tx, SequencerCommand::Pause).await;
        assert!(
            !storage.unread_for_participant(b).await.unwrap().rows.is_empty(),
            "the silence has to be the second pause, not an empty backlog"
        );
        seats[1].quiet().await;
        assert_eq!(
            storage.cursor_for(b).await.unwrap(),
            0,
            "the replayed completion was re-held by the second pause, so B was \
             handed no turn"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_user_message_releases_a_pause_and_wakes_the_ring() {
        // The steer. `AppState::resume_session` is a broadcast, so the Paused
        // bar's Resume button reaches this loop as a `UserMessage` and nothing
        // mints `Resume` at all — hold this command and the pause is
        // unreleasable by the only affordance the UI has. `state` and
        // `ActivityTracker::set_paused` both already say a user Send clears the
        // latch, and both role prompts promise the agents that the bridge halts
        // "until the next user message".
        //
        // The release is the full one: the ring resets to the front and the
        // front is WOKEN, not merely unblocked. Nothing here contradicts
        // inventory #19 — that is about waking the next participant off a TURN,
        // and this wake is the user's own.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "delivery is live in this run"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"], "epoch 2, B holds");

        // Paused mid-cycle with B holding, and a row written while stopped.
        send(&tx, SequencerCommand::Pause).await;
        post(&storage, "user", None, "u2").await;

        // The steer. It releases the latch AND does what a user message always
        // does: back to the front of the ring, carrying the row.
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["u2"],
            "the user's message released the pause and restarted the cycle at the front"
        );

        // And the release is durable rather than a single command slipping
        // through: the cycle runs on. Without the release this completion is
        // held and B is never woken.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, ending: SPOKE },
        )
        .await;
        assert_eq!(
            seats[1].expect(1).await,
            vec!["u2"],
            "the cycle is running, not just one command deep past the latch"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn the_pause_replays_what_it_held_in_arrival_order() {
        // The held queue comes back in the order it went in, and the misordering
        // is UNSAFE rather than untidy.
        //
        // The park here names **B**, which is not the participant holding the
        // turn — so it records a block rather than ending a turn (rc3 D22).
        // Replayed FIRST, the completion behind it then steps the ring into that
        // block, and the cycle yields with B never woken: it is waiting on the
        // user, so there is nothing it could do with a turn. Replayed the other
        // way round the completion steps the ring onto B before anything knows B
        // is blocked, and B is handed a turn it cannot use.
        //
        // The pair used to be park-then-completion with the park naming the
        // HOLDER, which no longer distinguishes anything: under D22 both orders
        // end with the ring one step along and A recorded as blocked. Naming the
        // participant the ring is about to reach is what keeps the ordering
        // observable.
        //
        // Two held commands is the minimum that can be misordered, and this is
        // the only test here that holds two. Both order-destroying mutations of
        // `release_held` — a pure reversal, and pop/push swapped — are invisible
        // to every other test in this file.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "delivery is live in this run"
        );

        // Unread by B throughout, so its silence is the halt rather than a ring
        // step that found nothing to hand over.
        assert!(
            !storage.unread_for_participant(b).await.unwrap().rows.is_empty(),
            "the silence has to be the replayed halt, not an empty backlog"
        );

        // Stopped, then two commands caught by the pause: the park FIRST, the
        // completion behind it.
        send(&tx, SequencerCommand::Pause).await;
        send(&tx, halt_by(b)).await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        send(&tx, SequencerCommand::Resume).await;
        // With the control channel still OPEN.
        seats[1].quiet().await;
        assert_eq!(
            storage.cursor_for(b).await.unwrap(),
            0,
            "the park was replayed FIRST, so the ring stepped into a participant \
             already known to be waiting on the user, and woke nobody"
        );

        // And the replay really happened — the park took effect rather than
        // everything being dropped. Without this the silence above would also be
        // what a resume that discarded its queue looks like: the cycle is halted,
        // and it is the user's message that restarts it. The row is posted first
        // because A's cursor is already past `go`, so the restart would otherwise
        // be a silent step.
        post(&storage, "user", None, "u2").await;
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["u2"],
            "the halt the pause held was applied, and the user's message released it"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_pause_behind_a_user_message_still_holds_the_cycle() {
        // The release fires where the message is READ, not where it is
        // dispatched — the same rule the drain's pause deferral follows. A user
        // message that reached the loop, released a pause and was re-queued
        // behind the commands that pause had held is dispatched LATER, and by
        // then a second Stop may have arrived. Letting it release again would
        // silently cancel that Stop: the user's last action was to stop the
        // session and the session would be running.
        //
        // Wire order here is Pause, TurnComplete, UserMessage, Pause. Nothing is
        // awaited between the four, so on the current-thread test runtime they
        // are all on the channel before the loop is polled — the first three set
        // up the re-queue, and the fourth arrives while the replayed completion's
        // own delivery is running, which is where the drain reads it.
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let a = seats[0].id;
        for i in 0..UNREAD_BATCH_LIMIT as usize + 1 {
            post(&storage, "user", None, &format!("row {i}")).await;
        }

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        let drained = seats[0].expect(UNREAD_BATCH_LIMIT as usize + 1).await;
        assert_eq!(drained.first().map(String::as_str), Some("row 0"));

        // A row for the re-dispatched user message to wake the front WITH, so a
        // release that wrongly fired twice is a wire rather than a silent step.
        post(&storage, "user", None, "u2").await;
        send(&tx, SequencerCommand::Pause).await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        send(&tx, user_message()).await;
        send(&tx, SequencerCommand::Pause).await;

        // A is the front of the ring, so a second release would reset to A and
        // hand it `u2`. It must not: the last thing the user did was stop.
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence has to be the second pause, not an empty backlog"
        );
        seats[0].quiet().await;
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn the_replay_is_dispatched_ahead_of_what_the_drain_had_already_deferred() {
        // `release_held` splices the held queue AHEAD of whatever is already
        // deferred, and this is the one arrangement where the two operand orders
        // differ. They agree whenever `deferred` is empty, which is every
        // ordinary state: `held` is non-empty only while paused, the loop drains
        // `deferred` completely before it calls `recv` again, and a drain only
        // ever runs unpaused.
        //
        // So the exception needs a drain to set commands aside and THEN stop on
        // a pause, with a `Resume` among the ones it set aside. Wire order is
        // `X, Resume, Y, Pause`, which the drain turns into
        // `[Pause, X, Resume, Y]` — the pause goes to the FRONT, everything else
        // to the back. The pause latches, `X` is held, and the resume then
        // releases with `held = [X]` and `deferred = [Y]`. `X` was read before
        // `Y`, so `X` must be dispatched first; spliced behind, the two swap.
        //
        // **`X` and `Y` are two completions naming the same live turn, differing
        // only in `done`, and that pair is what makes the swap observable.** The
        // module doc called this untestable for a while, having enumerated the
        // commands a drain merely sets aside — a completion, a join, a `Resume`
        // — and then looked for a pair of DIFFERENT ones. The pair that works is
        // a completion twice over. Both are live, so whichever is dispatched
        // first decides the cycle and the other names a turn that no longer
        // exists:
        //
        // - `X` (`done: true`) completes the tally B voted into at epoch 2, so
        //   the cycle halts and B is never woken;
        // - `Y` (`done: false`) clears that tally and steps the ring, handing B
        //   a turn.
        //
        // Spliced behind, this seat wakes with "row 0".
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "delivery is live in this run"
        );
        // `done: false`, so the step to B leaves the tally empty and B's vote
        // below is the only one standing. That is what leaves `X` one vote short
        // of consensus and `Y` one clear away from it.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[1].expect(1).await, vec!["go"], "epoch 2, B holds");

        // A backlog for A's epoch-3 turn to drain. Without one the drain returns
        // on an empty page BEFORE its select ever runs, the four commands are
        // then handled in plain arrival order with `deferred` empty throughout,
        // and both splices agree — the state this test exists for is unbuilt.
        for i in 0..UNREAD_BATCH_LIMIT as usize + 1 {
            post(&storage, "user", None, &format!("row {i}")).await;
        }
        assert!(
            !storage.unread_for_participant(b).await.unwrap().rows.is_empty(),
            "the silence below has to be the halt, not an empty backlog"
        );

        // Nothing awaited between the five, so on the current-thread test runtime
        // they are all on the channel before the loop is polled: B's completion
        // steps the ring to A at epoch 3, and A's drain meets the other four at
        // its first row, where the biased select reads them in wire order.
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: DONE },
        )
        .await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, ending: DONE },
        )
        .await; // X
        send(&tx, SequencerCommand::Resume).await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, ending: SPOKE },
        )
        .await; // Y
        send(&tx, SequencerCommand::Pause).await;

        // With the control channel still OPEN, as everywhere else here.
        seats[1].quiet().await;
        assert!(
            storage.all_active_voted_done("s1").await.unwrap(),
            "X was dispatched first: it completed the tally B voted into, and Y — \
             read after it — was discarded rather than clearing it"
        );
        assert_eq!(
            storage.cursor_for(b).await.unwrap(),
            1,
            "B was handed no turn, so its cursor sits where epoch 2 left it"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn every_command_is_consumed_and_the_loop_comes_back_for_more() {
        // "No-op" has to mean the loop CONSUMED the command and looped, not
        // that the first one ended the task or panicked an arm nobody wrote.
        // Buffered sends are drained before `recv()` reports the close, so
        // reaching the exit is proof all six were handled.
        let (deps, _storage, seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let (joined_input, _joined_seat) = late_stdin(a);
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        for cmd in [
            SequencerCommand::TurnComplete { participant_id: a, epoch: 0, ending: SPOKE },
            user_message(),
            SequencerCommand::ParticipantJoined { participant_id: a, input: joined_input },
            parked_by_nobody(),
            SequencerCommand::Pause,
            SequencerCommand::Resume,
        ] {
            send(&tx, cmd).await;
        }
        drop(tx);
        assert!(
            exited(task).await,
            "a command must not end the loop, and must not panic an arm"
        );
    }

    /// Router inventory **#2**, reframed for the ring. One participant saying
    /// the same thing round after round is the repetition the convergence
    /// breaker existed to stop; the cross-agent echo the original test used is
    /// impossible here, because exactly one participant holds the turn.
    ///
    /// A one-participant ring is what isolates it: the ring comes straight back
    /// to A, so three of A's turns are three commands rather than six. A never
    /// reads its own rows — `channel_page` excludes them — so each wake is
    /// driven by a host row posted alongside, which is also what makes the
    /// silence at the end mean something.
    #[tokio::test]
    async fn a_participant_repeating_itself_across_rounds_is_flagged() {
        const SAME: &str = "still waiting on the parser fix before i can continue";

        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // Round one is a BASELINE, not a score — there is nothing to be similar
        // to yet. Collapse `SpinState::last`'s `None` arm into an empty set and
        // this stops being true: `jaccard_from_sets` scores two empty sets as
        // 1.0, so a first turn of pure punctuation would trip against nothing.
        post(&storage, "participant", Some("a"), SAME).await;
        post(&storage, "system", None, "tick one").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[0].expect(1).await, vec!["tick one"], "epoch 2, A holds again");

        // One repeat is a streak of ONE — under `SPIN_BREAK_STREAK`, so the ring
        // keeps moving. This is the half that stops the detector firing on a
        // participant that merely restated itself once.
        post(&storage, "participant", Some("a"), SAME).await;
        post(&storage, "system", None, "tick two").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 2, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[0].expect(1).await, vec!["tick two"], "one repeat is not yet a spin");

        // The third identical turn takes the streak to two and halts the cycle.
        post(&storage, "participant", Some("a"), SAME).await;
        post(&storage, "system", None, "tick three").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 3, ending: SPOKE },
        )
        .await;
        seats[0].quiet().await;
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence has to be the halt, not an empty backlog"
        );

        // The halt is a yield. A user message releases it AND clears the streak;
        // without the clear, the first turn of the new cycle would be judged
        // against prose from before the user spoke and halt on its own step.
        post(&storage, "user", None, "try a different angle").await;
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(2).await,
            vec!["tick three", "try a different angle"],
            "the release wakes A with everything past its cursor"
        );

        // The same text a fourth time. With the streak cleared this is a
        // baseline again, so the ring steps instead of halting — which is what
        // proves the clear happened, rather than the detector going quiet.
        post(&storage, "participant", Some("a"), SAME).await;
        post(&storage, "system", None, "tick four").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 5, ending: SPOKE },
        )
        .await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["tick four"],
            "a user message cleared the streak, so this repeat starts a new count"
        );

        drop(tx);
        assert!(exited(task).await, "the loop outlives a spin halt");
    }

    /// **An all-pass yield leaves nobody busy and the input open.** The
    /// s-f6a441ff complaint, verbatim: "all agents passed, but input is still
    /// locked (I had to press Pause/Stop)". Whatever else held that lock (a
    /// self-woken background continuation is the prime suspect — carried_epoch
    /// 0, no turn-opened line), the RING's own contract is pinned here: a lap
    /// of passes yields with every ring-set flag clear, the tracker derives
    /// Idle, and the box opens.
    #[tokio::test]
    async fn an_all_pass_yield_leaves_the_input_open() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        let act = tracker(&["a", "b"]).await;
        deps.activity = Some(Arc::clone(&act));
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        deps.bridge = Some(Arc::clone(&bridge));
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // A passes: the pass posts its row (as pass_turn does) and the pump
        // clears A's flag at turn end — both simulated faithfully.
        post(&storage, "participant", Some("a"), "(passed — nothing to add this round)").await;
        act.set_busy_slug("a", false);
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: a,
                epoch: 1,
                ending: crate::core::sequencer::TurnEnding::Passed,
            },
        )
        .await;
        let _ = seats[1].expect(2).await;
        assert!(act.is_busy_slug("b"), "the ring dealt B its turn");

        // B passes too — a full lap of passes: the cycle yields.
        post(&storage, "participant", Some("b"), "(passed — nothing to add this round)").await;
        act.set_busy_slug("b", false);
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: b,
                epoch: 2,
                ending: crate::core::sequencer::TurnEnding::Passed,
            },
        )
        .await;
        seats[0].quiet().await;
        seats[1].quiet().await;

        assert!(
            !act.is_busy_slug("a") && !act.is_busy_slug("b"),
            "a yielded ring marks nobody working (a={}, b={})",
            act.is_busy_slug("a"),
            act.is_busy_slug("b")
        );
        assert_eq!(
            act.current(),
            crate::core::activity::SessionActivity::Idle,
            "the yield is when the user gets the floor — the input must open"
        );
        // And the yield said so on screen (D27's notice row).
        let notices: Vec<String> = storage
            .messages_for_session("s1", None)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.content.contains("Every participant passed"))
            .map(|m| m.content)
            .collect();
        assert_eq!(notices.len(), 1, "one all-passed notice: {notices:?}");
        // Every stop is a HALT (2026-08-15): the yield fills the session's
        // halt slot so even the laziest stop wears the banner.
        let halt = storage.session_halt("s1").await.unwrap();
        assert!(
            halt.as_ref()
                .is_some_and(|(by, reason, _)| by == "system"
                    && reason.contains("Every participant passed")),
            "the yield fills the halt slot: {halt:?}"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    /// Pull the next `StagedDeliveryDue` off a bridge subscription, skipping
    /// unrelated traffic, or `None` after a short deadline.
    async fn next_due(
        rx: &mut tokio::sync::broadcast::Receiver<crate::signaling::SignalingEvent>,
    ) -> Option<String> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1500);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(crate::signaling::SignalingEvent::StagedDeliveryDue { session_id })) => {
                    return Some(session_id)
                }
                Ok(Ok(_)) => continue,
                _ => return None,
            }
        }
    }

    /// **The Stage toggle's core promise: a staged response lands at a turn
    /// boundary, never mid-turn.** The holder's turn runs to completion
    /// untouched; the boundary then PARKS instead of dealing and hands
    /// delivery to the app, whose ordinary user message deals to the front.
    #[tokio::test]
    async fn a_staged_response_delivers_at_the_boundary_not_mid_turn() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let a = seats[0].id;
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        deps.bridge = Some(Arc::clone(&bridge));
        let mut events = bridge.subscribe();
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // The user stages while A holds the turn: flag only — nothing fires,
        // A's turn is untouched.
        send(&tx, SequencerCommand::MessageStaged).await;
        assert!(
            next_due_quick(&mut events).await.is_none(),
            "staging mid-turn must not deliver mid-turn"
        );

        // A completes: the boundary. The ring parks (B is dealt NOTHING) and
        // hands delivery to the app layer.
        post(&storage, "participant", Some("a"), "a spoke").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(
            next_due(&mut events).await.as_deref(),
            Some("s1"),
            "the boundary hands delivery to the app"
        );
        seats[1].quiet().await;

        // The delivery arrives as an ordinary user message and deals front.
        // A's own "a spoke" row is not in its backlog, so the batch is
        // exactly the staged text.
        post(&storage, "user", None, "the staged text").await;
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["the staged text"],
            "the staged send lands like a typed one"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    /// A short-deadline variant for asserting ABSENCE without stalling tests.
    async fn next_due_quick(
        rx: &mut tokio::sync::broadcast::Receiver<crate::signaling::SignalingEvent>,
    ) -> Option<String> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(150);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(crate::signaling::SignalingEvent::StagedDeliveryDue { session_id })) => {
                    return Some(session_id)
                }
                Ok(Ok(_)) => continue,
                _ => return None,
            }
        }
    }

    /// Staging with no turn in flight has no boundary to wait for — it
    /// delivers immediately, exactly like the Send an open box offers.
    #[tokio::test]
    async fn staging_while_the_ring_is_stopped_delivers_immediately() {
        let (mut deps, storage, _seats) = ring(&[("a", "active")]).await;
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        deps.bridge = Some(Arc::clone(&bridge));
        let mut events = bridge.subscribe();

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::MessageStaged).await;
        assert_eq!(next_due(&mut events).await.as_deref(), Some("s1"));
        drop(tx);
        assert!(exited(task).await);
    }

    /// Un-toggling Stage clears the flag: the boundary deals normally and
    /// nothing ever delivers.
    #[tokio::test]
    async fn an_unstaged_message_never_delivers() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let a = seats[0].id;
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        deps.bridge = Some(Arc::clone(&bridge));
        let mut events = bridge.subscribe();
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        send(&tx, SequencerCommand::MessageStaged).await;
        send(&tx, SequencerCommand::MessageUnstaged).await;

        post(&storage, "participant", Some("a"), "a spoke").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        // The boundary deals B normally — no park, no delivery.
        let got = seats[1].expect(2).await;
        assert!(got.contains(&"a spoke".to_string()));
        assert!(
            next_due_quick(&mut events).await.is_none(),
            "an unstaged message must never deliver"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// A halt declared with a stage pending: the staged response IS the
    /// user's next message, so it delivers as the release.
    #[tokio::test]
    async fn a_staged_response_delivers_as_the_halt_release() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        deps.bridge = Some(Arc::clone(&bridge));
        let mut events = bridge.subscribe();
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        send(&tx, SequencerCommand::MessageStaged).await;
        send(&tx, SequencerCommand::HaltDeclared { participant_id: Some(a) }).await;
        assert_eq!(
            next_due(&mut events).await.as_deref(),
            Some("s1"),
            "the staged response delivers as the halt's release"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// **The spin halt says so on screen.** s-f6a441ff: the detector halted an
    /// error volley and the session just went quiet — the user could not tell
    /// settled from stalled, which is the exact gap rc3 D33's halt banner
    /// exists to close. With a bridge wired, the halt fills the session's one
    /// halt slot with the repeating participant and the reason, same route as
    /// the provider-limit and error-streak halts. (The detector itself is
    /// pinned above with no bridge — the ROW-side behaviour must not depend on
    /// this field, which is why both tests exist.)
    #[tokio::test]
    async fn a_spin_halt_fills_the_session_halt_slot() {
        const SAME: &str = "still waiting on the parser fix before i can continue";

        let (mut deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        let bridge = SignalingBridge::new();
        bridge.set_storage(storage.clone()).await;
        deps.bridge = Some(Arc::clone(&bridge));
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        for (epoch, tick) in [(1u64, "tick one"), (2, "tick two"), (3, "tick three")] {
            post(&storage, "participant", Some("a"), SAME).await;
            post(&storage, "system", None, tick).await;
            send(
                &tx,
                SequencerCommand::TurnComplete { participant_id: a, epoch, ending: SPOKE },
            )
            .await;
            if epoch < 3 {
                let _ = seats[0].expect(1).await;
            }
        }
        seats[0].quiet().await;

        let halt = storage.session_halt("s1").await.unwrap();
        assert!(
            halt.as_ref().is_some_and(|(by, reason, _)| by == "a"
                && reason.contains("repeating itself")),
            "the spin halt names the participant and the reason in the halt slot: {halt:?}"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    /// Router inventory **#3** — the false-positive guard, and the one the plan
    /// says not to skip: it is what stops spin detection eating productive work.
    ///
    /// Four varied turns is deliberately two more than `SPIN_BREAK_STREAK`
    /// needs, so a detector that scored any completion as a repeat would have
    /// halted before the last one.
    #[tokio::test]
    async fn varied_substantive_output_never_trips_the_detector() {
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        for (epoch, body, tick) in [
            (1u64, "the parser drops trailing commas on nested maps", "tick one"),
            (2, "cursor lag sits two rows behind after every retry", "tick two"),
            (3, "storage gives up after the second attempt without logging", "tick three"),
            (4, "watchdog fires before any turn has been handed out", "tick four"),
        ] {
            post(&storage, "participant", Some("a"), body).await;
            post(&storage, "system", None, tick).await;
            send(
                &tx,
                SequencerCommand::TurnComplete { participant_id: a, epoch, ending: SPOKE },
            )
            .await;
            assert_eq!(
                seats[0].expect(1).await,
                vec![tick],
                "varied output must keep the ring moving (epoch {epoch})"
            );
        }

        drop(tx);
        assert!(exited(task).await, "no halt, so the loop is still draining");
    }

    // ---- the round cap (design §1b's second backstop; rc3 D2 + D7) ---------
    //
    // The unit is the whole point. A cap counted in TURNS or in MESSAGES would
    // pass most of what follows on a ring of two — it fires, it halts, it posts
    // a row — and fire at half the work the design sized it for. So the ring is
    // two participants wherever the count is the subject, and the assertion
    // that separates the units is always a POSITIVE one: the turn that a
    // wrongly-scaled cap would have refused to hand out.

    /// Drive one turn and read the wake it produces.
    ///
    /// Posts `body` first so the woken participant has something past its
    /// cursor: a ring step delivers no wire when there is nothing to deliver,
    /// and a silence that means "nothing was unread" cannot be told from one
    /// that means "the cap fired" — the trap
    /// `the_cycle_halts_when_every_active_participant_votes_done` documents.
    ///
    /// Returns the wires so the caller can say what the wake proves.
    async fn lap_step(
        storage: &Storage,
        tx: &mpsc::Sender<SequencerCommand>,
        seat: &mut Seat,
        holder: i64,
        epoch: u64,
        body: &str,
    ) -> Vec<String> {
        post(storage, "user", None, body).await;
        send(
            tx,
            SequencerCommand::TurnComplete { participant_id: holder, epoch, ending: SPOKE },
        )
        .await;
        seat.woken().await
    }

    #[tokio::test]
    async fn the_round_cap_counts_laps_of_the_ring_not_turns() {
        // **The measurement this whole feature rests on.** A lap is one full
        // pass over the ACTIVE participants, so on a ring of two a cap of 2 is
        // FOUR turns, not two. The load-bearing assertion is the one in the
        // middle: the third turn is handed out, which is exactly the turn a cap
        // that counted turns (or messages, or rounds-per-participant) would
        // have refused.
        let (deps, storage, mut seats, _dir) =
            capped_ring(&[("a", "active"), ("b", "active")], Some(2)).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // Lap 1: A hands to B, B hands back to A. Only the second of those two
        // steps closes a lap — the first moves FORWARD through the ring.
        let w = lap_step(&storage, &tx, &mut seats[1], a, 1, "t1").await;
        assert!(w.contains(&"t1".to_string()), "A -> B is mid-lap, not a lap");
        let w = lap_step(&storage, &tx, &mut seats[0], b, 2, "t2").await;
        assert!(
            w.contains(&"t2".to_string()),
            "the ring wrapped, closing lap 1 of 2 — a cap counting TURNS would have \
             halted here, two turns in"
        );

        // Lap 2: same again, and the wrap at the end of it is the cap.
        let w = lap_step(&storage, &tx, &mut seats[1], a, 3, "t3").await;
        assert!(
            w.contains(&"t3".to_string()),
            "still mid-lap: one lap of the two is spent, so the ring keeps moving"
        );

        post(&storage, "user", None, "t4").await;
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence below has to be the cap, not an empty backlog"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 4, ending: SPOKE },
        )
        .await;
        // With the control channel still OPEN — a dropped `tx` aborts an
        // in-flight delivery, so a halt asserted that way passes with the cap
        // deleted.
        seats[0].quiet().await;
        seats[1].quiet().await;

        drop(tx);
        assert!(exited(task).await);
    }

    /// `sessions.round_number` is written, and written from the ring's own lap
    /// counter.
    ///
    /// The column has existed since migration 0044 with **no writer at all** —
    /// `MAX(round_number)` was 0 across every session ever recorded when this
    /// was found (2026-08-13), the same shape `current_turn_participant_id` had
    /// before D19b. A column nobody writes is worse than no column: it reads as
    /// an answer.
    ///
    /// Driven through the real loop rather than by calling the setter, because
    /// the claim is that the COUNTER reaches the column. Calling
    /// `set_round_number` in a test and asserting it round-trips would pass with
    /// both call sites in `advance_turn` deleted — the CL's "test the WIRE, not
    /// the halves" rule, which this codebase has paid for five times.
    #[tokio::test]
    async fn a_lap_of_the_ring_is_recorded_on_the_session() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        assert_eq!(
            storage.round_number("s1").await.unwrap(),
            0,
            "a user message starts a stretch at zero laps"
        );

        // A -> B is mid-lap; B -> A wraps and closes lap 1.
        let _ = lap_step(&storage, &tx, &mut seats[1], a, 1, "t1").await;
        assert_eq!(
            storage.round_number("s1").await.unwrap(),
            0,
            "moving forward through the ring is not a lap"
        );
        let _ = lap_step(&storage, &tx, &mut seats[0], b, 2, "t2").await;
        assert_eq!(
            storage.round_number("s1").await.unwrap(),
            1,
            "the ring wrapped, so the column says one lap"
        );

        // A user message begins a new stretch, and the column follows the
        // counter back down rather than holding a lifetime total.
        post(&storage, "user", None, "new instruction").await;
        send(&tx, user_message()).await;
        let _ = seats[0].woken().await;
        assert_eq!(
            storage.round_number("s1").await.unwrap(),
            0,
            "the count belongs to the stretch, exactly as the round cap's does"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_round_cap_of_zero_never_halts_the_cycle() {
        // `0` = the backstop is OFF, for a deliberate unattended run (D2). It
        // is the one value a plain `laps >= cap` would get exactly backwards:
        // the first lap already satisfies `>= 0`, so "off" would become "halt
        // immediately".
        //
        // Three laps is arbitrary but not decorative — it is past the point any
        // off-by-one on a zero cap could still be hiding.
        let (deps, storage, mut seats, _dir) =
            capped_ring(&[("a", "active"), ("b", "active")], Some(0)).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        for lap in 1..=3u64 {
            let to_b = format!("lap {lap} first half");
            let w = lap_step(&storage, &tx, &mut seats[1], a, lap * 2 - 1, &to_b).await;
            assert!(w.contains(&to_b), "lap {lap}: A -> B");
            let to_a = format!("lap {lap} second half");
            let w = lap_step(&storage, &tx, &mut seats[0], b, lap * 2, &to_a).await;
            assert!(
                w.contains(&to_a),
                "lap {lap} closed and the ring kept going: a zero cap is off, not instant"
            );
        }

        assert_eq!(
            notices(&storage).await,
            nothing(),
            "no halt, so nothing announced one"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn the_capped_halt_posts_a_visible_row_and_yields() {
        // rc3 decision **D7**: a silent halt is indistinguishable from a hang.
        // Both halves are asserted here because either alone is a different
        // bug — a row with no halt is a lie, and a halt with no row is the
        // notification gap D7 exists to close.
        let (deps, storage, mut seats, _dir) =
            capped_ring(&[("a", "active"), ("b", "active")], Some(1)).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        let w = lap_step(&storage, &tx, &mut seats[1], a, 1, "t1").await;
        assert!(w.contains(&"t1".to_string()), "one lap is two turns here");
        assert_eq!(
            notices(&storage).await,
            nothing(),
            "mid-lap, so nothing has been announced yet — the row below is the CAP \
             firing, not a notice this path posts on every step"
        );

        post(&storage, "user", None, "t2").await;
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence has to be the cap, not an empty backlog"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 2, ending: SPOKE },
        )
        .await;
        seats[0].quiet().await;
        seats[1].quiet().await;

        let posted = notices(&storage).await;
        assert_eq!(posted.len(), 1, "exactly one row, once: {posted:?}");
        assert!(
            posted[0].contains("round cap"),
            "the row has to say WHY the session stopped: {:?}",
            posted[0]
        );
        assert!(
            posted[0].contains("Session Settings"),
            "and where to change it, or the user's only move is to guess: {:?}",
            posted[0]
        );

        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn the_cap_fires_on_its_last_lap_and_not_before() {
        // The boundary, on its own, because `>=` off by one in either direction
        // is a live bug the tests above cannot see: they drive to the cap
        // exactly, so a cap that fired a lap EARLY and one that fired on time
        // produce the same trailing silence.
        //
        // Cap 3. Laps 1 and 2 must each be followed by a handed-out turn — a
        // positive assertion, so it cannot pass by the ring being wedged — and
        // lap 3 must not.
        let (deps, storage, mut seats, _dir) =
            capped_ring(&[("a", "active"), ("b", "active")], Some(3)).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        for lap in 1..=2u64 {
            let to_b = format!("lap {lap} to b");
            lap_step(&storage, &tx, &mut seats[1], a, lap * 2 - 1, &to_b).await;
            let to_a = format!("lap {lap} to a");
            let w = lap_step(&storage, &tx, &mut seats[0], b, lap * 2, &to_a).await;
            assert!(
                w.contains(&to_a),
                "lap {lap} of 3 closed and the turn was handed out: the cap must not \
                 fire before its last lap"
            );
        }
        assert_eq!(
            notices(&storage).await,
            nothing(),
            "two laps of three announced nothing"
        );

        // Lap 3 — the one the cap is set to.
        let w = lap_step(&storage, &tx, &mut seats[1], a, 5, "lap 3 to b").await;
        assert!(w.contains(&"lap 3 to b".to_string()), "mid-lap still runs");
        post(&storage, "user", None, "lap 3 to a").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 6, ending: SPOKE },
        )
        .await;
        seats[0].quiet().await;
        assert_eq!(notices(&storage).await.len(), 1, "and now it fires");

        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn a_user_message_starts_the_lap_count_over() {
        // The cap's halt is a YIELD, so the user's reply has to be able to
        // release it — and a counter that survived the reply would re-fire on
        // the next lap and wedge the session shut, which is the opposite of a
        // backstop. It is also the unit D2 measured in: 3,561 UNINTERRUPTED
        // stretches, not sessions.
        //
        // Cap 2. One lap is spent, then the user speaks; it must take TWO more
        // laps to fire, not one.
        let (deps, storage, mut seats, _dir) =
            capped_ring(&[("a", "active"), ("b", "active")], Some(2)).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // Lap 1 of the first stretch.
        lap_step(&storage, &tx, &mut seats[1], a, 1, "t1").await;
        lap_step(&storage, &tx, &mut seats[0], b, 2, "t2").await;

        // The user speaks over A's turn. The ring resets to the front and the
        // count goes with it; A's in-flight turn is superseded, so the epochs
        // below carry on from the reset's own step rather than from it.
        post(&storage, "user", None, "new instruction").await;
        send(&tx, user_message()).await;
        let w = seats[0].woken().await;
        assert!(
            w.contains(&"new instruction".to_string()),
            "the reset re-woke the front of the ring"
        );

        // Two more laps. With the count carried over, the FIRST of these would
        // have hit the cap.
        lap_step(&storage, &tx, &mut seats[1], a, 4, "t3").await;
        let w = lap_step(&storage, &tx, &mut seats[0], b, 5, "t4").await;
        assert!(
            w.contains(&"t4".to_string()),
            "lap 1 after the message: the pre-message lap must not still be counted"
        );
        assert_eq!(
            notices(&storage).await,
            nothing(),
            "and nothing was announced on it"
        );

        lap_step(&storage, &tx, &mut seats[1], a, 6, "t5").await;
        post(&storage, "user", None, "t6").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: b, epoch: 7, ending: SPOKE },
        )
        .await;
        seats[0].quiet().await;
        assert_eq!(
            notices(&storage).await.len(),
            1,
            "two laps after the message is the cap"
        );

        drop(tx);
        assert!(exited(task).await);
    }

    #[tokio::test]
    async fn the_round_cap_defaults_to_500_laps_and_the_session_overrides_it() {
        // The default and the whole resolution table in one place, because they
        // are one decision: `None` at every tier means "nobody set it", which is
        // the ONLY reading under which 500 is a default rather than a value the
        // user happened to be given.
        assert_eq!(
            DEFAULT_ROUND_CAP_LAPS, 500,
            "rc3 D2, converted to laps — see the constant for the measurement"
        );

        // No data dir at all: the backstop stays ARMED at its default rather
        // than resolving to `0` (off).
        let (deps, _storage, _seats) = ring(&[("a", "active")]).await;
        assert_eq!(round_cap_laps(&deps), DEFAULT_ROUND_CAP_LAPS);

        // A snapshot that sets nothing inherits the same default.
        let (deps, _storage, _seats, _dir) = capped_ring(&[("a", "active")], None).await;
        assert_eq!(round_cap_laps(&deps), DEFAULT_ROUND_CAP_LAPS);

        // And the per-session override is read from the file the gear tab
        // writes — both the ordinary case and the one that turns it off.
        let (deps, _storage, _seats, _dir) = capped_ring(&[("a", "active")], Some(40)).await;
        assert_eq!(round_cap_laps(&deps), 40);
        let (deps, _storage, _seats, _dir) = capped_ring(&[("a", "active")], Some(0)).await;
        assert_eq!(round_cap_laps(&deps), 0, "0 is a value, not an absence");
    }

    #[tokio::test]
    async fn a_snapshot_that_is_missing_or_unreadable_leaves_the_cap_armed() {
        // The two arms the test above cannot reach. It covers "no data dir"
        // (`None`, the unit-test shape) and "a snapshot that parses" — so the
        // failure paths through a data dir that EXISTS were pinned by nothing,
        // and both could be changed to return `0` with the whole lib suite
        // green. `0` means the cap is OFF, so that is not a cosmetic default:
        // it silently disarms the backstop on exactly the sessions whose state
        // is already suspect, which is the lean the constant's doc rejects in
        // as many words.
        //
        // Asserted against `DEFAULT_ROUND_CAP_LAPS` AND against `0`
        // separately. The first alone passes if the function is rewritten to
        // return some other non-zero number; the second is the property that
        // actually matters and it is worth failing on its own terms.
        let (mut deps, _storage, _seats) = ring(&[("a", "active")]).await;
        let dir = tempdir().unwrap();
        deps.data_dir = Some(dir.path().to_path_buf());

        // Arm 1 — `Ok(None)`: the data dir is real, the snapshot is not there
        // yet. Every session looks like this between spawn and the first
        // policy write.
        assert!(
            !crate::policy::session_policy::session_policy_path(dir.path(), "s1").exists(),
            "the fixture has to start with NO snapshot or this arm tests nothing"
        );
        assert_eq!(
            round_cap_laps(&deps),
            DEFAULT_ROUND_CAP_LAPS,
            "a session with no snapshot yet must keep the default cap"
        );
        assert_ne!(round_cap_laps(&deps), 0, "a missing snapshot must not read as `off`");

        // Arm 2 — `Err(_)`: the file is there and does not parse. Written as
        // raw bytes rather than through `write_session_policy`, because the
        // whole point is a file that serializer could never have produced.
        let path = crate::policy::session_policy::session_policy_path(dir.path(), "s1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "policy: {round_cap: 3\ntool_gate: [\n").unwrap();
        assert!(
            crate::policy::session_policy::read_session_policy(dir.path(), "s1").is_err(),
            "the fixture has to actually be unreadable, or this arm silently \
             re-tests the `Ok(Some(..))` path"
        );
        assert_eq!(
            round_cap_laps(&deps),
            DEFAULT_ROUND_CAP_LAPS,
            "a snapshot that cannot be read must keep the default cap"
        );
        assert_ne!(round_cap_laps(&deps), 0, "an unreadable snapshot must not read as `off`");
    }

    #[tokio::test]
    async fn a_solo_ring_spends_a_whole_lap_on_every_turn() {
        // **N=1, and it is not an edge case** — rc3's default roster is heading
        // toward one participant, so this is the configuration the product is
        // moving to. `next_in_ring` steps a one-member ring to ITSELF, where
        // the `(turn_position, id)` key is EQUAL rather than smaller, so the
        // wrap test has to be `<=`. Narrowing it to `<` turns the round cap
        // completely OFF for a solo session, and the whole lib suite stays
        // green: every other cap test runs on a ring of two, where the wrap
        // step goes strictly backwards and `<` is enough.
        //
        // Cap 2, so the assertions separate "counts laps" from "never fires":
        // turn 1 closes lap 1 and the ring keeps moving, turn 2 closes lap 2
        // and the cap fires.
        let (deps, storage, mut seats, _dir) = capped_ring(&[("a", "active")], Some(2)).await;
        let a = seats[0].id;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // Lap 1: A hands to itself. One turn IS one full pass over the active
        // participants when there is only one of them.
        let w = lap_step(&storage, &tx, &mut seats[0], a, 1, "t1").await;
        assert!(
            w.contains(&"t1".to_string()),
            "lap 1 of 2 closed and the solo ring kept its turn"
        );
        assert_eq!(notices(&storage).await, nothing(), "one lap of two announced nothing");

        // Lap 2 is the cap.
        post(&storage, "user", None, "t2").await;
        assert!(
            !storage.unread_for_participant(a).await.unwrap().rows.is_empty(),
            "the silence below has to be the cap, not an empty backlog"
        );
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 2, ending: SPOKE },
        )
        .await;
        // Control channel still OPEN — see `the_round_cap_counts_laps_of_the_ring_not_turns`.
        seats[0].quiet().await;

        let posted = notices(&storage).await;
        assert_eq!(posted.len(), 1, "the cap must fire on a solo ring too: {posted:?}");
        assert!(posted[0].contains("round cap"), "{:?}", posted[0]);

        drop(tx);
        assert!(exited(task).await);
    }

    // ---- what a completed turn means (inventory #8-#11) --------------------
    //
    // Pure-function tests, deliberately. What each row means for the RING — a
    // done vote does not wake the next participant, a non-done one steps and
    // resets the tally — is already pinned end to end by
    // `the_cycle_halts_when_every_active_participant_votes_done` and
    // `substantive_output_resets_the_tally`. What is new here is the derivation,
    // so that is what these test; asserting the ring again would re-test the
    // half that already has cover and leave the new half resting on it.

    /// Router inventory **#8**. In the router a bare `peer_ack` suppressed the
    /// forward and skipped the counters; in the ring it is a done vote, which
    /// declines to wake the next participant by ending the cycle rather than by
    /// hiding a message.
    #[test]
    fn peer_ack_suppresses_and_doesnt_count() {
        assert_eq!(turn_ending(true, false, false, "ack"), TurnEnding::Done);
    }

    /// Router inventory **#9** — the guard that exists because four full reviews
    /// were destroyed by an agent posting its verdict and acking in one turn.
    /// Over the length floor, the ack does not become a vote and the row carries
    /// the override tag.
    #[test]
    fn peer_ack_on_substantive_turn_forwards_anyway() {
        let review = "x".repeat(PEER_ACK_MAX_SUPPRESSED_LEN + 1);
        assert_eq!(
            turn_ending(true, false, false, &review),
            TurnEnding::Spoke { peer_ack_override: true }
        );
        // The floor itself is still an ack — `<=`, not `<`. One byte decides
        // which of the two rows above applies, so it is worth pinning that the
        // boundary sits where the router put it.
        let at_floor = "x".repeat(PEER_ACK_MAX_SUPPRESSED_LEN);
        assert_eq!(turn_ending(true, false, false, &at_floor), TurnEnding::Done);
    }

    /// Router inventory **#10**. `final: true` is the agent asserting this is
    /// its closing turn, and it outranks the length proxy.
    #[test]
    fn peer_ack_final_suppresses_a_substantive_turn() {
        let closing = "y".repeat(PEER_ACK_MAX_SUPPRESSED_LEN + 1);
        assert_eq!(turn_ending(true, true, false, &closing), TurnEnding::Done);
    }

    /// Router inventory **#11**, the inverse of #10 — and the pair is what makes
    /// `final` load-bearing rather than decorative: same body, one flag apart,
    /// opposite endings. A turn that never acked at all is substantive too.
    #[test]
    fn substantive_turn_without_final_still_forwards() {
        let body = "z".repeat(PEER_ACK_MAX_SUPPRESSED_LEN + 1);
        assert_eq!(
            turn_ending(true, false, false, &body),
            TurnEnding::Spoke { peer_ack_override: true }
        );
        assert_eq!(
            turn_ending(false, false, false, &body),
            TurnEnding::SPOKE,
            "no ack at all is substantive, and carries no override tag"
        );
        // Trimmed before measuring, like the router: whitespace must not push a
        // bare ack over the floor.
        let padded = format!("   {}   ", "w".repeat(PEER_ACK_MAX_SUPPRESSED_LEN));
        assert_eq!(turn_ending(true, false, false, &padded), TurnEnding::Done);
    }

    /// **The pass, as a derivation.** Design §1's third ending: a participant
    /// declining a turn rather than burning one.
    ///
    /// The first two assertions are what makes it a THIRD ending rather than a
    /// spelling of an existing one — the whole slice fails if either collapses
    /// into `Done` or `SPOKE`.
    #[test]
    fn a_content_free_pass_is_its_own_ending() {
        assert_eq!(turn_ending(false, false, true, ""), TurnEnding::Passed);
        assert_eq!(
            turn_ending(false, false, true, "  \n "),
            TurnEnding::Passed,
            "whitespace is not content — trimmed like the ack ladder"
        );
        // The floor is shared with the ack, `<=` and all: one reading of
        // "content-free", so a pass and an ack cannot disagree about a body.
        let at_floor = "p".repeat(PEER_ACK_MAX_SUPPRESSED_LEN);
        assert_eq!(turn_ending(false, false, true, &at_floor), TurnEnding::Passed);
    }

    /// A pass and an ack in one turn CONTRADICT — the ack casts a done vote,
    /// the pass casts nothing — so one wins, and it is the pass.
    ///
    /// Not a preference. `Done` can complete the tally and halt the session;
    /// `Passed` can only cost an extra lap. Reading a confused turn as the
    /// halting one parks the session on a user who was never told they are
    /// being waited on.
    #[test]
    fn a_pass_outranks_an_ack_on_the_same_turn() {
        assert_eq!(turn_ending(true, false, true, "ack"), TurnEnding::Passed);
        assert_eq!(
            turn_ending(true, true, true, "ack"),
            TurnEnding::Passed,
            "even `final: true` — the pass is the non-halting reading of the two"
        );
    }

    /// A pass over the length floor is OVERRIDDEN, exactly as #9's ack is, and
    /// the row carries **no tag** (rc3 decisions, locked).
    ///
    /// The failure this prevents is arithmetic. `Passed` is the one ending that
    /// leaves other participants' done votes standing, so a substantive turn
    /// read as a pass would carry a vote cast BEFORE it over the top of real
    /// output — the same "one stale vote and one fresh one add up to an arrival
    /// nobody voted for" that `substantive_output_resets_the_tally` exists to
    /// stop, reached down a second path.
    #[test]
    fn a_substantive_pass_is_overridden_and_carries_no_tag() {
        let verdict = "v".repeat(PEER_ACK_MAX_SUPPRESSED_LEN + 1);
        assert_eq!(
            turn_ending(false, false, true, &verdict),
            TurnEnding::SPOKE,
            "the pass is overridden, and an overridden pass is NOT tagged"
        );
        // With an ack alongside it, the ack ladder decides what the overridden
        // pass left behind — #9 and #10 unchanged, which is the point: once the
        // pass is gone there is nothing left for it to outrank.
        assert_eq!(
            turn_ending(true, false, true, &verdict),
            TurnEnding::Spoke { peer_ack_override: true },
            "#9 still applies once the pass is overridden"
        );
        assert_eq!(
            turn_ending(true, true, true, &verdict),
            TurnEnding::Done,
            "#10 still applies once the pass is overridden"
        );
    }

    // ---- task 13: the delivery record ---------------------------------------

    /// **13a, as a pin rather than a feature.** Inventory #5 asked that a
    /// suppressed delivery become a visible row instead of a preview in a side
    /// table. On the turn path that upgrade is already paid and there is nothing
    /// left to record: the message is a `messages` row (task 2), and the module
    /// doc argues the forward ladder does not survive onto a PULL — every row
    /// past a cursor is offered when the turn arrives, so nothing suppresses.
    ///
    /// That claim had no test. `withheld_reason` stays in the schema for a
    /// policy that does not exist yet, and a policy added tomorrow would change
    /// the model silently. This is the guard: the turn path withholds NOTHING,
    /// and it records what it handed over.
    #[tokio::test]
    async fn the_turn_path_records_every_delivery_and_withholds_nothing() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "one").await;
        post(&storage, "system", None, "two").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(2).await, vec!["one", "two"]);
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        assert_eq!(seats[1].expect(2).await, vec!["one", "two"]);

        for (who, id) in [("a", a), ("b", b)] {
            let (total, withheld): (i64, i64) = sqlx::query_as(
                "SELECT COUNT(*), COUNT(withheld_reason) FROM participant_deliveries \
                 WHERE participant_id = ?",
            )
            .bind(id)
            .fetch_one(storage.pool())
            .await
            .unwrap();
            assert_eq!(total, 2, "{who}: both rows recorded");
            // `COUNT(column)` skips NULLs, so zero here is "every row carries no
            // reason" — the model's claim, asserted.
            assert_eq!(withheld, 0, "{who}: the turn path withholds nothing");
        }
        assert!(
            storage.withheld_for_participant(a).await.unwrap().is_empty(),
            "and the public reader agrees with the raw count"
        );

        drop(tx);
        assert!(exited(task).await, "the loop is still draining");
    }

    /// **13b — the measurement the inventory demands, not an assumption.**
    ///
    /// Inventory #6 (`a_delivered_forward_records_nothing`) is DROPPED on
    /// purpose: the invisibility it enforced is the defect the redesign exists
    /// to fix. But it existed because someone cared about the hot path, so the
    /// cost the channel model puts back has to be measured rather than waved at.
    ///
    /// **Compared against a write the turn already pays** — one message row —
    /// because that is the honest baseline. If a delivery costs the same order
    /// as the row it delivers, the model added no new class of cost; it did not
    /// become free, and this does not claim it did.
    ///
    /// **Asserted as a ratio, never a wall-clock bound.** An absolute threshold
    /// measures whatever machine runs the suite; a ratio measures the code. One
    /// `commit_delivery` call per row, deliberately: batching is the REMEDY if
    /// this is ever hot, so measuring the batched form would measure the fix
    /// instead of the cost.
    #[tokio::test]
    async fn a_delivery_costs_the_same_order_as_the_row_it_delivers() {
        use std::time::Instant;

        const N: usize = 300;

        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1", false).await.unwrap();
        let pid = s.participant_by_slug("s1", "hands").await.unwrap().unwrap().id;

        // Warm both paths, so neither measurement pays one-time pool, statement
        // cache or page setup that the other has already paid.
        let mut warm = Vec::new();
        for _ in 0..50 {
            warm.push((
                s.post_to_channel("s1", "user", None, "text", "warm", None)
                    .await
                    .unwrap()
                    .message_id(),
                None,
            ));
        }
        s.commit_delivery(pid, &warm).await.unwrap();

        let mut ids = Vec::with_capacity(N);
        let t0 = Instant::now();
        for _ in 0..N {
            ids.push(
                s.post_to_channel("s1", "user", None, "text", "body", None)
                    .await
                    .unwrap()
                    .message_id(),
            );
        }
        let per_row = t0.elapsed() / N as u32;

        let t1 = Instant::now();
        for id in &ids {
            s.commit_delivery(pid, &[(*id, None)]).await.unwrap();
        }
        let per_delivery = t1.elapsed() / N as u32;

        assert!(
            per_delivery < per_row * 10,
            "a delivery must stay the same order as the row it delivers — row \
             {per_row:?}, delivery {per_delivery:?}. If this fired, batch the cursor \
             advance the way BatchEmitter already batches emission (50ms / N=20)."
        );
    }

    // ---- real-data smoke -----------------------------------------------------

    /// **Run by hand:**
    /// ```text
    /// cp ~/.bot-hq/.local/bot-hq.db /tmp/smoke.db
    /// cargo test --lib the_ring_runs_against_a_real_session -- --ignored --nocapture
    /// ```
    ///
    /// `#[ignore]`d because it needs a database this machine happens to have.
    /// **Point it at a COPY** — it advances cursors and writes delivery rows, and
    /// it is not worth discovering that on the file the app is using.
    ///
    /// **Why it exists.** Every other test in this file builds a two-row session
    /// in memory. Task 14 deletes `router.rs` — the batch's only irreversible
    /// step — on the strength of those. This runs the ring against a real
    /// roster, real cursors and a real channel, which is the cheapest thing that
    /// can say the loop survives contact with production shapes before that
    /// deletion, rather than after.
    ///
    /// It asserts SHAPES, not counts: the data moves. What it pins is that a
    /// cold cursor drains, the cursor lands exactly on the last row handed over,
    /// every delivery is recorded, and none is withheld.
    #[tokio::test]
    #[ignore = "needs /tmp/smoke.db — a copy of a real bot-hq database"]
    async fn the_ring_runs_against_a_real_session() {
        let db = std::path::Path::new("/tmp/smoke.db");
        if !db.exists() {
            panic!("copy a real database to {} first", db.display());
        }
        let storage = Storage::open(db).await.unwrap();

        // The session with the largest roster-backed channel, chosen from the
        // data rather than hardcoded, so this keeps working on another machine.
        let (session, backlog): (String, i64) = sqlx::query_as(
            "SELECT s.id, (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) AS n \
             FROM sessions s \
             WHERE (SELECT COUNT(*) FROM session_participants p WHERE p.session_id = s.id) > 1 \
             ORDER BY n DESC LIMIT 1",
        )
        .fetch_one(storage.pool())
        .await
        .unwrap();

        let roster = storage.participants_for_session(&session).await.unwrap();
        eprintln!("session {session}: {} participants, {backlog} rows", roster.len());

        let mut inputs = HashMap::new();
        let mut seats = Vec::new();
        for p in &roster {
            // Generous, so the drain is bounded by MAX_TURN_BATCHES rather than
            // by a full buffer — the batch cap is what this is measuring.
            let (tx, rx) = mpsc::channel(16_384);
            inputs.insert(p.id, ParticipantInput::new(session.as_str(), tx));
            seats.push(Seat { id: p.id, rx });
        }
        let first = roster.first().unwrap().id;
        let before = storage.cursor_for(first).await.unwrap();

        let deps = SequencerDeps {
            session_id: session.as_str().into(),
            storage: storage.clone(),
            inputs,
            epochs: HashMap::new(),
            data_dir: None,
            bridge: None,
            activity: None,
        };
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;

        // Drain whatever the first turn hands over, stopping on the first gap —
        // the turn is done when nothing more arrives, and a deadline per row
        // keeps a stalled loop from hanging the run.
        let seat = seats.iter_mut().find(|s| s.id == first).unwrap();
        let mut wires: i64 = 0;
        while let Ok(Some(_)) = tokio::time::timeout(DEADLINE, seat.rx.recv()).await {
            wires += 1;
        }

        let after = storage.cursor_for(first).await.unwrap();
        // The wire carries `role`/`content` and no id, so the "did the cursor
        // overshoot" question is asked of the delivery rows instead — which is
        // the stronger place to ask it anyway: those rows ARE the record of what
        // was handed over.
        let (recorded, withheld, highest): (i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COUNT(withheld_reason), COALESCE(MAX(message_id), 0) \
             FROM participant_deliveries WHERE participant_id = ?",
        )
        .bind(first)
        .fetch_one(storage.pool())
        .await
        .unwrap();

        eprintln!(
            "delivered {wires} rows; cursor {before} -> {after}; \
             recorded {recorded}, withheld {withheld}, highest {highest}"
        );

        assert!(wires > 0, "a cold cursor must drain something");
        assert!(after > before, "the cursor advanced");
        assert_eq!(
            after, highest,
            "the cursor lands exactly on the highest row recorded as delivered — \
             never past a row the participant did not get"
        );
        assert_eq!(recorded, wires, "every row handed over is recorded");
        assert_eq!(withheld, 0, "the turn path withholds nothing, on real data too");

        drop(tx);
        assert!(exited(task).await, "the loop shuts down cleanly");
    }

    /// The epoch publish itself, which nothing pinned — the round-1 review of
    /// a later task found a proposed handler that omitted it entirely and every
    /// test still green. That is the shape of defect this closes: the publish is
    /// invisible from the ring's own behaviour, because a wrong epoch only
    /// surfaces later, as a completion the guard discards and a cycle that stops
    /// with nothing in the log.
    #[tokio::test]
    async fn the_epoch_is_published_to_the_holder_before_its_rows_go_out() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        let cell_a = Arc::new(AtomicU64::new(0));
        let cell_b = Arc::new(AtomicU64::new(0));
        deps.epochs.insert(a, Arc::clone(&cell_a));
        deps.epochs.insert(b, Arc::clone(&cell_b));
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));

        send(&tx, user_message()).await;
        assert_eq!(seats[0].expect(1).await, vec!["go"]);
        // The wire is the synchronisation point: A cannot have been handed rows
        // before its epoch was stored, because the store happens first.
        assert_eq!(cell_a.load(Ordering::Acquire), 1, "A holds epoch 1");
        assert_eq!(cell_b.load(Ordering::Acquire), 0, "B has not held a turn yet");

        post(&storage, "system", None, "next").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        // B's cursor is its own and never moved for "go", so its first turn
        // carries both rows.
        assert_eq!(seats[1].expect(2).await, vec!["go", "next"]);
        assert_eq!(cell_b.load(Ordering::Acquire), 2, "the step published B's epoch");
        assert_eq!(cell_a.load(Ordering::Acquire), 1, "and left A's alone");

        drop(tx);
        assert!(exited(task).await, "the loop is still draining");
    }

    #[test]
    fn jaccard_similarity_normalizes_and_handles_edges() {
        assert_eq!(jaccard_similarity("ready to go", "ready to go"), 1.0);
        assert_eq!(jaccard_similarity("OK.", "ok"), 1.0);
        assert_eq!(jaccard_similarity(".", "."), 1.0);
        assert_eq!(jaccard_similarity("...", "—"), 1.0);
        assert_eq!(jaccard_similarity(".", "check line forty two"), 0.0);
        assert_eq!(jaccard_similarity("alpha beta", "gamma delta"), 0.0);
        let partial = jaccard_similarity("the quick brown fox", "the quick red hen");
        assert!(
            partial > 0.0 && partial < SPIN_SIMILARITY_THRESHOLD,
            "partial overlap should not trip the breaker: {partial}"
        );
    }
    /// rc3 **D35** — an open approval gate parks the ring: "Approval gate halts
    /// the session, stop overcomplicating things like halting just for the
    /// agent that asked."
    #[tokio::test]
    async fn an_open_gate_parks_the_ring_until_the_user_answers() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (a, b) = (seats[0].id, seats[1].id);
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await; // epoch 1, A holds
        assert_eq!(seats[0].expect(1).await, vec!["go"]);

        // A's gated tool call parks an approval. A still holds its turn — the
        // gate cuts nothing — but when that turn ends, the ring deals no next
        // turn: the session is halted on the gate.
        send(&tx, SequencerCommand::GateOpened).await;
        post(&storage, "user", None, "for b").await;
        send(
            &tx,
            SequencerCommand::TurnComplete { participant_id: a, epoch: 1, ending: SPOKE },
        )
        .await;
        seats[1].quiet().await;

        // Resolving the gate deals NOTHING by itself — the wake is the user's
        // release, so there is no second path onto a turn. BOTH seats must be
        // quiet: the first cut of this checked only B, and a premature deal
        // goes to the FRONT (A), whose buffered row the later expect would
        // happily consume — the mutation passed until A was pinned quiet too.
        send(&tx, SequencerCommand::GateResolved).await;
        seats[0].quiet().await;
        seats[1].quiet().await;

        // The release drains normally once the gate is lifted.
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["for b"],
            "the release deals from the front once no gate is open"
        );
        let _ = b;
        drop(tx);
        assert!(exited(task).await);
    }

    /// rc3 **D35** — a user message does NOT run the session under a pending
    /// gate: the answer to the gate is what lifts it. The message waits.
    #[tokio::test]
    async fn a_user_message_does_not_deal_under_an_open_gate() {
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, SequencerCommand::GateOpened).await;
        send(&tx, user_message()).await;
        seats[0].quiet().await;

        send(&tx, SequencerCommand::GateResolved).await;
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "lifted gate + release = the turn deals"
        );
        drop(tx);
        assert!(exited(task).await);
    }

    /// rc3 **D35** — the latch survives a restart: a ring spawned over a
    /// pending Approve/Reject row starts parked. Without the seed, a respawn
    /// would deal turns under a gate that parked before the process died.
    #[tokio::test]
    async fn the_gate_latch_seeds_from_the_durable_rows() {
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        park_gate(&storage, "c-gate-1").await;
        post(&storage, "user", None, "go").await;

        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(run_sequencer(deps, rx));
        send(&tx, user_message()).await;
        seats[0].quiet().await;

        // Answering the gate (the row flips) + the resolve notify + the release
        // is the full production sequence, in its production order.
        storage.answer_tray_entry("c-gate-1", "Approve").await.unwrap();
        send(&tx, SequencerCommand::GateResolved).await;
        send(&tx, user_message()).await;
        assert_eq!(
            seats[0].expect(1).await,
            vec!["go"],
            "the seeded latch lifts exactly like a live one"
        );
        drop(tx);
        assert!(exited(task).await);
    }

}
