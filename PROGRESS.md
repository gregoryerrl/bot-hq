# bot-hq — Change Log

Recent work, newest-first. For the rebuild-era phased status (Phases
A–9 of the from-scratch rebuild), see
[`docs/rebuild-archive/PROGRESS-through-2026-05-15.md`](docs/rebuild-archive/PROGRESS-through-2026-05-15.md).

For what bot-hq IS see [`ARCHITECTURE.md`](ARCHITECTURE.md). For what's
planned next see [`PLAN.md`](PLAN.md).

> **Reading older entries:** this is an append-only change log, so entries
> describe the tree **as it was on their date**. Anything before 2026-08-13 may
> reference subsystems since deleted — the native agent loop (rc3 D9), the
> bilateral router `core/router.rs` (task 14), the Agents settings tab (D8), the
> Maintain-CL button (D15) — and the retired agent names Brian/Rain (D10). That
> is the record, not staleness, and it is deliberately not rewritten. For current
> reality read `ARCHITECTURE.md`; for the decisions that changed it, read
> [`docs/plans/2026-08-11-rc3-decisions.md`](docs/plans/2026-08-11-rc3-decisions.md).

---

## 2026-08-16/17 — the D10 hard retirement (round-3 tail)

The user took every scope option and named the reason: *"hard retire brian and
rain, it might cause issues (maybe hallucinations that bot-hq still has brian and
rain), or maybe context corruption. Settle all your pending tasks here, no
deferrals."*

That reframed the work. The schema was never the point — **the Context Library
is**, because it is what loads into an agent's context window by design. The
reviewer caught that the investigation had inventoried the repo and missed it.

Five batches, each committed separately, each reviewed before commit:

- **`83d17a5`** — the `Author` enum deleted. It could name `user`/`brian`/`rain`
  and therefore no participant this app creates. Every production `insert_message`
  already resolved to a user row, so the participant arm was test-only.
- **`95a9124`** — the activity wire names turn slots; 42 role-shaped identifiers.
- **`891b807`** — **migration 0060**: nine `sessions` columns dropped, six renamed
  across three tables, the last CHECK naming them rebuilt away, and **F8's
  proven-fired `datetime('now')` DEFAULT** removed. Backed up first, pre-flighted
  against a copy.
- **`7623a09`** — the create-session wire, which the reviewer found still carried
  six live fields and falsified the completeness claim.
- **this** — docs, the CL sweep, the acceptance number.

**F13, the finding closest to the user's stated worry.** `request_phase_advance`
wrote its receipt as `Author::parse(&agent).unwrap_or(Author::User)` — and `parse`
knew only the retired names, so for **every current role slug it already returned
`None`**. Every agent phase request was stored `origin = "user"`, and rendered as
ordinary prose under the user's name, because `ChatMessage.tsx` has no case for
`Text`. Agents read the transcript back. **The system was manufacturing a user
utterance** — the mechanical form of the fabricated-authorization failure the
general rules exist to prevent. A rename would have preserved it exactly.

**Acceptance: `cl_stale_refs` 27 → 25**, down two despite 0060 moving fifteen
columns.

**F14 — the metric penalises the documented remedy.** Its retirement detection is
per LINE, but a retirement banner spans lines, so the symbol names land on
continuation lines carrying no marker. A correct banner *raised* the count by 3;
rewriting it so each naming line carries its own marker dropped it by 5. The
number moved 27 → 30 → 25 with the repo unchanged. Recorded, not fixed: changing
an audit tool while it serves as that audit's acceptance metric is the sequencing
argument that kept F8 out of round 2.

**Method, and the most reproducible thing this session found: a word-boundary
rename cannot tell a name in use from a name being talked about.** It over-reached
five times — twice onto sentences describing history, once into generated
`bindings.ts`, once onto the **published driver API**, and once in the reviewer's
own two-line script while reviewing this very hazard. Each was caught by a
different mechanism (the suite, a read, `tsc`, a test, a self-guard); none by the
rename. The defect belongs to the technique, not the driver.

Corollary, three instances: **changing a parameter does not prompt a re-read of
the comment above it.** And **a test that reads the thing it is proving cannot
catch that thing changing** — the driver-wire test asserted against
`wire::MODEL_ARGS` and went green on the rewritten constant.

**Completeness, measured not asserted:** zero live code identifiers name them.
They survive only in 12 immutable migrations, the driver wire (now pinned by a
test), append-only evidence (`PROGRESS.md`/`issues.md`/`decisions.md`),
`conventions.md:5` which names them in order to retire them, and legacy-data
fixtures where the retired name IS the subject.

Suites: **1239** + 392 frontend; clippy **4**; `tsc` clean; release build green.

## 2026-08-16 (round 3) — R6 finished, and an audit run against the previous round's binary

Two mandates: finish R6, and audit again. Report in
[`docs/plans/2026-08-16-rc3-audit-round3.md`](docs/plans/2026-08-16-rc3-audit-round3.md).

**R6 shipped, in the three steps round 2 recorded.** `run_sequencer` **714 → 407
lines**; `advance_turn` **11 → 4 parameters**; clippy **10 → 4**; the suite green
at every step with **zero test files touched**.

The design was right and its count was wrong: it listed 11 ring-state locals and
there are **13**. The two it omitted — `open_gates` and `gate_seed_failed` — are
exactly the pair the struct most needed, since a `HashSet` cannot express "I
could not read the gates" and the source documents both in **one comment
paragraph while declaring them as two locals**. Caught at the start of step 1, so
it cost nothing; discovered mid-step it would have been the half-finished state
the plan warned about.

Two techniques carried the risk. The rename was **compiler-driven** — collapse
the declarations, let `E0425` name all 98 sites with a column — because comments
and strings mention these words constantly and produce no error, so they could
not be touched by accident. And the 212-line arm extraction is **verified
verbatim by byte-comparison**, not asserted: two of the four substitutions had to
be narrowed (`&mut state` must not match `&mut state.holder`), and both wrong
versions compiled far enough to look plausible. The compiler caught them only
because the types differed.

**The audit's new angle: this binary was built from round 2's fixes, so they
could be checked against a running system.** Migration 0059 applied at boot;
`participant_deliveries.delivered_at` is **4370/4370 RFC3339** where round 2
measured 4011 zone-less, and `participant_cursors.updated_at` **92/92**. Both
populations *grew* in between, so live traffic — not a fixture — is what confirms
the writers.

**Deletions leave phantoms that no line-number check can see.** Rounds 1–2 closed
citation rot in `file.rs:NNN` form. This round's largest finding is the next
layer: `run_stall_watchdog`'s doc claimed it "watches the peer-forward router …
emit a router-health event once", when `router.rs` was deleted by task 14 and a
grep for that event across `src/` returns **one hit — the sentence itself**. A
reader asking whether forwarding was monitored came away believing it was.
`pump.rs` had the same shape, pointing test coverage at the deleted module.

**Two write paths had no readers.** `set_session_effort_config` (4 columns) is
gone; `set_session_spawn_config` narrowed to the one column that is read. The
create path already carried a comment saying the writes changed nothing, and the
comment justifying the other one named `respawn_session` as its reader —
a four-line function that calls `ensure_session_started`.

Also: G4's duplicate timestamp parser closed (open since round 2), four clippy
items, and the round-2 report's own two stale `OPEN` headers corrected beside
themselves.

**Left deliberately:** `participant_cursors.updated_at` still carries the
`datetime('now')` default — the one default *proven* to fire, where round 2's
stated reason for leaving the other 15 was that they never had. One table, 92
rows, versus round 2's "rebuild 13 tables"; recorded with its cost rather than
shipped mid-refactor. The frontend is still spot-read, not cleared.

**Verifying the round's own work found the last one.** F11 looked like a
formatting change — clippy's own `is_none_or` suggestion in the tray staleness
check. Mutating it back in Verify left the suite **completely green**, because
`is_none_or` and `is_some_and` differ on exactly one input: an `asked_at` that
does not parse. That branch decides whether a gated command of *unknowable age*
gets one-click approval or a confirm step, and nothing tested it. A safety choice
with no test is a comment. Now pinned, by a test that is itself mutation-verified.

Suites: **1236** total (1174 lib + 59 integration + 2 doc + the new one), the same
count as round 2 by coincidence — F7 removed a test with the writer it pinned and
F12 added one. **392** frontend; `tsc` clean; all five gates run in order.

## 2026-08-16 (round 2, later) — the three findings the report left to the user

The user took all of them, and the push. Two shipped; the third was measured and
deliberately left, with its design written down.

**B3 — a cap enforced on one path of three.** `MAX_SESSION_PARTICIPANTS` was
checked in `resolve_participant_picks`, the create DIALOG's path. The other two
paths SEED a roster rather than picking one, and `ensure_session_roster` had no
ceiling: a plugin's `duo:true` or the driver's `solo:false` took every active
non-`on_mention` role. Three roles today, so it looked right; a fourth widens
every plugin-created session and every participant is a subprocess with its own
bill. The constant moved beside the invariant it protects and the seeder clamps.

The first wire fix was **inert** and the reviewer measured it: `participants: <n>`
reached only `rain_enabled = n > 1`, so 2, 3 and 8 were the same session — a wire
that reads honest over unchanged behaviour, worse than the vague flag it
replaced. `ensure_session_roster` takes a count now, and the roster is seeded
EAGERLY at create, which is what the dialog has always done; that removes a
divergence instead of adding a persisted count that could disagree with the
rows. `rain_enabled` is derived from what was actually seeded.

**R5 — the pre-flight found a column nobody had flagged.** Proving by query
which defaults actually fire, before writing the migration, turned up
`participant_cursors.updated_at`: 85 of 90 rows zone-less, from a live-firing
default the seeding INSERT omitted. Alongside `participant_deliveries`' 4011
un-backfilled rows, that is the whole contamination — the other 15 defaults
measured clean, so they are left rather than rebuilt across 13 tables.

The guard that missed it had *named* the column and *asserted* the right shape:
it reached the value through the UPDATE path, which writes correctly, and never
through the INSERT default that does not. That is the third guard this round
with the same defect — **the fixture reached the value by a path that is not the
one under suspicion** — and it is the sharpest pattern the round produced.

**R6 — not started, deliberately.** `run_sequencer` is 714 lines over eleven
`let mut` ring-state locals, and its largest match arm touches eight of them, so
extracting an arm means ~10 `&mut` parameters — exactly why `advance_turn`
already trips `too many arguments`. The real shape is a `RingState` struct, a
design change through a 3653-line module and 99 tests, in the code that deals
turns, where a half-finished state wedges sessions rather than failing loudly.
The report carries the three-step sequencing.

Suites: **1174** lib + 62; clippy 10.

## 2026-08-16 (round 2) — auditing the audit's own fix batch

A second audit pass over `632c163`, reported in
[`docs/plans/2026-08-16-rc3-audit-round2.md`](docs/plans/2026-08-16-rc3-audit-round2.md).
Its distinctive scope was the one surface round 1 could not cover — **its own
fix batch**: 42 commits and +5466 lines had landed since round 1's HEAD, and
round 1 audited the ring *before* it was rewritten. Two of the three most
serious findings sit in a **+146-line** delta inside `src/policy`, code that did
not exist when round 1 ran.

**A staged file could switch the commit gate off.** `git_output`'s strict
`from_utf8` returned `None` for any NUL-free non-UTF-8 file — git emits raw
bytes for those — and four callers read `None` as "nothing to scan": the
forbidden-word layer, the migrations gate, and BOTH halves of the post-commit
verifier. The backstop went blind on exactly the input that defeated
pre-commit, so for this bug class the two layers were not independent. Measured
with a probe: a staged latin-1 file carrying a forbidden term commits clean.
Lossy decode fixes all four, and keeps the word visible so the refusal can name
it.

**The violations log was losing most of itself.** A record was two `write_all`
calls under a per-instance mutex — but its writers are separate processes (five
hook sites plus the app). 8 threads × 50 records: **102–197 of 400 survived**
before, 400/400 after, red 9/9 and green 11/11 across both agents' runs. And
separately, rotating the log made its history unreachable: `read_all` never
opened the rolled file, so a rollover emptied the audit trail from the panel
and the driver both — strictly worse than the no-rotation state it replaced.

**A guard that had never run, and the reason nobody noticed.** The
tool-description name sweep had no `#[test]`: its attribute sat above the
previous test's doc comment, so both attributes bound to that test. The
duplicate registered it TWICE, so the suite total was identical with the guard
present or absent — **no count-based check could have caught it**, and both
compiler warnings had shipped in every build since.

**Fourteen of sixteen session drains were unpinned**, measured by cutting them
with a positive control in the cut set. Two guards now: behavioural (both
directions per map — a drain fails by not running *or* by over-reaching) and
structural (derived from the struct's own fields, so a seventeenth map cannot
arrive without a line).

**The framing sweep reached rendered text.** Round 1 swept doc prose; three
survivors were user-visible strings, including one tooltip duplicated verbatim
across two components. `lib/framing.ts` now sweeps every source file, with all
three exemptions stated as properties of the file. `core/duo.rs` became
`core/pump.rs`, and the dead `Author` pump discriminant went with it.

**`file.rs:NNN` citations cannot be audited, only converted.** Six of nine live
ones in `sequencer.rs`'s module doc were stale — and `src/` holds 13 duplicated
basenames (`tray.rs` has three copies), so a bare citation may not even name a
file. All nine are symbol-anchored now, including the three still accurate.

Suites: **1169** lib + 37 + 13 + 7 + 2 + 2 + 1; frontend **392** in 45 files;
tsc clean; clippy **17 → 10** diagnostics.

## 2026-08-16 (later) — batches 3–7, and three guards that were not guarding

Same session, second half. Where the first half fixed behaviour, this half was
mostly about docs and tests telling the truth — and the recurring find is that a
guard can pass for a WEAKER property than its own commit message claims. Three
of them, every one caught by the reviewer CUTTING the thing the guard protects,
never by reading it:

- a parity coverage guard rewritten as `checked == registry - SANCTIONED.len()`
  was an identity over its own loop, so deleting a whole tool left it green;
- a new doc guard's needle was a backticked NAME, so deleting an entire README
  table row passed — 30 of the 40 tools are also named in prose. It pinned
  "mentioned" where the message claimed "listed";
- a CL-editor dirty cleanup looked unpinned and turned out to be redundant with
  a second guard, so only cutting BOTH reddened anything.

**The CL editor lost unsaved text on every tab SWITCH**, not just on close:
`EditorArea` rendered the active tab alone, keyed by path, while the working
copy is component-local state. Type in A, open B, come back: gone, no prompt.
Fixed by keeping every open pane mounted and hiding the inactive ones — nothing
is restored, so the adoption logic cannot be handed a stale "clean" reading —
plus a tab-strip dirty marker, a confirm on closing a dirty tab, and a project
RENAME that retargets its tabs instead of closing them.

**The approval gate scrolled sideways** on the one surface where seeing half a
command is the whole risk; both boxes now wrap and scroll vertically only.

**Docs stopped describing a bot-hq that no longer exists.** ARCHITECTURE
contradicted itself about where a model comes from (the code resolves per
participant: participant pick → role default → `agent_configs` → built-in);
the public site led with "Two AI agents. One builds, one reviews.", against the
house framing rule, and carried 15 uses of the retired names; CLAUDE.md's plugin
paragraph was wrong on three counts; README and INSTALL still offered the
deleted native loop as a second backend; PLAN listed shipped work as unstarted
and a REVERSED fix as shipped. The tool list said 36 against a registry of 40 —
now checked by a test, anchored to the table row and the list paragraph.

**And the prose every agent is handed still said "the duo"** — inside
`GENERAL_RULES`, so it shipped in every session's system prompt. Fixed and
guarded; the comment layer is deliberately left, because a comment describing a
2026-07-10 incident in that day's vocabulary is a record, not drift.

Deleted, each verified dead at the line first: the router-health chain (end to
end — event, map, Tauri event, subscriber arm, view field, store slice,
component), two router-era mechanisms (`user_silent_forwards`, and a
`pending_paused_wakes` map one path filled and the other dropped), and the three
dead agent-config Tauri commands — whose deletion would have taken the LIVE
storage layer's only test with it, and which surfaced that
`agent_configs`' CHECK still excludes every rc3 role slug, so that spawn
fallback tier is unreachable for any current roster (pinned).

Suites at the end: **1165** lib + 37 + 13 + 7 + 2 + 2 + 1; frontend **389** in
44 files; tsc clean; release build clean, zero warnings.

## 2026-08-16 — the rc3 audit's batch 1 + enforcement, executed

The first self-hosted maintenance arc: bot-hq fixing what its own audit
found. Session `s-2b866c4a` picked up `s-fc6fe0fd`'s parked work — the
audit report and `CODEBASE.md` shipped there; the fix batches (§6 of
`docs/plans/2026-08-15-rc3-audit.md`) are what landed here. Batch 1
entire, plus the enforcement half of batch 2. Every fix is
mutation-verified: break it, watch the right test go red, restore.

**The three H items of batch 1.**

*The D15 close epilogue had never run.* It waited `await_both_idle`
immediately after a `broadcast` that marks nobody busy, so the wait was
answered by the idle state the broadcast itself left — 7 ms, "Declined",
agents SIGKILLed as the ring dealt the turn. It now ARMS on the turn
starting and then waits for the LAP to end (the halt slot refilling, or
sustained idle past `hand_turn_to`'s deliberate handover gap). Arming
alone was not enough, which EYES caught before it shipped: an armed
first-idle wait still returns between two turns of the same lap.
`Outcome::NeverStarted` separates "the ring never dealt it" from "the
agent took too long".

*A dealt turn that cannot complete now declares.* Three triggers, one
consequence: a participant whose stdin closed under the deal, a page its
own input refused, an unreadable backlog — each left the holder set, the
busy flag up, the halt slot empty and the input locked, with a Pause plus
a SIGKILL the only way out. `Dealt::CannotComplete` unwinds and fills the
slot. A pump that dies HOLDING a turn declares under its own slug so the
ring ends the turn in flight. And the third trigger — an empty backlog,
which happens on the most ordinary lap there is, since a peer turn spent
in tool calls leaves the next participant nothing to read — PASSES the
turn on (rc3 D25) rather than halting, bounded by the all-pass yield.

*Approval gates are identified, not guessed.* The latch was a counter
seeded from storage and incremented per notify, so one gate counted twice
could never be cleared and the session dealt nothing for the life of the
process; any resolve decremented, so a stranger's answer lifted somebody
else's gate. `GateOpened`/`GateResolved` carry the `choice_id` and the
latch is a set. The gate LIFT is now pinned — cutting it used to leave
1131 tests green while the ring stopped dealing forever — and so is its
twin on the withdraw path, which EYES measured as equally blind.

**Also in batch 1.** `halt()` clears `sessions.current_turn_participant_id`
on every path, so a yielded session stops reporting itself as working ·
`unregister_session` removes all sixteen per-session maps (the ring's
`Sender` was one of them: an orphan ring task per closed session) · the
ring declares its own halts without the phantom `system` participant ·
the PTY child is reaped instead of left a zombie · a second close joins
the epilogue in flight instead of SIGKILLing it · no host write bypasses
the ring any more, and the idle nudge wakes through a ring release that
marks the right participant · a staged message survives a relaunch
(migration **0058**) · and **Stop stops the ring**: the pause machinery
shipped complete with no producer, so a Stop flipped a banner while the
ring dealt the next participant. The user chose to wire it up.

**Enforcement (batch 2).** The internal MCP server authenticates its
callers — identity was the URL path alone, and every agent holds Bash, so
any of them could POST as a peer; a per-(session, agent) secret rides the
URL now, with an unregistered pair still admitted so an upgrade cannot
strand a live session. The push gate fails closed when its policy will not
parse. WAL. A violations log that rotates. Delivery timestamps in
RFC3339-Z like everything else. A question is withdrawable only by the
participant that parked it.

Suites: **1162** lib + 37 external-MCP + 13 storage + 7 signaling + 2
codebase-map + 2 doctests + 1 bin; frontend **381** in 44 files; `tsc`
clean; release build green.

## 2026-08-15 — the wedge-net declares; the handoff to self-hosting

The last build of the claude-code maintenance arc, closing the two wedges
the s-d6352684 restart incident exposed:

**1. The boot orphan sweep.** A restart over a mid-turn session kills the
turn without a stop — bannerless open box until the watchdog's grace.
At startup, every open session whose last recorded activity state was
`busy`/`cancelling` (and whose halt slot is empty — an agent's recap is
never overwritten) gets the restart halt: "that turn was lost, but the
participants keep their memory." Pinned + guard mutation-verified.

**2. The watchdog escalates from detecting to declaring.** The nudge-woken
generation that ends on prose with no tool lands outside every ring
backstop (its completion is discarded), and the nudge is once-per-window —
s-d6352684 sat bannerless twice in one night. Now, when the wedge outlives
the spent nudge, the watchdog fills the halt slot itself: every stop is a
HALT, even the stop nobody declared. Self-limiting — the filled slot flips
`halted` on the next poll.

**The handoff.** Per the user, bot-hq now maintains itself through its own
sessions; this claude-code lineage retires for bot-hq work. The standing
orientation is in the project CL
(`learnings-2026-08-15-selfhost-handoff.md`): the decreed state model, the
self-hosting hazards (never relaunch bot-hq from inside a bot-hq session;
`cargo build` transiently rewrites the Tool Gate hook's own binary), the
healing recipes, the dissection instruments, and pointers here. Deferred as
dogfood, per the user: everything else on the backlog.

Also of record: the arc's last hour included a full-disk outage (cargo's
incremental cache after ~15 rebuilds) severe enough to wedge the tooling
itself — cleared by the user; `target/debug/incremental` is the first
suspect when the machine chokes mid-maintenance.

## 2026-08-15 — every stop is a HALT; Working retired

The user's collapse of the state model, arrived at while reading s-d6352684
(a "Working" badge over a ring that had yielded 17 minutes earlier, its real
state — waiting on the 03:15Z sweep, self-waking 03:42Z, one command theirs —
scattered across an invisible tool-row reason, chat prose and a generic yield
line): *"What do we need WAITING for? HALT can be for any reason… on
turn-based, an agent doesn't hand the turn until they're finished. HALT means
the floor is the user's."*

**The model now:** BUSY is holding a turn (the busy map); everything else is
a stop, and **every stop fills the halt slot** — an agent recap, the
provider limit, the error streak, spin, and now mechanically the all-pass
yield, the round cap and consensus (host-declared, `system`, generic reasons
agents are taught to pre-empt with their own). Release is the user's message,
only — a self-wake may post findings and re-declare a fresher recap but
cannot start a lap. External-wait halts always name their wake time: nothing
expires a halt, so the timestamp is the dead-timer alarm ("wakes 03:42Z"
read at 04:10 speaks for itself).

**Retired whole:** the `declare_working` tool, capability (17 → 16;
migration 0057 scrubs the grant from role rows, `Capability::parse` already
dropped unknowns), the WORKING badge (tile + session header), the
`session:working` event/store plumbing, and the watchdog's TTL machinery.
The idle-unflagged watchdog is demoted to a wedge-net: with every stop
declared, idle + empty slot + empty tray is unreachable except through bugs
— its condition now reads the halt slot where it read the Working flag.

Pinned: the yield fills the slot (mutation-verified), the watchdog
suppresses on `halted`, seeded roles carry no retired grant, the parity
registry count records the deliberate 40 → 39, and the universal layer
teaches halt-for-any-reason with the user's own examples
(`every_stop_is_a_halt_and_working_is_retired`). Suites 1130 + 381.

The user's design, built as specced: *"remove the lock on the input box.
while agents are busy, instead of send button, it will be a toggle button…
lock-load the written message… to be sent in the most convenient time (in
between turns maybe?) along with the answers from tray."* Named **Stage**
(their ask for a term; consistent with the tray's staged answers, which ride
along).

**The insight that keeps it inside the contract:** rc3 D33's rule was always
about messages LANDING mid-turn — superseding the holder — never about
composing. Stage moves the lock from the BOX to the SEND: the textarea stays
writable while the ring runs, nothing can Send, and the submit slot becomes
a toggle that queues the message for the next turn boundary. Pause remains
the only interrupt.

**Mechanics.** Content lives in `AppState` (reload-safe, one slot,
re-stage replaces); the sequencer holds only a flag. At a boundary — a turn
completing, a consensus yield, a declared halt (the staged message IS the
release), or staging while already stopped — the ring PARKS instead of
dealing and emits `StagedDeliveryDue`; main.rs routes it to
`deliver_staged`, which sends through the ONE path (`send_user_response`:
answers first, message last, one release), then `StageDelivered` clears the
composer, the draft, and the consumed tray picks. A failed delivery
restores the stage rather than losing the message. The paste gate applies
at stage time. Picks staged after the message re-stage automatically so the
snapshot always equals the tray.

**Frontend:** the locked branch now shows the turn-status line above a
writable box; Stage ⇄ Staged ✓ toggles lock/edit; Enter stages while the
ring runs and sends otherwise; `get_staged_response` rehydrates the toggle
across reloads. Universal layer updated — "the user's messages LAND only
when the session stops or at a turn boundary… never cuts a turn in flight"
— since agents reason from that sentence.

Pinned: four sequencer tests (boundary-not-mid-turn with the full
delivery loop, immediate delivery when stopped, unstage never delivers,
staged-as-halt-release), ChatInput's rewritten lock pins (the lock is on
SEND now, twice re-subjected and saying so), stage/unstage/clear-on-deliver
flows, and the paste-gate count extended to the stage entry point.

The user, seeing the "Waiting on you" card: *"No i don't want that, revert
that, I only meant 'merge myself' cause i thought they're going to gate the
merges."* The diagnosis under the feature was wrong: their "Merge all 5
myself now" pick was never a personal to-do — it was an expectation that the
agents would PARK FIVE MERGE GATES for them to click. The right fix is
in-system routing, not a checklist surface.

Reverted whole (`git revert` of the feature commit + the prose): migration
0056 removed and the dev DB re-stamped (row 56 dropped, table dropped — the
applied-migrations rule honored by re-stamp, sanctioned pre-release), the
dashboard card, the tauri commands, the close_session argument, and the
universal-layer paragraph. In its place the layer now teaches the principle
the revert established: **nothing waits on the user outside the system** — a
mergeable PR means park the merge gate; a deferred decision is a question or
a line in the project's handoff file that the next session re-raises. The
three outstanding s-761704e8 items moved to the ad-manager tasks.md
next-actions (per the user's instruction), item 4 rewritten to "park the
five dependabot merge gates." The staleness-sweep term filter from the same
build is unaffected and stays.

The deeper s-761704e8 dissection found the failure class the clean run still
carried: **the user's own action items had no surface.** They picked "Merge
all 5 myself now" at 15:47 — recorded flawlessly in the wrap question and
tasks.md, shown nowhere ever again — and the five dependabot PRs sat
unmerged until a forensic pass rebuilt the user's own checklist. Same class:
the $_SERVER guard fix pushed at 16:16 whose `gh pr create` slid across a
Pause into prose, while the EOD had already told Tom "goes up for review
next."

**The ledger (migration 0056).** `close_session` gains `user_actions:
[...]` — one line per thing that now waits on the USER. Recorded before the
close gates fire (the staleness sweep refused the live first call), UNIQUE
per (session, action) so the retry re-passing the list is a no-op, and
surfaced as a **"Waiting on you" card on the dashboard** until checked off.
The universal layer teaches the arg; the tool schema carries the contract.
Pinned end-to-end: storage
(`user_actions_record_surface_and_check_off`), the JSON-RPC wiring
(`close_session_records_user_actions_even_when_the_close_is_refused`), the
card (Dashboard tests, including the null-returning-mock crash the jsdom
harness lesson predicted).

**The sweep filter.** Over 25 distinct retired-term candidates a cl_write is
a BULK REWRITE, and frequency-ranking selects prose ("real", "pass",
"empty" — the live 510-hit report). Bulk rewrites now keep only distinctive
candidates: term-shaped tokens (hyphen/underscore/digit) or words the old
body marked structurally (backticks, bold, headings) — so "sandbox" in a
heading still reports and "real" in prose never does, while the targeted-
edit path (the live "duo" specimen) is untouched.
`a_bulk_rewrite_reports_terms_not_vocabulary` pins it.

## 2026-08-15 — s-761704e8 dissected: the alignment build's first clean run

76 minutes, Fable5+Opus5, closed by consensus. **Zero ring/halt/lock defects**
— the first live validation of the alignment build: the one declared halt
opened the box in 17 ms with a clean busy map, four staged answers rode one
Send, all 27 dealt turns logged themselves, no error turns, no ghost turns,
no stuck flags, and the session ended on a done-vote consensus yield. Both
Pauses were the user steering (the #521 prod-risk hold; taking the floor for
Tom's reply) — the second honored in 54 ms, no SIGKILL. 17/17 tray questions
answered. CL delta landed and committed (`3238043`): a 41-line learnings
block (the `max_usage_pct` integer-floor rule), eod tweaks, and a tasks.md
refactor (1,515 → 116 lines against a byte-exact archive; the shrink guard
fired and was confirmed through properly).

**One bot-hq defect found: the close-out staleness sweep cried wolf 510
times.** The tasks.md shrink made the sweep flag "real", "pass", "second",
"empty", "php" — common English words — as "retired terms other files still
use". HANDS dismissed it in 18 seconds, which was the right call and is also
the problem: a gate that always cries wolf trains agents to slam through the
confirm, and the day it flags a real retired term (the sandbox→staging
class) nobody will read it. The sweep needs a term-shaped filter
(distinctive tokens — code-shaped, hyphenated, proper-noun — or a curated
list), not vocabulary diffing. Unfixed tonight; sized small.

Minor observations, no action: a reviewer that dies post-consensus surfaces
"reviewer down" in a close-out findings check with nothing to commit
(cosmetic; the override gate covers the case where it matters), and one
pass still carried a one-line narration (prose steers, not forces).

## 2026-08-15 (early) — vision alignment: the user decides, and the yield that dealt

The user's directive after reading the s-f6a441ff dissection against
vision.md: *"decisions should be mine, unless I lift all gates and explicitly
tell them to drive autonomously."* Four changes, plus a fifth found live
mid-pass when the user reported "all agents passed, but input is still
locked."

**1. The all-pass yield dealt the turn it refused — found, reproduced,
fixed.** `hand_over` called `hand_turn_to` (holder column + busy mark) as a
SIDE EFFECT of computing the next participant, BEFORE the caller's wrap
checks. Every all-pass yield (and round cap) therefore marked the front
participant busy, then refused the turn — a flag for a turn no pump would
ever run, so no pump would ever clear. The input locked under "send a
message to resume" until the user force-paused; at 12:45:39 a coincidental
ghost turn's ending cleared it, at 12:50:49 nothing did. Reproduced
deterministically in `an_all_pass_yield_leaves_the_input_open` (probe-driven
dissection: the deal printed before the wrap check), fixed by making
`hand_over` pure and moving the deal into `start_turn` — strictly after the
halt latch, gates, yield, and cap can refuse it. Side benefit: SUMMONED
turns are now recorded and marked, which the eager placement never did.
Deals also log themselves now ("sequencer: turn dealt") — the forensics
burned an hour on a silent deal.

**2. The reviewer-down override is the user's decision.** rc3's
`override_reviewer_block` let the executor self-override the dead-reviewer
commit block with a logged reason — reasonable at 12:26Z, and still an agent
asserting a path at a junction. It now parks an Approve/Reject gate: the
override takes effect only on the user's Approve; Reject leaves the block;
a reviewer that recovers first VOIDS the pending request (row withdrawn, so
a late Approve can't lift a future block). Pinned in findings + parity
tests ("a REQUEST alone lifts nothing").

**3. The spin halt says so on screen.** The repetition-net's halt was the
last silent stop — it now fills the session's halt slot via the same route
as the provider-limit and error-streak halts, naming the repeating
participant. `a_spin_halt_fills_the_session_halt_slot` pins it.

**4. Pass silently** (universal layer): the pass row is the whole message;
a no-change poll ("CI still running") is a pass, not a report — s-f6a441ff
burned five still-waiting narrations in one minute of CI-watching.

**5. PLAN entries the user must spec, not me:** park-on-external-signal
(the CI-wait shape, reconciling the ghost self-wake mechanism claude-code's
background tasks give every subprocess) and the release-scoped
autonomous-gate profile (the vision's "dangerously open every gate", as one
audited opt-in for released users).

Ghost turns are now a named phenomenon in the record: a subprocess whose
background task completes re-invokes its model OUTSIDE the ring —
carried_epoch 0, no turn-opened line, completion discarded, rows posted.
One did useful work at 12:46 (caught CI green); the park-on-signal design
is where they get a sanctioned home. Suite 1123 green; the yield fix and
the override-approve consumption both mutation-verified red.

## 2026-08-14 (late night) — the 2.9 MB paste and the error volley

**`s-f6a441ff` (still open, unrecoverable).** The halt fix held — at 10:54:19
the declared halt cleared the busy map in 62 ms and the box opened. What
killed the session came later: the user pasted the prod output HANDS asked for
— **one 2,977,078-byte message** (~750k tokens) riding the same Send as a
staged tray answer. It delivered, lodged in both participants' subprocess
transcripts, and from then on every prompt exceeded even the 1M window. The
ring dealt turns anyway: each ended `"Prompt is too long"`, each error ended
`Spoke` (one errored turn is survivable by design), and the two agents
volleyed **11 error turns across 5 minutes** until the text-repeat net halted
the cycle — silently. The user diagnosed it themselves mid-wreck ("i've put it
in temp.md logs are too long"), but the paste was already in both transcripts
where no delivery decision can reach it.

**Three fixes, three layers:**

1. **The paste gate** (`core::state`): `broadcast` and `send_user_response`
   refuse a message over 200,000 bytes at the top, whole — picks stay staged,
   the draft stays in the box, and the error carries the fix the user invented
   ("save it to a file, send the path"). ChatInput already renders rejected
   sends inline. Pinned by `an_oversized_user_message_is_refused_with_the_fix_in_hand`.
2. **The wire clamp** (`storage::participants::wire()`): no single ROW may put
   more than 200,000 bytes on a participant's stdin, wherever it came from — a
   user paste, an agent's dump, a replayed backlog. The record keeps every
   byte; the wire carries a truncation marker addressed to the reading agent.
   Same constant as the gate, so an accepted user message is never truncated.
   Pinned by `an_oversized_body_is_clamped_on_the_wire_but_whole_on_the_record`
   (+ char-boundary and at-the-cap edge tests).
3. **The error-streak halt** (`core::duo`): two errored turns in a row from
   one pump now fill the session's halt slot via the provider-limit route —
   the session STOPS with a ⚠ banner carrying the actual error line, instead
   of volleying until a repeat-net catches it in silence. One errored turn
   still steps the ring (unchanged). Pinned by
   `back_to_back_errored_turns_declare_a_visible_halt`.

All three mutation-verified red. The slowness the user also reported is the
ring's serialization itself — HANDS 10.3 min, EYES 5 min, HANDS 3 min, one
holder at a time — plus the heavyweight verify-everything discipline; the
levers (turn-picker, per-role model/effort, solo sessions) are the user's
call, recorded in PLAN.md's backlog rather than changed here. Also noted: the
repeat-net's own halt is still silent (pre-existing "a yield must say it
yielded" gap) — the error case that hit it today now stops earlier and
visibly, so the remaining exposure is agent-prose loops.

**Addendum, same evening — the session was healed, not written off.** The
poison was only in the two claude-side transcripts, so the heal was three
surgical steps: kill the zombie resume, NULL both participants'
`claude_session_id` (→ `is_first_spawn` → fresh boot), zero both cursors
(→ full chat replay, now survivable because the wire clamp truncates the
2.9 MB row). The paste itself moved out of the CL to the ad-manager repo
(gitignored) and its 2.9 MB of atoms left the global retrieval index on the
watcher's rescan. The user then drove the healed session for 75 more minutes
and every new mechanism validated live: the replay delivered the clamped
paste (its head carried the verdict JSON HANDS needed; the bulk came from
grepping the file); three isolated API errors were absorbed invisibly
(streak resets on success); and at 13:07 the error-streak halt fired for
real — two consecutive `Connection lost` turns → ⚠ banner with the error →
"recover please" → recovered. Same failure class as 11:20's silent 11-turn
volley, opposite surfacing. The session closed clean at 13:11: EYES caught a
real blocking finding (a three-account aggregate had inflated a page count;
the conclusion flipped and every outward draft was corrected), PR #517
merged squash `5ea8fb0c` through the push/PR/merge gates, 11/11 tray
questions answered, tasks.md + CL handoff written for the next session.
Heal recipe if it recurs: kill subprocesses → NULL claude ids → zero
cursors; a "restart participants fresh" button is the feature-shaped
version.

## 2026-08-14 (night) — the pre-mark that outlived the ring

**`s-ff729daa` (12 min, three force-pauses, three SIGKILLs).** The user opened
an ad-manager session on the fresh build; HANDS declared a clean halt at
09:54:04 — ring stopped where it stood, declarer interrupted, banner up — and
the input box **stayed locked for 110 seconds** until the user hit Pause.
Then the same thing twice more. "HALTS did not unlock the input box."

**Root cause: `AppState::broadcast` still pre-marked every agent busy** — the
duo-era delivery loop, redundant since D19b made `hand_turn_to` mark the
participant the ring actually deals. The pre-mark read fine while the ring
rotated (each flag was laundered into a real turn when its holder's deal came)
and lied the moment the ring stopped early: EYES was never dealt a turn before
the halt, so no turn end ever cleared its flag, `any_busy` stayed true, and
D33's map-authoritative lock held the box shut under the HALT banner. The same
stale flag read busy+silent to the stall watchdog, which called the rightfully
quiet EYES "stalled" at 09:48:25 — a false verdict that also feeds
`reviewer_block_decision`. Activity ledger, the proof:
`awaiting_user 0|1` — halted, declarer freed, and a participant that had done
nothing for six minutes still holding the lock.

**Fix: the loop is deleted.** The ring's `hand_turn_to` is the ONE busy-true
writer; the pump stays the clear. A flag only the ring sets is a flag a
stopped ring has always cleared — which makes `a_halt_leaves_nobody_busy`'s
claim finally true in production, not just in the sequencer tests.
`broadcast_marks_nobody_busy` (core::state) pins the loop deleted;
mutation-verified red both ways.

**Second defect, prose-layer: the halt reason referenced invisible artifacts.**
The halt said *"run the tinker command posted in chat"* — the command only
ever existed inside HANDS's own Bash tool input, which never renders for the
user. They searched the chat, found nothing, and burned two more round-trips
("WHERE IS IT ITS NOT POSTED IN THE CHAT") at a session that had stopped for
exactly that answer. The universal layer now states the visibility fact: chat
prose and the halt reason reach the user; tool inputs/outputs do not — anything
the user must act on goes VERBATIM into chat or the halt reason. Pinned by
`the_universal_layer_says_tool_traffic_is_invisible`.

Suite 1114 green. Also observed, noted not fixed: the declarer's own
`mark_awaiting_user` tool result reads "The user doesn't want to proceed"
in its transcript (the self-interrupt races the MCP response) — cosmetic but
confusing across resumes; watch whether it misleads a resumed declarer.

## 2026-08-14 (evening) — the discipline that manufactured work

**`s-86a81478`, full dissection (33 min, force-closed).** The session that
found four D35 holes in the morning found the fifth in the evening: with its
halt wiped (the gate bug, fixed at `6721abe`) and its question still parked,
HANDS hit the shipped yield discipline — *"never yield twice on a state the
user hasn't acted on; if anything in your queue is still workable, work it"* —
and said, in chat: *"Queue's empty and the next move is the user's — but I
won't halt on a state they haven't acted on."* Then it **invented work**: a
CL-delta draft written specifically to be reviewable, EYES reviewed it, EYES-2
confirmed EYES, and the three volleyed for 25 minutes (hands 15 / eyes 9 /
eyes-2 8 substantive messages post-08:20, only 7 passes all session — so D27's
all-pass net could never fire). The user sat locked out the whole time,
because D33 only opens the box when the session stops, and the discipline
taught the session that stopping twice is rude.

**The prose was the bug, and it was stale on two counts.** It opened with
"every `ask_user_choice` halts the session until answered" — false since D35
questions stopped touching the ring — and its anti-spam rule was written for
the world where re-halts piled up in a tray. Rewritten (`general_rules.rs` +
`HANDS_ROLE`): the D35 cost table (question stops nothing; approval and halt
stop everything), and the new rule verbatim from the user's model — **"when
your queue is empty and the next move is the user's, STOP. Declare the halt or
pass — never manufacture work to avoid stopping"** — with `s-86a81478` cited
in the prose as its evidence, like every other discipline carries.

**Moved by migration 0055**, the sanctioned reseed path (0050's shape:
byte-guard on the previous seed, a user's edit is never overwritten), with the
two-sided guard test the others have. HANDS 10569 → 10741 bytes; EYES
untouched.

Also confirmed in the same dissection: **the advisor config fix held** —
`on_mention` in this session's roster, 7 rows all boot orientation, zero ring
turns.

1109 lib + 378 frontend green.

---

## 2026-08-14 (later) — a halt is a halt

**The user ran `s-c41a4927` and returned four defects and one rule:** *"Again
stop overcomplicating things. A halt is a halt. Still working means still
working."* Mid-fix they widened the license — *"just redesign the halt
mechanism if thats better"* — so this is the redesign (rc3 **D35**), not four
patches.

**One surface and one ring effect per kind.** A **halt** stops the ring where
it stands — no D22 courtesy lap — and lives in the banner. An **approval**
halts the session until answered — the "asker blocked, peers keep working"
split I defended twice is gone — and lives in the gate. A **question** touches
nothing: no ring command, no awaiting flag, a tray card whose answer stages
into the next Send. The ring's whole mechanism is two latches (a halt bool, a
gate counter seeded from the durable rows so respawns can't deal under a
pending gate) and one release (the user's message). Dealing is refused before
any handover is minted, which made D31's busy-flag take-back unreachable
instead of carefully handled. `QuestionParked` is renamed `HaltDeclared`,
because that is what it is.

**The per-question Send button is gone** — the second one that component had
carried. Clicks stage, typing in Other stages, the composer's Send is the only
delivery, whatever the session is doing. The D34 box-open/box-locked split
shipped in the morning and was overruled by lunch; its test changed subject
the same day it was written.

**Halts are not tray items — at the storage layer, not just the surfaces.**
The badge said "one item on tray" over an empty tray because every count
included the halt row. First fix hid halts from the surfaces; the user's
reminder — *"halt is a session channel feature"* — finished it: migration 0054
gives the session ONE halt slot (`halt_declared_by/reason/declared_at`), a
later declaration replaces the earlier, `get_session_halt` feeds the banner,
and nothing writes `kind='halt'` tray rows any more.

**The advisor taking turns was config, not code:** the advisor ROLE predates
the mode picker and still carried `active`. The ring's on-mention filtering
held; the role and the session row now say `on_mention`.

D22's original defect cannot return through the redesign — a question no
longer reaches the ring at all, and a session where everyone runs dry yields
through D27's all-pass lap. Both new latches mutation-verified, including one
mutation the first cut of a test failed to catch: a premature deal at
`GateResolved` goes to the FRONT, whose buffered row the later expect happily
consumed — both seats are pinned quiet now. 1108 lib + 381 frontend green.
Decisions: [`docs/plans/2026-08-11-rc3-decisions.md`](docs/plans/2026-08-11-rc3-decisions.md) D35.

---

## 2026-08-14 — Pause is the only real interrupt

**The user set the rule, and it is one sentence:** *"users are never allowed to
type while agents are working, no halt = no type (except for pause button which
is the real interrupt)."*

That closes the item the previous entry left open, in the opposite direction
from the one it recommended. Yesterday's note called the mid-turn input lock a
band-aid and proposed **buffering** — hold the user's message, deliver it at the
next turn boundary. Buffering was never a decision, only a recommendation, and
the rule is strictly simpler: the lock is not a band-aid, it is the design. No
queue, no delivery point, no "it will land later" affordance, and no answer
needed for what happens if the session halts before the boundary. The cost is
chosen rather than discovered — arriving at a working session takes one extra
click.

**Locked ⟺ somebody is working, read from the busy MAP.** The session enum
cannot answer the question: `SessionActivity::derive` ranks `awaiting` above
`busy`, so parking a question reported `awaiting_user` while two participants
ran. The user screenshotted the result — an open textarea and a banner claiming
a halt, over a status line that correctly named a participant mid-turn.
`paused` is the one exception, because taking the box back is what the button is
for. `isLocked` had no unit test until it became load-bearing; it has one now.

**Approvals take the input slot (`ApprovalGate`).** Something is synchronously
blocked on the answer — a pre-push hook, a gated command that has not run. The
tray treated it as one more card with a Send button of its own, which is how the
user came to answer a row and watch nothing move. The gate replaces the box, is
answered on the spot, and keeps **Pause** reachable. The tray reports approvals
as a count and says where they go; it does not offer a second way to answer
them, because two paths into one row is the defect rather than the fix. Discard
went with it: for a gate the explicit no is Reject, which tells the hook.

**A discriminator that was wrong by a third.** `isApproval` first asked
`command_text !== null`, set for action-gate rows alone — so **10 of 31
approvals ever recorded**, every one a push gate, read as ordinary questions
while a hook blocked on each. Both gate kinds ask exactly `Approve`/`Reject`,
and no ordinary question in any session has. Same lesson as the two-hue palette:
a discriminator that holds on the cases in front of you reads exactly like one
that holds.

**Also shipped:** a refused handover takes its busy flag back (fourth instance of
*one event, two halves, nothing making them travel together* — invisible because
every sequencer test but one passes `activity: None`); `HALT` became a claim
about the session rather than about the tray; and Stop is renamed **Pause**,
which is what it does — it parks, and Resume picks the ring up where it left off.

**The ring is untouched.** Approvals still do not freeze it: the asker is
blocked, peers keep working, and D22's review lap survives. The gate is a claim
about where the answer goes, not a new way to stop the ring.

1105 Rust + 367 frontend green. Both new rules mutation-verified. Decisions:
[`docs/plans/2026-08-11-rc3-decisions.md`](docs/plans/2026-08-11-rc3-decisions.md)
D31–D33.

---

## 2026-08-13 — the input stays locked for the whole cycle, not one lap

**Reported by the user, from outside the system:** *"I can type while agents are
working, it might legitimately interrupt your turns, therefore corrupting the
quality of work you provide."*

`SessionActivity::derive` locks the chat input while any participant is busy.
Busy was set in exactly two places — `AppState::broadcast` (every agent, when the
user types) and `SessionHandle::send_to_all` — while each PUMP cleared its own
flag at its own turn end. A user message locked the input; each participant
unlocked its share as its turn finished; after **one lap** every flag was clear
and the input re-opened while the ring was still cycling (D22's lap, the
consensus tally, the round cap's 500). `SequencerDeps` carried no activity
handle at all, so the one component that knows a turn started could not say so.

The guarantee that used to cover this was the router's ordering — peer-busy set
before sender-idle, so `derive` never saw both idle mid-handoff — and it went
with `core/router.rs` in rc3. The replacement is stronger: the router closed the
gap between two agents; the ring holds the lock for the whole cycle.

**It closes a wedge, not just a cosmetic lock** — the reviewer's finding, and it
raised the severity. A message typed while a turn is in flight lands on the
holder's stdin mid-turn; the pump binds its epoch at turn-OPEN, so the
completion carries the pre-reset epoch and is discarded — and the discard arm
does not step the ring. The pump has cleared its state, the cursor is past the
message, and the loop waits in `rx.recv()` with no timeout, so nothing can
produce the epoch the ring now waits on. The only exit is another user message,
which is the same action that caused it. Holding the lock makes that landing
unreachable by construction.

Recording the holder and marking it busy are one event, so `hand_turn_to` does
both and each call site is one line. **That extraction exposed a second
defect:** rc3 D19b's `set_current_turn` was completely unpinned — deleting it
left all 1100 tests green, the same unpinned-wire class the CL says has shipped
five times here. Both halves are now independently mutation-verified.

A halt hands `None`, marks nobody, and the session falls to `Idle` — that is the
unlock condition and it needed no code of its own (`a_halt_leaves_nobody_busy`
pins that the fix does not over-correct into locking the user out).

**Deliberately left:** the pump still clears its own flag, so a sub-second
all-idle window remains between a completion and the next handover. Moving the
clear into the ring would buy a wedge where a completion that never arrives
locks the input forever. That window can be typed into, harmlessly — no turn is
in flight, so the message takes the designed fresh-turn path.

**Related, and NOT done here:** a consensus halt yields without flagging
anything, so a correctly-finished session is indistinguishable from a stalled
one. That is why shortening the idle nudge cannot land first — it would fire on
every completed task. Third instance of one pattern (D7's capped-halt row, item
4A's silent skip arms, this): bot-hq keeps producing ending states that look
identical from outside.

---

## 2026-08-13 — participants orient in parallel before the ring starts (rc3 D21)

Orientation — reading the CL, the conventions — depends on nothing and contends
for nothing, so serialising it through the ring cost N × the orientation time
before any work began. Boot runs it in parallel, **and only it**: D21's
refinement is the whole design, *"boot is ORIENTATION, NOT WORK"*, so the primer
asks for reading and forbids acting. Acting in parallel is the free-for-all D19
removed, and what produced three agents editing blind in `s-be58fdf0`.

**The hard part is that boot is not a turn**, and D21 said so in advance —
*"where this will break if rushed"*. The pump learns a turn started from its own
first event, but during boot no turn has been handed out: the epoch cell still
reads its initial `0` and `last_completed_epoch` is `None`, so D24's straggler
guard cannot see it (`Some(0) != None`) and the pump binds epoch 0. The
completion that follows is discarded forever — the `s-206e8921` wedge through a
different door. `DuoConfig` now carries an explicit `booting` flag: no epoch is
bound, no `TurnComplete` is sent, and readiness goes to the host on its own
channel.

That guard matters most on the **timeout** path, where a participant is still
mid-boot when the ring starts. `turn_epoch` is only re-read while it is `None`,
so a pump holding 0 would carry it into every real turn afterwards. Worth
recording how that got pinned: the first test written for it **passed with the
guard deleted** — the completion arm is guarded separately, so the bind guard was
invisible from outside — and its own doc comment claimed otherwise. The second
test reaches it through the timeout path, and the false claim was corrected.
Exactly the failure the queue's mutation rule exists to catch.

Boot output is a sixth `MessageKind`, `boot`, riding D19a's existing kind filter:
persisted, shown to the user, never in a peer's backlog — *"three near-identical
'CL loaded' rows are exactly the noise the channel does not need, and a peer
reading them learns nothing"*. No migration; `messages.kind` carries no CHECK
constraint, unlike `messages.author`.

The ring's kick moved out of `spawn_ring` into a `RingKick` the caller fires
after orientation — a value that must be consumed, because the failure it guards
against is silent: an unfired kick is a session that never starts, and nothing
else mints a `UserMessage` at spawn. Gating it after the pump loop also fixes an
ordering hazard that was previously incidental — the ring was spawned *before*
any pump existed to hear a turn.

A slow participant is waited out rather than waited on, and the timeout says so
in a visible row: a boot that truncated silently would be indistinguishable from
one that completed — the failure item 4A had just finished paying for on the
close epilogue.

**Not settled here:** whether the task text belongs in the primer. D21 asks for
that to be measured on a real session (*"three agents have opinions ready and the
first turn arrives into a room where everyone already decided"*), and it is one
function.

Mutation-verified four ways: dropping the kick, firing it early, removing the
epoch-bind guard, and sending `TurnComplete` during boot each redden exactly
their own test. 1097 lib + 60 integration green.

---

## 2026-08-13 — a participant has a name, and its peers read it (rc3 D20)

D20's remaining half. The ordinal shipped earlier and made two reviewers of one
role distinguishable — `EYES` and `EYES-2` — but it still says nothing about
which is which, which is the complaint it was answering. A name does.

**Migration 0053 copies 0052's shape**, including the roster-parity tripwire that
one had to satisfy: `session_participants` grew a column, so the pinned column
list in `n_of_two_is_byte_identical_to_the_default_roster` had to grow with it or
the dialog's creation path and the driver's could quietly diverge. Verified by
removing `"label"` from the list and watching that test fail by name.

The label replaces the role-and-ordinal half of the displayed name **and nothing
else** — the model suffix survives, because what a participant runs is a
different fact from what the user called it, and D8's per-participant model
picker exists to make that visible. Blank and whitespace fall back to the
ordinal rather than rendering an empty byline; the dialog sends `null` rather
than `""`, so an untouched field and a cleared one mean the same thing. Left
unvalidated, as `color` is: the fallback is total, so an unusable label costs the
override and never the participant.

**And the label is what peers read.** Shipping it into the roster while the wire
kept the slug would have fixed the confusion for the user and left every
participant reading the numbers — the same complaint, one layer down. The
`[speaker]` D23 puts on the wire is now the label when there is one, resolved at
READ time by a LEFT JOIN in `channel_page`. Read-time is the right way round and
matches `color`: renaming a participant re-labels what it already said, because
the transcript shows who that participant IS rather than a snapshot of what it
was called that minute. The join is LEFT, not INNER, for the reason the exclusion
clause beside it already documents — `user` and `system` rows carry no
participant, and an inner join would drop every user message and every host
injection from every backlog.

Neither `user` nor `system` takes a label, and that is not an omission: a label
names a participant, and an agent that reads a host notice as the user has been
handed a fabricated instruction (D23).

The write path needed no change at all, which is provable rather than assumed:
every caller that DELIVERS a write-time receipt posts as `system` or `user`
(`watchdog.rs:357`, `state.rs:779`/`:1176`, `broadcast.rs:47`). Participant rows
are written by the output pump (`duo.rs:404`), which only notifies — its peers
read the row back through the ring.

**`@mention` resolves a label too**, or the label would have broken the property
`speaker_of`'s own doc rests on: *"a participant reading `[eyes-2]` is reading
the string the user would type to summon it."* The slug is tried first and wins
outright — it is the key, unique by constraint and unchangeable — so renaming one
participant cannot silently redirect summons meant for another. A label that is
not mention-shaped simply is not typeable as a mention, and the slug still is.

Mutation-verified: dropping the label from the receipt reddens
`a_labelled_participant_says_its_name_on_the_wire` and nothing else.

---

## 2026-08-13 — two columns and a decision that nobody could read back

**The close-out learnings epilogue had never run — not once, in any session.**
Zero outcome rows and zero broadcasts of `CLOSE_LEARNINGS_PROMPT` across all 17
sessions in the live database, 13 of them closed after D15 shipped. The written
diagnosis blamed a slow or wedged ring and called the evidence "one-for-two";
the ring is not involved and there was no "one" — what looked like a success was
the agent writing the CL through A3b's nudge, a different mechanism entirely.

Two causes. `close_nudged` is set by A3b's write-the-delta soft gate, which runs
on the **agent's** `close_session` tool and nowhere else, but
`close_epilogue_decision` had no way to tell which path it was on. An agent that
called the tool once, got nudged and never retried left the flag set — and the
**user's** Close button then read it as already-handled, suppressing precisely
the case D15 exists for (its own doc: the user's Close *"kills every subprocess
without anyone being asked"*). A `ClosePath` now says who started the close.
`close_nudged` gates only `Agent`; `cl_written` gates both, because a write on
disk is evidence whoever closed. Threading an enum instead of a bool paid for
itself immediately — the compiler found a fifth call site in the external driver
server that the plan had missed.

And **three of `decide`'s four arms were silent, only one of them by
requirement**. D15 asks `SkipNoWriter` to post no ROW, which says nothing about
the log; `SkipBusy` and `SkipAlreadyHandled` were silent by omission. So no
session recorded which arm refused it, which is why the diagnosis had to guess.
The decision and its three inputs are now one INFO line — the same remedy D26
applied to agent health that morning, for the same reason.

Not claimed: that this makes an epilogue appear. It removes one proven
suppressor and makes the rest legible; the next real close is the measurement.

**`sessions.round_number` now has a writer.** The column has existed since
migration 0044 and nothing ever wrote it — `MAX(round_number)` was 0 across every
session ever recorded, the same shape `current_turn_participant_id` had before
D19b. It is written from `run_sequencer`'s own `laps` at both sites that move it
(the wrap in `advance_turn`, and the reset a user message performs), so the
column cannot drift from the number the round cap is measuring; it counts the
stretch, not the session's lifetime, because that is what the counter counts.
Kept rather than dropped: 0044 is applied and immutable, so removing it costs a
migration, and a lap count is the one number saying how far an unattended run got.

Both fixes mutation-verified — deleting either `set_round_number` call site
reddens `a_lap_of_the_ring_is_recorded_on_the_session`, and restoring the old
`cl_written || close_nudged` predicate reddens
`the_agents_nudge_does_not_suppress_the_users_close` and nothing else.
1086 lib + 60 integration green.

---

## 2026-08-13 — a turn's backlog is one stdin write

**The user's message stops arriving as an interruption.** `deliver_backlog`
called `input.deliver` once per row, so a nine-row backlog was nine separate
stdin writes — and since one outgoing message is one stream-json line and
claude-code opens a TURN on the first line it reads, that did not hand a
participant nine rows. It handed over row 1 and interrupted the turn eight times.
Measured across four sessions before the fix: the user's own message arrived
somewhere other than the front of the batch **37 times out of 44**, including row
9 of 9 and row 8 of 8. One session's reviewer spent its turn reviewing a peer's
test run while the user's actual instruction ("prepare to close") sat unread at
row 9; the user asked *"why does it feel like its not addressing my current
message?"*. D23 made that row identifiable — this is what makes it read last.

A page now goes out as ONE write: `ParticipantInput::deliver_batch` joins each
row's own wire with `WIRE_JOIN` (a blank line — D23's `[speaker]` already marks
where each row starts, so nothing heavier is needed). The **page**, not the turn,
is the unit: every realistic backlog is one page (the measured ones were nine
rows against a 200-row page), while a cold `on_demand` wake stays bounded by the
page instead of becoming one multi-megabyte line. Nothing caps a write by bytes —
the token cost is identical however the rows are split, and the page bound is
measured where a byte budget would be a number with nothing behind it.

Two deliberate consequences. The commit is **all-or-nothing per page** where it
used to be a prefix: there is nothing to cut between now, so a drain stopped by a
user message, a park or a pause leaves the page wholly past the cursor and the
next turn to reach it reads the backlog entire. And the `select!` still runs
biased against the command channel, so a full stdin still cannot hide a command —
that property was the reason for the row-at-a-time loop and it did not depend on
it. The rc3 D19a `kind` filter is untouched: it runs inside
`unread_for_participant`, upstream of the join, so coalescing cannot fold tool
rows back in.

**The test harness now counts rows, not wires.** A wire and a row stopped being
the same thing, and ~45 assertions in `sequencer.rs` are about ROUTING — who was
handed which rows, in what order — not about how many writes that took. `rows_of`
splits a wire back into rows for those, exactly as `unlabelled` already strips
D23's speaker prefix out of them, and the shape is pinned where it IS the
subject: `a_turns_backlog_arrives_as_one_message` and
`a_page_boundary_is_the_only_thing_that_splits_a_backlog`. Mutation-verified —
reverting to one write per row, with every row still delivered in order, reddens
exactly those two plus `a_delivered_row_says_who_wrote_it` and leaves the other
65 green.

One existing test had to be **refixtured rather than adapted**:
`a_backlog_larger_than_the_stdin_buffer_lands_in_full` is (by its own doc) the
only coverage of `deliver`'s parking path, and its 8 rows against a 2-slot stdin
became a single write that could never fill the buffer — it would have gone on
passing while covering nothing. It now posts three pages.

1083 lib + 60 integration tests green.

---

## 2026-08-14 — a session with no task deals no turns (rc3 D27–D30)

`s-8ac0d2d0` was force-closed four minutes in. The report was "they volley on
boot"; underneath it were four defects.

**The volley.** Boot finished before a task existed, and the ring dealt turn one
anyway. A participant with nothing to do can only pass, **and a pass is a row** —
so it becomes the next participant's input, and that one passes too. The ring
never runs out of something to hand over and never converges: 23 provider calls
in 77 seconds, ~240 KB each, to produce "(passed — nothing to add this round)".
The only floor was the 500-lap round cap, five hours away. Boot now ends by
yielding: the ring is spawned and idle, and the user's first message starts it
with something real in the backlog.

**The boot loop.** Stopping the volley SIGKILLed the agents, which made the
session stale, so the next message respawned it — and a respawn re-ran boot.
Three boots in four minutes, ~60k tokens per participant each time, and no way to
speak without triggering another. Boot now runs only on a first spawn; a reopen
resumes with its bearings already loaded, which the agents said themselves.

**Halts that never cleared.** Three code paths mean "the user responded" and each
did a different subset — answering a tray card released the ring and left the
halt row pending for ever. 52 occasions in the archive; the worst, one row under
six more for 53 minutes. The user reported it; I checked and said it had never
happened, having queried what was pending *now* rather than what had ever
overlapped. Both halves ride one function now, and the test pins that exactly one
place can do either.

**And the halt moved to where you answer it** — above the input box, as a recap
of what the session needs, one line per blocked participant. That is the user's
design, and it is aimed at the question that has cost the most time all week:
"why is it stopped?"

Also: the input locks while participants orient, and the session-start CL opener
no longer fires when boot ran — it repeated the primer, and it was the row that
seeded the first pass.

---

## 2026-08-13 — the wire says who spoke, and a straggler can no longer wedge a session (rc3 D22–D24)

Three fixes, all found by running real sessions and reading what they left behind
rather than by reading code.

**A parked question finishes the lap before it halts** (D22). A participant that
ends each turn by asking the user something made its peers structurally
unreachable: the park halted the ring where it stood, and the user's answer
restarted the cycle at the FRONT — which is the same participant. `s-e8a20797`
ran seven minutes with four deliveries to slot 0 and zero to slots 1 and 2, both
reviewers alive and initialised. Before rc3 the router forwarded the executor's
output to its peer regardless of any halt; the ring turned that forward into a
turn, and a halt stops turns. Now the park ends the ASKER's turn and the rotation
carries on, halting when it comes back to somebody blocked — bounded at N-1 extra
turns, which is the adversarial pass the roster was built for. Confirmed live in
`s-534b8761`: both reviewers loaded the CL and reported before the user answered,
and the halt landed 35 seconds later when the rotation returned to the asker.

**Every delivered row now says who wrote it** (D23). The wire carried no author
at all — `render_wire` rendered the envelope and the body, and nothing said whose
body it was. Three sessions' confusion traces to that one gap, most sharply
`s-81057bde`, where a reviewer reported "no task from the user and no HANDS
output" while the delivery table recorded eight rows handed to it. Both were
true: it had read them and could not tell what they were. Wires now lead with
`[speaker]` — the peer's slug, `user`, or `system`, the last two kept distinct
because an agent that reads a host notice as the user has been handed a
fabricated instruction.

**A straggler can no longer bind the next turn's epoch** (D24). `s-206e8921`
stopped dead for nineteen minutes with a live reviewer holding a turn it could
never hand back. The pump bound its turn on the first event after a completion,
which is not the same thing as the first event of the next turn: a participant
emitting anything in the gap read the cell as it stood — still the epoch it just
completed with — and every completion after that carried a number the ring had
retired. The trigger is the user typing while a participant is mid-turn, so it is
routine rather than rare; both reviewers in that session died this way, two
minutes apart, and one spoke exactly once in twenty-nine minutes. The message
that triggered it was a note asking what happens if the user types while the
agents are working.

All three were mutation-verified — the D24 test needed fixing first, because
`send` only queues and the original raced past the bug, passing with the guard
deleted.

Left deliberately unbundled: `deliver_backlog` still writes one stdin message per
row, so a backlog is N writes and rows 2..N land inside the turn row 1 opened.
With a speaker on every row that is far less confusing than it was, which is why
it should be measured before it is changed.

---

## 2026-08-13 — a participant you summon by name (rc3 D17 + D18)

Two participation modes now, and both of them do something.

**`observer` is gone** (D18). It was spawned, handed no turn, delivered nothing
and could not vote — a subprocess that read nothing, said nothing and billed for
existing. Zero rows used it, so the change was code and prose. The mechanical
rename surfaced two tests whose halves had quietly disagreed for weeks: one
asserted an observer IS spawned while an `on_demand` row is not, the other that
an `on_mention` roster is both refused and legal. Collapsing the modes made the
contradiction fail to compile past itself.

**`@advisor` hands the advisor the next turn** (D17), and only that turn. The
rotation then carries on from where it was — a mention is an INSERTION, not a
reset, so summoning somebody does not silently send the ring back to participant
1. Several mentions queue in the order written. Typing `@` in the composer opens
a picker of this session's participants, which makes mentioning a
non-participant impossible to EXPRESS rather than an error to report.

The mechanism is one field on `UserMessage` and one rule in the ring: it steps
from an ANCHOR that only a ring turn moves. For an ordinary turn the anchor and
the holder are the same participant, so the common path is unchanged; after a
summons they differ, and that difference IS "resumes where it was". Both new
invariants were mutation-verified — swap the anchor for the holder, or drop the
tally clear, and exactly the test written for each goes red.

**Only the user may summon**, structurally rather than by asking: the parse has
one call site, on the path that writes the user's own row, behind a function
private to `core::state`. A participant that types `@advisor` writes text and
nothing can act on it. Peer mentions would compose into a summon loop nothing
catches — every turn substantive, so the tally never completes, spin detection
never fires, and only the 500-lap cap ends it, at one real model call per lap on
the most expensive role in the session.

An `on_mention` participant is now spawned, which is a deliberate reversal: a
summons cannot reach a process that does not exist, and spawning lazily on the
first mention would be a second way into the rotation — the shape D19 spent a
day deleting.

**Live verification of D19a/D19b, from `s-81057bde`** (the user's N=3 run,
HANDS + two EYES). All three slots delivered (8/8/9). **Zero** of the session's
65 `tool_use`/`tool_result` rows reached a participant, where before every peer
read every peer's plumbing. `current_turn_participant_id` is written. Two
completions were discarded and both are the documented case — a parked question
took the holder and moved the epoch — with no `epoch = 0` carrier anywhere,
which was the whole of the D19 defect.

That session also diagnosed, from inside, why its reviewers looked mute: HANDS
parked a question on its FIRST turn, which halts the cycle unilaterally, so the
ring never reached slots 1 and 2 until the user replied — and a user reply
restarts at the front, i.e. at HANDS again. Working as designed and miserable to
watch. **D17 is the affordance that answers it**: `@eyes` hands EYES the turn
directly.

Noted, not fixed: `sessions.round_number` has no writer, exactly as
`current_turn_participant_id` had none before D19b. The ring counts laps in its
own frame; the column is dead until something carries that out.

---

## 2026-08-13 — CL claims that name code which is gone are detectable (rc3 P4)

Last dogfood-queue item, and the structural one. bot-hq's advantage over plain
claude-code is the Context Library; the library is maintained by bot-hq
sessions; so the loop closes only if something notices when it has drifted. On
2026-08-12 an audit found ~57 stale agent-name references and a whole learning
describing the native connector — deleted that day — in confident present tense,
and an outsider caught it.

`cl_stale_refs(project)` reports CL claims naming a symbol, flag or file the
project's repo no longer contains. It is an MCP tool, ungated like the other CL
reads, because the consumer is the manual maintenance session D15 describes.

**It reports; it never edits.** `decisions.md` and `issues.md` are skipped
outright — append-only history names dead code on purpose — lines carrying a
retirement marker are skipped for the same reason, and the report ends by saying
it is not a work order. D15's constraint applies directly: an agent handed a
list and told to fix it produces filler, and filler in this layer is fabricated
knowledge the next session builds on.

**Precision was measured against the real library, then tuned, then measured
again.** First run: 102 hits, mostly noise. Three rules fixed it — normalise
`file.rs::symbol` and `src/`-less paths before deciding a file is missing; drop
globs, URLs, templates and data-dir paths; and require a path's first segment to
name a directory the repo actually has. Second run: **25 hits, essentially all
genuine** — `strip_claude_code_tool_inventory`, `AgentRole`, `spawn_native_agent`,
`maintainClPrompt`, `core/router.rs`, and a cluster of router-era symbols. Noise
that never clears is what teaches a reader to skim the report, so the tuning was
the feature, not polish.

Complements `cl_refs`, which hashes the code an atom CITES and flags the atom
when it drifts — that path prunes references which do not resolve, which is
exactly this case, so the two are disjoint: drift there, absence here.

## 2026-08-13 — the Context Library pushes, behind a secret scan (rc3 P6)

Fourth dogfood-queue item. The library has a private remote and nothing pushed
to it, so it drifted from the first session onward — a snapshot, not a backup.
An agent's CL write now commits **and pushes**, detached from the tool call it
was made in and fail-open: a library that cannot push is merely un-backed-up,
and refusing to save an agent's knowledge over a network error is the worse
trade.

**The scan comes first, and that ordering is the feature.** A production
credential file sat committed in that repo for 153 commits and was caught only
because a human looked before the first push; `.gitignore` stops accidents, not
an agent running `git add -f`, and not a key pasted into a markdown note. So
`scan_then_push` refuses on a hit and names the files, and the refusal is posted
as a visible row — a scan that quietly declines to push is indistinguishable
from one that never ran.

It scans every TRACKED file, not the outgoing diff: a secret committed three
commits ago is still a secret this push would carry, and a diff-scoped check
would wave it through on the second attempt.

**Patterns are narrow on purpose, and the rate was measured, not assumed.** Only
self-identifying formats — PEM private-key headers, vendor-prefixed tokens
(`ghp_`, `sk-ant-`, `AKIA…`, `xoxb-`) — plus credential-bearing filename
classes. No generic `password=` matching: the library is full of prose *about*
credentials, including the write-up of the incident above, so a generic matcher
would refuse every push forever on the strength of a sentence. Run over the real
library: **135 tracked files, 0 hits.**

A rejected push (offline, or a concurrent session got there first) stays
rejected and is logged. Nothing auto-pulls or auto-merges — merging a knowledge
base behind the user's back is the hazard D15 named.

One correction to the queue's premise, checked rather than assumed: the library
is **currently level with its remote** (local HEAD, `origin/main` and the remote
ref were all `8ebb9a7`), so it has been pushed at least once by hand. The
structural complaint stands — nothing pushed automatically, and four files were
uncommitted at the time of checking.

## 2026-08-13 — context readings are persisted, and the meter's denominator is measured (rc3 P7)

Third dogfood-queue item. `ContextUsage` was forwarded to the UI and never
written down, so when a participant died mid-session with `Prompt is too long`
on 2026-08-12 there was no record of what its meter had shown — the failure
could be watched live and not diagnosed afterwards.

**Every `result` event now leaves a row** (`context_readings`, migration
**0051** — note 0050 was already taken by the D15 close-learnings prose, so the
queue doc's "take 0050" is one behind). The row carries the RAW operands and a
verdict: `usable`, `no_window`, `no_usage`, `implausible_window`. The unusable
readings are the load-bearing half — with only usable rows, "the provider never
reported a window" and "the agent never finished a turn" are the same empty
query result.

`AgentEvent::TurnComplete.context` changed from `Option<ContextUsage>` to
`ContextReport`, which keeps the operands even when they cannot be displayed.
The meter's reading is DERIVED from it (`ContextReport::usable()`) rather than
carried beside it, so the figure shown and the figures recorded cannot drift —
the drift class that shipped three wrong context numerators.

Each participant chip's meter opens its recorded readings, and the badge now
renders as a muted `ctx` when there is no live reading, because that state is
exactly the one needing an explanation. It reads `context_readings`, so a
**closed** session still answers.

**The open question, settled by measurement rather than argument.** The
proposal doc held that the meter divides by the user-typed
`models.context_window`. It does not: that column and `agent_configs.
context_window` have no readers in the tree, and the denominator comes from
claude-code's own `result.modelUsage[<model>].contextWindow`. The 2026-08-12
logs (`~/.bot-hq/.local/logs/bot-hq.2026-08-12.log`) show what actually
arrived:

- `contextWindow` **does** arrive through the DeepSeek gateway — every reading
  carries `context_window=200000`.
- The model's configured window is **1,000,000**. The provider's figure and the
  configured one disagree **5×**, and the configured one was never used.
- The last reading before the death, at `15:09:50`, was **146,787 / 200,000 =
  73%**. At `15:14:50` — the minute of the `Prompt is too long` message — the
  reading is `used_tokens=0`, because a failed turn carries no assistant usage.
- Zero `implausible context reading` warnings in any log, so the overshoot guard
  never fired.

So **the meter should not fall back to `models.context_window`**: doing so would
have shown 14.7% at the moment the agent was at 73% of the window its provider
actually enforces — strictly worse than what shipped. The real defect is the
opposite of the one proposed: the CONFIGURED window is the wrong number, and
nothing surfaces that it disagrees with what the provider reports. That is not
in the queue, so it is recorded here rather than fixed here.

## 2026-08-13 — a refused tool call leaves a row (rc3 P2)

Second dogfood-queue item. When the capability gate refused a tool call it told
the caller and nobody else, so **a gate that was silently open and a gate that
was simply never exercised looked identical from inside a session**. Capability
enforcement was decorative for weeks and no session would have shown it.

A refusal now posts a `system_notice` row — host-authored, `origin='system'`,
NULL participant, exactly as the capped halt (D7) posts — naming the three
facts a reader needs: WHO called (by the display rule, `ROLE · Model`, never the
slug), WHAT they called, and WHICH capability was missing. A wrong refusal
becomes something you watch happen instead of infer from an agent behaving
oddly.

**It records; it does not gate.** No halt, no awaiting flag, no tray entry — the
caller receives the same refusal text it did before, and a failed write is
warned about and swallowed, because losing the account of a refusal must not
also change what the agent is told.

Refusing and recording are ONE function (`refuse_gated_tool`). A second "and now
post the row" call at the gate would be a single deletable line; producing the
refusal and its record together means every path that refuses a gated tool
leaves a record by construction. The test asserts the ROW, not the return value
— asserting the return value alone reproduces the exact blind spot P2 exists to
remove.

## 2026-08-13 — the composed system prompt is viewable per participant (rc3 P1)

First item of the dogfood queue (`docs/plans/2026-08-13-dogfood-queue.md`). An
agent spawns with ~48 KB of standing instruction assembled from six layers and
**appended** to claude-code's own system prompt, and nobody — user or agent —
could see the result. Every "the prompt asserts an enforcement that is not
wired" defect was invisible by construction, and the Roles tab let the user edit
role prose with no way to view it in context.

Click a participant chip in the session header and the full-screen viewer opens
that participant's prompt, with the byte count and a standing note that this is
bot-hq's appended portion only. The note is rendered ABOVE the body, never
inside it — a caveat pasted into the prompt text would read as one more
instruction the agent was given.

**Nothing is recomposed and no filename is re-derived.** The view reads the file
the spawn WROTE, carried forward on `SessionAgent::system_prompt_path` from the
`SpawnConfig` that wrote it and that `build_command` points the CLI at. A
recomposition would answer a different question ("what would a spawn produce
today"), and a reader that rebuilt `{slug}-system-prompt.txt` from the temp dir
would be a second derivation that a rename could silently break. Dropping the
field fails to compile; a test pins that the path names the composed bytes AND
that the CLI was pointed at that same path.

The file lives in the session's `TempDir`, so it is gone once the session ends —
and a respawn writes a new one, which the view follows because the handle it
hangs off is rebuilt with it. All three empty cases say which absence they are
(session not live / participant never spawned / file unreadable, naming the
file); none of them renders a blank pane, which would teach the user that the
prompt is empty.

Audited for secrets before shipping the panel: the composed prompt carries no
token, env or gateway credential — those live in the agent's env and in the
sibling `{slug}-mcp.json`, and the view is wired to the prompt file alone.

## 2026-08-12 — closing four blocking findings against the name removal (+ D13, D14)

The previous entry claimed *"nothing in the runtime, the schema writes, or the
prompts is keyed on an agent's name."* **That was not true**, and this batch is
what makes it true. Four of the five remaining name checks were fail-quiet — they
did not error, they silently took the wrong branch — and one of them destroyed
data. The correction is recorded here rather than by editing the claim above,
because the claim is what the reviewer caught and the record of a wrong claim is
worth more than a tidy one.

**The fifth fail-quiet check, and the only one that lost work.**
`session_doc_write` routed a reviewer's phase-tagged doc with
`match caller.agent.as_str() { "rain" => … }`. No participant answers to that any
more, so every phase-tagged review write took the fallback arm and OVERWROTE the
executor's doc for that phase — while migration 0049's role prose kept promising
the co-located `<phase>-eyes` doc, so the prompt was lying. It keys on
`file_finding` now: the same capability the commit gate's reviewer registry is
built from, so the two cannot disagree about who a reviewer is. The doc's heading
(`### EYES findings (Rain)`) became a roster fact, and the display rule moved to
`Storage::display_name_of` so the prompt's peer roster and this heading are one
implementation.

**The untested join.** `bridge.register_session_reviewers` had exactly ONE
production caller and no test reached it — every test registered reviewers by
hand. Proven by deleting the call and running the suite: **1102 passed**. So the
reviewer-down commit gate could fail OPEN and nothing would say so. Both roster
answers now come off one call whose return value the spawn cannot proceed
without (`resolve_spawn_roster`): who spawns, and who the gate watches. Cutting
the registration turns a test red; cutting the call site fails to COMPILE. Both
verified.

**Per-agent Claude-config overrides were dead.** `resolve_agent_overrides`
matched the literals `"brian"` / `"rain"` while both callers passed a
role-derived slug, so every per-agent override resolved to the global `_all`
config — an editor, a file and a resolver that changed nothing at spawn. The
store is re-keyed by **role slug** (`{_all, per_role: {…}}`): not participant
slug, which is per-session and gains numeric suffixes, so a global panel could
neither enumerate nor address one. Resolution moved into one function the spawn
path calls and a test can reach, and the resolved value now rides on
`SpawnConfig` — the spawner and the command builder each loaded and resolved the
store separately off an agent name, which is two keys that went stale at once.
**Frontend follow-up required:** `ClaudeConfig.tsx` and `Dashboard.tsx` read
`.brian` / `.rain` and must read `per_role[<role slug>]`, enumerating roles via
`list_roles` instead of two fixed turn-slot blocks. `tsc` flags all six reads.

**Agent-visible names on the DEFAULT path.** `peer_forward_message` prefixed
every router-forwarded message with a hardcoded person name, and the router is
the default path (the sequencer is opt-in), so that string is what agents
actually read. It is a roster fact now, carried on `RouterDeps` and resolved once
at spawn; an unnameable sender degrades to an unattributed tag rather than a
wrong one. Also swept: six MCP tool descriptions (which now name the CAPABILITY
their gate reads, so the text states the rule instead of a lineup), the Apply
entry nudge, the Apply phase transition notice, the idle chat notice, the
external driver's tool text, the claude-config inheritance chips and MCP
forwarding chip, and the hook-attribution fallbacks. Two sweep tests walk every
tool descriptor and every inheritance surface, so the next one fails rather than
ships.

Deliberately left, each an internal key nothing displays: the legacy
custom-instruction templates (byte-compared to tell an untouched seed from user
content), `Author`'s wire strings (a turn-slot index), the legacy `agent_configs`
row keys, the test-only parity oracle, and `peer_shaped_reason`'s word list
(heuristic vocabulary, nothing keyed on it).

**`ParticipantView` gained `effort` and `ultracode`** — the create dialog wrote
both per participant (D12) and nothing could read them back, so the session view
could not show what a running participant was spawned with.

**D13 — `rain_disabled_default` is deleted.** The user: *"there is no 'disable
the reviewer by default' on rc3; just don't add the role to your session
creation."* The setting, `Storage::default_rain_enabled` and both readers are
gone. The consequence needed an answer, not a silent default: those readers are
the create paths with NO dialog (the external driver, the plugin arm), so per
design §1 they now seed **exactly one participant — the first active role by
`roles.id`**. One ROW, not N rows with the extras disabled, which is what the
fixed pair used to do: a disabled row for a role the creator never chose is a
participant the session view renders and nothing wakes. Stated at
`ensure_session_roster`, where the seeding happens. The Settings toggle is the
frontend's half.

**D14 — `AgentEvent::Error` is deleted.** D9 removed its only emitter with the
native loop; the variant, its handler and its two tests covered a path nothing
could take.

No migration in this batch — 0049 remains the highest.

---

## 2026-08-12 — rc3 D10/D12: the agent names are gone, and the 2-participant cap with them

The user, repeatedly: *"I already said multiple times that I'm dropping the
names, only the Role + Model Name."* A participant is now identified by the ROLE
it plays and displayed as **`role · model`**; nothing in the runtime, the schema
writes, or the prompts is keyed on an agent's name.

**What the cap actually was.** `spawn_session_handle` bound its two subprocesses
with `roster_row(&roster, "brian")` / `"rain"`. A third roster row was scheduled
by the ring, never woken, and the consensus halt then waited forever on a vote
nobody could cast — so `MAX_SESSION_PARTICIPANTS` was 2. Spawn iterates the
roster now (`spawnable`, one agent per enabled non-`on_demand` row in turn
order), and the cap is 8: a sanity bound on subprocesses, not a runtime limit.

**Identity.** `session_participants.slug` is the ROLE's slug; the second
participant of a role in one session takes `<role>-2` (`participant_slug`, which
reuses `first_free_slug` so a handle and a role slug are suffixed by one
function). `display_name` snapshots the role's display name; the model half is
resolved live, so a model swap is not stale on screen.

**Per participant, not per agent.** `effort`, `ultracode` and
`claude_session_id` come off `session_participants` (columns since 0044, unread
until now). `set_session_claude_id`'s `match agent { "brian" =>, "rain" =>,
other => bail }` is replaced by `set_participant_claude_id`: a third
participant's conversation used to hit the `bail` arm and restart blank on every
respawn.

**New read command `list_session_participants` → `ParticipantView`**, returning
`role_display_name` and `model_display_name` SEPARATELY with
`storage::participant_display_name` as the one implementation of the join.

**Capability predicates replaced four name checks**, per D11 (bot-hq must not
encode what a role means):

| was | now |
|---|---|
| `SessionHandle::hands()` = `by_slug("brian")` | the first participant holding `edit_files` |
| `agents().filter(\|a\| a.slug != "brian")` (cancel/kill order) | `!a.edits_files()` |
| `matches!(cfg.author, Author::Brian)` (pre-Apply nudge) | `cfg.edits_files` |
| `current_agent_health(session, "rain")` (reviewer-down commit gate) | the session's registered reviewers — participants holding `file_finding` |

The last two were **fail-quiet**, which is why they are called out: under
role-derived slugs the reviewer gate would have returned `ok` forever (a review
that cannot have happened stops blocking the commit) and the findings re-raise
guard's `has_message_from_author_since(.., "brian", ..)` would have stopped
escalating. Both now read the roster.

**`Author` survives as the router's two-party discriminant and nothing else.**
`core::router` forwards bilaterally and has no third case, so it runs only for a
two-participant session; `Brian` means turn slot 0 and `Rain` slot 1 there.
`ActivityTracker` holds the session's slugs in turn order and translates.

**Prompts.** `BRIAN_ROLE`/`RAIN_ROLE` are `HANDS_ROLE`/`EYES_ROLE`, and
`builtin_prose_for_role` is keyed on the ROLE slug (it went role → agent name →
constant, which breaks the moment a participant is slugged `hands-2`). The
opening roster sentence is DELETED rather than reworded — layer 2 already
generates `## Participants in this session` from the live roster (D4) — and every
other `Brian`/`Rain` became `HANDS`/`EYES`. `GENERAL_RULES` and the
`custom-instructions.md` template lost their name references too.

**Migration 0049 re-seeds both roles' prose**, guarded on the exact bytes
0046/0048 wrote so a user-edited row is never clobbered. Generated from the
resolved literals by `cargo run --example dump_role_prose`, and pinned to the
constants by the existing byte-parity oracle.

**Unread now, columns left in place** (the database is being reset, so dropping
them buys nothing and costs a migration): `sessions.brian_effort`,
`rain_effort`, `brian_ultracode`, `rain_ultracode`, `brian_claude_session_id`,
`rain_claude_session_id`, `brian_model_id`, `rain_model_id`.
`brian_model_at_spawn` / `rain_model_at_spawn` are still WRITTEN, positionally
from turn slots 0 and 1, because `SessionInfo` is frozen frontend shape;
`rain_enabled` is still written and read only as the solo/duo default for the
create paths that have no dialog.

**Behaviour changes, stated rather than buried.** A dialog-less session now
invites every live non-`on_demand` role in creation order, so a user with three
roles gets three participants (it was always exactly `hands` + `eyes`). And the
model chain's `agent_configs` tier is unreachable for a role-derived slug — the
row keys are CHECK-constrained to `('emma','brian','rain')` — so a participant
with no model and no role default falls to the built-in Anthropic default; that
branch now `warn!`s instead of being silent.

---

## 2026-08-12 — rc3 D9: the claude CLI is the only connector; the native loop is deleted

The user: *"I actually want to commit using the claude cli as the model
connector, defer the native loop/connector as a plugin I'll build in the future.
The reason is uniformity."* Chosen over dormant-code and feature-flag options
because a second runtime nobody builds still costs every reviewer a re-read and
every refactor a second case.

**Git history is the archive.** The future plugin starts from
`git show c7bba28:src/agents/native/`.

**What came out.** `src/agents/native/` (6,290 lines, 9 modules), the `native`
spawn branch in `core::session::spawn_agent_for` with `AgentKind` /
`resolve_agent_kind`, the native pre-flight in `tauri_cmd::docs`, the
`.local/native-accounting.jsonl` and `.local/native-history/` writers with their
session-close clear and startup orphan sweep, `tests/native_mcp_test.rs`, and
`examples/refusal_probe.rs`.

**`src/agents/roles.rs` went too, which the brief did not ask for.** After
removing `may_run_native`, `tool_policy` (returns a native `ToolPolicy`) and
`command_policy` (returns a native `CommandPolicy`), what was left — the
`AgentRole` enum, `for_agent` and `NAMES` — had **zero callers**: its three
consumers were the native spawn branch, the native-history sweep and one test
premise, all deleted. Its own module doc lists four motivating bugs and all four
are native. Leaving it would be the dormant code D9 exists to remove.

**`models.native` is now UNREAD, and the column stays.** The user is starting the
database over, so an unused column costs nothing and dropping it needs a
migration this phase must not write. `MODEL_COLUMNS` and `AGENT_CONFIG_COLUMNS`
no longer project it, the `Model` / `AgentConfig` structs no longer carry it, and
an upsert leaves it at its `NOT NULL DEFAULT 0`. Same for `agent_configs.native`
(0038). **`context_window` (0037/0038) turns out to be unread too** — its only
consumer was the native loop's own accounting; on claude-code the meter comes
from the CLI's per-turn `contextWindow` report. It is still round-tripped through
the Models tab so an edit does not destroy a saved value, and now says so.

**Two saved models become unspawnable, and the UI says where it can.**
`deepseek-v4-pro` and `moonshotai/kimi-k3` carried `native = 1`. Nothing
distinguishes them now, so the honest move is not to guess which gateways work —
it is to say what every model is subject to and point at the check:

- The **New Session dialog** names it beside the model picker: every model spawns
  through the claude CLI, so its endpoint must speak the Anthropic Messages API;
  use **Test**. Shown only when there are models to pick, so it cannot contradict
  the empty-registry hint. Two `Dashboard.test.tsx` tests hold both arms.
- The **Models panel** replaces the "Native loop" checkbox with the same
  statement, and its context-window help now says the field is unread rather than
  "used only by the native loop".
- **`validate_model` is the check, and it now tests the real runtime.** It used
  to fork on `native` and ping the gateway over HTTP; with one connector, `claude
  -p` through the model's own env IS what will spawn. That made
  `headless_claude_cmd` load-bearing and it was untested — two new tests assert
  the built command carries the model's own gateway and credential, and that a
  model with neither is given neither (so first-party ambient auth still works).

**D6 is retired, and the rule it was about is pinned instead.** D6 restored
`## Observations only` to the NATIVE EYES prompt after a strip span had been
swallowing it. `strip_claude_code_tool_inventory`, `RAIN_TOOLS_CLI`,
`RAIN_TOOLS_END`, `NATIVE_TOOL_ADDENDUM` and the six tests about them are gone —
but three of those tests were the only guard on text CLI EYES still receives, so
the guards moved rather than going with them:

| was pinned by | now pinned by |
|---|---|
| `the_native_prompt_keeps_the_observations_only_rule` | `prompts::tests::rain_carries_the_observations_only_rule` (on the constant) + `core::session::tests::the_composed_eyes_prompt_carries_observations_only_and_the_tool_inventory` (on the composed prompt) |
| `RAIN_TOOLS_END` as a strip boundary | `"Tools that are Brian's, NOT yours"` added to `the_surviving_deny_list_is_exactly_what_layer_2_cannot_generate` |
| `the_native_prompt_keeps_the_mutation_deny_list` | already covered by that same list |

**No prompt bytes changed.** `BRIAN_ROLE`, `RAIN_ROLE` and every layer-2 phrasing
are byte-identical — verified by diffing the non-comment lines of `prompts.rs`
and `capability_prompt.rs`. What changed is only which tests hold them and why.

**Layer 2 still names no claude-code tool, deliberately.** D9's brief permitted
restoring concrete names now that the "a promise the native loop cannot keep"
argument is dead. It was NOT restored: `prompts.rs` names `Edit`/`Write`/
`NotebookEdit` and layer 2 does not, and that split is what a merge broke
yesterday — one branch removed the names from the constant while another removed
them from layer 2, and the result was an EYES briefing refusing three tools
nothing named. Naming them in both places is one rule with two editable sources,
which is the drift D3 exists to prevent. `no_permission_line_names_a_claude_code
_tool` keeps the rule and loses only its dead premise (it asserted
`AgentRole::Eyes.may_run_native()`).

**`AgentEvent::Error` now has no emitter — the handler stays anyway.** The
native loop was its only producer: it routed API errors, model refusals, the
max-tool-cycle cap and the context-ceiling stop through that variant. The
`core::duo` handler that persists it is kept, and now says so out loud, because
it is the rendering path a future connector inherits and because losing it is
silent — before it persisted, every agent failure rendered in the UI as an empty
turn and the text lived only as long as the launching terminal's scrollback. The
no-buffering decision beside it was justified by a property of that emitter
("every native error is followed by an errored `TurnComplete`"); restated as
what it always was, since nothing constrains a new emitter to pair them.
**Flagged for review** — deleting the variant instead is a defensible call, and
one for the person who owns the connector plugin.

**Tests: 1274 → 1094 in Rust** (lib 1208 → 1034; `native_mcp_test.rs`'s 5 gone;
`storage_test` 14 → 13), **222 → 224 in the frontend**. The drop is the native
modules' own coverage, which is correct — every test deleted was about code that
no longer exists. Four tests were narrowed rather than deleted (both storage
round-trips, the `AgentConfigView` round-trip, the external-MCP `list_models`
payload), because each also guarded a hand-maintained column/bind sequence that
D9 shortened by one — a shift that would otherwise ship silently.

---

## 2026-08-12 — closing the review findings on the round cap, layer 2 and the Roles tab

Seven non-blocking findings from two adversarial reviews of the merges below.
Each was re-checked against the tree before it was touched; **one did not
reproduce and is recorded as not-reproducing rather than quietly "fixed"**.

**The round cap's two unread-snapshot paths are pinned.** `round_cap_laps`
resolves four ways and only two were held by a test. A data dir with no snapshot
yet (`Ok(None)`) and one that will not parse (`Err`) could each be changed to
return `0` — which turns the cap **off** — with the whole lib suite green,
contradicting the doc directly above them.
`core::sequencer::tests::a_snapshot_that_is_missing_or_unreadable_leaves_the_cap_armed`
holds both arms and asserts *not zero* separately from asserting the default, so
a rewrite to some other non-zero number still has to be deliberate.

**N=1 was unguarded, and it is the roster the product is moving toward.** The
lap-wrap test is `<=` because a one-member ring steps to *itself*, where the
`(turn_position, id)` key is equal rather than smaller. Narrowing it to `<`
disables the round cap completely for a solo session — measured on this tree:
**1177 of 1178 lib tests still passed**, and the one that did not is the new
`core::sequencer::tests::a_solo_ring_spends_a_whole_lap_on_every_turn`. Every
other cap test runs on a ring of two, where the wrap step goes strictly backwards
and `<` is enough.

**The 3.4× safety margin was an N=2 number quoted as a universal one.** The
corpus is counted in messages and the cap in laps, and the conversion divides by
N: the largest observed stretch (294 messages) is ~147 laps at N=2 but ~294 laps
at N=1, so the same 500 is ~3.4× at N=2 and only ~**1.7×** on a solo ring. The
constant's doc now states the dependence, and the second copy of the claim in
`advance_turn`'s wrap comment goes with it. **The default is unchanged** — 500
laps with `0` = off is the user's settled decision; the claim was wrong, not the
number.

**A disabled participant could be described as a live peer.** The `p.enabled`
filter in `resolve_roster_facts` was killed by no test, and without it a solo
HANDS session is told a reviewer is watching work nobody will review — a
confident false statement, which is the worst shape a prompt error takes.
`core::session::tests::a_disabled_participant_is_not_named_as_a_peer` asserts the
peer list AND the sentence the renderer only produces when it is empty.

**Layer 2 promised native EYES a tool the native loop does not have.**
`run_bash`'s permission line read "run shell commands with `Bash`" — but the
native loop implements `run_command` and no `Bash` at all, and EYES both holds
`run_bash` and is the one role allowed on that loop. It was inside the section
whose preamble declares itself authoritative over everything above it, which is
the "two contradictory tool lists" defect `strip_claude_code_tool_inventory`
exists to remove, in a span the strip cannot reach (layer 2 is appended after
it). **Fix: the generated section names no claude-code tool at all** — a
`Capability` is runtime-independent, so it describes the capability and points at
the runtime's own inventory for the call. `edit_files` loses its tool names for
the same reason. Pinned as a property over every variant by
`agents::capability_prompt::tests::no_permission_line_names_a_claude_code_tool`,
which also asserts the reachable case is still reachable (EYES may run native and
still holds `run_bash`) so the test cannot outlive its own motivation.

The refusals lose their tool names too, at unchanged claim strength: that
direction was never the contradiction — `prompts.rs` puts it as *"a PROMISE of a
tool that does not exist, not a refusal of one"*. The whole prompt delta is
**four lines**, verified by rendering both presets before and after: two on
HANDS' permission list, one on EYES', one on EYES' refusal list. Nothing else in
the section moved, and the CLI role prose still names `Bash`, `Edit`, `Write` and
`NotebookEdit` itself, so a claude-code agent loses no information.

**Two documentation overclaims.**
(a) The strip-span claim — *"exactly the CLI tool promises the native loop cannot
keep … and nothing else"* — was false in both `src/agents/prompts.rs` and the
entry below. The span is one contiguous slice, so it also carries `terminal_read`
and the `mcp__bot-hq-signaling__web_search` half of the web bullet, both of which
the native loop DOES have and `NATIVE_TOOL_ADDENDUM` grants a few lines later. A
redundant strip, not a lost capability. Corrected in both places and pinned by
`agents::prompts::tests::the_stripped_span_also_takes_two_tools_the_native_loop_does_have`.
(b) **Did not reproduce.** The finding was that the reported test paths in these
entries lack their `agents::` prefix, so `cargo test --exact` matches nothing.
The two 2026-08-12 entries name **no test paths at all** — zero `::tests::`, zero
`cargo test`, zero `--exact` across all 97 lines. The only `::`-bearing tokens
are three type names in prose (`Capability::ALL`, `Capability::parse`,
`CapabilitySet::from_slugs`), which `cargo test` was never going to run. Nothing
was rewritten for it. What the finding was really after is served instead: every
test this batch adds is cited above by its full path, and each was run under
`cargo test --lib <path> -- --exact` before being written down.

**Two Roles-tab defects, both user-visible.**
1. **Restore failed silently.** The handler awaited `archive.mutateAsync` bare —
   the rejection escaped unhandled and nothing rendered, so a failed restore was
   indistinguishable from the click not registering, one button away from an
   Archive path that wraps and displays its error. Restore now does both, and the
   test asserts both halves separately (the alert, and that no rejection escapes)
   because either fix alone leaves the other live.
2. **Clearing the instruction is a silent revert.** An emptied box sends
   `description_prompt: null`, and the spawn path reads NULL as "use the
   built-in" — so clearing it to *silence* a role reinstates the shipped prose
   instead. The behaviour is right (it is the "restore defaults" affordance
   0044's schema comment describes `builtin` as existing for) and is unchanged;
   what was missing was saying so. The textarea now warns while the box is empty,
   and branches on `builtin`: a seeded row is told this restores the default, a
   role the user added is told bot-hq ships no built-in for it.

Gates: **1181 lib + 66 integration Rust** (from 1176 + 66; five new tests),
**218 frontend** (from 215; three new). `cargo build` clean, `cargo clippy
--all-targets` byte-identical to the pre-change baseline, `tsc --noEmit` clean.
Every new test mutation-verified: twelve mutations applied one at a time to the
exact production line each test claims to catch, all twelve red, all reverted
green.

---

## 2026-08-12 — a session is created from N picked participants, not two literals

**`Storage::seed_session_roster` writes a roster from a list of
`(role, model, display name)`**, turn slots in the order given. It is the
N-participant counterpart of `ensure_session_roster`, whose two literal
`(SELECT id FROM roles WHERE slug = 'hands' / 'eyes')` subqueries were the last
thing that made "who is in this session" a product constant. Same table, same
invite-time capability snapshot, same cursor-from-birth invariant, one
transaction; only the source of the roles moved.

**Parity is proved, not asserted.**
`n_of_two_is_byte_identical_to_the_default_roster` builds one session each way
from the same inputs and compares EVERY column of both rosters — the column list
comes from `pragma_table_info` and is pinned, so a column added later fails the
test rather than escaping the comparison. `joined_at` is compared as a relation
(it is the session's own `created_at` on both paths) rather than as a clock
reading. `a_session_created_before_rc3_still_opens_with_the_same_roster` covers
the other half: a 0044-backfilled roster comes back unchanged on re-open, and a
session from the rosterless window is still healed into the same shape.

**`ensure_session_roster` now seeds only into a session that has NO roster.**
`OR IGNORE` on `UNIQUE (session_id, slug)` was sufficient idempotence while
`brian` + `rain` were the only rows that could exist; against a
one-participant roster it collides on the first insert and sails through the
second, handing the user a Rain they did not invite. Stated delta: a session left
with a PARTIAL legacy roster is no longer healed by the next spawn. That state is
only reachable if the second of two non-transactional inserts failed, and every
session in the live database has both rows (0044 backfilled 385 × 2; 0045's
precondition re-counted 770).

**The New Session dialog picks participants** — add/remove rows, each choosing a
role from `list_roles` and optionally overriding the model. **Default 1.** No
role is pre-selected and Create stays disabled until every row has one: guessing
is how a session silently gets an agent nobody chose. Gone: the "Disable Rain"
checkbox and the two by-name model selects. Their values are now DERIVED from the
roster — `sessions.rain_enabled` from its length, `brian_model_id` /
`rain_model_id` from each participant's resolved model — so spawn reads the same
columns it always did. rc3 **D8**: a participant with no model pick inherits
`roles.default_model_id`; with no role default the column stays NULL and
`resolve_spawn_config` falls through to the per-agent config — the same
"(agent default)" the old dialog labelled, now resolved at spawn instead of
pre-selected in the dialog.

**Two limits are enforced rather than described.** At most 2 participants, because
`spawn_session_handle` starts two literally-named agents and finds their rows with
`roster_row(&roster, "brian")` / `"rain"` — a third row would be scheduled by the
ring and never woken. And `on_demand` roles are not offered (rc3 **D1**: the
`@mention` wake is not built), alongside a refusal of an all-observer roster,
which `all_active_voted_done` would report as vacuously finished.

**What the user picks is the ROLE each slot plays; the two runtime identities
stay put.** Slot 0 is `brian`/`Brian`, slot 1 is `rain`/`Rain`, because the slug
is what spawn looks up and what `messages.author` carries, and because the role
prose migration 0046 seeded opens with `You are **Brian**` — naming a participant
after its role would put `**HANDS** (HANDS)` two paragraphs from that. Both halves
lift with the name removal.

---

## 2026-08-12 — B7 layer 2: the prompt's permissions and refusals are one set

**`src/agents/capability_prompt.rs` generates layer 2 of every spawned agent's
prompt** from `session_participants.capabilities` — permissions from what the
set contains, refusals from what it does not (rc3 **D3**). One exhaustive
`match` over `Capability` produces both directions plus a third-person label, so
adding a variant is a COMPILE error until all three are written; there is no arm
that can absorb one silently. Prompt and gate cannot drift because they read the
same data.

**Peer names come from the live roster (D4).** The section names each other
enabled participant by its `display_name` and its role's `display_name`, and
lists what that peer holds and you do not — the actionable half. Renaming a
participant renames it in the prompt; nothing is keyed on an agent name.

**Ordering is the enforcement.** Layer 2 is emitted after EVERY editable input —
the role prose, `custom-general-rules.md`, `custom-instructions.md` — so free
text never gets the last word on what the gate enforces, and the section says so
in its preamble. 0044's schema comment is the reason: *"a role must not be able
to author rules that contradict its own capability set."*

**A latent bug fixed (D6): native EYES had lost `## Observations only`.**
`strip_claude_code_tool_inventory` ran to `## Silence on transitions and holds`,
which swallowed the mutation deny-list, the user-facing-tools paragraph and the
whole never-assert-what-you-did-not-read rule. The span now ends at the deny-list
heading, which restores all four and keeps the CLI promises the native loop
cannot keep — `Read`/`Grep`/`Glob`, `WebFetch`/`ToolSearch`, `TodoWrite`,
read-only `Bash` — inside it. Confirmed by reverting the boundary: three tests go
red, and the suite was green with the old one.

**Corrected 2026-08-12** — this originally read "exactly the CLI tool promises …
and nothing else", which is not true. The span is one contiguous slice of the
role, so it also carries two bullets naming tools the native loop DOES have:
`terminal_read`, and the `mcp__bot-hq-signaling__web_search` half of the
web/reference bullet. `NATIVE_TOOL_ADDENDUM` grants the whole
`mcp__bot-hq-signaling__*` set and names both a few lines later, so this is a
redundant strip rather than a lost capability. Pinned by
`agents::prompts::tests::the_stripped_span_also_takes_two_tools_the_native_loop_does_have`.

**Nothing user-visible was deleted.** `BRIAN_ROLE` / `RAIN_ROLE` are untouched —
migration 0046 pins them byte-for-byte, so retiring the now-duplicated
hand-written deny prose from layer 3 needs a re-seeding migration (none was
reserved for this slice). The CLI prompt therefore carries both the old prose and
the generated section for now; the native one carries only what survives the
strip.

Degradation is deliberate: an unreadable roster, a missing participant row, or a
`capabilities` column that is not a JSON array of slugs yields NO layer 2 rather
than an empty set, because an empty set renders as "you may do nothing".

---

## 2026-08-12 — the Roles tab is live, and HANDS/EYES are rows you can edit

**Settings → Roles ships** (`frontend/src/app/RolesPanel.tsx`), on top of the
role CRUD commands merged earlier. Master/detail rather than a modal: a rail of
roles on the left, one role's whole form on the right, because the **role
instruction** — the `description_prompt` prose injected into every session the
role joins — is the point of the tab and does not fit in a dialog. It gets a
tall resizable monospace editor.

The tab does list (with a **Show archived** toggle), create, edit, and
**archive/restore**. Archive is confirmed and the copy says outright that
nothing is deleted; migration 0047 explains why removal cannot be a delete.
`builtin` rows render a badge and are otherwise ordinary — HANDS and EYES are
editable like anything else, which is the whole point of seeding them.

**New backend: `list_capabilities`** (`src/tauri_cmd/roles.rs`), plus
`Capability::ALL` / `label` / `description` / `group` in
`src/agents/capability.rs`. The checklist is served, never hardcoded in
TypeScript — a hardcoded slug list drifts silently the first time a capability
is added, and the new grant simply never appears as a box.

**Two modes only — `active` and `observer`.** rc3 D1 makes `on_demand` wake on a
user `@mention` and mention-wake is not built, so offering it would ship a role
that is enabled, rostered and never handed a turn. A role *already stored* as
`on_demand` still shows it, disabled: a picker that hides a value the row holds
is how editing the prose silently rewrites the mode.

**Default model** comes from the saved-model registry, with an explicit "none"
(rc3 D8). **The Agents tab is untouched** — D8 retires it, but that waits on
N-participant session create.

**Three defects found while building it, all fixed and pinned by tests:**

1. Re-seeding the form from each new server row let a background `list_roles`
   refetch discard a half-written instruction. The draft is now seeded once and
   moves only when a save returns a stored row.
2. Toggling **Show archived** changes the query key, so the list went
   `undefined` for a frame and unmounted the editor with the draft inside it.
   Fixed with `placeholderData`.
3. Saving with the capability list not yet loaded would have written an empty
   grant list, stripping a role's permissions with nothing on screen that looked
   like a change. Save is held until the checklist arrives.

**One thing the tab works around rather than fixes:** migration 0044 seeded the
`hands` role with `route_gated_command`, which `Capability::parse` does not
know. Submitting the stored list back verbatim makes HANDS permanently
unsaveable, so the form shows unrecognised slugs in a marked block and drops
them on save. Lossless here — the seed also carries `gated_bash`, and
`CapabilitySet::from_slugs` already discards the stray slug at spawn — but the
row itself still holds it. Migrations are immutable, so correcting the stored
value needs a new one.

Suite on the merged tree, with the round cap and layer 2: **1176 lib + 66
integration Rust, 212 frontend.**

---

## 2026-08-11 — the ring ran live; a drift audit found three unscheduled decisions

**The sequencer drove a real session for the first time.** `BOT_HQ_SEQUENCER=1`
on `s-156543b6`: turns alternated, backlogs drained, cursors advanced, delivery
rows recorded, both agents worked the task. The turn model is sound.

**One live defect, now FIXED (`d874d33`) — and the first reading of it was
wrong.** A native participant took a fresh turn every ~5s while Brian held the
turn, all correctly discarded by the epoch guard. The mechanism was NOT
self-driving: `run_loop` blocks on `input_rx.recv()` like the subprocess does.
The ring writes **one stdin message per channel row** and this runtime answered
**each with its own API request** — 87 rows in three drains answered by 135 API
calls and 7,523,266 prompt tokens, 84 of them after the last row had arrived.
That breaks design §1's "a turn is one participant's entire turn, not one
message". The fix folds what is already queued at the wake into one turn,
leaving completion cardinality untouched — because one-turn-per-message was
accidentally the mechanism that re-snapshots the epoch, and the obvious
alternative (re-fold after the turn) would have frozen the ring. **No test could
have caught it** — all 1,101 used fake seats that sit still until fed.

**Then a full audit of the design's `✅ VALIDATED` decisions against the code**,
prompted by the user asking whether the redesign had lost its direction. It has
not — Section 2 (Roles) is recorded exactly as the user restated it, the `roles`
table exists with `description_prompt`, and HANDS/EYES are seeded as editable
rows. But **three validated decisions exist in neither the code nor any batch**:

1. **The round cap** (§1b backstop #2) — and inventory row #1 dissolves the L2
   hard cap *citing it as the replacement*. Neither half exists;
   `sessions.round_number` is a column nothing reads. **This blocks task 14**:
   deleting `router.rs` removes the only bound on a cycle whose consensus never
   arrives.
2. **PASS** (§1) — a participant declining a turn without voting done. A done
   vote is not a pass; today "nothing to add" must either inflate consensus or
   emit filler.
3. **`on_demand`** (§1) — skipped in rotation correctly, but the "posts when
   addressed" half has no mechanism, so the mode builds a participant that
   cannot participate.

**Why they were lost:** all three are prose-only decisions that never became
rows in the router inventory. The implementation plan was built from the
inventory, and task 14's gate walks rows — so a decision that never became one
was never scheduled and cannot be missed by the gate. **Guard: every validated
decision becomes an inventory row before a batch is planned from it.**

Full detail, severities and fixes:
[`docs/plans/2026-08-11-design-drift-audit.md`](docs/plans/2026-08-11-design-drift-audit.md).

---

## 2026-08-11 — B5 in progress: the turn sequencer exists, `router.rs` is still live (HANDOFF)

**Status: plan tasks 1–13 complete; 14, 15 and 16 remain. Nothing in the
system behaves differently yet** — the sequencer is built but nothing spawns it,
and `core/router.rs` still routes every peer forward. Everything through task 13
is additive; task 14 is the first irreversible one.

Plan: [`docs/plans/2026-08-06-b5-channel-and-sequencer.md`](docs/plans/2026-08-06-b5-channel-and-sequencer.md).
Acceptance criterion: [`docs/plans/2026-08-06-router-behaviour-inventory.md`](docs/plans/2026-08-06-router-behaviour-inventory.md).

### Done

| task | what landed |
|---|---|
| 1, 1b | `PersistedMessage` — a receipt mintable only by a row insert or `from_row`; the two `messages` insert paths converged so exactly one `INSERT INTO messages` exists |
| 2 | **the payoff**: a participant's stdin is private, so text cannot reach an agent without a persisted row. Six invisible host injections became rows |
| 3 | subsumed by 2, verified by sweep (`send_unrouted` has one call site; zero raw `input_tx.send`) |
| 4, 4b | sequencer skeleton — which, as first consumer, found **five defects in already-shipped storage helpers**, fixed in 4b with migration `0045` |
| 5 | ring advance + backlog delivery |
| 6 | consensus halt |
| 7 | parked-question preemption |
| 8 | user-message cycle reset (inventory #12, #13) |
| 9 | pause holds wakes (inventory #19) |
| 10 | Jaccard helpers moved out of `router.rs`, verbatim |
| 11 | spin detection (inventory #2, guarded by #3) — one participant repeating itself across rounds halts the cycle. Mints its own `SPIN_*` constants; `VOLLEY_SIMILARITY_THRESHOLD` stays `router.rs`'s until task 14. **M7 re-measured: it survives** — `halt()` leaves no holder, so `advance_turn(reset = false)` is still unreachable with a `None` holder, and the doc's guess that spin detection might be the second path is now answered in place |
| 12 | `peer_ack` → done-votes (inventory #8, #9, #10, #11) as one pure `turn_ending()`, called by the sender rather than widening `TurnComplete`; #9's override tag moved from a spliced sentence to an `Envelope` field, same wire text |
| 13a | **a pin, not a feature — the plan's framing did not survive contact.** Inventory #5 wants suppressed deliveries recorded as rows; on a PULL path nothing suppresses, and the module doc (§"the forward ladder does not survive onto the turn path") already argued why. The upgrade #5 asked for was paid by task 2: the message is a real row, not a preview in a side table. What was missing is the inverse guard, so that is what landed — `the_turn_path_records_every_delivery_and_withholds_nothing`. `withheld_reason` stays in the schema for a policy that does not exist yet; the storage round-trip was already covered by `a_withheld_delivery_is_still_a_visible_row` |
| 13b | **the measurement, not an assumption.** Per message row **68.3 µs**, per delivery (row + cursor advance, one call each) **87.6 µs** — a ratio of **1.28×**, in-memory sqlite, N=300 after warm-up. A delivery costs the same order as the row it delivers, so **the cursor advance is not hot and needs no batching**. Guarded continuously as a RATIO — an absolute bound would measure the CI machine, not the code |

`src/core/sequencer.rs`: 43 tests. Full suite: 1101 lib + 66 integration.

**Both tasks were mutation-tested, and both turned up a hole a green suite hid.**
Task 11's two sequencer tests passed with `participant_text_since`'s `kind =
'text'` filter deleted — the filter that keeps `tool_use` rows, which outnumber
prose four to one and repeat by construction, out of a similarity test; it is
pinned at the storage layer now. Task 12's envelope renderer had no test at all.
Also worth knowing for anyone re-running these: the `TurnEnding { done: false,
peer_ack_override: true }` literal appears in the implementation AND in two test
expectations, so a naive `sed` mutates all three, they agree, and the probe
reports a false survival. Anchor on the four-space indent.

### Left
- **14 — delete `router.rs`. THE ONLY IRREVERSIBLE TASK**, gated on walking all
  20 inventory rows: every PRESERVED row needs a named green test, every
  DISSOLVED row needs the structure that makes it impossible to actually exist,
  every DROPPED row needs its written reason re-confirmed.
- **15** — live smoke. Constraint 0 says parity is verified against a real
  session, "not just 'it compiles'", with **serialisation asserted from
  `activity_events`** (today's rows show `busy | 1 | 1`; after B5 they must not).
- **16** — the `system_notice` render lane is sized for a one-line notice and
  now carries five host injections, one ~450 chars. Frontend, B8 territory.

### Three things the next session should not have to rediscover

1. **`VOLLEY_SIMILARITY_THRESHOLD` is still shared with live code**, and so is
   `router.rs`'s private `PEER_ACK_MAX_SUPPRESSED_LEN`. `0.85` and `200` cannot
   be retuned until task 14 deletes that file. Tasks 11 and 12 minted their own
   (`SPIN_SIMILARITY_THRESHOLD`, `SPIN_BREAK_STREAK`,
   `PEER_ACK_MAX_SUPPRESSED_LEN` in `sequencer.rs`) rather than inherit by
   inertia — same values on purpose, because both inventories say the proxy
   MOVES rather than being retuned. Task 14 is where they become tunable, and
   where the router copies go.
2. **Mutation M7 survives task 11 — re-measured 2026-08-11, all 41 tests.**
   Spin detection halts, so it does produce a `None` holder, but it reaches it
   the way the parked question does: `halt()` clears the holder, every later
   completion then fails the `live` guard, and the `live` guard is what calls
   `advance_turn(reset = false)`. So a `None` holder and a `reset = false` call
   still cannot co-occur. **The shape that WOULD break it is a recovery that
   steps past a stuck participant instead of halting** — measure against that,
   not against spin detection. The reasoning is recorded at the condition
   itself; re-run the swap rather than trusting this paragraph.
3. **A `UserMessage` releases a pause** (host contract: `state.rs:737`,
   `activity.rs:217`, and `resume_session` is a `broadcast`, so the Resume
   button IS a user message). Three non-human writers mint `origin = "user"`
   rows — `advance_phase`, the watchdog's idle NOTICE, and
   `request_phase_advance`'s slug fallback — and **`advance_phase` does not flip
   the pause latch**. A producer of `UserMessage` must either flip it or not be
   minted from a non-human writer.

### What this arc cost, honestly

Six days, and the review layer found something real in **every single round** —
including in the fixes to earlier findings, twice. The recurring defect was not
in the code: it was **claims written by someone who had just verified something
adjacent to the claim**. Sixteen instances. A doc naming a regression test that
did not exist; a test that passed with its own guard removed; a "cannot be
tested" sentence standing guard over a surviving mutation, killed as soon as
someone wrote the probe; a migration predicate reading `enabled = 1` while the
code filtered `enabled != 0`. Every one was caught by a different agent than the
one that wrote it, and never by its author's own re-read.

Full detail in `~/.bot-hq/library/projects/bot-hq/learnings-2026-08-07-b5-channel-batch.md`.

## 2026-08-06 — rc3 begins: the session-focused redesign (design, B0–B4a, migration 0044 verified live)

**This arc is `1.0.0-rc3`.** rc2 was the harness as built — agent-focused, with
Brian and Rain compiled into the core. rc3 is the redesign that makes the
session the unit and agents its configurable participants. Version bumped in
`Cargo.toml`, `tauri.conf.json` and `frontend/package.json`; the arc is
partially shipped (B0–B4a + the schema), not complete.

The architecture pivot. User's diagnosis: bot-hq's *doing* is correct but the
*design of the doing* is the problem — agent-focus makes agent plugins hard, and
"the chat stream is not really a chat stream" because peer forwards travel a
hidden pipe that can hold, suppress or rewrite text with no record. Measured
blast radius: **1333 agent-name occurrences in Rust, 312 in the frontend**, six
invisible injection points, and a 21-subsystem inventory showing that everything
carrying meaning *between* agents is invisible while everything carrying meaning
*to the user* is visible.

**Design** (`docs/plans/2026-08-06-session-focused-redesign-design.md`) — all
four architectural forks decided by the user:
- **Participants, not agents.** A session owns N participants; a participant is
  a model plus an invite-time snapshot of a user-owned **Role**. The Agents tab
  becomes a Roles tab; HANDS/EYES become the user's own configurations. This
  deletes the planned Rain-plugin migration outright.
- **Turn cycle** (user's own model, beat all three options offered): a fixed
  ring over active participants, **O(1) wakes per turn** vs broadcast's O(N),
  with each participant waking to everything after its cursor. Deletes the
  volley-breakers, wake rules, addressing, and `router.rs`'s routing.
- **Consensus halt**, derived from vision.md's AI-car: the cycle runs until
  every active participant votes done; a parked question halts immediately.
  A round budget was rejected as contradicting "the first prompt sets the
  destination, not the arrival".
- **Serialisation accepted** as the one deliberate client-visible change —
  today's duo runs concurrently, which is *why* the reviewer ran a phase behind
  all session. "The staleness was the bug, not the speed."

**Shipped, 24 commits:**
- **B0 — the parity oracle.** `src/signaling/parity.rs` pins today's tool
  authorization + commit-gate contract; `docs/plans/2026-08-06-router-behaviour-inventory.md`
  gives a verdict to each of the 20 behaviours `router.rs`'s 822 test lines
  encode (11 preserved, 8 dissolved, 2 dropped with reasons). Two behaviours are
  invisible in the code and present only in the tests.
- **B2 — capability model** (`src/agents/capability.rs`): grants only,
  dependencies validated, session policy as ceiling. Plus a cross-check proving
  the model reaches the same verdict as the live dispatch layer for all 38 tools.
- **B3a/B3b — participants storage + channel primitives**
  (`src/storage/participants.rs`): roles, participants, cursors, deliveries,
  turn helpers, `post_to_channel`/`unread_for_participant`.
- **B4a — dual-write**: `insert_message` now fills `participant_id`/`origin`
  alongside `author`, so the channel is correct with no backfill needed.
- **B4a.1 — `ensure_session_roster`**: 0044's backfill was a one-shot over the
  sessions that existed when it applied, and nothing created participants for a
  *new* one — so the first post-migration session ran with an empty roster and
  its every message resolved `participant_id` to NULL (60 rows and climbing when
  caught). Seeded pre-spawn from `ensure_session_started`, the one choke point
  every creation path funnels through; idempotent, and it repairs what the
  rosterless window wrote. B4b's rekey needs a roster to key by.
- **B4b — the structural rekey**, in three gate-green slices.
  - `ActivityTracker`'s `brian_busy`/`rain_busy` become maps keyed by
    participant slug (not id: `set_busy(cfg.author, …)` runs at every turn end
    and `Author` lives until B5, so an id key would buy a lookup for nothing).
    Its public API is unchanged, so all 22 call sites compiled untouched.
    `derive` collapses to `any_busy` — which is exactly why per-participant
    edges are tracked separately: the collapsed state cannot express a hand-off.
  - `SessionHandle`'s `brian` + `rain: Option` become `participants:
    Vec<SessionAgent>` ordered by `turn_position`. A Vec, not the design's
    HashMap: the ring needs deterministic order. The 24 field sites collapse
    into `agents()` / `hands()` / `agent_count()`. Kill and interrupt order is
    now explicit — it used to come from field order, and sorting HANDS to
    position 0 would have inverted it (HANDS may be mid-tool, so it stops last).
  - `DuoConfig` gains `participant_id` beside `author`; both it and
    `SessionAgent` resolve through one `roster_row` lookup, so a pump cannot
    report a different participant than its own handle.

  **Parity checkpoint:** B0's oracle green and byte-identical across all of
  B4b, `activity_events` still one row per transition, and the **frontend diff
  is empty** — `SessionRuntime`'s two booleans are derived from the map, not
  migrated, because B8 owns the UI.
- **B1 — `migrations/0044_session_participants.sql`, applied and verified live.**

**Verification standard used throughout:** a guard or oracle that passes on
first run proves nothing. Every one was proven by injecting a regression and
confirming exactly the right test failed. That caught a wrong assertion each
time it was applied.

**Live migration result:** `applied=1`, 201,999 messages with `author` preserved
on every row, **0 unmapped**, 2 roles / 768 participants / 768 cursors across
384 sessions, integrity + FK clean. B4a's dual-write confirmed in production —
of 538 post-restart rows, all 503 participant rows attributed, and **0
attribution mismatches across all 202k rows**. That verification covered only
sessions the migration had backfilled; the first session created *after* it
exposed the roster gap B4a.1 closes.

**Three findings worth carrying** (full set in the CL's `notes.md`):
`RAISE(ABORT,…)` is trigger-only and `sqlite3` walks past the parse error with
exit 0, so guards silently no-op; a table rebuild transiently needs ≈2× the DB
size (found via a real disk-full); and keeping `author` as a transitional column
is what decoupled an irreversible migration from a 153-site refactor.

**A test-shape finding worth carrying into B5:** removing the per-participant
edge check didn't fail the guard, it **hung it** — a bare `rx.recv().await`
waits forever for an emit the regression deleted. The activity tests' 16 recv
sites now go through a 2s timeout helper, and the same injection then failed in
2.4s. B5's consensus-halt tests have the identical "wait for something that
should arrive" shape.

Remaining: B5 (channel + sequencer, deletes `router.rs`), B6/B7/B8. Plan:
`docs/plans/2026-08-06-session-focused-redesign-implementation.md` — which asks
for a fresh `superpowers:writing-plans` pass per batch now that B4 has landed.

Gates at close: cargo 1092 green, frontend 199 vitest / tsc / build, release
build green.

---

## 2026-08-06 — Track 1 harness fixes: tray preempt, gate-refusal wording, CL close-out sweep

The first three deliverables from the Aug-5 plan (PLAN.md "Next
deliverables", Track 1), one commit each:

- **`d71c4d1`** — **#27 tray answers preempt a running turn.** An OOB tray
  answer was injected on stdin with no interrupt, so it waited out the agent's
  whole current turn. `resolve_choice`'s `AgentReceiverDroppedFellBack` arm now
  mirrors `broadcast`: interrupt both agents (`tray-answer-preempt`) BEFORE
  `send_to_both`. The paused branch still stashes without interrupting, and the
  `Delivered` arm (blocking `request_approval`) is untouched. Branch extracted
  as the pure `tray_wake_step`, plus a source-order guard test on the
  interrupt-then-send contract. Note `ask_user_choice` is non-blocking, so
  *every* tray answer to an agent question takes the OOB arm.
- **`c0a66b7`** — **#29 gate refusals.** The issue's premise did not survive
  measurement: the "19 refusals" were 19 `tool_blocklist` rows (18 approved, 1
  denied) — `action_gate` outcomes, not refusals, which are never logged as
  violations. Real refusals: 5 across Aug 4–5, none a same-command retry. 3
  converted correctly (each paying a `ToolSearch` round-trip, issue #14's tax);
  **2 reworded to evade the keyword**. Since the exact-call text was already
  shipped and live through all five, the refusal now forbids rewriting around
  the gate by name, with an ask-instead escape hatch; matching bullet in the
  agent general rules.
- **`525d452`** — **#31 close-out staleness sweep.** `cl_write_file` computes
  the concepts a write retired (old→new token diff, reusing the status-lint's
  body read; stopword-filtered, occurrence-ranked, capped) and `close_session`
  sweeps the project's living CL for files still citing them — before and
  independently of the learnings nudge, since an agent that wrote the CL clears
  that nudge and is exactly the one who may have stranded a term. `decisions.md`
  and dated `learnings-*` / `notes-<date>-*` are skipped; whole-token match;
  ≤ 20 hits; fires once and can never hold a close shut.

- **`19ec620`** — **#29(ii) auto-park** (user-picked after the wording fix
  landed). The PreToolUse hook now parks the approval for the command it just
  blocked: on a `Gate` match with a known session it POSTs the new
  `/hooks/tool-gate` route, and the refusal becomes "already queued as gate_id
  X — do NOT call `action_gate`". That removes both measured costs (the
  ToolSearch round-trip on every conversion, and any incentive to reword).
  The route calls the newly-extracted `park_gated_command`, **not**
  `action_gate`: the latter re-resolves the keyword list and EXECUTES on
  `auto_allow`/no-match, so a route wired to it would run a command with no
  approval whenever its resolve disagreed with the hook's — and would make this
  the first localhost route that can execute anything (`/hooks/pre-push` only
  parks). Transport mirrors `run_pre_push` with a 10s timeout (parking returns
  at once). Every failure path — no session, app down, non-2xx, 2xx without a
  `gate_id` — degrades to the previous call-`action_gate` wording, and the exit
  code stays 2.

Gates: cargo 1051 tests green (987 lib + 64 integration); frontend 199 vitest /
tsc / build green; release build green. Live-verified already: the new gate
refusal fired on this session's own commit (a fresh specimen of issue #12 — the
matcher hits `rm -rf` quoted as prose in a commit-message body). #27, #31 and
the auto-park route are in-process and need an app relaunch to go live.

---

## 2026-08-05 — _globals always agent-retrievable + per-file user-only toggle

User report: agents repeatedly couldn't find `_globals` CL files (eod.md) and
invented their own inside repos. Root cause was structural invisibility, not a
gate: `cl_retrieve` hard-scoped to one project and every prompt trains agents
to pass their session project. Three commits:

- **`26c1548`** — agent-facing CL surfaces always union `_globals` in:
  `cl_retrieve(include_globals)` runs ONE query over `project_id IN
  (?, '_globals')` so BM25 relevance decides across scopes; cross-scope atoms
  render as `## [_globals] file > heading`; project-scoped
  `cl_index_search`/`cl_folder_search` concat `_globals` rows; plugins get the
  same contract. Staleness is now per-atom (a `_globals` row inside a project
  query takes the age fallback). The Library UI tree stays strictly
  per-project.
- **`67a85cc`** — per-file agent visibility: migration 0043 adds
  `cl_index.agent_visible` (default visible; rescan upserts can't clobber it).
  Agent/plugin search + retrieval + the spawn primer filter hidden files;
  agent `cl_write_file` refuses them (no blind diary overwrites); new
  `cl_set_agent_visibility` command emits `cl:changed` itself (DB-only flip,
  invisible to the fs watcher); eye/eye-off toggle on the Library tree row.
  Known limit, stated in the tooltip: raw path reads are NOT blocked — that's
  the read-scope gate (issues.md #1).
- **`50e673e`** — bindings regen.

Gates: cargo 1038 tests green; frontend tsc/vitest/build green. Live smoke
pending an app relaunch (the running app predates the commits).

---

## 2026-08-05 — vision consolidated in the CL; duo→plugin direction recorded

The project CL's `vision.md` became the single home for vision material:
the AI-car bullet rewritten (gate semantics clarified: a hands-off run is
impossible while any gate is closed; full autonomy = first-prompt
authorization + deliberately opening every gate), the duo/conductor
paragraph moved out of the CL `notes.md`, and the memory-hierarchy analogy
(context = cache, session-docs = RAM, CL = disk — from PLAN.md's
CL-stitching item) given a vision-level writeup. ARCHITECTURE.md's
app-as-conductor line was de-metaphorized (`ed49a87`).

The user then stated a direction shift: **Rain becomes a plugin only;
the core's identity is the agent harness/system itself, never an agent
count** (PLAN.md "Direction — the agent harness is the focus" + CL
`decisions.md`). The canonical docs
now carry it as planned/unscheduled; code and README still
implement/describe the live duo, deliberately, until a migration is
scoped.

---

## 2026-08-05 — declare_working: background work is a declared state, not a stall

Found live within hours of the watchdog shipping: HANDS ran the five commit
gates as a harness-background task, its turn ended, and the watchdog — correct
by its own rules — nudged a session that was actually mid-build. Harness
background work is invisible to the activity tracker, so bare-Idle and
working-invisibly were indistinguishable. The workaround (an honest `halt`)
mislabeled the state as awaiting-user.

New HANDS-only MCP tool `declare_working(reason, expected_seconds?)`: sets a
per-session in-memory flag (spawn-registered with the bridge, mirroring
`awaiting`) that the idle-unflagged watchdog treats like a pending tray row —
chip + nudge suppressed — and shows a neutral primary-tinted WORKING badge
(tile + header) with the reason. The TTL is load-bearing (clamp 30–1800 s,
default 600): an unexpiring flag would recreate the silent stall one level up,
so expiry is checked every poll (badge can't linger through a busy stretch)
and, because `idle_since` keeps accruing under suppression, an expired
declaration fires the nudge within one poll — a dead background task surfaces
in ~10 s, not 90. Cleared only by expiry, the user's next message, or close —
never by activity transitions (EYES blocking finding on the v1 plan: a
declared state persists across turns; the original shape was a one-way clear
that the first Busy blip would have killed). In-memory on purpose: a restart
kills the background tasks the declaration was about, so it must not survive
one. Prompt contract added to the never-stall-silently section: re-declare on
each wake, honest reason + duration, never for user- or peer-waits.

## 2026-08-05 — never stall silently: the idle-unflagged watchdog (issue #25)

The user's own words defined the invariant: "work should be continuous after
the first prompt. User should be asked when halts/paused… I go AFK when I
ask/instruct, hoping to get back to a question or halt flag." The archive says
the invariant was broken routinely — **24 "what happened?" messages across 22
sessions**; of 13 bare stall-probes analyzed, **9 interrupted a session with
zero open tray flags**, after silent gaps of 2 min to 9.7 h. Four shapes: turns
that die mid-tool; duos that settle without parking anything (Rain's last words
before a 3-hour silence: "*(Holding.)*"); dangling promises ("committing when
the gate suite completes" — nothing followed); and waits on a peer the user
cannot see.

The mechanical hole: every proper yield (`ask_user_choice` / `halt` /
`mark_awaiting_user`) lands the session in a visible `AwaitingUser`, but a turn
that simply ends lands in bare `Idle` — and the Batch-7 stall watchdog only
monitors *busy* agents, so `Idle` conflated "settled, your move" with "stalled
mid-task" forever.

The fix extends the per-session watchdog (`core/watchdog.rs`): a session
continuously `Idle` past `IDLE_GRACE` (90 s), with ≥1 user prompt broadcast and
zero pending `session_tray` rows, now (a) flips a deduped `session:attention`
event → an amber **NEEDS DIRECTION** chip on the dashboard tile + session
header, (b) persists a chat-visible `system_notice` row, and (c) nudges HANDS —
once per user-silence window — to declare state with a tool: continue / park a
question with a recommendation / halt / close-ask. The nudge is skipped (chip
stays) while HANDS health is dead/retrying/stalled; a new user prompt re-arms
the window via a race-free in-memory `user_broadcasts` counter bumped in
`AppState::broadcast`. Detection is host-side by design — a claude-code Stop
hook fires before the final text is routed and cannot know whether the peer is
about to wake, so it would false-block ordinary turns.

Pure decision fn + 6 new unit tests (once-per-window, pre-first-prompt quiet,
pending-tray suppression, hands-down chip-only); full suite green (968 lib).
`get_session_runtime` seeds the chip on mount; `system_notice` renders as a
centered warning line, never as user prose. Prompt half: a "Never stall
silently" section in the HANDS general rules, including the nudge contract.
User-picked variant (2026-08-05): chip + one-time nudge at 90 s.

**Follow-up (`96d8a64`), found by the live smoke:** first smoke came back NO
FIRE with the trigger condition present — the in-memory `user_broadcasts`
counter was 0 because the app had restarted and every input since arrived via
the tray-resolve path, which never bumped it. The counter now seeds at spawn
from `count_user_messages` (user text rows only; synthetic `author=user`
phase/notice rows excluded) and bumps on Delivered / fell-back tray resolves
(`StaleGateNeedsConfirm` excluded — nothing was delivered). A mid-task app
restart no longer disarms the watchdog, and tray-only engagement arms it.

## 2026-08-04 — stop waking the reviewer to say "holding" (#8, first half)

`advance_phase` fed its transition notice to BOTH agents' stdin. For EYES that is
a wake with nothing attached: no new content to review, so the turn it produces
is an acknowledgment — "Old plan — holding for Brian's plan", 40 chars — and each
one burns a slot of the `VOLLEY_HARD_CAP` budget that #24 showed was being
exhausted before substantive reviews could get through.

Measured in the session that made the change, rather than assumed. Rain's filler
turns land 7–45 s after each phase change:

```
15:16:53 phase → 15:17:00 (7s)  "Old plan — holding for #24 apply output."   40 chars
14:52:37 phase → 14:52:45 (8s)  "Old plan — waiting for the new plan…"       64 chars
15:03:00 phase → 15:03:35       "Clean verify. Brian's triage was correct…" 112 chars
```

Exactly the sub-200-char shape #24 found burning the budget in `s-d16364ee`. One
more arrived, unprompted, while this fix was being written.

The notice now goes to HANDS only. EYES loses no information: every peer forward
carries the current phase in its envelope
(`peer_forward_message(from, body, phase, …)`), so she reads the new phase on the
next message that actually contains something. Provider-limit peer notices still
wake her deliberately — different path, unchanged.

**Not shipped: the second half.** EYES proposed also auto-suppressing turns whose
tool calls include `session_doc_write` / `advance_phase`, leaving the router's
length proxy to forward substantive ones anyway. That is a sound design — but
there is measurement that phase notices drive the filler and none that doc-write
turns do. Shipping an evidenced change and an inferred one together would make
the next measurement unreadable. Filed as #8's remaining half; the telemetry that
can settle it landed today.

**No unit test, stated plainly.** This is a call-site routing change with no pure
logic to isolate, and `state.rs`'s test module covers only static pieces (live
session tests need `RUN_LIVE_TESTS=1`). The real verification is the same query
that motivated it, re-run after a restart: no EYES turn should follow a
`phase_change` row.

## 2026-08-04 — the volley breaker stops the loop without eating the message (#24)

Found by the observability shipped hours earlier, in a session we weren't even
working in. `forward_events` showed **40 of Rain's forwards destroyed** in
`s-d16364ee` (2026-08-01) with `reason='hard_cap'` — among them
*"`58fae66` is the risky one — rejection without repair"*.

The chain, all four links previously invisible and now measurable: Rain is woken
on every HANDS event (issue #8) → 54 of her 61 text turns in that window were
under 200 chars → those filler turns burn the 18-slot `VOLLEY_HARD_CAP` budget in
minutes → the breaker fires → and what it discards is the substantive minority.
The user was NOT quiet: 19 typed messages in the window, one every ~5 minutes,
each resetting the counter. The duo simply exceeded 18 peer-forwards between
them, repeatedly.

The cap exists to stop a runaway LOOP. It never needed to lose the MESSAGE — the
same realisation already applied to `awaiting`, which used to silently discard
turns and now holds and replays them. `route_forward` now RETURNS the capped
forward instead of dropping it; the router keeps the newest per agent (so a
genuine runaway still cannot grow the queue — one slot each, newer overwrites)
and releases it on the next `FlushHeld` once the budget has room. The volley is
still broken, the input still unlocks, the message still arrives.

Convergence stays lossy on purpose: that breaker suppresses REPETITION, where a
held copy would duplicate what already landed.

Two things this cost. `record_drop(reason='hard_cap')` is gone — the message is
no longer lost, and a table whose contract is "a row means a message was lost"
must not log successes; the breaker firing is now a `warn!`, which survives
because the log sink shipped the same day. And the `FlushHeld` guard had to stop
short-circuiting on `held.is_empty()`, which would have skipped the new release
path in the common case — caught by the test, not by review.

## 2026-08-04 — close the two loose ends the same day's work opened

Both items are debt created earlier today, cleared before closing the session.

**A held forward that never flushes is no longer invisible.** `forward_events`
is drops-only by design — and a HOLD is not a drop, so the forward fixed in
`b87f97a` (held while a question was parked, never flushed because
`resolve_choice` / `advance_phase` didn't send `FlushHeld`) left no trace
anywhere. Fixing the bug did not close the blind spot that hid it.

Held entries now carry the instant they were held. Two rows can result, and both
mean what the table says it means — a message was lost:

- `held_late` — released, but after ≥15 minutes. A hold ends at the user's next
  action, so an old one means some path cleared `awaiting` without flushing and
  the peer sat half-deaf until an unrelated message shook it loose. That is bug
  B's exact signature.
- `held_stranded` — still held when the router's command channel closed. The
  session went away; it was never delivered at all. Previously these vanished
  with the router's local queue.

A prompt hold→flush records nothing, so every row keeps meaning a loss — pinned
by a test in both directions.

**`activity_events` gets a retention sweep.** `purge_activity_events(90)` runs
from the same boot sweep as `purge_resolved_tray(90)`. Volume is small by
construction, but "small per session, forever" is unbounded, and this data home
already carries one unrotated append-only sink — a second would have been a
choice rather than an oversight.

## 2026-08-04 — record the activity timeline so the input-lock question is answerable

Second half of the observability work. The file sink (below) rescues what the
host SAYS; this records what state the session was IN, which is the half the
reported bug needed.

`messages` already persists every agent text / tool_use / tool_result with a
timestamp. `SessionActivity` — the derived per-session state that gates the chat
input — was broadcast-only: `notify_session_activity` is a fire-and-forget
`event_tx.send`, so the state side evaporated the moment the UI consumed it.
"Brian emitted while the input was unlocked" was therefore reportable by a user
and reconstructable by nobody. It is a join, and half the join wasn't written
down.

`0042_activity_events` writes one row per transition: state, both per-agent busy
flags, timestamp.

**The per-agent flags are the point, not decoration.** The derived state
collapses both agents into a single `busy`, and `awaiting`/`paused` outrank it
entirely — so `state` alone cannot answer "was anyone actually working then?".
A row is written on a change to EITHER the derived state or a per-agent flag,
mirroring exactly what the frontend already receives. Recording only state
changes would leave a flag flip inside a stable `awaiting_user` unrecorded, so
the newest row would keep asserting `brian_busy = 1` after Brian stopped — a
stale claim, which is the failure mode this whole day's work was about. A test
drives that exact sequence.

Written detached: `recompute_locked` is synchronous and holds its state mutex.
`Handle::try_current()` rather than a bare `tokio::spawn`, because the tracker's
mutators are plain `&self` methods with no guaranteed runtime at every call
site — no runtime means no row, never a panic. The `Storage` handle is taken
with `try_lock` (every holder does clone-and-drop, so the window is nanoseconds)
and a miss drops the row rather than blocking the signal that gates the input.

## 2026-08-04 — give tracing somewhere to go

bot-hq had no log sink. `init_logging` was `tracing_subscriber::fmt()`, which
writes to stdout — and a `.app` launched from Finder has no terminal attached,
so every `warn!` the host emitted was discarded.

That is not a small gap, and the repo already documents the cost twice in its own
migrations. `0040_cancel_events`: *"there is no log sink configured, so they went
to a stdout nobody captured. 21 Stops across 13 sessions left zero forensic
trace."* `0041_forward_events`: dropped peer-forwards were *"a bare `debug!`"*.
Both tables exist to persist what a log line already said. Two of today's own
fixes were also invisible for the same reason — `"router FlushHeld not sent"` and
`"peer-forward DROPPED"` have been firing into nothing since they were written.

Now: a rolling daily file under `<data_dir>/.local/logs/`, 14 files kept, ANSI
off, alongside the unchanged stdout layer.

The reason it never existed is an ordering problem rather than an oversight —
`init_logging()` ran at `main.rs:63`, before `.env` loaded and before
`Paths::from_env()`, so the data dir wasn't known yet. Logging now comes up right
after `paths.init()`; everything before that propagates through `?` and is printed
by main, so no diagnostic is lost by the wait. Side effect: a `RUST_LOG` set in
`.env` now actually applies, where before the filter was built first.

Two failure modes are pinned by tests: `Paths::init` must create `logs_dir` (the
appender panics without it), and the appender config must really produce a
written file — no other test could catch a bad prefix or a builder error, because
every CLI subcommand returns before `init_logging` and a full launch needs the
single-instance lock. `main` holds the non-blocking `WorkerGuard` for the
process's lifetime; dropping it silently discards buffered lines.

## 2026-08-04 — two self-audit fixes found by running the system

Both surfaced during an ordinary session, not by code reading.

**The Apply-entry nudge ordered a call the tool refuses.** It told HANDS that if
Rain hadn't reviewed the plan yet, "wait for it (`mark_awaiting_user`)". But
`mark_awaiting_user` hard-refuses any reason naming a peer
(`jsonrpc.rs::peer_shaped_reason`, shipped `3282708` to end a 100-minute
mutual-deferral deadlock) — and its refusal text then tells the agent to do the
opposite of what the nudge just ordered. The nudge fired three times in one
session; it was declined each time, and a compliant agent would have stalled.
Reworded to name the real mechanism (a turn's output forwards to the peer
automatically, so saying it in chat IS the wake) and extracted to
`AppState::APPLY_ENTRY_NUDGE` with a guard test asserting it never mentions
`mark_awaiting_user` or `ask_user_choice` again.

**Held peer-forwards only flushed when the user TYPED.**
`RouterCommand::FlushHeld` had exactly one sender — `broadcast`. But
`clear_awaiting` has three callers: `broadcast`, `advance_phase`, and
`resolve_choice`. So a forward the router held during a parked question stayed
held when the user ANSWERED that question from the tray, or when the phase
advanced; it surfaced only on the next typed message, leaving the peer half-deaf
in between. Extracted `flush_held` and called it from all three paths.

Two details that shape the fix: it must run at the END of each path, never inside
`clear_awaiting` — clearing happens before the user's own message is delivered, so
flushing there would release held peer chatter ahead of what the user just said.
And it is deliberately NOT called in `resolve_choice`'s paused arm, where the wire
is stashed for the next broadcast on purpose. The helper takes the bare channel
rather than `RouterControl` so it can be tested without standing up a session.

## 2026-08-04 — stop the chat stream hiding what the agents are doing

User report: "agents don't fully surface on the chat stream what's happening
under the hood. The input sometimes unlocks (I thought they are not working
anymore), then after a few seconds Brian surfaces something." Demonstrated live
— they were typing while HANDS was mid-investigation.

Two causes, both in the frontend; no backend change was needed.

**The unlocked input implied "they stopped".** `SessionActivity::derive`
(`core/activity.rs`) ranks `awaiting` above `busy` deliberately: parking a
question must re-open the textarea or the user couldn't answer it. But
`TurnStatus` rendered only inside `ChatInput`'s locked branch, so the per-agent
busy flags — which `recompute_locked` emits on EVERY activity event regardless
of derived state — had nowhere to land. Added `StillWorkingNotice`: when the
input is open and an agent is mid-turn it reads "Waiting on your answer · Brian
is working — the turn hasn't ended yet" (or "Stopping · … — finishing the
current tool" when paused). The textarea stays enabled throughout; this is a
labelling fix, not a locking one. `isLocked` and the derive priority are
unchanged.

**Tool calls were captured but not legible.** `duo.rs` already persists every
`ToolUse` with its full input and every `ToolResult`, live. The losses were in
rendering: the collapsed pill clipped its preview to 80 characters on a single
`truncate`d line — enough to hide the tail of any real command — and a
`tool_use` still executing looked exactly like a finished one, so a five-minute
`cargo build --release` and a 20 ms `Read` were indistinguishable with nothing
moving on screen meanwhile. Previews now hold 200 characters and wrap to two
lines (`break-all`, no horizontal scroll), and `ChatPane` derives the set of
resolved `tool_use_id`s so an unresolved call renders with a `⟳` marker and a
live elapsed counter. Elapsed is computed from the message's `created_at`, not
mount time — the pane is virtualized, and a mount-anchored timer would restart
at zero on every scroll.

Deliberately not done: holding a "still working" label across turn boundaries.
The pump cannot distinguish "about to auto-continue" from "done", so a sticky
label would trade one wrong impression for the opposite one.

## 2026-08-04 — name the gates that overtook a question in its replay (#18)

Second half of issues.md #18. `40e876e` age-stamped out-of-band replays; this
makes the warning specific — the replay now names the gated commands the user
approved *after* the question was parked, which is the event that actually
mooted the premise in the 2026-06-23 incident (a staging-push choice sat
through the push it asked about, then replayed as live state).

Deliberately NOT auto-withdraw, which is what the issue originally asked for.
Two facts from the code closed that door: `action_gate` already dedupes
identical pending gates at park time (`pending_gate_for_command`), so
"withdraw the gate whose command later ran" is unreachable; and the row that
was actually mooted in the incident was a plain `choice` with no
`command_text`, so linking it to the command needs question-text matching —
a heuristic whose failure mode is silently binning a live question. Evidence
on the replay kills the same failure with no guessing.

The wording is load-bearing: a tray row proves the user APPROVED a command,
not that it SUCCEEDED (`maybe_run_gated` writes failures into the message
body, not back onto the row), so the block says approved and tells the agent
to check the outcome rather than asserting either way.

`Storage::answered_gates_for_session` + `SignalingBridge::gates_approved_since`
(fail-open everywhere — this decorates a delivery that must never fail) +
a `mooting` param on `oob_resolution_body`. Session-scoped: a push from
another session can moot a question here too, but nothing in `session_tray`
observes that. 6 new tests.

## 2026-08-04 — correct the dev data-dir guidance in the shipped docs

Surfaced during a Context Library audit: four committed docs still told a
developer to run against `~/.bot-hq-dev/`, a split the repo retired
2026-05-15. `.env` has the override commented out and the app runs on
`~/.bot-hq/` for dev and prod alike, so `.env.example` — which set
`BOT_HQ_DATA_DIR=~/.bot-hq-dev/` uncommented — made `cp .env.example .env`
contradict the checked-out setup on step one of the README.

`BOT_HQ_DATA_DIR` is unchanged and still read by `paths.rs`; only the
guidance moved. It's now documented as an opt-in for the one case that
still needs it: running an installed release beside a source build, where
both would otherwise share a Context Library, sqlite DB, and instance lock.
`.env.example` ships commented out (its header also still said "Rust +
Slint rebuild" — Slint went in May), and `CLAUDE.md` / `README.md` /
`ARCHITECTURE.md` now agree with it. `README.md`'s env-var table was
already correct. `PROGRESS.md` history and `docs/rebuild-archive/` are
left as written.

## 2026-08-04 — misinformation archive study → three evidence rules (s-11b73814)

Searched all sessions (May 20 → Aug 4) for the user's misinformation
frustration moments: 15 genuine, 9 real incidents, each root-caused with
message-id evidence (session docs, s-11b73814). Verdict: model supplies
the false claim (present in all 9 delivery paths), CL/reports carry it,
app gaps enable it (never primary). Shipped the prompt-rule halves into
`general_rules.rs` (+3 pinning tests):

- **Status words need same-turn evidence** — no PENDING→RESOLVED by
  inference at CL close (the 2026-07-24 inversion of Tom's decision).
- **Re-verify state claims before an outbound report ships** — EOD
  snapshots decay same-day (2026-08-04, caught by a stakeholder).
- **Third-party signals are dated claims** — stale assignee/status doc
  vs verbal handoff → ask, don't infer (the 2026-07-10 P1/P2 mixup).

R1–R6 recommendations filed in CL issues.md (#18–#23).

Second slice (same session): three mechanical halves shipped —
- **#18** OOB question replays carry "**Asked:** Xh Ym ago" + a re-verify
  warning when ≥10 min old (a mooted 2.5h-old premise was once adopted
  as current repo state).
- **#23** repo-less projects (incl. `_globals`) get an age-based
  `⚠ possibly stale` fallback (≥30 d), worded as age, not code drift.
- **#19** `cl_write_file` advisory status-lint: a pending→resolved flip
  with no evidence marker (sha/URL/date) beside it warns in the result;
  the write still lands. Auto-withdraw (#18) and a blocking lint (#19)
  remain open as design work.

## 2026-07-28 — archive-study remediation batch (s-92f76f02)

Studied 9 archived duo sessions (Jul 23–27, both build eras) with 7
parallel deep-read auditors; synthesized 30 systemic quality limiters
(investigate doc, session s-92f76f02), then shipped the mechanical
batch — one commit per fix:

- **Doc archive-on-supersede** (`0c1acc9`) — phase-doc rewrites archive
  the old body as `{slug}@{n}`; a 23-finding audit had been destroyed
  by four batch rewrites.
- **CL write safety** (`8194ca6`) — shrink/empty-replace guard +
  `confirm_shrink`, `mode:"append"`, lazy git-versioning of the whole
  library (the "archived in git history" belief is now true), and
  `abs_path` on cl_index_search rows.
- **peer_ack substance guard** (`4b05e66`) — a >200-byte acked turn
  forwards anyway, tagged; four reviews had been silently destroyed.
- **OOB answer completeness** (`a3c278e`) — resolutions restate the
  full option menu; off-menu picks flagged as the user's own words.
- **Reviewer liveness** (`69088d5`) — bridge RPC activity within 60s
  overrides a stale Stalled/Dead health verdict in the commit gate.
- **mark_awaiting_user peer-reason guard** (`3282708`) — refuses
  peer-shaped reasons (the 100-min mutual-deferral deadlock).
- **action_gate park redesign** (`2ab07b4`) — parks with a `gate_id`
  instead of blocking to client timeout; OOB outcome delivery; new
  `gate_status` tool; pending-only dedupe; age-based (15 min) stale
  confirm; bash executor (sh heredoc deaths); reject-with-reason
  affordance in the gate card.
- **Provider-limit classification** (`1212d4c`) — quota deaths become
  stalled-health + tray halt + one peer notice instead of agent speech
  (3h13m of silent downtime in the archive).
- **Native window hygiene** (`9cfa2e8`) — spawn-time chat warning when
  a native model row lacks `context_window`; Kimi K3 row set to 1M
  (user-verified OpenRouter value).
- **Prompt pack** — HANDS question discipline (no what-next polls under
  an open mandate; options carry constraints; batch decisions), EYES
  same-turn-evidence rules (five fabricated-assertion incidents), a
  shared existence-claims rule, and the parked action_gate contract.

Remaining findings filed as specced issues in the CL (`issues.md`
#3–#17): read-scope gate, review↔SHA binding, EYES verify channel,
wake-policy filtering, arc ledger for long-context EYES, scratch dirs,
zombie-session repair, Tool Gate argv matching, outward-text gating,
close-time sweeps, quota auto-resume, and two `needs-user-decision`
items (disposition re-verify; EYES user-channel).

---

## Current state

944 Rust tests passing (881 lib + 37 external MCP + 5 native MCP + 7
signaling + 14 storage) plus 175 frontend Vitest. Release build clean. Version
**1.0.0-rc2** (pre-release for Windows friend-testing; `1.0.0` reserved
for the official market launch). The codebase has moved well past the May
Tauri v2 migration — live on main since: the **EYES-sign-off commit
gate**, the **interrupt redesign** (stdin `control_request` cancel +
`SessionActivity` state machine), the **peer-forward router extraction**
(`core/router.rs`), the **plugin runtime v1** (2026-07-04), four
plugin-runtime workstreams from 2026-07-05 (**per-plugin CSP
override tier**, **spawn_session capability**, **linked installs**, the
**push-event + view-alignment paper-cuts**), and the **session subtabs
arc** (2026-07-18): Workspace | Context | Terminal, and the
**performance optimization sweep** (2026-07-19, below), and the **native
agent loop** (2026-07-26/27, below).

---

## 2026-07-27 — Native agent loop, then an audit that found it half-wired

`b8548cc..2a17593`. Two halves: the connector itself, and six remediation
commits from auditing it.

**The connector.** An agent can now run on bot-hq's own Rust loop
(`src/agents/native/`) instead of a `claude-code` subprocess, opted into
per saved model. On a third-party API key the CLI buys nothing —
subscription OAuth is bound server-side to claude-code, so a gateway-key
agent was paying ~418 MB RSS for an opaque loop and context accounting we
could only scrape. Owning the loop means computing occupancy from our own
request accounting, gating tools inline instead of through the exit-2
PreToolUse hook, and enforcing a read root that `Read`/`Grep`/`Glob`
never had (they aren't `Bash`, so they never reached the Tool Gate).

It was additive, not a rewrite: `AgentHandle` is a pure channel struct,
so the duo pump, the router, the policy layer, the UI and the context
meter cannot tell which backend they got. v1 is EYES-only —
`resolve_agent_kind` hard-guards HANDS onto the CLI, since Brian's
subscription pins him there.

**Then the audit.** Twenty-three findings; nineteen fixed across five
batches. The ones worth remembering:

- **Native failures were invisible.** Every user-facing failure went
  through `AgentEvent::Error`, which the duo pump only `warn!`ed — so API
  errors, refusals, the tool-cycle cap and the context-ceiling stop all
  rendered as empty turns.
- **A ceilinged reviewer read as healthy.** The ceiling latch keeps the
  event channel open on purpose (closure would respawn the agent with no
  history), so `Dead` was never emitted and a latched refusal completes
  too fast to look stalled — leaving the fail-closed commit gate with
  nothing to match on. It now reports terminal health.
- **The feature was unreachable from the Agents tab.** `native` lived
  only on `models`, but every session that names no model resolves
  through `agent_configs` — so assigning a native model there silently
  spawned claude-code. Migration 0038 mirrors both columns across.
- **`search_files` answered "no matches" for everything.** Enumeration is
  capped at 500 entries and an alphabetical walk of this repo reaches
  66,667 before the first `src/` path. The first fix pruned
  `.git`/`node_modules`/`target` and *still* left 48,039 ahead of it — a
  2.4 GB gitignored `bench/` no hardcoded list would name. Enumeration
  now asks git (`ls-files -c -o --exclude-standard`), which is 354
  entries here and exactly the set a developer means by "the repo".
- **Four answers to "is this agent EYES?"**, two of them wrong the same
  way: `CommandPolicy::for_agent` promised an unrecognised agent gets no
  shell and had tests asserting it, while nothing called it and
  production handed that agent a read-only one. Collapsed into
  `AgentRole` (`src/agents/roles.rs`).

Also: HTTP timeouts on both native clients, repo walks moved off the
async worker, concurrent tool execution, a startup sweep for orphaned
conversations, `list_models` for the external driver, and the refusal
probe stopped littering the repo root.

**Open:** B6 overflow handling (see PLAN.md). The native loop neither
compacts nor stops — it reports occupancy, says so once past 85%, and
keeps working; past 100% the gateway drops the oldest turns and the user
decides whether to close the session. An 85% hard stop shipped briefly
and was removed the same day: on a 1M window it discarded 150K tokens of
usable capacity and ended the session for the user rather than by them.
`native-accounting.jsonl` is the measurement input for whatever replaces
it.

---

## 2026-07-22 — Stop is now pause-first; a stopped session can't wake itself

The Stop button is multi-purpose via a **post-stop bar**, not a pre-menu:
one instant click interrupts both agents exactly as before, but the
session lands in a new **`Paused`** state instead of `Idle` — textarea
open, plus "⏸ Paused — [Resume] [Close session]". Walk away (nothing
auto-wakes a paused duo), type to steer (Send clears the latch and rides
the existing preempt + reconcile machinery), Resume re-nudges both
agents, Close routes to the existing force-close confirm. Origin: the
user wanted a way to park a session before folding the laptop, plus a
fix for "Stop sometimes doesn't fully stop."

- **Root cause of "keeps working after Stop" (confirmed):** when an
  agent didn't honor the interrupt within 2s, the SIGKILL fallback fired
  and the pump's `Exited` handler best-effort forwarded the trailing
  buffer — waking (respawning) the peer. The duo restarted itself after
  the user believed it stopped.
- **Structural fix:** the router now HOLDS all Forwards while
  `cancelling || paused` (`ActivityTracker::holds_wakes`, read at
  dispatch time inside the single router task; gate raised BEFORE the
  interrupt fires). Held forwards still settle the sender idle so the
  escalation window sees the interrupt land. `RouterCommand::FlushHeld`
  (sent by `broadcast` after clearing the latch) releases them FIFO
  behind the user's message.
- **`SessionActivity::Paused`** (wire `"paused"`): priority
  `cancelling > paused > awaiting > busy > idle`; ordering contract
  set-cancelling-before-set-paused (latching first would flash an
  input-enabled frame — caught by test).
- **Answered tray questions can't restart a paused duo:**
  `resolve_choice`'s OOB wake stashes into `pending_paused_wakes` while
  paused; drained by the next `broadcast` behind the user's message.
- **`resume_session`** (new tauri command): guard (live + paused) then
  broadcast a host-authored resume notice — reuses auto-heal, preempt,
  and the post-Stop reconcile directive wholesale.
- Audit: every dispatch path already clears `awaiting` before waking
  agents — the suspected AwaitingUser-masks-Busy window doesn't exist.
- Tests: +5 Rust (derive matrix, wire lock, settle-to-Paused ordering,
  idle-latch, router hold/flush-exactly-once), +4 Vitest (paused bar
  render/resume-latch/close-routing/hidden-outside-paused).
- Commits: `0d7e5c3` (Paused state), `c50cf4b` (latch + wake gating),
  `5d718d9` (paused bar UI), `92d6249` (bindings).

---

## 2026-07-21 — CL review queue removed; agents write the CL directly

The CL's human-review queue (shipped in the CL v2 arc) is gone: in
practice every queued edit was approved unread, so the queue → review →
approve loop added friction without safety. Agents now write CL content
directly; user-side Library editing is unchanged.

- **New `cl_write_file(project, file_path, content)` MCP tool**
  (`bridge/cl_write.rs`, HANDS-only via `CL_MUTATE_TOOLS`): guarded
  create-or-replace inside the project's CL root — relative-path +
  traversal checks, 1 MiB cap, atomic tmp+rename, mkdir-p for new
  subfolders, automatic `cl_rescan`, and it lifts the close-out learnings
  nudge like `cl_rescan` does. Bot-hq-owned `_globals` system files
  (`custom-instructions.md`, `custom-general-rules.md`, legacy `agents/`)
  are refused so an agent can't rewrite its own standing rules.
- **Review queue removed end-to-end:** its storage module + row types,
  its bridge module (file/list/approve/reject + conflict detection),
  both MCP descriptors + dispatch arms, its four Tauri commands + view
  types, its queue-changed event chain, and the frontend queue surfaces
  (queue component + diff util, Context Manager queue pill + badges,
  SessionContextTab queue tab). Migration `0035` drops the queue table
  (historical rows discarded — they were rubber-stamped approvals).
- **Prompts re-pointed at direct writes:** the general-rules CL section
  is now "Keeping the CL fresh — write the delta at close" (read the
  CURRENT body, append under `## Learnings`, write the FULL replacement),
  Brian's role close-out line, the `close_session` nudge, and the
  Maintain-CL dispatch prompt (queue-triage step dropped) all point at
  `cl_write_file`.
- **Kept:** the measurement layer (`retrieval_events` + the Context
  Manager Measurement card, `cl_reads` audit) and every user-side editor
  surface.

## 2026-07-19 — performance optimization sweep (heat/lag on MBP 14)

A whole-app perf pass (brief: "optimize for maximum performance, remove
no features — the MacBook heats up / sometimes lags"). An audit put the
sustained cost in the webview render path plus two Rust hot paths; five
feature-preserving batches (A–E) landed across four commits (`f6d45ed`,
`8d0171c`, `9ea25ee`, `5c7519d`). Continued from session `s-5b3fe603`,
which scoped the audit and shipped A–C before running out.

**A — pure-Rust untracked-file diff (`f6d45ed`).** `compute_apply_diff`
spawned one `git diff --no-index` subprocess per untracked file on every
Apply-tab recompute; the add-only unified diff is now built in-process
from the file bytes (one `git ls-files` remains).

**B+C — chat list virtualized into a ChatPane render boundary (`8d0171c`).**
Extracted the chat list out of `SessionView` into `components/ChatPane.tsx`,
virtualized with `@tanstack/react-virtual`:

- **Render boundary:** ChatPane owns the chat-store subscription + the
  `agent:messages:batch` listener, so per-batch re-renders stop inside it
  — the SessionView shell (header, subtabs, DocumentPane, ChatInput) no
  longer re-renders on every batch.
- **Virtualized** via `useVirtualizer`: only the visible window (+
  overscan) is in the DOM regardless of history length. Sticky-bottom
  auto-follow + "↓ Jump to latest" preserved; `useStickyScroll` retired
  (inlined — the bottom-pin effect needs the virtualizer's `totalSize`).
- **Tool-pill expand state lifted** into ChatPane (a Set keyed by message
  id; virtualized rows unmount on scroll). **DocumentPane** memoized + its
  diff grouping moved to `useMemo`. New `ChatPane.test.tsx` (3 cases).

**E — fs-watcher build-churn pre-filter (`9ea25ee`).** The notify
debouncer callback forwarded every changed path over the mpsc; a
`cargo build` / `npm ci` flooded the channel with `target/` /
`node_modules/` paths that only woke the watcher task to be dropped
downstream. They're now dropped on the notify thread first. Build-dir
NAMES only, **not** the `.`-prefix rule — the callback sees the absolute
path and the CL dir lives under `~/.bot-hq/`, so a dot-rule check would
match `.bot-hq` and drop every CL event (the dot-rule stays downstream on
the repo-relative path). Unit test pins the `.bot-hq` case.

**D — session terminal on the xterm WebGL renderer (`5c7519d`).** The
keep-mounted terminal used xterm's DOM renderer (the slow path for
streaming build output). Loads `@xterm/addon-webgl` after `open()`; on
context loss the addon disposes and xterm falls back to DOM, and if WebGL2
is unavailable the try/catch leaves the DOM renderer in place — so it can
only speed up or no-op (no feature change).

Deliberately not touched (audit-deferred): the rare `session:resync`
key-invalidation herd, `parse_diff_lines` allocations (noise after A),
BatchEmitter / fs-watcher debounce / plugin heartbeat (load-bearing),
`cursorBlink` (UX). `Markdown` is already `memo`'d, so the virtualization
alone neutralized the "markdown re-parse per batch" concern.

Follow-ups (advisory, non-blocking): ChatPane test-coverage gaps (loading
/ empty / sticky states); the pre-existing narrow race between the initial
`get_session_messages` load and the first live batch. A live GUI eyeball
is still worth doing for the scroll/virtualization + terminal render feel
(not headless-provable).

---

## 2026-07-18 — session subtabs: Workspace | Context | Terminal (agent-drivable PTY)

The session view is now a tabbed container (from the CL `ideas.md`
brief), five commits `08ab03d` → `db020c4`:

- **Subtab scaffold** (`08ab03d`): SessionView restructured — full-width
  header, `SubTabButton` pill row, three keep-mounted `role="tabpanel"`
  panels (inactive = `hidden`, so chat scroll / editor state / the xterm
  buffer survive switches). Workspace = the previous chat ⇄ splitter ⇄
  DocumentPane view, unchanged.
- **Context subtab** (`cef0e2c`): the Context Library scoped to this
  session's project, in-room — Files (project tree via
  `cl_index_search(project)` + a lean `cl_read_file`/`cl_write_file`
  editor with the same truncated/binary lossy-save guard as the main
  editor) and a review-queue tab (queue removed 2026-07-21).
  Mounts on first activation; repo-less sessions get an empty state.
- **PTY terminal backend** (`35e9e19` + `eefe88c` bindings): one
  `portable-pty` shell per session (`core/terminal.rs`), spawned lazily
  in the session's working repo (worktree-aware), killed on
  `close_session`. Bounded 200 KB scrollback ring with monotonic offset
  + `Notify`; `wait_settle(offset, quiet, cap)` is the output-settle
  completion signal. Dedicated reader thread; ≤40 ms coalesced
  `terminal:output` events (base64). Commands: `terminal_open` (snapshot
  replay) / `terminal_input` / `terminal_resize`.
- **Terminal subtab UI** (`faf8c1e`): xterm.js + fit addon,
  Industrial-Terminal theme, snapshot replay with events queued until
  the replay completes, active-aware refit.
- **Agent terminal tools** (`db020c4`): `terminal_exec` (HANDS-only,
  BLOCKING by default — types the command into the user-visible PTY,
  awaits settle, returns the captured tail; `block:false` for
  long-running processes) + `terminal_read` (both agents; scrollback
  tail as pasteable evidence, works after shell exit). Gate parity: the
  command is classified against the same two-tier Tool-Gate list the
  PreToolUse hook uses via the new shared
  `tool_gate::resolve_keywords`; gate-matched commands are refused and
  routed to `action_gate`. That refactor also fixed `action_gate`'s
  pre-existing global-only keyword resolution. Internal tool count
  35 → 37.

Follow-ups deliberately not in this arc: `terminal:exec` activity dot
on the Terminal pill (design against real usage), plugin-contributed
session subtabs (`session_subtab` inline slot — deferred plugin tier),
PNG terminal capture (text evidence + `webview_screenshot` on a visible
tab cover v1), offset-based dedupe of the open-race snapshot/event
overlap (cosmetic, ≤40 ms window).

---

## 2026-07-18 — issues.md fixes: prime-at-create, consolidated custom-instructions, no horizontal scroll

Three fixes from the CL `issues.md` list:

- **Sessions prime at create (no click needed):** `create_session` now
  `tokio::spawn`s a background `ensure_session_started` after persisting
  the row, so the duo spawns and the CL-opener primer nudge fires the
  moment a session is created — previously agents only spawned when the
  user opened the session (SessionView mount → `respawn_session`).
  Background (not awaited) so the create dialog never blocks on worktree
  materialization; the mount-time respawn doubles as the retry path.
- **One `custom-instructions.md` for all agents:** the per-agent
  `library/agents/<name>/custom-instruction.md` files are consolidated
  into a single `library/custom-instructions.md` appended to EVERY
  agent's prompt (layer 5). One-time migration in `Paths::init` deletes
  untouched template seeds and folds user-modified copies in under a
  `## Migrated from …` heading, then prunes the `agents/` dirs. The
  protected-path guards (`assert_not_protected_globals_path` +
  `isInternalGlobalsPath`) now cover the new root file and keep the
  legacy `agents/` prefix protected for partially-migrated installs.
- **Horizontal scrolling removed app-wide (wrap instead):** every
  `overflow-auto` container is now `overflow-y-auto overflow-x-hidden`
  (the explicit pair — `overflow-y` alone computes the other axis to
  `auto`), all `<pre>` blocks wrap (`whitespace-pre-wrap break-words`),
  the violations table drops `whitespace-nowrap`/x-scroll, the CL editor
  tab bar wraps, and the CL editor textarea soft-wraps (dropping
  `wrap="off"` and the line-number gutter, which can't stay aligned with
  soft-wrapped lines). Shell `<main>` already clips as the backstop.
  A second hardening pass caught what class-greps can't: Markdown prose /
  inline-code / links now break long tokens (URLs), GFM tables got a
  `table-fixed` renderer component (none existed — wide tables clipped),
  ModelsPanel's fixed grid tracks became compressible `minmax()` floors
  (the old ~51rem row minimum clipped the actions column on narrow
  windows), and `html/body/#root` carry `overflow-x: hidden` as the
  page-level backstop.

---

## 2026-07-09 — plugin_sessions: plugins drive their OWN agent sessions (zero-token tutor transport)

`spawn_session` let a plugin CREATE a session; `plugin_sessions` lets it
create AND drive its own — send / wait / read / close — so a panel can
hold a full agent conversation (Cognotify's tutor chat) with NO driver
token and NO port. The host owns the machinery; no credential ever enters
plugin JS. Generic + multi-tenant by design (usable by any plugin); the
safety property is an OWNERSHIP FENCE.

- **Ownership fence (migration 0034, `sessions.created_by_plugin`):** the
  create arm stamps the session with its plugin id; every other arm gates
  on `require_owned_session` (`created_by_plugin == this plugin`) BEFORE
  any core access. A plugin reaches ONLY sessions it created — never the
  user's own, never another plugin's. Absent and foreign sessions fail
  identically (no existence probe).
- **One capability, five commands:** the manifest requests
  `plugin_sessions`; the iframe dispatches `plugin_session_{create,send,
  wait,messages,close}`. A small `required_capability` / `is_dispatchable`
  map in `catalog.rs` bundles them — the five are useless individually
  under the fence, so one honest consent decision beats five checkboxes.
  `required_capability` is IDENTITY for every existing 1:1 command (guard
  test over the whole catalog), so no existing grant changes behavior.
- **Single agent by default; `duo:true` opts into a Brian+Rain pair**
  (`dispatch_session_inner` gained `rain_override: Option<bool>`). `close`
  archives (recoverable). Sessions are dashboard-visible.
- **Consent = install grant, no per-call dialog** (unlike spawn_session).
  For the chat use the user typed the first message, so a per-message
  dialog is redundant; driving is fenced + visible. Residual risk (a
  same-origin material calling create with a non-user prompt) is
  documented in PLUGINS.md — use spawn_session when you need the
  per-create human gate.
- `wait_for_change` promoted to `pub(crate)` (the invoke tier has no
  timeout, so the 25 s server-side await is safe). Reuses the
  `AgentMessage` view (the `list_messages` contract) — no new
  plugin-facing message type.
- Files: migration 0034, `storage/{row_types,sessions}.rs`,
  `tauri_cmd/{sessions,plugin_api}.rs`, `plugins/catalog.rs`,
  `signaling/external_jsonrpc.rs`, `docs/PLUGINS.md`. 6 new lib tests
  (grant mapping, ownership fence owner/foreign/absent, fence-before-core,
  create validation, catalog identity). 731 Rust tests (677 lib + 36
  external MCP + 7 signaling + 11 storage).

---

## 2026-07-08 — External driver: CORS for plugin-panel callers

Plugin panels (custom-scheme `bhq-plugin://` documents) can now `fetch()`
the external MCP driver directly: the webview preflights any cross-origin
request carrying `Authorization` + a JSON body, and the driver previously
405'd the OPTIONS and sent no `Access-Control-*` headers — browser callers
were blocked before auth. `external_server.rs` now answers `OPTIONS` with
204 and stamps every response (401s included, so bad-token is
distinguishable from server-gone in-page) with `Access-Control-Allow-*`
+ `Access-Control-Max-Age: 600`. ACAO `*` grants nothing by itself — the
bearer token still gates every call, and `*` is incompatible with
credentialed mode. First consumer: cognotify's in-viewer tutor chat.
3 new integration tests (external MCP 33 → 36).

---

## 2026-07-07 — CL review-queue conflict handling + Maintain CL triage

Hardened the then-live CL review queue against multi-session races:
filing-time validation + a base content snapshot (migration 0033),
conflict recompute before approval (never dead-ends; explicit force
paths), conflict badges + a lazy line-diff in the queue UI, and queue
triage in the Maintain-CL prompt. The entire review-queue subsystem was
removed on 2026-07-21 (see that entry); migration 0033 remains as
applied history.

## 2026-07-07 — Plugin-runtime hardening from Cognotify operation

Seven prioritized items from building + operating the first real panel
plugin against api_version 1 (14 commits, `b826289` → `a474006`; the
request came in from the Cognotify session):

- **Orphan install dirs unblocked (BUG, reproduced):** install
  conflicts are now registry-first for every mode. A surviving
  `~/.bot-hq/plugins/<id>/` with no registry row is an orphan — the
  install dialog offers consented cleanup ("Remove leftovers &
  install") instead of hard-failing; cleanup never touches a
  registered install. Also closed the latent twin: registered row +
  missing dir used to fall through to INSERT OR REPLACE and
  cascade-wipe plugin_kv.
- **Reinstall… in place:** the drift/re-approve machinery generalized
  to full reinstall — copy↔linked conversion AND same-mode refreshes,
  one consent dialog (target-mode toggle inside), registry row
  UPDATEd, **KV survives**. Managed-copy replacement/removal is stated
  in the dialog, never silent. `materialize_serve_root` extracted and
  shared with install.
- **Spawn confirm hardening:** the per-spawn dialog's prompt pane grew
  (192→288px) with a line-count signpost, plus an advisory warning
  when the last non-empty line ends with ":" (the empty-"Task:"-tail
  incident). The structured task-summary field was considered and
  FILED (PLAN.md backlog) — a plugin-authored summary can itself
  mislead.
- **Push-event scoping pinned by test:** `plugin_events_for_batch`
  extracted pure; two-plugin tests at both layers (watcher emit
  mapping + PluginHost iframe forward). No leak found — the
  "YOUR served directory" contract holds.
- **Consent screen states install mode:** both branches lead with
  "Install mode:" (Linked serve-live vs Copy frozen).
- **Update from source (copy-mode):** migration 0032 records
  `plugins.source_path`; copy-mode cards re-copy assets in place with
  no consent while the source manifest byte-matches the approved one
  (drift routes through Reinstall; URL installs re-fetch via
  Reinstall). Retires per-plugin sync scripts.
- **KV lifecycle documented:** survives disable/re-approve/Reinstall;
  only uninstall deletes it.

---

## 2026-07-05 — Plugin paper-cuts: bhq:event push tier + agent-aligned views

Fourth and final plugin-runtime workstream. Two small surfaces:

- **`bhq:event` push tier** (two hardcoded topics, no general pub/sub):
  `plugin_assets_changed` — the existing fs-watcher (same debounce +
  build-churn filter the A-tab uses) now watches enabled plugins' served
  dirs and PluginHost forwards the nudge into the mounted iframe, so a
  linked plugin's shelf UI refreshes on save without a manual reload; no
  grant, it's the plugin's own directory. `sessions_changed` — rides the
  `list_sessions` grant off `session:created`/`session:closed`. SDK
  grows `onEvent(topic, cb)`; hello-plugin demos the sessions refresh.
- **BREAKING view alignment:** plugin-side `cl_index_search` /
  `cl_folder_search` rows now match the agent MCP shape exactly —
  `project` (was `project_id`), internal `id`/`created_at` dropped —
  via narrow plugin-owned views (the PluginAtomView pattern), leaving
  the shared UI views untouched. Field names pinned by a contract test;
  loud breaking note + per-command row schemas in docs/PLUGINS.md;
  hello-plugin updated. Cognotify unaffected (doesn't request them).

---

## 2026-07-05 — Linked installs: serve plugins from their source repo

Third plugin-runtime workstream (kills Cognotify's dual-write tax:
every material previously had to land in the repo AND the installed
copy). A "Linked" toggle on install serves the bundle straight from the
source directory — one write location, git as truth, edit → tab reload.
Seven commits.

- **Consent-freeze completed for ALL plugins:** grant enforcement moved
  off the disk loader onto a `granted_caps` registry cache seeded from
  the DB-stored (consented) manifest — for normal and linked installs
  alike. Serving resolves through a `serve_roots` cache (normal →
  data_dir copy; linked → the user's repo). Both immune to `reload()`.
- **The consent rule, tested at dispatch level:** editing a linked
  manifest.json changes NOTHING enforced; the Plugins tab surfaces
  "Manifest changed — review and re-approve" (byte-compare vs stored),
  and only the consented re-approve applies new grants. Re-approve is
  an in-place UPDATE — found + fixed a real bug where `INSERT OR
  REPLACE` would cascade-delete the plugin's KV rows via the plugin_kv
  FK (re-approving would have wiped plugin state).
- **Uninstall never touches a linked source** (guard + test); traversal
  and symlink guards treat the linked repo as the boundary (tested with
  roots outside data_dir).
- Contract in docs/PLUGINS.md ("Linked installs (dev mode)"). Live pass
  (link the Cognotify repo, edit, reload) queued with the other two.

---

## 2026-07-05 — spawn_session: one-click session spawn for plugins

Second plugin-runtime workstream (Cognotify's "Manage materials" button
motivator: copy-prompt-and-paste becomes one click). Session CREATION
is now grantable — a conscious, narrow revision of "session control is
not grantable": creating with double consent yes; touching EXISTING
sessions still never. Four commits.

- **Route:** internal only. `dispatch_session` refactored to the house
  `_inner` pattern; the plugin proxy's `spawn_session` arm calls
  `dispatch_session_inner` directly — no HTTP hop, no token, external
  driver fallback rejected (would add an auth surface for zero gain).
- **Double consent:** install-time grant (catalog entry, consent
  screen) PLUS a mandatory per-spawn confirm dialog (plugin name,
  target project, FULL prompt; Reject → invoke rejects). Why: plugin
  content can include user-commissioned materials rendered same-origin
  with the panel — the grant can't distinguish material scripts from
  panel code, so a human sits between any in-origin script and a new
  session. Shell pre-checks the grant so ungranted plugins never raise
  a dialog (Rust's rejection stays the single error source); the bridge
  fails CLOSED if a mount site ever omits the confirm channel.
- **Arm hardening:** creation-only by construction (fresh `s-<uuid8>`,
  no path to existing sessions); empty prompt and unknown projects
  rejected; narrow `{ session_id }` return.
- Contract + rationale in docs/PLUGINS.md. Live spawn (real duo +
  prompt delivery) joins the WS1 e2e in the pending live pass.

---

## 2026-07-05 — Per-plugin CSP extra-origins tier (consent-frozen)

First of four plugin-runtime workstreams from building Cognotify (the
first real panel plugin) against api_version 1. Plugins can now request
extra `script-src` / `style-src` / `font-src` / `img-src` origins via a
`csp_extra_origins` manifest field — additive over the default CSP,
explicit `https://host[:port]` origins only (wildcards, keywords,
schemes, data:/blob: all rejected at install), consent screen lists the
exact origins per directive. Six commits (`a2554c6` → `1a2ef1f`).

- **Consent-frozen grant:** the approved origins are recorded in a new
  `plugins.csp_json` column (migration 0030) at install time; serving
  reads ONLY a prebuilt sync header cache seeded from that column
  (mirrors the `enabled`-cache pattern — the scheme handler can't
  await). Editing an installed manifest never changes the served CSP.
  Closes the upgrade hole: a manifest stored by a pre-CSP host can
  never activate origins after a host upgrade — NULL grant = strict
  default until a re-install re-consents.
- **Two-tier validation** (Rain's refinement): struct parse tolerates
  unknown directive keys (old installs stay loadable + uninstallable
  after upgrade); preview/install re-validate the RAW manifest JSON and
  reject unknown directives + every forbidden origin form.
- `build_plugin_csp(None)` is asserted byte-identical to the previous
  const — non-opted plugins serve an unchanged header.
- Contract documented in `docs/PLUGINS.md` (rules + old-host compat);
  hello-plugin deliberately NOT given origins (least-privilege example).
- E2E against Cognotify (jsdelivr script executing in the viewer
  iframe, non-granted origin blocked) pending a live-app pass — needs
  the running host.

---

## 2026-07-04 — Plugin runtime v1: plugins actually run

bot-hq is now modular for real: the plugin system executes plugins
instead of just managing their rows. Five batches on main
(`e077aab` → `9f9b4cb` → `6418a76` → `b262fd0` → this docs pass), each
Rain-reviewed clean. Author contract: [`docs/PLUGINS.md`](docs/PLUGINS.md);
working example + integration fixture: `examples/hello-plugin/`.

- **Investigation first** (session docs hold the full audit): the
  scaffolded design was partly DEAD wiring, not just unbuilt — per-plugin
  capability JSONs were generated into `<data_dir>/capabilities/`, which
  Tauri's build-time capability glob never reads; `withGlobalTauri` was
  off so no frame ever had `window.__TAURI__`; the heartbeat state
  machine had zero non-test callers and lost even its registrations
  across restarts. The user ratified replacing the Tauri-ACL model with
  a host-proxy architecture (ask_user_choice, 2026-07-04).
- **Serving (`e077aab`):** one `bhq-plugin://` scheme registered at
  Builder time; `plugins::serve` resolves `<id>/<path>` (id-in-host on
  macOS/Linux, id-in-path under the Windows fold) for installed+ENABLED
  plugins only — enabled state lives in a new sync `PluginRegistry`
  cache seeded from storage at boot (which also re-registers enabled
  plugins with the heartbeat). Canonicalize+prefix traversal guard incl.
  symlink escapes; strict charset; MIME map; default plugin CSP.
- **Enforcement + data (`9f9b4cb`):** `plugins::catalog` is the
  versioned grantable-command contract (`api_version: 1`; 12 read-first
  commands; consent-copy descriptions). `plugin_invoke_proxy`
  (`tauri_cmd/plugin_api.rs`) is the single Rust dispatch point —
  re-checks enabled ∧ granted ∧ catalog per call; JSON-string args and
  returns; args/KV size caps. New `plugin_kv` table (migration 0029) —
  per-plugin KV namespaced server-side, CASCADE-wiped on uninstall.
  Manifests gained `api_version` (parse rejects ≠1); install rejects
  unknown capability names (loader stays tolerant). `cl_read_file` /
  `compute_apply_diff` extracted into shared `_inner`s.
- **Frontend runtime (`6418a76`):** `pluginBridge.ts` (platform-aware
  entry URLs via a `convertFileSrc` probe, per-mount nonce, pure message
  triage — 13 vitest tests), `PluginHost.tsx` (sandboxed iframe, 5s ping
  loop, `plugin:crashed` → Reload fallback card, clean-unmount pong),
  `plugin_note_ping`/`plugin_note_pong` feeding the existing heartbeat —
  crash detection is live end-to-end. `PluginPanel.tsx` behind
  `/plugins/view/:pluginId` + dynamic Shell topbar tabs for enabled
  panel plugins.
- **Consent + retirement (`b262fd0`):** install is two-step —
  `preview_plugin_manifest` fetches + validates WITHOUT installing and
  the PluginManager consent dialog lists each requested capability with
  its catalog description before an explicit confirm. `CapabilityGen` +
  the capability-JSON write path deleted (dead since inception);
  `PluginRegistry.capabilities_dir` dropped; plugins module doc
  rewritten to match the shipped runtime.
- **Example + docs (this commit):** `examples/hello-plugin/` (manifest +
  entry + copy-in `bhq-sdk.js`; lists sessions, reads the CL index,
  persists a KV counter) doubles as the fixture for a full-flow
  integration test (real install → gate → dispatch arms → serve
  resolution). `docs/PLUGINS.md` documents the author contract incl.
  per-platform origins, the RPC shapes, the catalog table, the security
  model, and the deferred tiers (agent surface, new agents,
  child-webview Browser tab, background execution, zip installs, inline
  slots — with the external MCP driver server named as the interim lever
  for backend-style plugins).

---

## 2026-07-03 — CL follow-ups: retrieve-first prompts, telemetry postmortem, polish

Post-restart follow-ups to the subtab restructure (below).

- **"No bcc-ad-manager telemetry" diagnosed — not a bug.** Read-only DB
  dig: migration 0028 (`retrieval_events`) was applied 2026-07-02
  05:24 UTC, and since that instant the only session activity in the app
  has been bot-hq (messages-joined-to-sessions, which also catches
  reopened sessions) — so bcc sessions have had zero opportunity to log.
  bcc's CL is healthy (568 atoms, largest in the store); events flow the
  moment a bcc session retrieves.
- **The real signal: agents under-use `cl_retrieve`** (7 events against
  1,701 messages in the telemetry window; agents index-search then
  whole-file `Read`). Root cause was prompt steering: the GENERAL_RULES
  workflow paragraph said "Open `conventions.md`, `decisions.md`…" and
  the session CL orientation never mentioned `cl_retrieve`.
  **Fix (prompt-only):** both sites now frame
  `cl_retrieve(project, query)` as the first move for CL content with
  whole-file `Read` as the explicit fallback ("Index-first,
  retrieve-second"); guard tests pin the wording in both prompts.
  Effect check later: Measurement-tab retrievals should rise, and
  `retrieval_events` should grow non-bot-hq rows once other projects run.
- **Polish:** Library Tree toolbar icons left-aligned (the right-float
  lost its purpose with the header label gone); ARCHITECTURE's sessions
  schema corrected (no `project`/`phase` columns — project derives from
  `working_repo_path` basename at spawn; phase is in-memory) + the
  per-agent spawn-metadata columns documented.

+1 lib test (retrieve-first guard) → 594.

## 2026-07-03 — Context Library subtabs: Library Tree | Context Manager

The CL page now splits into two Settings-style subtabs, fixing
review-queue discoverability (it was buried behind "pick project in
dropdown → click icon"). The pill row
IS the page header; no panel repeats its label as a heading.

- **Library Tree** — the file explorer + editor, simplified: the
  "Library Tree" sidebar header, the project-filter dropdown (YAGNI),
  and the queue/measurement toolbar icons are gone. Rescan is now
  always all-projects (the parallel branch that already existed);
  the per-project form moved to the Context Manager header.
  `OpenTab` shrinks back to `file | folder`.
- **Context Manager** — a per-project management surface (NOT a file
  explorer): left rail lists registered projects (`_globals` pinned
  last) with open-queue count badges; the right panel shows the
  selected project's header strip (repo path, per-project Rescan,
  Maintain CL preselecting the project) over queue | Measurement
  inner pills. Default selection = first project with open entries.
  The Context Manager subtab pill carries the cross-project open total,
  visible the moment the page opens.
- **Badge freshness (backend).** A queue-counts Tauri command
  (one `GROUP BY` over open entries) + a new bridge queue-changed
  event emitted from the file/approve/reject paths → a Tauri event →
  Providers invalidation. Needed because filing + rejection are DB-only
  writes the CL fs-watcher can't see (approval rewrites a file, so it
  incidentally fired `cl:changed`; the overlap is a harmless refetch).
- **P3 consolidation (partial).** The queue component + `MeasurementView`
  extracted out of `ContextLibraryEditor.tsx` into their own files
  (with their tests); `SubTabButton` extracted from `Settings.tsx` into
  a shared component (+ optional `badge` prop).

+1 storage test (counts aggregate), +1 bridge test (filing emits the
event), +4 ContextManager Vitest; 5 editor tests migrated to the
extracted components' files. Rust 591→593 lib; Vitest 114→118.

## 2026-07-02 — CL v2 audit + P1/P2 remediation

An audit swept the whole CL v2 arc (`f1bd3a7..HEAD`, ~20 commits) against
the `ideas.md` brief — holes, redundancies, staleness, incomplete
implementations. The cheap fixes (P1) + doc refreshes (P2) landed here;
the design remainder (P3) is now tracked in PLAN.md ("Context Library
v2").

- **Queue approval race (fix).** Approval wrote the CL
  file BEFORE the open→approved CAS, so a lost approve/reject race
  mutated the file and then reported "no-op". Now: validate kind → CAS
  claim → write → best-effort reopen revert on write
  failure (scoped to `approved` rows so it can't resurrect a rejection).
- **Poison-grader signal (fix).** `verified_source` used substring
  matching (`compute_total_v2` counted as seeing `compute_total`); now
  whole-word via `_uses`. The tool-name regex casing is documented as
  deliberate (lowercase `read` would match ordinary prose). 9→11 grader
  tests.
- **Frontend hygiene.** Measurement tabs no longer mis-report the queue
  kind in the tooltip / close aria-label (TabStrip fallback now uses
  `t.kind`); a local view-type duplicate is gone (imports the generated
  binding).
- **Prompt/dispatch hygiene.** The GENERAL_RULES "Tools:" list now
  enumerates `cl_retrieve` + the then-live queue tools
  (prose-only since 2026-06-29); an empty `cl_retrieve` result carries a
  "does NOT mean no constraints" advisory (brief failure-mode #5); two
  stale comments fixed (`body_hash` purpose, `cl_register_read`
  "fire-and-forget").
- **Docs.** ARCHITECTURE.md caught up to CL v2 (tool list 32→35,
  `cl_atoms` / queue / `retrieval_events` schema, atom+retrieval
  CL section, the then-current queue flow, queue/Measurement tabs); PLAN.md
  now tracks the arc + its deferred remainder.

+2 Rust lib tests → 591; +2 grader tests → 11.

## 2026-07-02 — CL measurement (Stage 4b): CL-poison behavioral eval

The active complement to the passive telemetry: does the duo OBEY a CL atom
that contradicts the code, or VERIFY against the source (brief failure-mode
#2)? A standalone `bench/cl_poison/` harness (not part of the cargo/npm
build).

- **Reuses the swebench external-MCP plumbing** (stdlib only): imports
  `../swebench/bothq_client.py` (session driving) + `completion.py`
  (completion detection + headless gate auto-resolve) via `sys.path`.
- **`scenario.py`** — seeds a fixture repo (`calc.compute_total`, the truth)
  + a poison CL project asserting the helper is `calculate_sum` (the lie); a
  task that names neither token so the agent must choose. The app's fs watcher
  auto-indexes the seeded CL.
- **`grade.py`** — PURE obey/verify/inconclusive verdict from the produced
  diff (whole-word token match) + a `verified_source` signal from the
  transcript. Unit-tested by `tests.py` (9 tests, `python -m unittest`, $0).
- **`run_poison_eval.py`** — driver: setup + preflight + N trials
  (`--dry-run` is $0; live trials spend model calls) → `runs/poison.jsonl` +
  a verdict tally. README documents the one manual step (confirm the poison
  is indexed) + caveats.

Authored, not run (live trials cost model calls — the user runs when ready).
Grader: 9/9 unit tests green.

## 2026-07-02 — CL measurement (Stage 4b): retrieval stats surfacing

Makes the B1 telemetry readable — a project-scoped Measurement view in the
Context Library so the "is the CL helping" numbers are visible, not just
logged.

- **`cl_retrieval_stats` command** (`src/tauri_cmd/cl.rs`) — thin wrapper over
  `Storage::retrieval_stats(project?, since?)` returning a
  `RetrievalStatsView` DTO; registered in `tauri_specta_gen.rs`, bindings
  regenerated (`export-bindings`).
- **Measurement tab** — new `OpenTab` variant `measurement`
  (`contextLibraryShared.tsx`) + a tertiary sidebar action, mirroring the
  earlier queue-tab wiring. `MeasurementView` (`ContextLibraryEditor.tsx`)
  renders stat tiles: tokens/session (the tokens-per-task headline),
  tokens/retrieval, retrievals, sessions, atoms, total tokens, and stale-hit
  + retrieval-miss rates (warn-colored when > 0), with a loading/empty/error
  state.

+2 frontend Vitest (114). Escape-hatch ratio (whole-file CL Read vs retrieve)
deferred to a follow-up per plan.

## 2026-07-02 — CL measurement (Stage 4b): retrieval_events log

First slice of the deferred CL measurement layer — the assessment made
measurement "the gate" for the retrieval engine, but the engine shipped
ahead of it, so there was zero telemetry on whether `cl_retrieve` helps.
This lands the append-only spine so tokens-per-task, stale-hit rate, and
retrieval-miss rate become answerable with data.

- **`retrieval_events` table (migration 0028).** Append-only, FK-free
  (immutable telemetry: a `_globals` retrieval must log, an insert must
  never fail on an absent session row, and pruning a session must not
  rewrite history via cascade). Columns: session_id/agent (nullable audit),
  project_id, query, atom_count, tokens_returned, budget_tokens,
  stale_count, returned_atoms (JSON), used_atoms (reserved, unused in v1),
  created_at; indexed by (project_id, created_at) and (session_id, created_at).
- **`Storage::log_retrieval_event` + `retrieval_stats`** (`src/storage/
  retrieval_events.rs`). Stats aggregate in one query via the
  `(? IS NULL OR col = ?)` filter idiom; ratios (avg tokens/event, avg
  tokens/session = the tokens-per-task proxy, stale-hit rate, empty-return
  rate) are derived in Rust to dodge SQL float/NULL edges. `RetrievalStats`
  in `row_types.rs`.
- **Hook = jsonrpc dispatch, best-effort.** The `cl_retrieve` arm
  (`src/signaling/jsonrpc.rs`) logs via a new
  `SignalingBridge::log_retrieval_event` (`bridge/cl_facade.rs`) that
  derives counts/tokens (reusing `estimate_tokens`) + a returned-atoms JSON
  from the atoms already in hand. Only the dispatch layer has both
  `caller.session_id`/`caller.agent` and the atoms. A logging failure warns
  and is swallowed — measurement never breaks a retrieval.

Surfacing (a `cl_retrieval_stats` command + a Library measurement card) and
the CL-poison behavioral eval are the next two slices. +4 lib tests → 589.

## 2026-06-29 — CL v2 deferred remainder: close-out re-wire, atom bounding, stale-flagging

Landed the three deferred items the audit entry below left open
(P1.1/P2.3/P1.2 from `plans/2026-06-29-cl-audit-remaining-handoff.md`).

- **Close-out re-wired to the review queue (P1.1).** Agents stopped
  `Write`-ing learnings straight into `notes.md` at session close — they
  filed the delta into the then-live review queue for user approval (the
  brief's keystone at the time). Re-pointed the three prompt sites
  (general_rules close section, prompts.rs close-ask, jsonrpc close nudge)
  and added the `cl_retrieve` advisory contract. Filing marked the
  close-delta gate, so an agent wasn't re-nudged into a duplicate. Prompt
  + one Rust line. (Queue removed 2026-07-21; close-out now writes
  directly via `cl_write_file`.)
- **Bullet-level atomization + token bound (P2.3).** `split_into_atoms`
  sub-splits a section over ~200 tokens into bounded atoms at column-0
  bullets + blank-line paragraphs (fence-aware); sections within the bound
  keep their text verbatim. A growing `## Learnings` block was previously
  one ever-growing atom that crowded the retrieval budget. `estimate_tokens`
  is now `pub(crate)` (shared with the budget), and `cl_retrieve` gains a
  `rowid` final tie-break so same-heading sub-atoms trim deterministically.
- **Retrieval-time stale-flagging (P1.2, migration 0027).** `cl_atoms`
  gains a `code_hash` column. A new `cl_refs` module extracts repo-relative
  source refs from an atom body (disk-validated, `:line` stripped) and
  hashes their content; `cl_rescan` stamps each atom's `code_hash` against
  the project's `working_repo_path`, and the bridge `cl_retrieve` wrapper
  recomputes it at read time, prefixing a ⚠ when the cited code has drifted.
  Storage stays pure (hash passed in / returned; repo I/O lives in the
  bridge). Whole-file granularity; `body_hash` stays as-is (it hashes the
  atom's own text — a different question). Measurement (Stage-4b) and
  embeddings remain deferred.

---

## 2026-06-29 — Context Library audit + retrieval/index hardening

Audited the shipped CL against the "Context Library v2" brief
(`ideas.md`) and the in-repo assessment
(`docs/plans/2026-06-27-context-library-v2-assessment.md`). The
implementation is a deliberate FTS5-first slice of that plan; the audit
flagged a few cheap correctness/structural wins, landed here. The larger
items (close-out keystone re-wire to the review queue, retrieval-time
stale-flagging, bullet-level atomization) are deferred.

- **`kind` on atoms (migration 0026).** `cl_atoms` gains an UNINDEXED
  `kind` column (convention|decision|policy|issue|idea|handoff|gotcha|
  note), derived per-file from the path (`cl_kind_for_path`). FTS5 can't
  `ALTER`, so the disposable vtable is dropped + recreated; the per-project
  boot rescan + zero-atom backfill repopulate it (the migration-0024 path).
  Backbone for future kind-specific freshness / concept-map / pin-by-kind.
- **Retrieval ordering.** `cl_retrieve` now pins by `kind IN
  ('convention','decision')` (was hardcoded filenames) and adds a
  `file_path, heading_path` final tie-break, so identical queries return a
  deterministic order — full BM25/pin/mtime ties were SQLite-unspecified,
  so a token-budget trim could drop different atoms run-to-run.
- **Rescan change detection.** The "did this file change?" check parses the
  stored index timestamp and the disk mtime as RFC3339 instants instead of
  comparing strings (`now_utc()` writes Z/millis, disk mtimes are
  +00:00/nanos — a lexicographic compare mis-orders the two formats).

Requires a rebuild + restart to apply migration 0026; `cl_atoms`
repopulates on the next boot scan.

## 2026-06-29 — Context Library atom backfill fix

Smoke after the CL Phase 3 rebuild revealed that `cl_retrieve` returned no
results on existing installs: `cl_index` had rows, but `cl_atoms` was empty.
The migration creates the FTS5 table, but unchanged files that were already in
`cl_index` before migration 0024 hit `cl_rescan`'s no-op branch and were never
atomized.

- **Backfill atomless indexed files.** `cl_rescan` now checks existing unchanged
  files for zero `cl_atoms` rows; if missing, it splits the file into atoms,
  inserts them with `replace_atoms_for_file`, and reports the file as touched.
- **Regression coverage.** Added a test for an already-indexed unchanged file
  with zero atoms, then verifies `cl_rescan` backfills atoms and `cl_retrieve`
  can return the content.

+1 Rust lib test → 574 lib.

## 2026-06-29 — Context Library review-queue UI

Made the queue backend human-operable from the Context Library: Tauri
commands plus an editor tab rendering open entries as compact Industrial
Terminal review cards with approve/reject controls (`correct` warned it
replaces the whole file; `delete` approval deferred). The entire
review-queue subsystem was removed on 2026-07-21 (see that entry).

+3 frontend Vitest cases → 112 frontend tests.

## 2026-06-29 — Context Library Phase 3 (review-queue MVP)

Added a durable per-project review queue (migration 0025) so agents
could suggest CL edits without mutating files directly: agent-facing MCP
filing/listing tools plus a host-mediated approval path that wrote
atomically and rescanned. The entire review-queue subsystem was removed
on 2026-07-21 (see that entry); migration 0025 remains as applied
history.

+11 Rust tests (2 storage, 8 bridge, 1 JSON-RPC dispatch) → 573 lib.

## 2026-06-28 — Context Library Phase 3 (FTS5 queryable retrieval)

The CL becomes queryable: agents pull the relevant CL *content* on a topic
instead of reading whole files. On branch `brian/cl-phase3-fts5` (retrieval
increment; review-queue + measurement deferred to a follow-up).

- **FTS5 atom index (migration 0024).** Each CL file splits into
  heading-delimited atoms (`split_into_atoms`); `cl_rescan` populates a
  standalone FTS5 `cl_atoms` table (BM25-rankable, porter stemming) on the
  same disk walk that feeds `cl_index`. SHA-256 `body_hash` stored for
  future stale-flagging.
- **`cl_retrieve(project, query, paths?, budget_tokens?)`** — the headline
  read side: FTS5/BM25 ranking + project scope + optional path filter +
  conventions/decisions pin + freshness, returning atom bodies inline under
  a token budget. Raw queries are sanitized into a safe MATCH expression
  (operators/metacharacters quoted as literals) without losing stemming.
  Exposed as an MCP tool (read-only, both agents).
- **Cold-start primer** now advertises `cl_retrieve` for CL content instead
  of "Read whole files", so the tool actually gets used.

+16 Rust tests (8 index/population, 7 retrieval, 1 stemming guard) → 562 lib.

## 2026-06-27 — Context Library Phase 1 (index freshness + primer pins)

First slice of the CL token-efficiency arc (assessment:
`docs/plans/2026-06-27-context-library-v2-assessment.md`). Two surgical
fixes so the cold-start CL surface stops drifting and stops burying the
highest-value files. On `main`; both fixes carry a unit test (+2 → 546 lib).

- **Index descriptions no longer freeze (Fix B, `5766291`).** The
  `cl_rescan` changed-file branch now re-derives the description from the
  fresh on-disk snippet via `refresh_cl_index_description`
  (`storage/cl_index.rs`) — and preserves user-set tags — instead of only
  bumping the timestamp. Before, a row's description was stuck at
  first-index even as the file's content changed.
- **Cold-start primer pins the stable files (Fix C, `acf096d`).**
  `render_cl_primer` (`core/session.rs`) now pins `conventions.md` /
  `decisions.md` to the front and excludes `plans/*` handoffs, then fills
  the remaining slots by recency — instead of a pure top-N-by-recency list
  that let ephemeral handoffs crowd conventions/decisions out of the TOC.

## 2026-06-27 — codebase audit round 4 (optimizations + enhancements)

A read-only audit through a NEW lens — performance + architecture (rounds
1–3 exhausted dead-code / dedup / staleness) — with the safe wins shipped.
An independent fan-out audit and EYES both converged: the codebase is lean
and the schema well-indexed; the wins are targeted, not sweeping. On branch
`brian/audit-round4` (6 commits).

- **Peer-forward path allocations** — the per-turn forward path no longer
  runs a `SELECT COUNT(*) FROM findings` + storage-lock for the banner (now
  a lock-free `Arc<AtomicUsize>` the findings mutators recompute on their
  cold path); the volley-convergence check caches the previous turn's token
  set instead of cloning the body + re-tokenizing both sides; tool-event
  rows serialize via borrow-structs (byte-identical JSON) not an
  intermediate `serde_json::Value`; the batch emitter swaps its dirty set
  back (`mem::take`) for zero steady-state flush allocs; and `session_id`
  rides the message-persist path as `Arc<str>`.
- **SQL** — added `idx_messages_session_author_time` (migration 0023) for
  the one uncovered hot predicate (`has_message_from_author_since`); the
  rest of the schema was confirmed well-indexed (no N+1; purpose-built
  partial indexes already cover the tray + findings reads).
- **Frontend renders** — the Context Library sidebar memoizes its O(files)
  tree build (was rebuilt on every search keystroke + sidebar drag) plus its
  nodes / editor pane (callbacks stabilized so the memos hold); the shared
  Markdown renderer is memoized so IPAV docs stop re-parsing on tray / TL;DR
  toggles.
- **Policy-mutation hash → SHA-256** — replaced the non-cryptographic,
  toolchain-unstable `DefaultHasher` with SHA-256, with a re-baseline guard
  so the 16→64-char format change doesn't log a spurious `PolicyMutation`
  for every policy file on upgrade.
- **Docs** — un-staled PLAN.md's "persistent IPAV phase log" (phase changes
  are already persisted as `kind='phase_change'` messages; only a queryable
  view is missing).

## 2026-06-26 — codebase audit rounds 2 + 3 (remediation)

Two read-only audit passes over the post-interrupt-redesign codebase, with
remediation shipped.

**Round 2** hardened + swept: the `force_push` gate is now enforced by the
pre-push hook (non-fast-forward detection via `git merge-base --is-ancestor`,
checked before the push-gate short-circuit; `ForcePushMode` default Allowed,
opt-in). Plugged a per-session map leak in `unregister_session` (`pending` +
`router_health`). Removed dead code (`QuestionStatus`, `DuoConfig.peer_author`,
`IpavState.phase_log`, `FindingStatus::Stale`, et al.) and stale comments.

**Round 3** deep-swept the under-covered `tauri_cmd` + frontend-screen slices and
deduped:
- **Phase vocab unified** — `session_doc_*` phase validation routes through
  `IpavPhase::parse` (now case-insensitive, canonical `tag()`); the divergent
  `VALID_PHASES` list is gone, so a lowercase `"apply"` can't be valid for one
  phase tool yet rejected by another.
- **Backend dedup** — `hook_runtime()` (the current-thread runtime built 5×),
  `emit_halt_row`, shared `JsonRpcError::app_handle_missing`/`webview_missing`,
  and `PROJECT_COLUMNS` / `AGENT_CONFIG_COLUMNS` / `cl_columns` SELECT consts.
- **Frontend shared hooks/atoms** — `useServerDraft` (6 panels), `useDragResize`
  (2), `useEscapeKey` (6 dialogs), `useListEditor`, a shared `SegToggle`,
  `ErrorBanner`, `FieldLabel`; dropped dead exports + the hand-rolled
  `ProbeResult` (→ generated `ValidateResult`).
- **Hygiene** — removed the dead `session_doc_read` / `cl_register_read` Tauri
  command wrappers (the MCP tools stay); fixed stale doc-comments (tool counts,
  CL paths, a removed-type rustdoc link).

## 2026-06-26 — peer-forward router extraction + turn-status + collapse-all UI

**Host-mediated reroute (the deferred deep fix), shipped.** Peer-forwarding
is no longer peer-to-peer: each pump (`core/duo.rs::pump_agent`) now emits a
`RouterCommand::Forward` to a single central task (`core/router.rs::run_router`)
that owns the forward decision (`9908ac7`). All of the old `flush_buffer`
logic — the awaiting guard, `peer_ack` suppression, the L2 hard-cap
(`VOLLEY_HARD_CAP=18`) and convergence breaker (Jaccard ≥ `0.85`, break after
`VOLLEY_SIMILAR_BREAK=2`) — moved into the router with full cross-agent
visibility; `flush_buffer` is gone and the pump shrank to persist + emit. The
router's liveness is tracked on `SessionHandle` (`Option<RouterControl>`; `Drop`
aborts the task) and surfaces as a per-session router dot via the
`router_alive` field on `SessionRuntime` (`a8cdb9e`, `d22151e`). See
ARCHITECTURE.md "Bilateral duo coordination" for the forward ladder.

**Per-agent turn status in chat (`05678c1`).** While the duo is busy the chat
header labels which agent is working (Brian vs Rain), driven by the per-agent
`brian_busy`/`rain_busy` flags on `SessionRuntime` rather than the collapsed
`activity` string.

**Collapse-all toggles (`fd33373`, `c0a1f17`).** A collapse-all control on the
Context Library tree and on the Apply-tab git diff.

## 2026-06-25 — peer_ack + halt duo-yield tools (behavioral layer on L2)

Two new internal MCP tools that let an agent signal volley-ending intent
explicitly, instead of the L2 breaker inferring convergence from text. They sit
strictly ON TOP of L2 (the hard-cap + convergence breaker stay the mechanical
floor) — weak models that never call them still hit L2.

**`peer_ack`** (either agent) — "acknowledge the peer without waking them."
Pump-observed: when the pump (`core/duo.rs::pump_agent`) sees the `peer_ack`
ToolUse during a turn, the turn's `flush_buffer` suppresses the peer-forward
(text is still persisted, so the user sees it) — the duo settles to Idle instead
of bouncing another volley turn. Suppression happens BEFORE the L2 counters, so an
explicit ack never bumps the hard-cap or the convergence streak. One-shot per turn
(reset after every TurnComplete, so an errored turn can't leak it). No bridge
state, no new DuoConfig field — `flush_buffer` just gained a by-value `peer_ack: bool`.

**`halt`** (HANDS-only) — "yield + unlock." Reuses `mark_awaiting_user`'s machinery
(set the awaiting flag + Halt tray row + AwaitingUser event). Because
`SessionActivity::derive` ranks `awaiting > busy`, the chat input unlocks
immediately even mid-turn — no busy-flag poking — and the existing `is_awaiting()`
guard in `flush_buffer` stops further peer-forwarding until the user's next message.
HANDS-only mirrors `mark_awaiting_user` (Brian owns user-facing yields; Rain
converges via `peer_ack`).

No `spawn.rs` change (the `mcp__bot-hq-signaling` server is granted as a unit, so
new MCP tools are auto-allowed for both agents); no `bindings.ts`/frontend change
(MCP tools, not Tauri commands; reuses the existing Idle/AwaitingUser states).
Internal MCP tools 30→32. Role prompts (`agents/prompts.rs`) document both verbs.
`src/signaling/{protocol,jsonrpc}.rs` + `src/core/duo.rs` + `src/agents/prompts.rs`
+ README/ARCHITECTURE tool lists. +6 lib tests.

---

## 2026-06-24 — Interrupt redesign + L2 volley breaker + activity-freshness fixes

Branch `brian/interrupt-redesign` (22 commits). Three related arcs: make Stop
a real interrupt, give the duo a mechanical volley floor, and fix two
activity-event freshness gaps.

**Interrupt redesign.** Stop is now a genuine control-plane interrupt, not a
process kill: `cancel_session_turn` issues a stdin `control_request` interrupt
(claude-code v2.1.186 wire format) with a ~2s SIGKILL escalation fallback. A new
`SessionActivity` state machine (`core/activity.rs`) drives the chat-input lock +
Stop button; explicit `Cancelling` state + a post-cancel reconciliation nudge; a
mid-flight atomic op (git commit/push/migration) defers the kill until it
completes so the worktree is never half-written; agent kills reap the whole
process group. All IPAV phases are turn-based (the I/P 1.5s interleave timer
retired), and the compensating machinery was peeled — heartbeat-ack suppression,
the idle-volley breaker, and the buffered-forward timer all removed. A
per-session stall watchdog + a queryable agent-health registry emit a `Stalled`
dot; the commit gate fails closed when a duo reviewer is down. The chat input is
always typeable — sending preempts the agents (warm interrupt, no SIGKILL).

**L2 volley breaker.** Re-introduced a mechanical floor against the idle volley
(the peel above removed the old ones on a bet that turn-based forwarding +
prompts would suffice; they didn't — proven live). Two layers in
`core/duo.rs::flush_buffer`, both unlocking the input on trip: a hard-cap (>18
consecutive peer-forwards with no user message) and a convergence detector
(>=85% token-set-Jaccard-similar consecutive forwards → break after 2).
Shape-based, not the old length/keyword heuristics that false-fired on real
collaboration.

**Activity-freshness fixes.** (B) `set_session_awaiting` now refreshes the
ActivityTracker via a registered `Weak` ref, so `AwaitingUser` emits at park time
instead of lagging to the agent's next turn-complete. (C) a new
`get_session_runtime` command + a run-once backfill in `Providers.tsx` seed the
event-driven activity/health stores on mount, so the footer/tiles are not grey
after a restart (events fire during respawn before the React listeners mount).

575 tests green (524 lib + 33 external MCP + 7 signaling + 11 storage) + 108
frontend Vitest; release build clean. Live-GUI smoke (the dot/footer visuals +
Stop-mid-commit) pending the merge-time human pass — not headless-testable.

---

## 2026-06-18 — Windows agent-spawn fix: prompt via file + full error chain (rc2)

rc1 launched + created sessions on Windows but agent spawn failed
(`spawning claude-code for agent rain; bin=claude`) and the session went
dead (`no live session`). Root cause: the full ~30KB assembled system
prompt was passed inline as `--append-system-prompt`, blowing past
Windows' 32,767-char command-line limit (`CreateProcessW`). Rain's prompt
is larger than Brian's, so only her spawn tripped it; the atomic
session-create then dropped the already-spawned Brian's handle (its `Drop`
kills the child) → no live session. Unix `ARG_MAX` (~1MB) hid it in dev.
Version → **1.0.0-rc2** pre-release.

- **prompt via file** (`spawn.rs` + `session.rs`). `SpawnConfig.system_prompt`
  → `system_prompt_path: PathBuf`; the assembled prompt is written to
  `{agent}-system-prompt.txt` in the per-agent temp dir (beside the
  mcp-config, same session-lifetime `TempDir`) and passed via
  `--append-system-prompt-file` (append, not the replace variant). Command
  line drops to a few hundred chars; cross-platform safe. Regression guard
  asserts the inline `--append-system-prompt` is never emitted.
- **full error chain** (`tauri_cmd/error.rs`). `AppError::from(anyhow::Error)`
  used `e.to_string()` (outermost context only), which hid the OS error
  behind the spawn-context string. Now `format!("{e:#}")` renders the whole
  chain so the next failure is self-diagnosing (+1 lib test).

Both fixes verified on macOS (528 tests green); end-to-end Windows
confirmation pending the rc2 build.

---

## 2026-06-17 — Windows runtime support: close usability gaps for friend testing (shipping)

Windows already *compiled + bundled* (CI 3-platform green since 2026-06-10), but had
never actually *run* there. Closed the runtime gaps so the user's Windows friends can
install + drive sessions. Branch `brian/windows-shipping`; all five local gates green;
CI `windows-latest` compiles **and** bundles the NSIS installer (validation run
27693697614, 3-platform success). Version → **1.0.0-rc1** pre-release (`v1.0.0` stays
reserved for the official market launch).

- **claude spawn pre-flight** (`spawn.rs::ensure_claude_runnable` + the two `docs.rs`
  headless callers). `Command::new("claude")` finds `claude.exe` (native installer) but
  NOT npm's `claude.cmd` (Rust appends `.exe`, ignores `PATHEXT`). New `#[cfg(windows)]`
  PATH probe: native `.exe` → OK; npm `.cmd`/`.bat` → actionable "install the native
  build (`irm https://claude.ai/install.ps1 | iex`)" error; not-found → install error.
  **No `cmd /c`** — the multi-KB `--append-system-prompt` is unsafe through cmd.exe
  regardless of the post-BatBadBut escaping. Unix = no-op (+1 lib test). Called at the
  top of `spawn_agent` (fails before the retry supervisor starts — no respawn-spam) and
  in `run_summarizer`/`probe_model` (the model **Test** button surfaces it at setup).
- **Single-instance lock** (`paths.rs::pid_alive`). The `cfg(not(unix))` stub returned
  `false`, so the lock was always stolen on Windows (no single-instance guarantee — two
  instances would share the DB + collide on the MCP port). `#[cfg(windows)]`:
  `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `GetExitCodeProcess` == `STILL_ACTIVE`
  (259) → `CloseHandle`, mirroring the existing `kill_child` windows-sys 0.59 usage.
- **git hooks** (`hooks.rs::render_hook_body`). On Windows the hook's binary path is now
  forward-slashed + double-quoted so Git-for-Windows' bundled MSYS2 `sh` execs it as a
  native path (a single-quoted backslash path defeats MSYS handling); unix output
  byte-identical; the `--data-dir` arg is untouched (passed literally to bot-hq.exe).
- **mcp-token ACL** (`paths.rs` init). No chmod on Windows — `#[cfg(windows)]`
  `icacls <token> /inheritance:r /grant:r <USERNAME>:F` restricts to the owner (mirrors
  the unix `0o600`), best-effort (`warn!` on failure, never aborts init).
- **`/dev/null` → `NUL`** (`docs.rs::untracked_diff`, the Apply-tab diff) — picks the
  platform null device; cleared the `TODO(win)`.
- **`docs/WINDOWS-TESTING.md`** — tester guide: native-installer prereq (the spawn
  sidestep), Git-for-Windows, WebView2; the unsigned-installer SmartScreen step; a smoke
  checklist mapping each gap to a tester action; feedback → GitHub issues.

Verification reality: this dev box is macOS and can't run/compile the Windows code
(`cargo check --target …-windows-msvc` dies in ring's C build). Local gates verify only
the non-Windows build + no regression (527 tests / 102 Vitest, release + frontend clean);
the `#[cfg(windows)]` paths are type-checked only on CI (windows-latest now green).
Runtime behavior (hooks via MSYS2, icacls, lock, spawn) is for the friends' smoke
checklist. Deferred: WebView2 install mode stays Tauri's default `downloadBootstrapper`;
macOS signing/notarize unchanged.

---

## 2026-06-17 — Forbidden-word check made case-insensitive (enforcement security review)

Security-focused adversarial pass over the policy/enforcement layer (forbidden-word check, Tool
Gate, git hooks). One real gap found and fixed; the rest of the surface verified clean.

- **`contains_word` is now case-insensitive** (`policy/mod.rs`). It matched case-sensitively, so
  casing variants of a forbidden word slipped through the disguise check — notably the AI
  co-author attribution trailer, whose lowercase form GitHub honors identically to the canonical
  casing. Lowercased both sides (match + word-boundary check on the same lowered string).
  Word-boundary semantics unchanged (embedded identifiers like `fooConfig` still don't trip).
  One change fixes all four enforcement paths (the `check_commit_message` MCP tool + the
  commit-msg / pre-commit / post-commit git hooks share this matcher). The case-sensitivity had
  been a documented deliberate choice (branded-names rationale); flipped with user sign-off since
  for a disguise check the safer bias is to catch every casing. New test uses a synthetic
  hyphenated stand-in so the test file doesn't trip the scan it exercises.
- Verified clean (no action): pre-push fail-closed + session-less-push block, Tool Gate
  (gate-wins-over-allow, case-insensitive, fail-open-on-corrupt), commit-msg comment stripping,
  post-commit backstop, hook install (linked worktrees + foreign-hook sidecar), session-policy
  snapshot fail-soft.
- Also this session: a `duo.rs` deep review (verdict sound — doc/test gaps closed, `a4d6f7d`) and
  the `s-5e7007af` deny-list refactor + leftover-branch cleanup (logged below).

526 tests green (475 lib + 33 ext + 7 sig + 11 storage), release build clean, frontend trio green.

---

## 2026-06-17 — EYES deny-list refactor + leftover-branch cleanup (review of `s-5e7007af`)

Reviewed session `s-5e7007af`'s duo-quality work (the four survey-followup commits) and
applied the cleanups it left behind. The work itself was sound; these are pure-win.

- **Const-driven EYES deny-list** (`refactor(spawn)`, `9e97979`). Rain's `--disallowedTools`
  value was a ~1KB hand-rolled string, and the `rain_denies_*` tests spot-checked only 9 of
  17 git-branch forms (plus a subset of gh) while the comment claimed "every mutating form is
  blocked". Extracted `*_WRITE_VERBS` consts + `deny_write_verbs()` /
  `build_rain_disallowed_tools()`; production code AND the tests now iterate the same consts,
  so enforcement and its test can't drift and a newly-added verb is covered automatically.
  Generated deny-set is unchanged (existing tests stayed green across the refactor). Closed the
  pre-existing gh test gap the same way.
- **Prose accuracy** (`docs(prompts)`, `c2bf274`). RAIN_ROLE listed the blocked git-branch
  forms as a closed set of seven; the code denies seventeen — added a trailing ellipsis so it
  reads as illustrative, matching the gh list beside it.
- **Removed leftover branch** `brian/duo-quality-followups` (local + remote). Fast-forward
  merged to main (`git log main..branch` empty → tip == main tip), pure dead weight.

524 tests green (473 lib + 33 ext + 7 sig + 11 storage), release build clean, frontend trio
green (invariant — backend-only). Deferred deliberately: consolidating the 9 near-synonymous
`HEARTBEAT_LEADS` (cosmetic, would risk changing match behavior for no real gain).

---

## 2026-06-17 — Duo quality from the cross-model survey (IPAV reframe, EYES git branch, ack list)

Acted on the June-17 cross-model survey — three bcc-ad-manager sessions, one per model combo
(DeepSeek-HANDS + GLM-EYES, Opus-HANDS + DeepSeek-EYES, GLM-HANDS + DeepSeek-EYES) — recorded in
the project CL `ideas.md`. Live-DB investigation found the surveys' self-reported "clean IPAV"
masked a real gradient: IPAV discipline tracks the HANDS model (Opus drove 3 phase advances, GLM 1,
DeepSeek-Brian 0 on a review task). Root cause was a prompt defect, not capability. Branch
`brian/duo-quality-followups`; all five gates green per commit; 473 lib tests (+2).

- **IPAV task-shape reframe** (`61b2c4f`, `general_rules.rs` + BRIAN_ROLE). The prompts coded
  "Apply = code mutation," so non-code tasks (review/deploy/investigation) read as "no Apply needed"
  and stalled in Investigate, stranding the deliverable in the investigate doc/chat. Reframed Apply
  as "produce the deliverable, whatever its shape" (code=diff, deploy=merge+smoke,
  investigation=findings); right-size phases, don't skip them. Kept the code-specific Apply verbs
  (add the generalization, don't replace). New test `ipav_apply_is_task_shape_agnostic`.
- **EYES read-only git branch** (`e375828`, `spawn.rs` + RAIN_ROLE). The blanket `Bash(git branch:*)`
  deny in Rain's `--disallowedTools` blocked read-only listing — DeepSeek-EYES hit 10+ false denials
  on legit `git branch --show-current`/`-a` reads (incl. compound `git branch … && echo …`) across
  the survey sessions; flagged in 5 consecutive surveys. Applied the gh deny-by-write-verb shape:
  enumerate mutating forms, let read forms fall through. New test
  `rain_denies_git_branch_write_allows_read`.
- **Heartbeat-ack list extension** (`22b3dab`, `duo.rs`). The duo ALREADY suppresses peer acks two
  ways (`is_heartbeat_ack` prefix filter + idle-volley circuit breaker, both since May, pre-survey).
  A planned redundant filter in `broadcast.rs` was dropped during Apply once the existing mechanism
  surfaced (a full-suite gate caught the conflict). Instead added the two survey-observed pure-ack
  leads the keyword list missed — `"on standby"` + `"ready when you are"` — safe under the existing
  `starts_with` match; ambiguous leads (ok/noted/confirmed) deliberately left to the circuit breaker.

Confirmed already-shipped (no action): EYES co-located `<phase>-eyes` doc (`dbbfdd7`), peer-message
provenance prefix (`6a3a30e`). Out of bot-hq scope / deferred (my YAGNI call): Laravel-Cloud console
bridge, known-flaky-suite signal, batch approvals, WebFetch-on-JS-docs, clip TTL, survey-diff UI,
an EYES write-suggestion tool.

---

## 2026-06-17 — Dashboard refinements (live Quickview, richer cards, roomier create dialog)

Dashboard UI/UX pass (user-requested). Two commits on `main`, all five gates green.

- **Quickview live preview** (`203cb50` backend, `e25266d` frontend). The Quickview
  footer was a dead stub showing generic phase text. Now it shows the first line of a
  session's latest `kind='text'` message with a color-coded author tag (Brian/Rain/You),
  falling back to the phase hint when a session has no messages. Backend adds a
  `SessionWithPreview` DTO + `list_active_sessions_with_preview()` — two correlated
  subqueries (latest text content capped at 200 chars + author) on the existing
  `idx_messages_session_id`, so no extra per-tile round-trips; `SessionInfo` gains
  `last_message`/`last_author`. The `Session` row type is untouched.
- **Quickview liveness** (`e25266d`). The dashboard refetches `list_sessions` on
  `agent:messages:batch`, throttled to 2.5s (leading + trailing edge) and scoped to the
  dashboard (the listener unmounts with it), so it stays monitorable live without
  re-running the preview query on every batch. `agent:messages:batch` stays out of the
  global invalidation map (`Providers.tsx`) — handled locally instead.
- **Richer card subtitle** (`e25266d`). `Working in <repo>` → `<repo> · worktree · created <rel>`.
- **Roomier create dialog** (`e25266d`). De-cramped vertically: capped height with internal
  scroll, more field spacing, 2-column model pickers (Disable-Rain moved above so the grid
  collapses when Rain is off). Projects dropdown shows names only (dropped the repo path).

---

## 2026-06-16 — Duo-survey improvements (EYES verifier, peer provenance, EYES phase doc, non-blocking ask_user_choice)

Acted on the converged findings from two independent BRAIN-duo retrospectives
(sessions `s-66ef6ad2` + `s-115dfd`) of bcc-ad-manager work, raised by the user via the
project CL `ideas.md`. Four batches on `brian/duo-survey-improvements`, all five gates
green per commit.

- **EYES → adversarial verifier** (`5ddd763`, `src/agents/prompts.rs`). The #2 converged
  limiter was producer/producer waste — EYES re-deriving HANDS's findings in parallel.
  Reframed RAIN_ROLE: verify what Brian PRODUCES, read his output first, and recast the
  "bottom-up investigation" section as a review *lens* (not a parallel re-derivation).
- **Peer-message provenance** (`6a3a30e`, `src/core/broadcast.rs`). The claude-code harness
  wraps peer forwards with the same "IMPORTANT: you MUST address the user's message" line
  as real user turns; bot-hq's only marker was `[Brian]`/`[Rain]`. Replaced with
  `[PEER MESSAGE — from Brian (HANDS), not the user]`. (The harness wrapper itself is
  outside bot-hq's control.)
- **EYES co-located phase doc** (`dbbfdd7`, `src/signaling/{jsonrpc,bridge/session_docs}.rs`).
  A phase-tagged `session_doc_write` from rain was hard-rejected. It now lands in a
  co-located `<phase>-eyes` doc (same phase tag → same IPAV tab), clobber-proof in both
  directions — chosen over appending into Brian's single doc, whose overwrite-on-upsert
  would wipe the EYES section on his next rewrite. New `bridge.session_doc_write_eyes`.
- **Non-blocking `ask_user_choice`** (`f55d2b5`, `src/signaling/*` + prompts). The #1
  converged limiter: the agent's MCP client timed out (~30s) waiting on a human, forcing a
  call → timeout → `list_my_pending_questions` → wait dance every decision. `ask_user_choice`
  + `supersede_question` now return a parked ack (`{status:"parked", choice_id}`)
  immediately; the pick arrives via the existing out-of-band stdin path (the same path that
  handled timeouts — now primary). `request_approval` / pre-push stay BLOCKING (a git hook
  awaits a synchronous bool). `ask_user_choice_inner` gained a `blocking: bool` param.
- Deferred (see PLAN.md): EYES compound-`&&` read Bash — a claude-code `--disallowedTools`
  denylist limitation, not a bot-hq gate.

---

## 2026-06-16 — Context Library registration UX redesign

Fixed the project-registration trap (user registered `model-connector` by putting the
repo path in the modal's required "Folder path" field → the whole repo got indexed as CL
content, and "unregister" left 47 ghost `cl_index` rows that kept showing under Projects
with no UI way to remove them). Root cause: the modal wired "Folder path" → `cl_path` (an
index-this-folder power feature) as the prominent required field, the only entry point to
add a repo; `unregister_project` was a soft no-op; `walk_cl_dir` skipped only dotfiles;
no hard-delete existed. Full redesign (Tier 1+2), all five gates green.

- **Name-first New-project modal** (`ContextLibraryRegisterModal.tsx`) — Name + optional
  Working repo + Description; `cl_path` demoted to a collapsed Advanced section. Default
  submit calls the new `cl_create_project` (managed dir at
  `library/projects/<name>/`, seeds `conventions.md`/`notes.md`, binds repo but does NOT
  index it); Advanced still does `cl_register_project` + `cl_rescan` for the index-a-folder
  case.
- **Real delete + rename** (`cl_delete_project`, `cl_rename_project`, `storage::projects`)
  — Delete purges the row (FK `ON DELETE CASCADE` clears `cl_index`/`cl_folders`/`cl_reads`)
  with an opt-in "delete files" for managed dirs only (never a custom `cl_path`/repo);
  rename repoints rows under `PRAGMA defer_foreign_keys` + renames the managed dir.
  FolderView gains Rename + Delete; "Unregister" → "Unbind working repo". `onProjectGone`
  closes/retargets the stale tab.
- **Build-dir ignore-list** (`walk_cl_dir`) — skips `node_modules`/`target`/`dist`/`build`/
  `vendor`/`coverage`/`__pycache__` (mirrors `fs_watcher`), so even an Advanced cl_path on
  a code repo no longer pulls dependency files.
- **Native folder picker** (`tauri-plugin-dialog` + `dialog:default`) — `pickFolder()`
  Browse buttons on every path field.
- **Ad-hoc New-session repo** (`Dashboard.tsx`) — pick a folder not registered as a
  project (project derived by basename, general policy tier); stale onboarding copy fixed.
- **Path B clarified** — right-click "Register as project" → "Promote to project" (it moves
  an existing Global folder; distinct from the modal's create — left intact, not folded in).
- New commands wired in `tauri_specta_gen.rs` + bindings regenerated. Events: create/delete/
  rename emit `project:changed` + `cl:changed` (DB-only mutations fire no fs-watcher event).
- ARCHITECTURE.md CL-tab section refreshed (was stale — predated the 06-12 categorized tree).



A comprehensive duo audit (5 read-only sweep agents + Rain's adversarial pass, every
finding re-verified at the call site) found the codebase healthy — clippy-clean, no
dead-code / TODO / marker debt — with the real debt in **doc staleness** from the fast
06-15 arc plus a small set of genuine fixes. Remediated as 13 commits, all five gates
green per commit; 514 tests (+1).

- **rename storage questions module to tray** (`08518b3`) — the last "questions" naming
  (table = `session_tray` since migration 0010; the bridge + tauri_cmd modules were
  renamed earlier). Fns + const renamed; `QuestionKind`/`QuestionStatus` enums + bridge
  methods + MCP tool-names kept.
- **surface dropped peer-forward** (`27dc0ea`) — `broadcast.rs::peer_forward_message`
  swallowed its send error with `let _` (the caller `warn!` at `duo.rs` was dead, could
  never fire); now warns in-function like its `broadcast_user_message` twin — closes the
  symmetric half of the #4 invisible-desync class.
- **log swallowed tray/rescan errors** (`32121ef`); **purge resolved tray rows at
  startup** (`5d8d9f2`) — `session_tray` grew unbounded; a 90-day boot GC
  (`COALESCE(answered_at, asked_at)` since withdraw/supersede leave `answered_at` NULL),
  +1 test.
- **CL filesystem ops off the async runtime** (`3f4b83f`) — the 6 CL file-op commands
  `spawn_blocking`-wrapped (path-traversal helpers byte-identical); **atomic CL writes**
  (`a040c08`) via an adjacent temp + rename (no EXDEV). **remove dead storage accessors**
  (`f60e530`, 4 of 5 — `author_typed` kept, a test uses it).
- **surface mutation errors** (`f0c0bb8`, `516ac57`) — 9 silent save/mutation sites
  (create-session, Claude-config, agent-config, tool-gate, model-delete, plugin
  toggle/uninstall) now show inline errors instead of failing silently. **FE hygiene**
  (`c63e8cc`) — dedup the terminal-input class, extract a shared message header, guard the
  event-subscribe path.
- **docs** (`8b6557d`, `6a5c0c5`, `9e196be`) — logged the 06-15 arc; corrected test
  counts (514/94) + internal tool count (26→25 across ARCHITECTURE/README); documented the
  fs-watcher; dropped the violations-viewer + worktree-kept-indicator backlog items (both
  shipped).

Deferred by decision (user scope pick): Tier-4 net-new test coverage (ViolationsPanel,
cl-rescan↔index reconcile, tool_args, core/state.rs) — a separate session. Full audit
findings in the session's investigate doc.

## 2026-06-15 — Live UI freshness, semantic tokens, violations viewer, provenance

A frontend-freshness + market-prep arc: the UI now refreshes live from a filesystem
watcher and event-emitting commands (the last `refetchInterval` polls dropped), the
Apply-tab diff recomputes on working-tree changes, semantic status tokens replace raw
color literals, and the deferred Violations viewer + session-project provenance shipped.
All five gates green per commit.

**Live freshness (event-driven, no polls).**
- **Filesystem watcher** (`1fde789`) — a `notify-debouncer-mini` (500ms) over the CL dir
  + per-session repos (`src/tauri_events/fs_watcher.rs`); re-indexes the affected scope
  THEN emits `cl:changed` / working-tree events (the index is the search source, so emit
  without rescan would serve stale rows). Working-repo churn is filtered by an ignore-list
  (`target`/`node_modules`/`.git`/dotdirs) so `cargo build`/`npm ci` don't thrash the A-tab.
- **Live Apply-tab diff** (`a4ea46b`, `91de8a2`) — recomputes on working-tree changes;
  untracked files now appear (side-effect-free `git diff --no-index`, never `git add -N`).
- **Live CL editor** (`23d15f4`) — an open CL file re-reads on external change, guarded so
  a CLEAN editor refreshes but an in-progress (dirty) edit isn't clobbered.
- **Event-driven lists** (`587a474`, `2c7dc77`) — project/session/model lists refresh via
  `app.emit` from their mutating commands (`project:changed` / `model:changed`; adding
  `app: AppHandle` to a command doesn't change generated bindings). The last 60s
  `refetchInterval` polls dropped — only the PluginManager 10s heartbeat + the
  broadcast-`Lagged` `session:resync` backstop remain by design.

**Semantic tokens** (`2ca657a`, `aad3a44`) — added `success` / `warning` color tokens and
migrated the raw `emerald` / `amber` / `red` literals to semantic tokens.

**Violations viewer + provenance (deferred items, now shipped).**
- **`read_violations` + `ViolationsPanel`** (`16a078d`, `db2d540`) — the Settings →
  Violations viewer (was a stub): tails `violations.jsonl`, filters by kind / outcome /
  session.
- **Session-project provenance** (`2df8cce`, `0b3dcc6`) — a `ProjectProvenance` enum +
  `get_session_project_info` command + a policy-origin badge in Session Settings (surfaces
  whether the session's project resolved by registered-repo match vs basename inference).

**UX polish** — idle footer state when no agents are tracked (`6251870`), neutral Archive
resume copy (`e711aa9`), root `.vite` cache ignored (`ec27cdb`); agent models moved from
the session header into Session Settings (`2c899d0`, 2026-06-14). Bindings regens are their
own `chore: regen bindings` commits (`7ae1dde`, `1aad5c7`).

## 2026-06-14 — Adherence + worktree optional follow-ons

The three explicitly-deferred optionals from the reliability arc, on branch
`brian/optional-polish` (off the merged arc). Each its own commit, all gates
green; 452 lib tests (+3).

- **Worktree-kept indicator** (`ebf6b27`). `close_session` keeps (never
  force-removes) a dirty worktree; the Settings → Archive list now shows a
  "⚠ Worktree kept" badge + path for a closed worktree-session whose worktree
  dir still exists on disk. New `session_worktree_kept` command — no migration,
  the kept dir is deterministic via `worktree::session_worktree_path`.
- **A3a — Edit-before-Apply nudge** (`9514724`). The duo pump self-nudges Brian
  once per session when he uses Edit/Write/NotebookEdit during Investigate/Plan,
  pointing him at Apply. `DuoConfig` gained `self_input_tx` (the agent's own
  stdin, distinct from the peer's); gated by `adherence_nudges`.
- **A3b — close-delta soft-gate** (`9f72beb`). The agent's first `close_session`
  with no `cl_rescan` this session is rejected with a write-then-prune reminder
  (two-call gate: append learnings + cl_rescan, then close on the retry); a
  per-session `CloseGateState` in the bridge tracks it. The UI force-close path
  is separate + ungated. Gated by `adherence_nudges`.

Bindings regen for the worktree command in its own chore commit (`f6103e1`).

## 2026-06-14 — Model-agnostic reliability + UX hardening (pre-market-ship)

An 8-slice arc on branch `brian/model-agnostic-reliability` to make the duo
workflow behave consistently regardless of which model drives Brian/Rain, and to
close the UX gaps that left agent/model failures invisible. Investigation found
reliability splits in two: transport + enforcement are already model-agnostic
(the normalizing LLM proxy, the forward-compatible stream-json parser, the retry
supervisor, the git-hook policy backstop), but workflow *quality* — CL-first,
IPAV, peer review — was 100% prompt-driven with no mechanical backstop, the gap
weaker non-Anthropic models fell into. Each slice is its own commit; all five
gates green per slice.

**Model-adherence nudges (Track A — the consistency fix).** Extends the `duo.rs`
silence-on-hold precedent (a prompt rule WITH a mechanical backstop) to the two
highest-leverage decision points, behind a new `adherence_nudges` opt-out app
setting (default on):
- **Session-start CL-opener nudge** (`87ac003`). One-shot stdin nudge on a first
  spawn (real project; not a `--resume` reopen) paging Brian + Rain at
  `cl_index_search` before the first task — the most-skipped rule in every audit.
  `core/session.rs` (`cl_opener_nudge`) + `storage/models.rs` (the setting).
- **Pre-Apply peer-ack nudge** (`37b6019`). On a duo session's Plan→Apply
  transition, a Brian-only reminder to confirm Rain reviewed the plan — the P→A
  handoff the prompts never enforced. `core/state.rs::advance_phase`
  (`should_peer_ack_nudge`).

**Reliability-visibility (Track B — make failures visible).**
- **Per-agent models in the session header** (`815dbda`, frontend — data was
  already in `SessionInfo`).
- **Agent-health dots** (`03f25bd`). The retry supervisor emits
  running/retrying/dead transitions via a new `AgentEvent::Health` → the duo pump
  (which also flags `Dead` on loop-end) → `SignalingEvent::AgentHealth` →
  `session:agent_health` → a Zustand store → green/amber-pulse/red dots on the
  dashboard tiles + session header.
- **Dynamic footer status** (`f09c53c`). The hardcoded green "Online" now reflects
  the worst-of-all-sessions health.
- **Force-close uncommitted-work warning** (`94d0808`). The close confirm runs
  `git status --porcelain` (`working_tree_dirty_count`) and warns the work will be
  kept, not committed.
- **Pre-flight model validation** (`4dd64b9`). A Models-settings "Test" button
  runs a one-shot `claude -p` ping through the model's real token + gateway (the
  same path live agents use), so a bad token / wrong model id / unreachable
  gateway fails at setup instead of as a silent mid-session API error. Extracted a
  shared `headless_claude_cmd` builder (DRY with the doc summarizer).

**Onboarding (Track C).** Create-dialog "no saved models" hint + a state-aware
welcome checklist — register project → add model → create session, with a ✓ on
steps already done (`81528e7`).

Tests: **449 lib (+6)** + 92 frontend Vitest (+5); release build clean. Bindings
regens are their own `chore: regen bindings` commits (`4e39058`, `0bc285e`).
Deferred (optional): secondary IPAV-hygiene/close-delta nudges; a
worktree-dirty-state close indicator; a `is_stale()` load-seed so health/footer
survive an app restart. Branch not yet merged.

## 2026-06-12 — Spawn cwd pinned: repo-less sessions no longer inherit app cwd

Sessions with no `working_repo_path` spawned claude-code with the app
process's inherited cwd — in dev that's the bot-hq repo itself, so the
agents adopted its `CLAUDE.md` + user-scope auto-memory as session
context (s-79f8aafe's duo quoted stale trio-era memory exactly this
way). `build_command` (`src/agents/spawn.rs`) now always pins the child
cwd: the session repo when set, else the bot-hq data dir — neutral (no
CLAUDE.md, no .git, empty auto-memory namespace) and guaranteed to exist
by `paths.rs` boot init. Two tests cover both branches.

## 2026-06-12 — Full sweep: 9 fix/cleanup commits from a duo audit

Brian + Rain swept CL + codebase + docs post-1.0.0 (4 parallel review
agents, findings adversarially re-verified — 3 agent claims dismissed as
false positives). Landed `7fef038..9916514`:

- **Register-Project "doesn't work" root-caused and fixed** (`9916514`,
  closes the issues.md 2026-06-11 item). DB forensics showed the 06-11
  registration SUCCEEDED — the new project was just invisible: tree roots
  required indexed entries matching the active filter/search. Roots are
  now indexed ∪ registered (`treeProjectIds`, 5 tests), and a successful
  register clears the search + pins the tree to the new project.
- **Session project now resolves via registered-repo lookup** (`9210194`).
  Was pure basename(working_repo_path); a registered project whose repo
  dir is named differently silently got general policy + no CL context.
  Canonicalized exactly-one-match lookup (base repo first for worktree
  sessions), basename fallback, 9 tests. Also explains the 06-11
  "full forbidden list" surprise: that session ran on an UNregistered
  repo (`~/Projects/test`) — designed inheritance, now at least
  inferrable from logs. Provenance badge in the gear tab = follow-up.
- **register-from-global migration correctness** (`054d29c`): folder
  descriptions re-home from a fresh `_globals` fetch (was: view-filtered
  state — active search silently skipped descendants); partial failures
  surface in the action error.
- **Rescan failures visible** (`2380006`): single-project rescan failure
  was entirely uncaught; all-projects failures hid in console.warn. Both
  now feed a "✗N failed" chip beside the report.
- **Dialog parity** (`e4a906e`): Escape for ActionModal / ModelDialog /
  RegisterProjectModal, focus trap for SessionPolicyPanel (+1 vitest).
- **Enforcement-path observability** (`f9641c4`): check_commit_message's
  policy-audit + violation-log failures now `warn!` instead of `let _ =`.
- Docs/comments (`7fef038`): config/-split note un-staled, tool count
  25→26, cl_write_file comment. Tokens (`ca5eaf8`): blue-400→tertiary,
  neutral-600→outline-variant, red-400→error (no success/warn tokens
  exist yet — follow-up). Hygiene (`1b5437f`): root `/node_modules/`
  ignored + stray vite cache removed.

Deferred by decision: module splits (May-21 precedent), provenance
badge, semantic success/warn tokens, CL-content edits (user-gated).

## 2026-06-12 — Context Library tree overhaul + Models list redesign

Four UI improvements from user spec (categories scheme picked by user:
Projects / Global / System).

- **add: CL sidebar header icon actions** (`8de5538`). Rescan / Register
  project / Maintain CL moved from full-width block buttons into icon-only
  header buttons (RefreshIcon w/ spin-while-rescanning, PlusIcon, WrenchIcon
  in primary). No count on rescan-all per user. Search + project filter +
  rescan report stay below.
- **add: resizable context library sidebar** (`5d8c9e2`). VS-Code-style
  drag-resize, ported from SessionView's split-handle pattern in absolute px:
  clamp [180, 480], default 240, persisted to
  `localStorage["bot-hq.cl.sidebarWidth"]`.
- **add: categorized CL tree (system guard, register-from-global)**
  (`1d2f546`). Tree now groups under three collapsible category headers
  (sentinel collapse keys `@cat:*` in the existing persisted set; left-click
  only toggles — never opens a tab): **Projects** (registered, `text-primary`),
  **Global** (loose `_globals` files, header right-click → New file/folder),
  **System** (`agents/**` + `custom-general-rules.md`, `text-amber-400`,
  read+update only — no context menu, and `cl_rename`/`cl_delete_path` now
  reject protected `_globals` paths server-side via
  `assert_not_protected_globals_path`, canonicalized-path compare). Top-level
  Global folders gain right-click **Register as project**: physically moves
  the folder under `projects/` (in-place registration would double-index),
  upserts the project row, re-points folder-description rows, rescans both
  sides. `splitGlobals`/`isInternalGlobalsPath` helpers + 4 Vitest cases +
  1 cargo guard test.
- **redesign: models settings as list with edit dialog** (`dc3e70c`).
  Settings → Models card grid replaced by a 5-column list (name / provider /
  model id / updated / actions). Create + edit go through a ModelDialog
  (RegisterProjectModal scaffold); the model id is generated at save time so
  cancelling Add leaves no ghost "New model" row (the old grid pre-created
  one). Delete keeps the ConfirmDialog flow.

---

## 2026-06-11 — v1.0.0 stabilization: worktrees, dispatch defaults, prompt drafts, UX polish (shipping)

The four-area stabilization pass for the first stable release.

- **fix: dispatched sessions honor the solo/duo default** (`c215392`). The
  Maintain-CL button (`dispatch_session`) and the external driver
  (`open_session`) never called `set_session_spawn_config`, so the DB default
  (`rain_enabled=1`) always spawned the duo — `rain_disabled_default` was
  ignored. Both now resolve `Storage::default_rain_enabled` before spawn;
  models stay NULL (= agent defaults). Modal copy de-hardcodes "Brian + Rain".
- **feat: per-session prompt drafts** (`d48a02b`). ChatInput gained a
  `draftKey` prop — drafts persist to `localStorage["bothq:draft:<sid>"]`
  through navigation/restart, clear on successful send, survive failed sends.
  SessionView keys the input per session. 6 new Vitest cases.
- **fix: blank repo paths store as NULL** (`41b13d7`). A session created with
  `''` read as repo-backed everywhere `working_repo_path` is consumed
  (action_gate hard-error before its approve prompt). `create_session`
  normalizes; migration 0019 repairs pre-guard rows.
- **add: per-session git worktrees** (`b5d1d7d`). Repo-backed sessions default
  to an isolated worktree at `<data_dir>/.local/worktrees/<sid>/<repo-basename>`
  on branch `bothq/<sid>` — parallel sessions per project. `working_repo_path`
  stores the worktree (all consumers unchanged); new `base_repo_path` column
  (migration 0020) remembers the source repo. Idempotent ensure at spawn with
  direct-mode fallback (row converted so row-readers agree); clean-only removal
  at close (never `--force`); `install_hooks` resolves the hooks dir via
  `git rev-parse --git-path hooks` — a linked worktree's `.git` is a FILE and
  hooks live in the shared common dir (previously worktree repos silently
  skipped hook install). Opt-out per session or via `worktree_default`
  (Settings → Agents → Session defaults). ARCHITECTURE.md "Session worktrees"
  section added.
- **ux: 1.0 polish** (`59d39b7`). Activity-ordered dashboard tiles (newest
  message, created_at fallback), ⌘/Ctrl-N → New-session dialog + ⌘/Ctrl-, →
  Settings, welcoming empty-state copy, inline session rename
  (`rename_session` command).
- **release: version 1.0.0** across Cargo.toml / tauri.conf.json /
  frontend/package.json. Violations-log viewer deliberately deferred to v1.1
  (user scope pick).

---

## 2026-06-10 — fix the release-build CWD litter found by the smoke (shipping)

Same-day fix for the smoke's incidental find (`957d6a9`). The startup
tauri-specta export is now guarded on `frontend/src/lib` existing in the
CWD — the export creates intermediate dirs, so unguarded it littered a
`frontend/` tree into any writable launch directory. Repo-root launches
keep the documented auto-regen (verified: mtime advances, content
byte-identical at HEAD, tree stays git-clean); foreign-CWD launches skip
with a debug log (verified: temp CWD stays empty). `specta_builder`
construction stays unguarded — it also feeds `invoke_handler` (first
attempt scoped it into the guard; only the compiler caught the second
use). All five gates green (465 Rust + 71 Vitest).

## 2026-06-10 — first-run + migration smoke: PASS (shipping); MIT license

Closed the shipping.md "first-run + migration smoke" item — the GUI-startup
wiring over a legacy data dir, which the `paths.rs` unit tests don't reach.
Also shipped the MIT `LICENSE` (root + `frontend/package.json` field,
`02bba46`) and corrected INSTALL.md's Gatekeeper steps for macOS Sequoia
(right-click → Open no longer bypasses for unsigned apps; `9a2d2a0`).

- **Method.** Release binary launched 3× over TEMP `BOT_HQ_DATA_DIR` dirs
  (never the live `~/.bot-hq/`), neutral CWD, stderr captured, SIGTERM after
  ~12s. Fixtures carried per-file sentinel content with md5 manifests.
- **v0 → v2 full migration: PASS.** 13-entry root-layout fixture (all three
  stages). One "migrated legacy layout" warn; outcome `Repaired`; all 12
  content entries relocated to `library/`/`.local/`/`config/` byte-identical;
  `cl-version.txt` removed; `version.txt` stamped "2".
- **Idempotency: PASS.** Relaunch over the migrated dir: zero migration
  lines, outcome `Existing`, marker + content untouched.
- **Crash-window self-heal: PASS.** Simulated interrupted migration
  (dest-exists + conflicting root copy + unmoved entry): existing dest not
  clobbered, conflicting root copy left as residue, unmoved entry migrated.
- **Second-instance safety confirmed.** Ran beside the live prod bot-hq:
  internal signaling binds an ephemeral port; the external MCP's fixed port
  collision warned + skipped exactly as coded — app fully functional.
- **Incidental find (follow-up candidate).** The startup tauri-specta
  bindings export writes `<cwd>/frontend/src/lib/bindings.ts` from ANY
  writable CWD in release builds (it creates intermediate dirs) — launching
  the app from a terminal in `~` litters `~/frontend/`. Polish: gate the
  export to dev builds or an existing path.
- Also live-verified the new unix shutdown handler (`d039ffa`) 3× — SIGTERM
  → child reap → clean exit 0.

## 2026-06-10 — Windows compile fix: cfg-gate reaper + shutdown handler, restore CI lane (shipping)

Un-deferred Windows from the release matrix by fixing the compile blocker
found in the 2026-06-09 CI dry-run.

- **Per-platform child kill** (`d039ffa`). `spawn.rs::reap_all_children` kept
  its `try_lock` + iterate shape; the kill itself moved into a per-platform
  `kill_child` — unix keeps the verbatim `libc::kill`/SIGKILL body, Windows
  does `OpenProcess(PROCESS_TERMINATE)` → `TerminateProcess` → `CloseHandle`
  via `windows-sys` 0.59 (version already in the lock transitively; new
  `[target.'cfg(windows)'.dependencies]` entry, `libc` moved to the unix
  twin). Windows has NO kill-children-on-parent-exit semantics (no process
  tree; would need Job Objects), so an empty stub would have reintroduced
  Ghost-Brian there — the reap walk is equally load-bearing on both
  platforms. Job-Object hardening (covers hard parent kills, parity with
  un-catchable SIGKILL) noted as a follow-up candidate, not blocking.
- **Windows shutdown task** (`d039ffa`). The unix signal task is unchanged
  under stmt-level `#[cfg(unix)]`; a `#[cfg(windows)]` twin selects over
  `tokio::signal::windows::{ctrl_c, ctrl_close, ctrl_shutdown}` (≈ SIGINT /
  SIGHUP / SIGTERM) with the same reap + exit tail. Panic hook untouched.
- **CI lane restored** (`bcca313`). `release.yml` windows-latest matrix entry
  back, exactly the previously-validated shape (`args: ''` — an explicit
  `--target` would move bundle output out from under the upload globs, which
  already cover `target/release/bundle/nsis/*.exe`).
- **Verification.** All five gates green post-change (465 Rust + 71 Vitest,
  tsc clean, release + frontend builds clean; gates 4–5 run by Rain pre-halt,
  tree unchanged since). Local `cargo check --target x86_64-pc-windows-msvc`
  dies environmentally in `ring`'s C build script (no Windows SDK headers on
  macOS) before reaching our crate — the definitive Windows check is the CI
  windows-latest lane (workflow_dispatch validation run on push).
- **Still staged for later** (shipping.md): Windows runtime gaps once it
  ships — PID lock, bash hooks, `mcp-token` ACL.

## 2026-06-10 — separate personal rules from shipped standard rules (shipping)

User classified the rule inventory: commit hygiene + the forbidden-in-commits
brand list are PERSONAL conventions; everything else is standard product.

- `src/agents/general_rules.rs` — `## Commit hygiene` removed from
  `GENERAL_RULES`; replaced with a neutral `## Commit conventions` pointer
  (style + forbidden words come from policy files / custom-general-rules.md).
  Fresh installs ship NO house commit style. `Policy::default()` already has
  an empty forbidden list, so an all-default policy renders no Enforcement
  block (`is_effectively_empty` short-circuit, verified).
- `src/core/session.rs` — prompt-assembly doc comment + 4 test anchors moved
  from the deleted section to "Working directory".
- `templates/cl/custom-general-rules.md` — example line no longer references
  "baked-in hygiene".
- User-side migration (not in repo): personal hygiene block appended to
  `~/.bot-hq/library/custom-general-rules.md`; redundant outward-actions copy
  trimmed (that rule is hardcoded in the binary). Leftover `agents/emma/` CL
  dir flagged for manual deletion — blocked by a dogfooding find: `action_gate`
  errors "session has no working_repo_path" on a session that has one.

## 2026-06-09 — macOS + Linux distribution: bundle config, release CI, v0.1.0 draft, Homebrew cask (shipping)

Set up end-to-end distribution for macOS + Linux (Windows deferred) and cut the
first release. The app had no `bundle` config, no real icons, and no CI; now a
tagged `v*` push builds + drafts a GitHub release with platform artifacts.

- **Bundle config + icons** (`69ff1d1`). Added the `bundle` section to
  `tauri.conf.json`: `targets [app,dmg,deb,appimage,nsis]` (per-OS subset
  auto-selected — mac app+dmg, linux deb+appimage, win nsis), `DeveloperTool`
  category, per-OS metadata, macOS min 10.15. Generated the full icon set
  (icns/ico/PNGs) from a temporary brand-matched `>_` terminal mark (orange
  chevron = HANDS, purple cursor = EYES) kept in `icons/src/` for regeneration.
  Fixed a latent bug: `beforeBuildCommand`/`beforeDevCommand` were `cd frontend
  && …`, but Tauri v2 runs them from the frontend dir (inferred from
  `frontendDist`), so the `cd` double-cd'd — `tauri build` never worked before
  (the app was always built via separate `npm run build` + `cargo build`). Now
  `npm run build` / `npm run dev`. Verified: `bot-hq.app` bundles with the icon +
  correct Info.plist (category developer-tools, id, version).
- **Release CI** (`525ba2b`). `.github/workflows/release.yml`: a `tauri build`
  matrix (macOS universal `.dmg`, ubuntu-22.04 AppImage + `.deb`, Windows NSIS)
  on a `v*` tag (draft-release upload) or manual dispatch (validation artifacts).
  Manual-build approach, not tauri-action (whose layout auto-detection fights the
  flat layout): invokes the frontend's tauri CLI from the repo root. Unsigned;
  APPLE_* signing env stubbed. A `workflow_dispatch` dry-run validated all three
  platforms — macOS + Linux green (the runner builds the `.dmg`; the local `.dmg`
  failure was just headless GUI, `bundle_dmg.sh` needing Finder), Windows caught a
  real blocker (below).
- **Windows deferred** (`d1e94e0`). bot-hq doesn't compile on Windows: the
  Ghost-Brian reaper (`spawn.rs` `libc::kill`/`SIGKILL`) and the shutdown signal
  handler (`main.rs` `tokio::signal::unix`) aren't `#[cfg(unix)]`-gated. Commented
  the windows-latest matrix entry (restore TODO) so `v*` tags yield clean mac +
  Linux releases instead of a failed run. Follow-up: cfg-gate both + add Windows
  equivalents (windows-sys TerminateProcess, tokio ctrl_c).
- **v0.1.0 draft release.** Tagged + pushed `v0.1.0`; CI produced a draft release
  with `bot-hq_0.1.0_universal.dmg` (15.8 MB), `bot-hq_0.1.0_amd64.AppImage`
  (86 MB), `bot-hq_0.1.0_amd64.deb` (9 MB). First real exercise of the
  draft-upload path — one clean release, no matrix race.
- **Homebrew cask + docs** (`4c00c23`, `58b9bd5`). `packaging/homebrew/bot-hq.rb`
  (real sha256, livecheck, claude-code + Gatekeeper caveats, deliberately no zap
  of `~/.bot-hq`), `INSTALL.md` (per-platform), `docs/SIGNING.md` (notarization
  upgrade path + Windows / auto-update follow-ons). Ships via an own tap
  (`gregoryerrl/homebrew-bot-hq`).

Remaining (manual/user): publish the draft `v0.1.0`; create the tap repo + add the
cask. Deferred: the Windows compile fix, the real app icon (swap `icons/src/` +
re-run `tauri icon`), and macOS notarization when an Apple cert exists.

---

## 2026-06-09 — check-for-updates: GitHub-release update banner (shipping)

Added the first user-facing "you can update" path (shipping/market-prep track).
On launch bot-hq polls the GitHub releases API, semver-compares the latest tag to
the running version, and shows a dismissible download banner when a newer build
exists — plus a Settings → Updates subtab (installed-vs-latest + manual "Check
now"). This is the **check-and-notify** scope (A): no code-signing / updater
plugin, so the install is manual; the command + banner shell graduate cleanly to
full auto-install later. Decided scope with the user up front — real auto-install
is blocked on code-signing (a separate roadmap item), check-and-notify is not.

- **`core::updates`** — the testable core, split from the network glue:
  `is_newer` (semver compare, strips leading `v`, false on garbage),
  `release_from_response` (**404 → no release, NOT an error** — the current
  zero-releases state), `build_update_info` (`None` → not-available). Thin async
  `fetch_latest_release` / `check_for_update` set the GitHub-required
  `User-Agent`. 13 unit tests, all network-free.
- **`check_for_update` command** (`tauri_cmd/updates.rs`) — returns `UpdateInfo`,
  compares against `app.package_info().version` (the shipped version, not a
  constant). Registered in `collect_commands!`; bindings regenerated.
- **`tauri-plugin-opener`** (+ `opener:default` capability) opens the release page
  in the system browser — `window.open` isn't reliable cross-platform per the
  Tauri v2 docs.
- **Frontend** — app-wide `UpdateBanner` (Shell) with per-version localStorage
  dismissal; Settings Updates subtab. Both share one `check_for_update` query.
- Fails quiet: offline / rate-limit / no-release never nags. The banner shows
  nothing live until a release > installed is cut (`gh release list` is empty
  today); Settings "Check now" proves the round-trip now. Live endpoint verified
  returning 404 for the zero-releases state, handled gracefully.
- All 5 gates green; commit `4c054f8`.

---

## 2026-06-09 — v1.1 config/ split: host machine config moved under config/

Followed the `library/` carve-out (`cf72e72`) with the deferred v1.1 step from
the shipping roadmap: the three host-side machine-config files —
`general-policy.yaml`, `tool-gate.json`, `claude-overrides.json` — moved from the
data-dir root into `<data_dir>/config/`. The root now holds only `version.txt`
plus the four subtrees (`library/`, `config/`, `plugins/`, `.local/`).

- **`config_dir` is part of `Paths`.** New `config_dir` field + a free
  `paths::config_dir_path(data_dir)` helper (mirrors `read_signaling_addr` — the
  policy / claude-config path builders receive a bare `data_dir`, not a `Paths`,
  e.g. the CLI hook subprocess). `policy::general_policy_path`,
  `tool_gate::config_path`, and `overrides::config_path` route through it; the
  policy audit reuses `general_policy_path` so the resolver and the
  mutation-audit can't desync on the new location.
- **Schema bumped 1 → 2 with a REQUIRED migration.** Not a pure rename: an
  existing v1 install carries these files at the root, so changing the paths
  without moving them would make the loaders read an empty `config/` and
  silently fall back to defaults — dropping the user's configured policy /
  tool-gate / overrides. `migrate_legacy_layout` is now gated on
  `schema_version() < SCHEMA_VERSION` (was `version.txt` absence) and stages
  cumulatively: v0→v1 (root CL → `library/`, host state → `.local/`) then v1→v2
  (root config → `config/`), each exists-guarded + idempotent; `init` stamps the
  marker afterward.
- **Docs re-pointed:** `paths.rs` header layout, ARCHITECTURE.md storage map +
  policy hierarchy (also fixed a stale `projects/` → `library/projects/` left
  over from the carve-out), README policy/tool-gate paths + de-hardcoded a stale
  "288 tests" line, and the in-code doc comments across `policy/hooks`,
  `agents/spawn`, `tauri_cmd/*`, and the signaling bridge.
- New test `migrates_v1_config_files_into_config_dir`; the existing migration
  suite still passes under the schema-version gate. The deferred README
  install-docs item rode along in the same batch (prerequisites were already
  present from `cf72e72`).

---

## 2026-06-09 — Context Library carved into its own `library/` folder (market-prep layout)

Reshaped the `~/.bot-hq/` data home so the Context Library lives in its own
`library/` subtree — separable for backup / a future cloud-sync-CL plugin, and
so host-only state stops intermixing with user content at the data-dir root.
Decided the full install topology first (the binary stays platform-bundled —
`/Applications/bot-hq.app` etc., NOT under `~/.bot-hq/`); shipped the v1
`library/` carve-out and deferred a `config/` split to a cheap v1.1. Target
platforms now macOS + Linux + Windows; the layout is base-agnostic so no
platform branches were needed.

- **`Paths` is the single source of truth.** New `cl_dir` (`<dd>/library`),
  `plugins_dir`, `version_path` (`version.txt`, renamed from `cl-version.txt`),
  and `.local/`-rooted `mcp_token_path` / `violations_path` /
  `policy_hashes_path` / `screenshots_dir`. A `project_dir(name)` helper is the
  one per-project convention path, shared by the storage resolver, policy
  resolver, and policy audit so the `library/` location can't desync them.
- **One-time migration** (`Paths::migrate_legacy_layout`, gated on `version.txt`
  absence): moves root CL → `library/`, host-only state → `.local/`, renames the
  marker. Idempotent; explicit-`cl_path` projects untouched. Uses a
  rename-with-copy-fallback (`move_path`) robust to cross-filesystem EXDEV /
  locked files.
- **Resolvers + policy re-pointed** through `cl_dir` / `project_dir`:
  `cl_path_for_project`, `cl_project_root`, `walk_cl_dir`, `cl_startup_init`,
  `read_system_prompt` (also fixed a pre-existing missing-slash bug in the CL
  anchor), `resolve_at_root`, `audit_policy_files`. Host-only files
  (violations.jsonl, .policy-hashes.json, screenshots/, mcp-token) moved under
  `.local/`.
- **Cleanup + docs:** removed the dead Emma template; refreshed ARCHITECTURE.md,
  README.md, .env.example. Added migration tests (×2) + a resolver/audit parity
  test. All 5 gates green; `config/` split + a `bin/` symlink are deferred
  follow-ups.

---

## 2026-06-09 — under-the-hood health sweep: dashboard halt bug, spawn invariant, enforcement tests, cleanup

A full-codebase audit (5 read-only sub-agents + Rain's adversarial sweep) on an
otherwise-healthy codebase surfaced one real silent bug, one latent invariant
gap, under-tested enforcement seams, and a staleness tail. Remediated as 8
self-contained commits, all 5 gates green per commit.

- **fix: dashboard tiles flag halt waits via the durable tray** (`ce4d49b`). Tiles
  counted pending input from the in-memory `list_pending_choices`, which
  `mark_awaiting_user` / `request_phase_advance` never populate — so a halted
  session showed no badge and counts reset on restart, while the header bell (on
  the durable source) disagreed. Point the tile at `list_pending_tray`;
  `SessionTile` is indicate-only so its prop is now a plain `pendingCount`.
- **fix: exclude max-effort and ultracode at the spawn merge** (`e155897`). A
  persistent `effort=max` + a session `ultracode=1` (or the reverse) emitted both,
  which claude-code treats as mutually exclusive. Reconcile at the overlay
  honoring session-wins; tests for both collision directions.
- **test: push-gate approve/reject classification** (`315e05a`). Extract a pure
  `classify_push_response` seam from `decide_push` and lock the fail-closed
  property (reject / missing / malformed / non-2xx never resolve to Approved).
- **test: external MCP soft-fail + same-length token reject** (`63724f9`). Auth was
  already integration-tested; filled the two genuine gaps — port-in-use soft-fail,
  and a same-length wrong token (exercises ct_eq's content path, not just length).
- **refactor: rename `tauri_cmd/questions.rs` → `tray.rs`** (`faec167`), decouple
  ChoicePrompt from the dropped `PendingChoiceView` (`37a0036`), regen bindings
  (`a88b58b`). Drops the orphaned `list_pending_choices` Tauri command; the bridge
  method stays for the external driver.
- **chore: prune stale Emma/PluginSlot refs, the dead author.emma tailwind token,
  and a broken rustdoc link** (`d128185`).

Tests now 448 (397 lib + 33 external MCP + 7 signaling + 11 storage) + 63 frontend
Vitest. Landed on branch `brian/health-sweep-2026-06-09`.

---

## 2026-06-08 — June 8 QoL batch: metadata toggle, resizable split, collapsible diff, doc TL;DR

Four independent frontend QoL features from `ideas.md` (June 8 list), built
easiest→hardest, one commit each. All five gates green per commit.

- **feat: CL metadata editor behind a toggle** (`5d24f4c`). The CL file editor's
  description/tags panel is collapsed by default behind an "Edit metadata" header
  button (amber dot when collapsed with unsaved metadata); it stays mounted
  (CSS-hidden) so an in-progress edit survives a toggle.
- **feat: resizable chat/document split** (`1853c81`). SessionView's fixed
  `grid-cols-[3fr_2fr]` became a flex layout with a drag handle; the ratio clamps
  to [25,75]% and persists to `localStorage["bothq:split:leftPct"]`.
- **feat: collapse the Apply-tab git diff per file** (`9cab075`). A new pure
  `lib/diffGroups.ts` (`groupDiffByFile`, unit-tested) splits the classified diff
  on `diff --git` headers; each file renders as a native `<details open>` with a
  `+adds −removes` summary. No backend change — reuses `compute_apply_diff`.
- **feat: TL;DR summarize button on session docs** (`aa54329`, bindings `827f254`).
  New `summarize_session_doc` command resolves a model (`default_model_id` → session
  Brian model → agent config via the now-`pub(crate)` `resolve_spawn_config`) and runs
  a one-shot headless `claude -p … --max-turns 1 --strict-mcp-config` (60s timeout,
  kill-on-drop), rendered in a dispose-on-close dialog. The live model path is
  static-verified only (compiles, wired, binding present) — not runtime-exercised.

---

## 2026-06-06 — web_search, prompt fixes, UI outline pass + health-audit sweep

Backfills the 2026-06-06 feature work (shipped but previously unlogged) plus a
CL + codebase health sweep.

- **feat(web_search): model-agnostic web search via a headless webview.** A new
  `web_search(query, engine?)` internal MCP tool navigates a hidden webview and
  reads the rendered DOM from Rust (`eval_with_callback`), cascading
  Google→Startpage→Bing with a title-filter + nav-junk drop. Lets agents on
  gateways without a server-side search tool fetch live results. Rain's `--bare`
  was dropped (`fa57a92`) so her client-side tool loader works again (the
  llm_proxy already handles the system-message hoist `--bare` was guarding).
- **fix(prompts): interpolate the project name + sharpen investigate guidance**
  (`1bf3faa`). The `<your project>` placeholder in the CL anchor + role prompts
  was never substituted; now resolved to the session's project (or `_globals`
  when repo-less). Added a "tight turns while coordinating" nudge.
- **feat(ui): outline icons + confirm dialog** (`2fa33f1`). Replaced text/glyph
  icons with a hand-rolled outline SVG set (`components/icons.tsx`); a reusable
  `ConfirmDialog` replaced all `window.confirm` sites; the session-view close
  button is now a force-close danger dialog. Dropped the redundant role chip.
- **fix: bell self-heal on out-of-band resolve** (`e43e5f3`) — the OOB resolve
  paths now emit `ChoiceResolved` so the notifier badge clears.

Health-audit sweep (CL + codebase):
- **refactor: rename `bridge/questions.rs` → `tray.rs`** (`8035b38`) to match the
  `session_tray` table (renamed in migration 0010).
- **refactor(ui): share the phase→bucket mapping** (`9c48390`) between PhasePill
  and SessionPhaseChip via `lib/phase.ts` (`phaseBucket`); +3 frontend tests.
- **docs: correct drifted counts** — internal MCP tools 24→25 (web_search),
  external driver tools 21→19, test counts 410→425 (377 lib + 31 ext + 7 sig +
  10 storage + 56 frontend). Documented the `bench/swebench/` eval harness in
  ARCHITECTURE.md.

---

## 2026-06-05 — Rain read-only gh, bell self-heal, Emma removed

- **feat(policy): give Rain read-only `gh` access (write-verbs denied).** Rain
  (EYES) could not touch `gh` at all. Loosened to read-only: write verbs (e.g.
  `pr create`/`merge`, `issue close`, `release`) stay denied, and `gh api` stays
  fully blocked (it can mutate behind an innocuous-looking call). Rain can now read
  PR/issue/CI state for review without gaining any write path.
- **fix(ui): boot-time sweep withdraws stale tray rows on dead sessions.** The
  notification bell counted pending tray rows from sessions that had since closed or
  orphaned, so the badge showed phantom items that never cleared. Added a startup
  sweep that withdraws pending tray rows whose session is closed/missing — the count
  now self-heals stale cruft on launch instead of accumulating it.
- **chore: remove Emma from the core entirely (migration 0017).** The solo helper
  agent Emma is gone — prompt, auto-spawn, overlay, signaling, and her seeded data
  are purged (migration 0017 drops the `emma` row; the legacy CHECK constraints stay
  permissive). The duo (Brian = HANDS, Rain = EYES) is the whole agent model now.
  Emma is slated to return as the first bot-hq plugin — TBD. Canonical docs
  (ARCHITECTURE.md, CLAUDE.md) updated to match.

---

## 2026-06-04 — audit remediation (continuous pass)

Working through the full-codebase audit (findings in the session's investigate doc),
priority order, one commit per cohesive batch. Newest bullet first.

- **perf(ui): lag-resync recovery, then drop the redundant polls (E2).** The
  PendingTray 2s, Dashboard per-tile phase 5s + pending-choices 5s, and Emma 3s
  `refetchInterval` polls were the only recovery if the bridge subscriber dropped a
  `session:*` event on `Lagged` (it just logged). Added that recovery properly: the
  subscriber now emits a `session:resync` on `Lagged`, and `GlobalEventSync`
  invalidates every event-backed query on it — so a dropped burst self-heals. With
  that net in place the four event-backed polls are redundant and dropped (the
  60s/10s polls on projects/models/plugins stay — no event source). Net: no constant
  background refetch churn, and lag is now self-healing instead of silently stale.
- **perf(ui): lazy-mount Settings subtabs, keep once visited (E8).** Settings
  rendered all 6 panels up front (CSS-`hidden`), firing every panel's queries on
  first visit. Now a panel mounts only once its tab is visited and then stays
  mounted — so the default "agents" tab's queries fire on open, the rest only when
  the user actually clicks that tab. Keeps the intentional edit-survival across
  subtab switches (panels stay mounted once shown).
- **fix(storage): cl_index/cl_folders upsert returns the real id (F1).** Both
  `upsert_cl_index` and `upsert_folder_description` returned `last_insert_rowid()`,
  which on an upsert that takes the DO UPDATE branch can report the bumped (unused)
  AUTOINCREMENT value instead of the real row id — the exact footgun already fixed
  in `session_docs.rs`. Switched both to `RETURNING id`. Latent (no caller trusts
  the returned id today), so zero behavior change — purely removes the landmine.
- **fix(ui): Enter sends in the chat; Shift+Enter for a newline (D8).** The chat
  textarea required ⌘/Ctrl+Enter — bare Enter just inserted a newline, which read
  as "Enter doesn't work". Now bare Enter sends (⌘/Ctrl+Enter still works as an
  alternate), Shift+Enter inserts a newline, and IME composition is respected so
  multibyte input isn't cut off. Hint updated to `↵`. (Emma's single-line input
  already sent on Enter.)
- **refactor: parse ViolationKind via serde, not a hand match (C3).** The
  request_approval `kind` parser (`jsonrpc::parse_violation_kind`) duplicated the
  enum's snake_case wire names in a hand-written match that had to be kept in
  lockstep with `ViolationKind`. Parse through serde so it can't drift. (Only delta:
  it now also accepts `policy_mutation` — benign, since command execution gates on
  `ToolBlocklist` specifically, not on the kind being parseable.)
- **a11y(ui): focus-trap the dialog modals (D7).** New `useFocusTrap` hook: focuses
  the first focusable on open, traps Tab/Shift+Tab inside the dialog, and restores
  focus to the trigger on close. Applied to the four dialog modals (ActionModal,
  New-session, MaintainCL, Register) — keyboard/screen-reader users could
  previously Tab out into the obscured page behind the scrim. (The Emma/Policy
  slide-over drawers follow the same pattern — left for a focused follow-up.)
- **fix(ui): don't offer file/folder creation under the `_globals` root (D12 rem.).**
  The CL right-click menu offered "New file/New folder" on the `_globals` virtual
  bucket (cross-project system files), which would create files at the CL system
  root. Guarded. (Left as intentional/low-value: D11 bell counts sessions — by
  design, the dropdown already shows per-session item counts; #31 ok/yes→Approved —
  reasonable approval semantics.)
- **perf(cl): de-quadratic cl_rescan + parallelize all-projects rescan (E5).**
  Backend: `cl_rescan` did an `existing.iter().find()` per on-disk file (O(disk ×
  index)); now builds a `HashMap<path,&row>` once for O(1) lookup. Frontend: the
  "rescan all" button ran each project's rescan serially (`for…await`); now
  `Promise.all` with per-project error isolation. (Left: wrapping the per-row
  upsert/touch/delete writes in one transaction — cl_rescan is on-demand with a
  modest row count, so the sequential awaits aren't worth threading a tx through
  three storage methods.)
- **fix(ui): share the clock-time formatter (C5, partial + zone bugfix).** Emma's
  overlay had its own `formatClockTime` that used `new Date(iso)` directly — NOT
  zone-safe, so a zone-less timestamp misparsed (the staleness-bug class the
  `parseUtcMs` baseline exists to prevent). Moved a zone-safe `formatClockTime` into
  `lib/time.ts` and used it. (Left as-is: the SessionView↔Emma respawn-banner / jump
  button extraction is pure maintainability; Emma's author-color maps use a
  deliberately different terminal palette, NOT a dup of `authorColor.ts`.)
- **fix(agents): align Rain's prompt with her enforced tool blocks (C1).** RAIN_ROLE
  listed `git branch`, `gh pr view`, `gh pr list`, `gh issue view`, `gh issue list`
  as allowed read-only investigation — but `spawn.rs --disallowedTools` blanket-blocks
  `git branch:*` / `gh pr:*` / `gh issue:*`, so Rain was told she could run commands
  the mechanism denies. Tightened the prose to match enforcement (kept the security
  boundary intact — enumerating "safe" gh subcommands would risk missing a mutating
  one like `gh pr comment`/`review`). Guard test asserts the blocked commands aren't
  advertised as allowed. (Deferred: F3 — `auto_supersede_prior_pending`'s supersede
  +insert aren't transactional; the proper fix is a combined atomic storage op, a
  moderate refactor on the critical tray path to close a microsecond crash window —
  poor ROI/risk for a sweep.)
- **fix(ui): distinguish DocumentPane load errors from empty (D6).** The
  `session_doc_search` and `compute_apply_diff` queries didn't expose `error`, so a
  failed fetch rendered identically to a genuine empty ("No {phase} documents yet."
  / a blank diff). Surface the error text distinctly for each.
- **tidy: session_doc timestamp + dedup push-gate action string (F2, C4).** F2:
  `upsert_session_document` used `chrono::Utc::now().to_rfc3339()` (`+00:00`) instead
  of the project-standard `now_utc()` (`Z`) — cosmetic, but it broke the single
  UTC-baseline invariant. C4: the `git push (<branch>)` violation/approval action
  string was built independently in `policy::hooks` and `signaling::server`; hoisted
  to one `policy::push_gate_action` helper so the audit log can't show two shapes.
  (F1 — cl_index `last_insert_rowid` → `RETURNING id` — left as-is: it's latent, no
  caller trusts the returned id, and the table's PK shape makes the change higher
  risk than the dormant landmine warrants.)
- **docs: correct the PluginManager status (finding 33).** ARCHITECTURE.md called
  the Plugins tab "Placeholder UI" and PLAN.md said the frontend install/heartbeat
  wiring "is not" done — both stale: `PluginManager.tsx` has working
  install/enable/disable/uninstall + a `plugin:crashed` heartbeat indicator. What
  actually remains is live plugin *execution* (the per-plugin iframes + ping/pong;
  `PluginSlot` was removed as dead code). Updated both docs to match.
- **fix(core): stop dropping control events under load (A3).** `SessionCloseRequest`
  / `AgentAdvancePhase` shared the one 64-slot broadcast channel with per-chunk
  `MessagePersisted`, and the main.rs handler `.await`ed the slow core work (close
  kills subprocesses) INLINE in the recv loop — so a chunk flood during a close
  could lag the channel and silently drop a close/advance (subprocess kept
  running / phase never advanced on the backend). Now the recv loop only matches +
  hands off to a serial unbounded worker (never blocks), and the channel headroom
  went 64→1024. (E2 — dropping the redundant tray/phase polls — stays deferred:
  the *frontend* subscriber still only logs on Lagged without re-syncing, so the
  polls remain a cheap safety net; low value now that E1 made invalidation
  targeted.)
- **fix(agents): recover a deaf agent instead of bridging to it forever (A2).** Root
  of the #4 user→HANDS desync. The supervisor holds the public input receiver and
  forwards to the per-incarnation stdin pump with `let _ =`; when that pump died
  (stdin write failed) the error was swallowed, the public `input_tx` stayed open
  (so `is_stale()` read false), and the supervisor kept bridging to a now-deaf
  child as long as its event channel lingered — Brian silently ignored all input
  while Rain kept working, with no signal and no recovery. Now: a failed forward
  tears the supervisor down (kill + return) so the public channel closes →
  `is_stale()` true; and `core::broadcast` respawns a stale handle before
  delivering, so the next user message auto-heals the session. Test:
  `supervisor_terminates_when_incarnation_input_pump_dies`.
- **fix(core): prune bridge session maps on close; log swallowed Emma sends (A4, A5).**
  `close_session` cleaned the sessions map, tray, and policy snapshot but never the
  bridge's `session_projects` / `session_awaiting` maps — each open→close leaked a
  map entry + a dangling `Arc<AtomicBool>` for the process life. Added
  `bridge.unregister_session` and call it from `core::close_session`. Also two
  Emma stdin sends (`broadcast` + the OOB resolve-wake) used `let _ = …send()` with
  no log — if Emma's input pump died the message persisted + showed in chat but she
  never saw it, zero signal; now logged (same diagnosability fix as the duo desync
  paths).
- **fix(policy): make silent policy-disarm visible (B1, B2).** Two paths could
  silently weaken enforcement with no signal. (B1) `Policy`/`SessionPolicy` had no
  unknown-key handling, so a typo (`push-gate:`, a mistyped `tool_gate:`) was
  dropped and the setting resolved to the permissive default. Added a loud
  `tracing::warn` on unrecognized top-level keys in the policy + session-snapshot
  load paths — deliberately NOT `#[serde(deny_unknown_fields)]`, which would break
  older files carrying the retired `tool_blocklist` (failing parse → disarming the
  git-hook enforcement) and is unsupported alongside `SessionPolicy`'s
  `#[serde(flatten)]`. (B2) `audit.rs` `unwrap_or_default()` silently reset the
  policy-hash cache on a corrupt file → every file re-registered as `FirstSeen`,
  disarming mutation detection for that cycle; now logs the reset loudly.
  Non-breaking — enforcement behavior unchanged, the disarm is just no longer
  silent. Tests cover the key-detection.
- **add(ui): close-session action in the SessionView header (D1).** The
  `close_session` command had zero UI callers — a human could start/configure a
  session but never end one (only an agent could, via MCP). Added a confirm-gated
  "✕" close button (kills Brian + Rain, archives the session); the existing
  `session:closed` listener navigates back to the dashboard, and the Archive tab
  can reopen it (resumes via --resume). Surfaces a close-failed inline error.
- **fix(ui): confirm destructive actions; drop dead CL "New file" button (D5, D12).**
  Saved-model Delete (removes the stored auth token, irreversible) and Unregister
  Project now require a `window.confirm` (matching the plugin-uninstall pattern).
  Removed the permanently-disabled "New file — backend not yet wired" sidebar
  button — creation is wired via right-click (which has the folder + name context
  the header button lacked). (Deferred: guarding new-file/folder on the `_globals`
  virtual root — lives in the ContextLibrary menu builder, folded into a later CL
  batch; harmless meanwhile.)
- **remove(ui): dead top-bar search + dead footer links (D2).** The "Search
  sessions, agents, tasks…" topbar input stored state but never filtered or
  navigated anything, and the footer "API Docs"/"Support" were `href="#"`
  placeholders. Removed both (+ the now-unused `SearchIcon`/`useState`). Real
  session search is a clean follow-up feature if wanted — left out here since it
  promised searching agents/tasks (not first-class entities) and crosses routes.
- **fix(ui): surface errors on the silent HITL paths (D3, #32).** Three core
  human-in-the-loop actions failed silently: broadcast-send (`ChatInput` try/finally
  with no catch → unhandled rejection, user thought the message sent), tray-resolve
  (`DocumentPane` `console.error` only → answer stuck pending, no signal), and
  config restart-agents (`ClaudeConfig` loop with no catch). Each now catches and
  shows a dismissible inline error; the broadcast fix lives in shared `ChatInput`
  so it covers both SessionView and the Emma overlay.
- **perf(ui): scope event-driven query invalidation + concat chat batches (E1, E3).**
  `GlobalEventSync` called `invalidateQueries()` with no key on every `session:*`
  event → an app-wide refetch storm (10-20+ queries incl. `compute_apply_diff`
  spawning `git`) on a single choice-resolve. Now each event invalidates only the
  query families it can affect (tray / phase+docs+diff / close lists). Also
  `chat.ts applyBatch` spread `[...current, msg]` inside the per-message loop
  (O(N·K) — a 20-msg batch copied the history up to 20×); now accumulates per
  session and concats once. (E2 poll-removal deferred until the lossy-channel fix
  A3 — the bridge subscriber drops `session:*` events on `Lagged` without
  re-syncing, so the safety polls stay load-bearing until then.)

## 2026-06-04 — fix: resume the duo after the user answers a choice

The Brian↔Rain peer-forward went silent after the user clicked an
`ask_user_choice`/`request_approval` button, staying frozen until the user typed
free text or advanced a phase. Root cause: `ask_user_choice`/`request_approval`
set a shared `awaiting` `AtomicBool` (via `bridge::set_session_awaiting`), but the
common resolve path — `bridge::resolve_choice` → `ResolveOutcome::Delivered`
(oneshot send succeeds, agent resumes via the tool return) — never cleared it, so
`duo::flush_buffer` (gated on the flag) kept dropping every peer-forward. Only the
OOB-fallback arm, `broadcast`, and `advance_phase` cleared it. Likely the root of
the long-standing "answer didn't round-trip" symptom (notes #2).

Fix: `bridge::resolve_choice` now clears the halt (`clear_session_awaiting`) right
before delivering the pick — the bridge owns the awaiting map and set the flag, so
it clears it symmetrically. Clearing *before* `p.tx.send` (not after) avoids a
1-chunk race where the resumed agent's first reply could be suppressed before the
flag flipped; it also covers the Err/OOB fall-through (core then re-clears + wakes
stdin, harmlessly redundant). Covers choices, approvals (incl. pre-push), and
`action_gate`. Regression test `resolve_choice_delivered_clears_awaiting` asserts
the flag is set after the ask and cleared after a Delivered resolve.

## 2026-06-04 — remove user-facing screenshot button

The 📸 "share window" button (SessionView header + Emma overlay) was designed as
an agent context tool, not a user action — YAGNI for humans, who have no real
use-case for it. Removed the button + dismissible error banner + the shared
`useScreenshotCapture` hook (deleted, zero consumers) from both surfaces, plus
the frontend-only `capture_window_screenshot` Tauri command (+ its specta
registration + regenerated binding). The `webview_screenshot` MCP tool — the
agents' "eyes on the UI" — is unaffected: it uses the separate `capture_main_window`
helper, which stays.

## 2026-06-04 — remove UI manual phase-advance

User-directed removal of the UI's ability to advance a session's IPAV phase. The
interactive `PhasePillRow` in the `SessionView` header (which called the
`advance_session_phase` Tauri command) is gone; the backing command was
frontend-webview-only, so it was removed end-to-end — command + `IpavPhase`
import + specta registration + regenerated binding — rather than left dead. The
header still *displays* the current phase (read-only) and the
`session:phase_changed` listener stays: agents still drive phases via the
`advance_phase` MCP tool → `AgentAdvancePhase` → `core.advance_phase` (untouched).
Resolves the "double phase-control surface" gap — the identical-looking
`DocumentPane` pills are a view-only tab selector and stay.

## 2026-06-04 — codebase + docs cleanliness pass

Swept the codebase, CL, and docs for redundancy/staleness/dead code (clippy was
already near-clean; the debt was mostly stale docs + small frontend dup). 9
commits, gated per batch; no behavior change except the two UI fixes below.

- **Docs synced to reality.** ARCHITECTURE.md/README.md documented 3 tools that
  no longer exist (`grant`/`revoke`/`list_session_permissions` — the subsystem
  was deleted); "26 internal tools" was actually 24 and omitted `action_gate`;
  the `questions` table was renamed `session_tray` (migration 0010); the whole
  Claude Config surface + `models`/`app_settings` registry + `llm_proxy` were
  undocumented; one event name used illegal dots. Rewrote the "Session
  permissions" section for the current `session_policy.rs` frozen-snapshot model
  and fixed the in-repo `CLAUDE.md` push-grant + data-path references. Pruned
  PLAN.md (dropped hardcoded test counts; noted shipped model/Claude-config work).
- **`fix`: ToolSearch added to Rain's allowlist** — her role prompt promised it
  but `spawn.rs` blocked it (WebSearch was already allowed). Test locks the
  prompt↔allowlist contract.
- **`tidy`: dedup + clippy-clean** — hoisted the hand-synced
  `AGENT_FILTERED_MCP`/`RESERVED_MCP_KEYS` pair to one crate constant, refreshed
  stale `session_tray` comments, collapsed identical `HookKind::filename`/
  `subcommand`, resolved all clippy nits.
- **`refactor`: `storage::model` → `row_types`** — the module holds all 15 row
  types, not one Model; the name collided with the sibling `models.rs`.
- **`chore`: deleted dead frontend** — `PluginSlot.tsx` + `stores/layout.ts`
  (zero consumers).
- **`refactor(ui)`: extracted `GatedKeywordList`** shared by Settings + the
  session policy panel; dropped a `formatTimestamp` shadow of `lib/time`.
- **`fix(ui)`: purge a closed session's messages** from the chat store on
  `session:closed` (was a latent leak — `clear()` was test-only).
- **`refactor(ui)`: SessionTile indicates, doesn't answer** — removed the inline
  `ChoiceBanner` (a second answer surface duplicating the Tray tab); the tile now
  shows a `[Need User Input · N]` count and points to the session's Tray tab.
- **`refactor`: dropped the completed legacy-CL startup import** (one-shot
  `~/.bot-hq-legacy-2026-05-15` mirror; the dir is gone, so it was a no-op).

Verified-no-change (reported, not edited): push_gate `ask` docs already match the
code (the June-2 hard-block issue was fixed by the June-3 tray work); the two
`resolve_agent_overrides` call sites are intentional layering, not a double-resolve.

## 2026-06-03 — close_session withdraws pending; backfill stale closed-session pending

`close_session` left a session's pending tray rows as `pending` forever, so already-closed sessions
accumulated dead pending (61 rows across ~55 old sessions). Fixed:
- `core::close_session` now calls `storage.withdraw_pending_tray_for_session` after marking the
  session closed → a closing session's pending questions/approvals/gated commands are withdrawn.
- migration `0011`: one-time backfill — withdraws pending rows belonging to already-closed sessions
  (clears the existing 61; no-op on a fresh DB and after the first run).

Test: `withdraw_pending_tray_for_session` is session-scoped + only touches pending (already-answered
rows untouched).

## 2026-06-03 — Tray pill count + pulse; remove the in-chat question popup

Final tray polish:
- The in-chat `ChoicePrompt` popup (`SessionView`) is removed — the Tray tab is the sole answer
  surface now. Dropped the now-dead `list_pending_choices` query, resolve handler, and state.
- The Tray pill shows a pending-count badge and pulses (bell-style `animate-pulse` + primary tint)
  when count > 0, so accumulated input is visible from any tab without opening the Tray. Count comes
  from `list_session_tray` (shared query cache; `GlobalEventSync` keeps it live event-driven).

## 2026-06-03 — Event-driven UI reactivity (no more "stale until tab-switch")

The UI only refetched on mount/tab-switch, so backend state changes didn't show until the user
navigated away and back: new session docs, a tray answer reflected in chat, and session close
(which stranded the user inside the now-closed session). Fixed event-driven — emit an event for
every relevant change + invalidate queries on it, no polling. (Supersedes the 2s Tray poll
`83385fe`.)

- Backend: new `SignalingEvent::DocChanged` + `SessionClosed` (+ `tauri_events` types +
  `bridge_subscriber` routes → `session:doc_changed` / `session:closed`). `session_doc_write` emits
  DocChanged; `core::close_session` emits SessionClosed via `bridge.notify_session_closed`.
  `resolve_choice`'s two OOB arms now call `notify_message_persisted` (capturing the insert id) →
  fires `agent:messages:batch`, so a tray-answered choice shows in the chat live.
- Frontend: `GlobalEventSync` (in `Providers`, inside `QueryClientProvider`) listens to the
  `session:*` events and `invalidateQueries()` (all) — event-driven, no timers. Dropped the Tray
  `refetchInterval`. `SessionView` navigates to the dashboard on `session:closed` for the current
  session. `agent:messages:batch` is excluded from the global invalidation (the chat consumes it
  directly; invalidating everything on each message batch would be wasteful).

Tests: `bridge_subscriber` routes for the 2 new events; the OOB-fallback test now asserts
`MessagePersisted` fires.

## 2026-06-03 — Tray tab live-refreshes (2s poll)

The Tray tab fetched `list_session_tray` once on mount, so newly-parked pending didn't appear (and
answered items didn't drop) until the user switched tabs and back. Added `refetchInterval: 2_000` to
the query (same cadence as the notification bell; the query only mounts while the Tray tab is shown,
so it's idle otherwise) so the inbox updates live.

## 2026-06-03 — auto-supersede only true re-asks, so pending accumulate

`auto_supersede_prior_pending` marked ANY prior pending from the same agent as `superseded` on
every new ask — so distinct questions/gates collapsed to the latest, defeating the "pending
accumulate while the user is AFK" goal (the tray showed only the most recent of an agent's asks).
Now it matches on `prompt`: a re-ask of the SAME prompt (the timeout-retry case it was built for)
still supersedes, but distinct prompts both stay pending and accumulate.

- `bridge/questions.rs`: `auto_supersede_prior_pending` gains a `prompt` param; the find filter adds
  `q.prompt == prompt`. Callers (`ask_user_choice`, `request_approval`) pass the new question.
- Tests: re-ask of same prompt supersedes + links via supersedes_id; two distinct prompts both stay
  pending.

Known related (not yet fixed): `close_session` doesn't withdraw a closed session's pending tray
rows, so they linger as `pending` forever (currently 61 across ~55 old closed sessions). Harmless —
the notifier's open-session filter hides them — but worth a cleanup + a close-time withdraw.

## 2026-06-03 — Notifications grouped per session ("Session-X needs your input [N]")

The header notification tray (`PendingTray`) now groups pending across sessions: one row per
session — "Session {id8} · needs your input [N]" with the per-session pending count and a
go-to-session CTA — instead of one row per item. The bell badge counts SESSIONS awaiting input (not
raw items). Stays notify-only (decision #7): answering happens on that session's Tray tab.

Source is the live in-memory `list_pending_choices` (covers the normal AFK-while-running case).
Reflecting durable pending that survived a restart would need a global durable pending query —
flagged follow-up.

## 2026-06-03 — Tray tab → actionable pending inbox (not a history log)

Reframed the session Tray tab (shipped read-only in `a91a603`) into an actionable **pending
inbox**: it shows only PENDING items and answers them inline. Pending questions / approvals / gated
commands accumulate there (durable — survive AFK + restart) and the user resolves them from the tab
when they return. Resolved history is intentionally dropped (it was noise — an inbox, not an audit
log).

- `DocumentPane` `TrayList`: filter to `status === "pending"` (removed the resolved-history
  rendering), reuse the shared `ChoicePrompt` (preset options + mandatory "Other") per item, wire to
  `resolve_choice(choice_id, picked)` and invalidate the `list_session_tray` query on settle so the
  answered item drops out. action_gate rows show the gated command above the prompt.
- Notifications (header `PendingTray`) deliberately stay notify-only (go-to-session CTA); a
  per-session "needs your input [N]" count is a planned follow-up.

## 2026-06-03 — Session-view Tray tab (Tray · I · P · A · V)

Surfaced the durable `session_tray` as a tab before the IPAV phase tabs, so every accumulated
question / approval / gated command (pending + resolved history) is visible per session — including
items that survived a restart.

- `tauri_cmd/questions.rs`: `SessionTrayView` + `list_session_tray(session_id)` reading the durable
  rows via `bridge.list_questions_for_session` (decodes `options_json`; carries `command_text` /
  status / kind / timestamps). Registered in `tauri_specta_gen.rs`; `bindings.ts` regenerated.
- `DocumentPane.tsx`: a phase-independent `Tray` pill before `PhasePillRow` (now `selected: Phase |
  null` so no phase highlights while Tray is active). Read-only v1: kind/agent/status badges, prompt,
  gated command, options + picked, timestamps; pending highlighted + ordered first. A phase
  transition updates the underlying phase but does NOT pull the user off the Tray.

Read-only for now — inline Approve/Reject from the tab is a possible follow-up (the in-chat
`ChoicePrompt` already resolves the active pending choice).

## 2026-06-03 — Durable tray (session_questions → session_tray) + execute-on-approve anytime

Renamed `session_questions` → `session_tray` (it outgrew "questions" — it durably mirrors every
awaiting-input tray item: questions, approvals, action_gate gated commands, halts) and made an
approved action_gate command execute whenever it's resolved — hours/days later, or after a restart
— not just within the in-memory oneshot's lifetime. Closes the gap `ae79f3a` documented (the
post-restart `None` branch couldn't execute because the command wasn't persisted).

- migration `0010`: `ALTER TABLE session_questions RENAME TO session_tray` + `ADD COLUMN
  command_text TEXT` + recreate the partial pending index under the new name. (Type
  `SessionQuestion` → `SessionTrayEntry`; method names kept to bound churn. The type isn't surfaced
  via tauri-specta, so bindings.ts / the frontend are untouched.)
- `command_text` persists the gated command on the row (set for ToolBlocklist approvals in
  `ask_user_choice_inner`, extracted before `approval` moves into `PendingChoice`).
- `resolve_choice` executes the approved command from the durable row on BOTH receiver-gone paths —
  the same-session timeout `Err` arm (generalizes `ae79f3a`) and the post-restart `None` arm — via a
  shared `maybe_run_gated` helper. The `Delivered` (in-band) path is excluded (action_gate's own
  future runs it there) → no double-fire.
- Exactly-once is now durable: gated on `answer_question`'s atomic pending→answered flip
  (`rows_affected == 1`), so a duplicate / stale / post-restart resolve can't re-run the command.
  Replaces the in-memory oneshot's exactly-once guarantee with a DB one.

Tests: `post_restart_action_gate_executes_from_durable_row` (None arm runs from `command_text`),
`resolve_twice_executes_gated_command_once` (exactly-once via the flip gate), plus the existing
`timed_out_action_gate_still_executes_on_approve` (now flip-gated).

push-gate unchanged: it blocks a live `git push` and can't be deferred days; stays now-or-times-out.

## 2026-06-03 — action_gate executes on approve even after a client timeout

`action_gate`'s approved command runs server-side via `execute_gated`, which lived only inside the
MCP request future. When claude-code's MCP client timed out (~30s) waiting on a human, that future
was cancelled before `execute_gated` ran, and the OOB fallback (`resolve_choice`) re-delivered only
"Approve" — so a gated command the user approved would silently never execute. (Surfaced live while
testing the push-gate work: a timed-out `gh api user` approval returned no output.)

Fix: run the approved command at resolve time, decoupled from the dead request future.
- `bridge/action_gate.rs`: `execute_gated` → `pub(super)` so the sibling `bridge::questions` module
  can call it (private-to-module otherwise → E0624).
- `bridge/questions.rs` `resolve_choice`, receiver-dropped arm: when the parked approval is an
  `action_gate` request (`ViolationKind::ToolBlocklist`) resolved `Approved`, run `execute_gated`
  and append its output to the OOB message body. In-band (`Delivered`) and dropped (`Err`) paths are
  mutually exclusive on one `tx.send`, so the command runs exactly once. Scoped to ToolBlocklist —
  `ask_user_choice`, `per_action`, and `push_gate` paths are unchanged.

Distinct from the June-1 fix (#2), which made the OOB fallback deliver the user's *decision* — that
works (verified). This covers the one tool whose *action* executes server-side.

Test: `timed_out_action_gate_still_executes_on_approve` aborts the request future to simulate the
client timeout, then resolves Approve and asserts the command ran (marker file) + output is in the
OOB body.

Known limitation: the reopened-session branch of `resolve_choice` (bridge lost the in-memory Parked)
can't execute — the durable `session_questions` row stores prompt/agent/session but not the command
string. Rare (needs a bridge restart between ask and answer); would need the command re-issued.

## 2026-06-03 — push_gate "ask" prompts per-push instead of hard-blocking

`push_gate: ask` used to make the `pre-push` git hook hard-block every `git push`
(exit 1, "flip the toggle to auto") — it never actually asked, unlike `action_gate`
for other gated commands. Now `ask` surfaces a per-push Approve/Reject prompt to the
user (reusing the `request_approval` → `PendingChoice` → `resolve_choice` →
`PushGate`-violation path) and blocks on their pick: approve → push proceeds, reject
→ blocked.

The `pre-push` hook runs as a separate subprocess that can't reach the running app's
bridge, so:
- `src/main.rs` persists the internal signaling server's bound address to
  `<data_dir>/.local/signaling-addr` at startup (`paths.rs::write_signaling_addr` +
  free `read_signaling_addr`); `SignalingServer` removes it on clean shutdown (Drop).
- `src/signaling/server.rs` adds a dedicated `POST /hooks/pre-push` route that calls
  `bridge.request_approval(kind=push_gate)` directly (no HANDS-only MCP gate, no agent
  identity in the URL path) and replies `{"approved": bool}`.
- `src/agents/spawn.rs` exports `BOT_HQ_AGENT` so the prompt attributes to the pushing
  agent (covers solo Emma; Rain can't push).
- `src/policy/hooks.rs::run_pre_push` POSTs that route inside a current-thread runtime
  (reqwest, 30-min timeout) and maps Approve→0 / Reject→1. Fail-closed (exit 1 + its
  own `PushGate`/Denied violation, distinct reason per failure: no addr / connect /
  timeout / non-200 / malformed) when the app is unreachable. A push with no
  `BOT_HQ_SESSION_ID` (manual human terminal push) stays hard-blocked with guidance —
  avoids an `env -u BOT_HQ_SESSION_ID` bypass.

Lockstep prompt/doc text (7 spots) flipped from "ask = hard block, flip the toggle" to
"just run `git push`; the hook prompts the user Approve/Reject per push; you don't call
a grant tool or flip a toggle": `policy/mod.rs` (field doc, `Ask` variant, system-prompt
block), `policy/hooks.rs` (module + `run_pre_push` doc), `agents/general_rules.rs`,
`agents/prompts.rs`. ARCHITECTURE.md + README.md push-gate sections corrected (also fixed
adjacent pre-existing B-series drift: `push_gate`/`force_push` are scalar `auto|ask` /
`blocked|allowed`, no `.mode` / `remembered_approvals`).

Known follow-up: ARCHITECTURE.md's "Session permissions" / `grant_session_permission`
section + the Tool-Gate push-grant reconcile line still describe the pre-B-series grant
mechanism (removed when push/force-push became pure toggles) — left for a separate
doc-sync pass, out of scope here.

Tests: +1 paths (addr round-trip), +2 server (`/hooks/pre-push` approve/reject + missing
session_id → 400), +2 hooks (ask-without-session block, no-addr fail-closed Blocked).

## 2026-06-02 — Surface + control Claude Code config in Settings

bot-hq's agents are `claude-code` headless subprocesses, so the user's
`~/.claude` config (skills, plugins, hooks, CLAUDE.md/memory, MCP, effort)
**leaks into the agents** — a self-invoking skill or a plugin hook can derail a
Brian/Rain workflow, and that inherited config was invisible in the UI. New
**Settings → Claude Config** subtab surfaces it and lets the user control it,
both globally (edit their real `~/.claude`) and per-agent (an override layer
bot-hq injects at spawn), without bot-hq ever writing its own config into
`~/.claude`. Design: [`docs/plans/2026-06-02-claude-config-surface-design.md`](docs/plans/2026-06-02-claude-config-surface-design.md).

- **Read/resolve layer** (`src/claude_config/reader.rs`): resolves the config
  dir (honors `CLAUDE_CONFIG_DIR`), reads `settings.json` + `~/.claude.json` +
  `CLAUDE.md`/memory + `skills/` + `enabledPlugins`, with secret masking and the
  known traps flagged (e.g. `settings.json` `mcpServers` is ignored by
  claude-code — it loads MCP from `~/.claude.json`; bot-hq forwards both).
- **Inheritance lens** (`src/claude_config/mod.rs`): the single source of truth
  for which agents pick up each surface (Brian/Emma inherit; Rain `--bare` skips
  skills/plugins/hooks/CLAUDE.md; model/permissions overridden). Drives the
  per-surface badges in the UI.
- **Override store** (`src/claude_config/overrides.rs`):
  `<data_dir>/claude-overrides.json` (0600), `_all` fan-out + per-agent entries.
  Wired into `spawn.rs::build_command` (merged into the injected `--settings`
  `skillOverrides`/`enabledPlugins`/`ultracode` + effort/auto-memory/CLAUDE.md
  env) and `session.rs` (per-agent MCP filtering). `skillOverrides` (verified on
  claude 2.1.160) is the clean lever for "disable a self-invoking skill for the
  agents only" — the headline use case.
- **Global write-back** (`src/claude_config/writer.rs`): read-modify-write of
  `settings.json` that preserves all other keys + secrets; typed commands for
  string/bool knobs + plugin enablement. Malformed `settings.json` errors
  without clobbering.
- **UI** (`frontend/src/app/ClaudeConfig.tsx`): the Settings tab is now tabbed
  (Agents · Claude Config · Tool Gate). Claude Config is a 2-pane tree
  (surface-first) reusing the Context Library shell idiom, with the inheritance
  lens, global editors, and per-agent override controls. **All edits (global +
  override) batch behind one Save** (review before writing `~/.claude`); after
  saving, a banner offers to **restart running agents** so they pick up the new
  config (read at spawn). New `export-bindings` CLI subcommand regenerates the
  frontend bindings headlessly.
- **Force-restart primitive**: `CoreAppState::restart_session` + the
  `restart_session` Tauri command evict a live duo and re-spawn it (re-reading
  overrides + per-agent mcp-config; agents resume via `--resume`). Distinct from
  `respawn_session`, which is the idempotent on-mount "ensure started" and a
  no-op on a healthy session.
- **Out of scope** (deferred, noted in the design): SKILL.md global edit, MCP
  list/markdown/hooks rich widgets, full precedence engine. `policy.yaml` is
  intentionally excluded (bot-hq-internal, not user Claude config).

## 2026-06-02 — Auto-resume agents on transient API errors

A transient upstream API error (e.g. Anthropic `529` Overloaded) killed an
agent's claude-code subprocess and **nothing respawned it** — the session sat
dead on "API Error: Overloaded" indefinitely (observed on `s-c9f509d2`:
Brian died mid-Apply on 2026-06-01 after B1+B2 of the policy redesign had
landed + pushed). The only restart path in the codebase was the manual
`restart_emma` tool; an agent that hit a self-clearing blip was stranded.

Root cause: `events.rs` collapsed the result event's `api_error_status` (the
HTTP code) into a bare `is_error` bool, discarding the transient-vs-permanent
signal; on the subsequent `Exited`, `pump_agent` drained the buffer and the
supervisor task simply ended — no retry, no backoff, no respawn.

- **Signal plumbing.** `AgentEvent::TurnComplete` now carries
  `api_error_status: Option<u16>`; `events.rs::extract_api_status` coerces the
  wire value (number or string) and `spawn::is_transient_api_error` classifies
  it (`408/425/429/500/502/503/504/529` transient; `400/401/403/…` permanent —
  the DeepSeek system-role `400` stays a hard stop).
- **Retry supervisor.** New `spawn_supervised_agent` wraps the per-incarnation
  `spawn_agent` in a respawn loop that exposes STABLE event/input channels, so
  the peer-forward and `SessionHandle` survive a respawn with zero rewiring. On
  a transient failure it resumes the agent (`--resume <uuid>`, UUID captured
  from the tapped `init` event) after capped exponential backoff
  (2/4/8/16/30s, 5 attempts) and nudges it to continue where it left off; a
  clean turn resets the budget; a permanent error or an exhausted budget
  surfaces a clear message and unwinds. `Exited` is suppressed mid-retry —
  channel-close is the race-free end-of-incarnation signal, so the final
  errored `TurnComplete` is always seen before classifying.
- `core/session.rs` spawns Brian/Rain via `spawn_supervised_agent`
  (`RetryPolicy::default()`); `pump_agent` / `duo.rs` behaviour is unchanged
  (the error text is still persisted for UI visibility and never peer-forwarded).
- +9 lib tests (classifier ×2, status propagation ×3, backoff cap, supervisor
  resume-then-clean / permanent-no-resume / give-up-after-cap). Lib suite
  **305 passing**; clippy clean on touched files; release build green.

**Follow-up — evict stale session handles.** The supervisor closes its
channels on a *permanent* death (a non-retryable error or exhausted budget),
but the dead `SessionHandle` lingered in the in-memory map, so
`ensure_session_started` fast-pathed on `contains_key` and never re-spawned —
the session was stuck until an app restart (`ensure_emma_started` had the same
zombie). Added `SessionHandle::is_stale` / `EmmaHandle::is_stale` (true once a
supervisor drops its input receiver, closing the stable sender; stays false
during a healthy run *and* a transient-retry backoff). Both `ensure_*_started`
now treat a stale handle as absent: evict it (killing already-dead agents is a
no-op) and re-spawn via the resume path on the next interaction. Transient
deaths already self-heal via the supervisor; this closes the permanent-death
case.

## 2026-06-01 — Fix nested-runtime panic in policy-mutation audit

Sending a message to a session panicked the tokio worker with "Cannot start
a runtime from within a runtime" (`policy/audit.rs:181`), wedging session
start. Root cause: `log_sync` built a nested tokio runtime and `block_on`'d
it to append a `PolicyMutation` entry — harmless in the hookless
`policy-check` subprocess, fatal from the in-process async call sites
(`spawn_session_handle`, the signaling bridge). The Tool Gate commits had
rewritten the policy YAML, so the stale `.policy-hashes.json` made the next
session-start audit take the `Changed` branch → `log_sync` → panic.

- `ViolationsLog`: private `write_lock` switched from `tokio::sync::Mutex`
  to `std::sync::Mutex` (the guard is never held across an `.await`), and
  added synchronous `append_blocking` / `record_blocking` siblings sharing a
  `build_record` helper. The async `append`/`record` keep identical
  signatures, so all existing callers are unchanged.
- `audit.rs::log_sync` now calls `record_blocking` directly — no runtime, so
  it's valid in every context. One fix covers all three call sites.
- Self-healing: the first post-fix audit logs the (audit-only, non-blocking)
  `PolicyMutation` for the changed files and refreshes the hash cache; no
  data files touched.
- +1 regression test (`change_detected_inside_runtime_does_not_panic`) that
  runs the audit inside a `#[tokio::test]` runtime — it reproduced the exact
  panic before the fix. cargo test (347) + release build clean.

---

## 2026-06-01 — Maintain CL dispatch button (Context Library)

A "Maintain CL" button in the Context Library sidebar opens a dialog → the
user picks a project → a Brian + Rain session is dispatched pre-loaded with
a hardcoded, engineered prompt to maintain that project's CL (audit the
where-things-live map, sharpen descriptions, prune stale notes — keeping
the CL lighter than the codebase). Delegating CL upkeep to a session.

- New generic Tauri command `dispatch_session(id, title, project,
  repo_path, prompt)` (`tauri_cmd/sessions.rs`): create row → register
  project → `ensure_session_started` (spawn duo) → `broadcast(prompt)` in
  one atomic call. A fresh session spawns blank (`resume None`) and bot-hq
  doesn't replay storage to stdin, so the prompt must reach a LIVE session —
  hence spawn-then-broadcast in the command (avoids both the
  broadcast-before-spawn "no live session" race and a SessionView
  route-state hook that could double-send).
- The engineered prompt is a frontend const (`lib/maintainClPrompt.ts`,
  vitest-tested) so it's HMR-iterable; the Rust command stays generic.
- UI: `MaintainCLModal.tsx` (project picker → dispatch → navigate to
  `/sessions/<id>`), the sidebar button, wired in `ContextLibrary.tsx`.
- 7 files (3 new, 4 modified); +2 frontend tests (prompt anchors).

## 2026-06-01 — CL ⇄ IPAV workflow tightening

Tied session docs and the CL more tightly to the IPAV workflow so each
phase leaves ONE rewritable doc the next phase builds on, and the CL
stays fresh without bloating. Three commits:

- **One doc per phase, structurally** (`2d205d0`): `session_doc_write`
  keys phase-tagged docs by phase via an `effective_slug` helper
  (`bridge/session_docs.rs`) — repeated writes (even under a varied slug
  like `plan-v2`) overwrite the single `plan` doc, latest body wins. No
  migration; untagged scratch docs still key by slug. Tool descriptor
  (`protocol.rs`) now says "rewrite, never -v2".
- **Markdown doc preview, no count chips** (`3819c9b`): the chat's
  react-markdown renderer is extracted to a shared `Markdown.tsx` (GFM,
  code blocks, new-tab links, Industrial-Terminal styling) and reused in
  `DocumentPane`; the raw `<pre>` is gone and the per-phase doc-count
  indicators (`PhasePill` `·{n}` + the `{n} docs` span) are removed.
  Session docs aren't user-editable, so a rendered preview beats raw text.
- **Phase-doc chaining + CL model + close-loop** (`03d7615`): prompts now
  require Plan to build on the Investigate doc, Apply on Plan, Verify on
  Apply; HANDS authors the single phase doc while Rain reviews in chat (no
  two-author clobber on the shared, author-less `session_documents` row);
  CL is framed as "study notes, not a textbook" (a where-things-live map,
  not a code copy); and a write-then-prune close loop has HANDS append
  ≤~5 non-obvious one-liner learnings to a project's `notes.md` before
  `close_session` (user curates later in the CL tab). `ARCHITECTURE.md`
  softened to match.

Tests: 296 lib (+9: 2 session-doc helper, 4 general_rules anchors, 3
prompts anchors) + 29 frontend (+2 Markdown, −1 PhasePill count). Agents
pick up the new prompts and the markdown doc view only after a rebuild +
app restart.

**Follow-up bug fixes (same session):** `c19e0e0` — `session_doc_write`
returned a bogus row id on overwrite, because `last_insert_rowid()`
reports the bumped AUTOINCREMENT value on an upsert's UPDATE branch;
switched to `INSERT … RETURNING id`. `547d364` — agent self-advance (the
`advance_phase` MCP tool) only moved the frontend chip:
`SignalingEvent::AgentAdvancePhase` was consumed solely by the Tauri emit
subscriber and never routed to `core.advance_phase`, so the backend
`IpavState` (and every `[PHASE: X]` peer envelope) stayed stuck on the
default phase — the same no-op class as the old close_session bug. Added
the missing arm to the main.rs signaling consumer. Tests: 297 lib (+1
upsert-id regression).

## 2026-05-31 — Tool Gate: global gated-Bash keywords + action_gate

Replaced the per-project `tool_blocklist` PreToolUse gate (2026-05-29) with a
global, user-configurable **Tool Gate**: one keyword list
(`<data_dir>/tool-gate.json`, edited in Settings → "Gated Bash Keywords") over
agent Bash commands. A `gate` keyword blocks the command (PreToolUse exit 2) and
routes the agent to a new `action_gate` MCP tool, which surfaces Approve/Reject
and — on approve — EXECUTES the command in the session repo and returns its
output (an action request, not a permission request); `auto_allow`/no-match runs
normally. Gate-run pushes pre-record a session push grant for the current branch
so the pre-push hook doesn't double-gate.

- Backend: `src/policy/tool_gate.rs` (config + case-insensitive substring matcher
  + timeout-bounded executor), `action_gate` MCP tool
  (`src/signaling/bridge/action_gate.rs`), hook rework `run_tool_blocklist` →
  `run_tool_gate` (`hooks.rs` + `spawn.rs`).
- Frontend: Settings "Gated Bash Keywords" section; removed the commit/push
  GrantPills (+ their now-unused session-permission Tauri commands — the bridge
  methods + MCP tools are retained).
- Cleanup: retired `.claude/hooks/approval-gate.js` (+ its settings wiring);
  `policy.yaml` `tool_blocklist` marked RETIRED (parses, unenforced); reconciled
  agent prompts + canonical docs.

NB: the global keyword list defaults EMPTY — configure `gh` / `git` / `push` /
etc. in Settings to restore the 2026-05-29 outward-command protections.

Gates green: cargo test (287 lib + 49 integration), frontend vitest 28, tsc,
release build, frontend build.

---

## 2026-05-31 — SWE-bench Verified harness + test-feedback loop

Added `bench/swebench/` — a harness that benchmarks the duo (Brian/Opus-4.8 +
Rain/DeepSeek-V4-Pro) on SWE-bench Verified by driving the external MCP
(create_session → send_message → poll snapshot for the SWEBENCH_DONE sentinel →
`git diff` → predictions.jsonl), then scoring with the stock swebench harness.
Stdlib-only rollout driver; dataset via the HF datasets-server REST API.

**Result:** 27/39 resolved (69%) across all 12 Verified repos, 0 scoring errors.
The duo trails strong single-model Opus-4.8 — a structural gap (no test-feedback),
not the model.

**Test-feedback loop** (`--verify`): after the duo signals done, run the repo's
EXISTING tests against the patch in the prebuilt container (model_patch only, no
test_patch — leakage-free), bounce regressions back with their errors, revise,
cap at K rounds. On 3 known-wrong instances it flips SHALLOW regressions
(astropy-13398: 6 broken tests → resolved) but not CATASTROPHIC ones (requests:
43–45 broken tests → unresolved even with error-rich feedback). Also surfaced a
duo discipline gap: it signalled DONE while existing tests still failed.

Notes: `--instance-ids` (hand-picked diverse sets), incremental-save,
`.git/info/exclude` artifact guard (an agent's venv got swept into a 39MB diff via
`git add -A`), datasets-server retry. On Apple Silicon, score with prebuilt images
under emulation (`--namespace swebench` + `DOCKER_DEFAULT_PLATFORM=linux/amd64`);
never `--namespace none` (forces a rebuild that hits SWE-bench's bit-rotted
`setup_env.sh`). Run outputs gitignored.

---

## 2026-05-29 — Mechanical gate for outward/mutating agent commands

After an agent confabulated a "third party confirmed X" instruction inside its
own reasoning and published it as a GitHub issue comment under the user's
identity — via the honor-system `request_approval` path, with no mechanical
gate — added defense-in-depth so a fabricated or assumed instruction can't
reach an outward action. (Authored by the Brian+Rain trio in a session that
wedged on a `cargo test` turn before it could commit; verified — `cargo test`
+ release build clean, 314/314 — and landed by the maintenance operator.)

- **Anti-confabulation rule baked into `GENERAL_RULES`**
  (`src/agents/general_rules.rs`), not just the deletable
  `custom-general-rules.md`: ground every action in real inputs (actual user
  messages + actual tool results); never publish a claim about what a third
  party said/did without a verbatim in-session source; outward actions under
  the user's identity need a real in-session instruction + an approval gate.
- **PreToolUse `tool-blocklist` hook injected at spawn for HANDS/Emma**
  (`src/agents/spawn.rs` → `run_tool_blocklist` in `src/policy/hooks.rs`). They
  run `--dangerously-skip-permissions`, where claude-code SILENTLY IGNORES a
  JSON `{"decision":"deny"}` result — so the gate blocks via **exit code 2** (a
  "blocking error" honored under bypass), matching the project `tool_blocklist`
  against the Bash command before it runs. Fail-open on parse/IO error.
  Injected via `--settings` (a process arg) so nothing lands in the working
  tree. Rain is exempt — `--bare` skips hooks and she is already read-only.
- **`approval-gate.js` corrected** to whole-command prefix matching
  (`startsWith` on the trimmed command, same semantics as the Rust gate) —
  prior substring/`&&`-split versions over-blocked commands that merely
  *mentioned* a pattern (`echo "git push"`). Note: the JS hook uses
  claude-code's JSON-deny form, a no-op under bypass — the Rust exit-2 hook is
  the real gate for the trio; the JS hook backstops interactive sessions.

+9 tests (314 total: 265 lib + 49 integration).

## 2026-05-29 — Context Library view rework (post-migration UX fixes)

The Tauri v2 migration left the Context Library tab with several regressions vs
the old Slint UI. Brian + Rain triaged all five user-reported issues (plus
extras) and shipped four batches, each holding all five gates (cargo test,
release build, tsc, vitest, vite build):

- `fix: make Context Library files editable with save guards` (`49ff094`) —
  files were read-only (no `cl_write_file`). Added the command (sharing
  `cl_read_file`'s path-traversal guard via a new `resolve_existing_cl_file`
  helper), made the editor a real textarea with dirty tracking + a single
  primary Save, demoted the description editor to a secondary "Metadata" action
  (killing the duplicate-Save-button confusion), and added a `binary` flag so
  non-UTF-8 / truncated files stay read-only (can't be corrupted on save).
  Renamed the sidebar header "WORKSPACE" → "Library Tree".
- `add: Context Library recursive folder tree and folder-view` (`0869fe4`) —
  the tree was a flat per-project file list; rebuilt it as nested collapsible
  folders (`buildTree`). A folder click now toggles collapse AND opens a
  folder-view tab that edits the folder's description/tags
  (`cl_set_folder_description`). `OpenTab` became a file|folder union.
- `add: register and unregister Context Library projects from the UI`
  (`d43ce20`) — a sidebar "Register project" modal promotes an arbitrary
  on-disk folder to a project (`cl_register_project`, path validated as a real
  dir); the project-root folder-view configures the working-repo + soft-
  unregisters (`cl_unregister_project`). `cl_path` added to `ProjectView`.
- `add: Context Library right-click menu and file/folder disk ops` (`b2d1a6c`)
  — VSCode-style right-click: new file, new folder, rename, delete
  (`cl_create_file` / `cl_mkdir` / `cl_rename` / `cl_delete_path`, all path-
  guarded; delete is confirm-gated). Each op runs `cl_rescan` to resync the index.

Net: 15 files, +2076/−134. The Rust side stayed within the existing thin
`#[tauri::command]`-over-storage/bridge pattern. Deferred: native folder picker
(text-input path for now — needs the Tauri dialog plugin), rename re-derives
descriptions, hard delete (no OS trash).

## 2026-05-29 — round 6 refactor sweep (docs + plugin-module organization)

Another maintenance sweep. Brian + Rain ran parallel scans; Brian verified each
finding against the tree before applying. The codebase remains clean after round
5 (no dead code, no Slint staleness, no unused deps), so the round is small —
three commits, all zero-behavior-change:

- `docs: fix stale bridge.rs paths and test count` (`c0f5617`) — ARCHITECTURE.md
  referenced `src/signaling/bridge.rs` at two sites, but Batch 6 split it into
  the `bridge/` submodule tree; PLAN.md's test count (288) lagged the real suite
  (300).
- `refactor: extract InstalledPluginView::from_row constructor` (`267720c`) —
  `install_plugin_inner` and `list_installed_plugins_inner` built the view from
  `(row, manifest, heartbeat)` with the same `status_of(id).unwrap_or(Healthy)`
  resolution; collapse both to a `from_row` constructor. (install previously
  keyed status off `manifest.id`, list off `row.id` — equal, since the row was
  just inserted from that manifest.)
- `refactor: move PluginRegistry to plugins module` (`28de0bf`) — `PluginRegistry`
  has zero Tauri deps (wraps `plugins::Loader` + `plugins::Heartbeat` over plain
  `PathBuf`/`Mutex`). Moved the struct + impl + its three runtime tests from
  `tauri_cmd/plugins.rs` to a new `plugins/registry.rs`, re-exported as
  `crate::plugins::PluginRegistry`, so the command file holds only Tauri shims.

**Deliberately NOT done** (recorded so they aren't re-proposed): R3 — unifying
`jsonrpc::parse_optional_phase`'s error message onto `IpavPhase::error_hint()`
would be an *accuracy regression*: `error_hint()` advertises chip-form
(`I/P/A/V`), but `parse_optional_phase` accepts only lowercase full names, so the
message would tell agents to send inputs the validator rejects. A real unify
(make the two session_doc tools accept chip-form via `parse_phase_arg` +
normalize) is a behavior change beyond a polish round. F1 — moving the
`ContextLibrary*` components from `src/app/` to `src/components/` is
organizational-only with no duplication saved; deferred per the same precedent as
the Round 2 F8/F9/F10 splits. R4 — `arg_clear_on_empty` is not duplicated (single
def in `external_jsonrpc.rs`, used twice locally).

Gotcha worth carrying: removing `PathBuf` from `tauri_cmd/plugins.rs`'s top-level
imports (it lost its last non-test user when the struct moved) cleared an
`unused_import` warning in the non-test build — but the `#[cfg(test)]` helper
`write_plugin_source() -> PathBuf` still needed it. A warnings-only or
non-test-build gate masks this; only a full `cargo test` build surfaces the
`cannot find type PathBuf`. The fix is a test-module-scoped `use std::path::PathBuf;`.

300 Rust + 14 frontend tests green; release build clean.

## 2026-05-29 — round 5 refactor sweep (storage / signaling / tauri cleanups)

A maintenance sweep after the Rain fix. The codebase came back clean (zero
TODO/dead-code, no Slint staleness, docs accurate), so the round is small and
low-risk — three commits:

- `refactor: dedupe SQL column lists in storage queries` — `MESSAGE_COLUMNS` /
  `QUESTION_COLUMNS` / `DOCUMENT_COLUMNS` consts so a projection can't drift
  between the query branches / sibling methods that select the same row shape.
- `refactor: simplify signaling closures and clearable-arg parsing` — drop the
  redundant closures wrapping `internal_err_no_prefix`; extract
  `arg_clear_on_empty` for the base_url/auth_token "empty clears, absent keeps"
  parsing that `set_agent_config` repeated.
- `refactor: use ? for anyhow-sourced internal errors in tauri commands` —
  collapse 15 `.map_err(|e| AppError::Internal(e.to_string()))` sites that wrap
  bot-hq's own anyhow calls to `?` (via the existing `From<anyhow::Error>`).

**Deliberately NOT done** (recorded so they aren't re-proposed): a shared
CL-result-shape helper for `cl_index_search`/`cl_folder_search` (the two map
*different* row structs; a generic helper ≈ the duplication it removes), and
`&*TOOLS`→`&TOOLS` (would revert the deliberate Round-2 explicit-deref choice
in F13). Also scoped OUT of the tauri error sweep: `DbError`/`NotFound`/
`Validation` sites (the frontend `useInvoke` switches on kind for retry /
redirect) and `Internal(format!("ctx: {e}"))` sites over io/reqwest errors
(they add a context message `?` would drop). 300 Rust + 14 frontend tests
green; release + frontend builds clean.

## 2026-05-29 — fix: normalize role:system in Rain's gateway requests (local proxy)

The `--bare` spawn fix (`c0fa928`) for Rain's DeepSeek 400 was **insufficient**:
a fresh `--bare` Rain still 400s on a fixed `messages[11].role: unknown variant
`system``. Evidence: the live `--bare` Rain transcript logged 25 such error
turns this session (prior Rain sessions: 140, 65); every DeepSeek session errors
heavily, every Brian/Emma (real Anthropic) session ≈0.

**Root cause:** claude-code 2.1.156 injects a `SessionStart` hook's
`additionalContext` (and possibly other request-build-time context) as a
`role:"system"` entry inside the request `messages` array. It is NOT stored in
the transcript (so bot-hq can't sanitize it at the source) and `--bare` does
NOT suppress it. DeepSeek's Anthropic-compat gateway rejects `role:"system"`;
the real Anthropic API tolerates it (hence Brian/Emma are fine).

**Fix** (`src/agents/llm_proxy.rs`): a local normalizing reverse proxy. Any
agent with a custom `base_url` gets `ANTHROPIC_BASE_URL` pointed at it
(`http://127.0.0.1:<port>/<hex(real-upstream)>`); per request the proxy hoists
every `role:"system"` message out of `messages[]` into the top-level `system`
field, then forwards to the real upstream (reqwest + rustls + the `stream`
feature) and streams the SSE response straight back. Source-agnostic — it
strips the alien role regardless of which hook injected it. Brian/Emma (no
`base_url` → real Anthropic) never touch the proxy. Started at boot in
`main.rs`; address held in a process-global `OnceLock` and read by
`spawn::build_command` through the pure, unit-tested `resolve_anthropic_base_url`
helper. `--bare` is retained as defense-in-depth + Rain leanness; the misleading
spawn.rs comment claiming it fixes the 400 was corrected.

+11 tests (hex round-trip, base-url resolution, body normalization across
string/array/absent `system` shapes, and an end-to-end test asserting a body
that would 400 on a strict gateway returns 200 through the proxy with
`role:system` stripped and hoisted). 300 Rust tests green; release + frontend
builds clean.

**Live confirmation pending:** the fix only takes effect after bot-hq is rebuilt
+ restarted — the running instance keeps the old binary, so the current Rain
keeps 400ing until then. Rebuilding the binary does not disrupt the running
process (the running image keeps its old inode).

## 2026-05-29 — fix: break agent API-error spam loop (turn-failure signal)

A Rain session resumed on a pre-`--bare` (contaminated) transcript 400s on
*every* turn (DeepSeek rejects the injected `system`-role message). claude-code
emits that "API Error: 400…" as an assistant **text** block, which bot-hq
peer-forwarded to the other agent; the peer replied, that re-triggered the
failing agent, and the volley looped unbounded — burning tokens with zero user
input. (Same family as the idle-volley heartbeat loop fixed in `79114bf`, but
the error text is long + non-ack so the heartbeat breaker didn't catch it.)

**Root cause:** bot-hq discarded the turn-failure signal. claude-code's `result`
event carries `is_error` / `api_error_status`, but `ResultEvent`
(`agents/protocol.rs`) never parsed them — so a failed turn looked identical to
a successful one and its text was peer-forwarded like any prose.

**Fix** (`83c72f7`): parse `is_error` + `api_error_status` on `ResultEvent`;
propagate `is_error` onto `AgentEvent::TurnComplete` (`spawn.rs` + `events.rs`,
derived as `is_error || api_error_status.is_some()` — deliberately *not* from a
non-`success` subtype alone, to avoid false-positive suppression of legit
turns); in `core/duo.rs::pump_agent`, a failed turn drains its buffer WITHOUT
peer-forwarding (the error stays in the agent's own transcript for UI
visibility). +4 tests (1 duo: errored turn not forwarded; 3 events: error/
api_error_status/success derivation). 240 lib tests green (288 total).

**Known limit:** forward-looking — does NOT heal an already-contaminated
transcript. A resumed pre-fix Rain still 400s; restart her for a clean session.
This stops the loop/spam; it does not recover the agent.

## 2026-05-29 — fix: Rain spawns `--bare` (DeepSeek 400 after claude 2.1.156)

After upgrading claude-code to **2.1.156**, Rain (EYES — routed to DeepSeek
via `ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic`) began failing
*every* turn with `API Error: 400 ... messages[1].role: unknown variant
`system``. Brian + Emma (real Anthropic API) were unaffected.

**Root cause:** claude-code ≥ 2.1.156 serializes a `SessionStart` hook's
`additionalContext` — the user's global **superpowers** plugin injects one —
as a `role:"system"` entry *inside* the request's `messages` array. The real
Anthropic API tolerates that; DeepSeek's Anthropic-compatible gateway only
accepts `user`/`assistant` roles and rejects it. Captured + diffed the raw
HTTP body across versions: **2.1.153 → `messages:[user]`** (clean);
**2.1.156 → `messages:[user, system]`** (broken). bot-hq builds none of this
body — it's claude-code reacting to a globally-installed plugin hook.

**Fix** (`src/agents/spawn.rs`): spawn Rain's subprocess with `--bare`
(minimal mode — skips plugin sync, so the offending hook never loads and the
body stays clean). Verified end-to-end against the *real* DeepSeek gateway
with Rain's actual token: identical flags, `--bare` turns the 400 into a clean
reply. `--bare` still honors `--mcp-config` (signaling) and the
`ANTHROPIC_AUTH_TOKEN` bearer header. Scoped to Rain; Brian/Emma keep
CLAUDE.md autodiscovery + LSP. +1 test (`rain_gets_bare_minimal_mode`); 236
lib tests green.

**Known caveat:** `--bare` prevents *new* contamination but does NOT heal
transcripts written before the fix — every existing Rain transcript already
has the superpowers attachment baked in, so **resuming a pre-fix session still
400s**. New sessions are clean. Heal-existing options if needed: start fresh,
sanitize the stored `.jsonl` transcripts, or front DeepSeek with a
system-message-normalizing proxy.

## 2026-05-28 — post-rebuild cleanup (7 batches)

A cleanup pass after the Tauri v2 migration: a tray-delivery bug fix,
doc/CL reconciliation, and four pure refactors (zero behavior change).
Batches 1–5 shipped first; batches 6–7 (the two big module splits) were
deferred to a clean context window and landed last.

- **Batch 1** (`8dd3198`) — fix: route UI `resolve_choice` through core so a
  tray answer arriving after an MCP client-timeout still wakes the agent
  (the `AgentReceiverDroppedFellBack` stdin-injection path).
- **Batch 2** (`28db6d9`) — docs: correct MCP tool counts (26 internal /
  21 external), drop stale Slint references, archive spent CL files.
- **Batch 3** (`99530db`) — refactor: `once_cell` → std `LazyLock`/`OnceLock`.
- **Batch 4** (`0cc5ab8`) — refactor: split `ContextLibrary.tsx` into shell
  + sidebar + editor + shared modules.
- **Batch 5** (`c24a4b2`) — refactor: extract shared `webview_*` JS builders
  into `signaling/webview_js.rs` (+3 tests → 283 baseline).
- **Batch 6** (`8118247`) — refactor: split `signaling/bridge.rs` (1965 LOC)
  into a `bridge/` directory — `mod.rs` (types + struct + constructors +
  session/policy/event-bus methods), `questions.rs`, `permissions.rs`,
  `cl_facade.rs`, `session_docs.rs`, `util.rs`. Each submodule carries its
  own `impl SignalingBridge` block; the `pub use bridge::{…}` re-exports are
  unchanged. Cross-sibling private fns bumped to `pub(super)`; private
  fields stay private (submodules are descendants of the bridge module).
- **Batch 7** (`5d1da96`) — refactor: split `storage/mod.rs` (1197 LOC) into
  per-table submodules (`sessions`, `messages`, `agent_config`, `questions`,
  `projects`, `cl_index`, `session_docs`, `plugins`); `mod.rs` keeps the
  `Storage` struct, `open`/`memory`/`pool`, and the shared `cl_search_table`
  generic. No visibility bumps — every query method is `pub` and
  `cl_search_table` stays a private parent method reachable from descendants.

Batches 6–7 are pure file-splits: 283 Rust tests + 14 frontend Vitest stay
green, release build clean, after each commit.

---

## 2026-05-26 — Tauri v2 migration landed (7 batches)

After the design doc (`docs/plans/2026-05-26-tauri-v2-migration-design.md`,
committed at `7d5d400` + `a9c0abf` on main) and a Plan-phase correction
to the Batch 1 BatchEmitter design (event-triggered batch fetch via the
existing `messages_for_session(since_id)` query, not content-pushing —
the bridge is zero-delta), the migration shipped across 7 batches on
branch `tauri-v2-migration`:

- **Batch 0** (`eba536e` + `83d4ca7` + `3f39ce2`) — Tauri v2 + Vite +
  React 18 + Tailwind + Vitest foundation. `tauri-specta` smoke-tested
  with empty command set; frontend smoke test renders.
- **Batch 1** (`6bc81ee`) — Tauri events layer. `src/tauri_events/`
  with `BatchEmitter` (since_id watermark, N=20 / 50ms coalesce) +
  `bridge_subscriber` routing `SignalingEvent` variants to typed Tauri
  emits. 12 new tests.
- **Batch 2** (`1579eb7`) — Tauri command layer. `src/tauri_cmd/` with
  19 commands across sessions / messages / agent_configs / cl / policy /
  questions / docs domains + `AppError` enum + view types. tauri-specta
  exports to TypeScript with i64 → number bigint behavior.
- **Batch 3** (`30432d4`) — Plugin module scaffolding. `src/plugins/`
  with manifest parser (strict id validation), loader, per-plugin
  capability JSON generator (`https://plugin-<id>.localhost/*`),
  heartbeat watcher (3-strike model). 25 new tests including the
  design-doc coverage-gap (dummy iframe origin chain).
- **Batch 4** (`6aa9f1e`) — main.rs Tauri bootstrap. Slint event loop
  out, `tauri::Builder` in. Tokio multi-thread on workers, Tauri on OS
  main thread. All existing setup (CLI dispatch, panic hook, child
  reaper, signal task, MCP servers, Emma auto-spawn, CL init,
  tauri-specta TS export) preserved verbatim. Bridge subscriber wired in
  Tauri `setup()`.
- **Batch 5** (`84cddb4`) — React frontend. App shell + 5 routes
  (Dashboard, SessionView, Settings, ContextLibrary, PluginManager) +
  Emma overlay. shadcn-style minimal primitives by hand. Zustand stores
  (chat watermark dedupe), TanStack Query hooks (`useTauriQuery`,
  `useTauriMutation`), `useTauriEvent` wrapper. 12 Vitest passing.
- **Batch 6** (`8dbb03d`) — Slint removal. Deleted `src/ui/`, `ui/`,
  dropped `slint` + `slint-build` deps. Updated `ARCHITECTURE.md` +
  `CLAUDE.md` to reflect the new UI. -11,875 LOC across the diff
  (Cargo.lock shed Slint's transitive dep tree).

**Zero-delta verified:** `src/agents/`, `src/core/`, `src/policy/`,
`src/storage/`, `src/signaling/` untouched through every commit. The
Rust core's 202 baseline tests (now 253 with new Tauri layer tests)
stay green at each batch boundary.

**Path A locked** for force-flush on turn-end: design doc's
`SignalingEvent::TurnEnded` variant deferred (would be ~10 LOC core
delta). Accepting ≤50ms tail latency at turn-end as the cost of true
zero-delta. Revisit only if profiling shows perceived lag.

**Push grant:** session-level `scope=specific`, `branches=["tauri-v2-migration"]`
granted at start of Apply phase. Each batch pushed without per-action
prompt; main branch protections unaffected.

**Open items deferred:**

- `broadcast_to_session` Tauri command — `ChatInput` callbacks wired but
  inert until a `core::broadcast` helper lands.
- Live `compute_apply_diff` rendering in the A tab — port
  `view_model::parse_diff_lines` to a Rust-side command + frontend
  renderer.
- Plugin install flow + heartbeat ping/pong frontend channel.
- Real bot-hq app icon (current `icons/icon.png` is a 32×32 placeholder).
- Manual smoke checklist run-through (new-session → agent streams →
  Emma overlay → IPAV tabs → close).
- CL doc updates (`~/.bot-hq/projects/bot-hq/conventions.md` + `notes.md`)
  to drop Slint references — deferred until merge to main since the CL
  is shared across sessions.

**Reference:** Elves (mvmcode.github.io/elves) — Tauri v2 + sqlite + PTY
+ AI agents, validates the architecture in the same domain.

---

## 2026-05-26 — Tauri v2 migration decided (big-bang)

After ~28% of recent commits going to Slint layout fixes and the planned
plugin roadmap (Discord, Clive, themes, future UI-mutation plugins) being
structurally hostile to Slint's compile-time component model, the user +
Brian + Rain brainstormed a migration to Tauri v2 + React. All four
anchors validated through `ask_user_choice` gates:

1. **Migration shape:** Big-bang — branch off main, focused UI-shell
   rebuild, no parallel Slint maintenance.
2. **Frontend stack:** React 18 + TypeScript + Tailwind + shadcn/ui
   (Vite build).
3. **Plugin model:** Slot-extend + custom panels via iframes (per-plugin
   origin via Tauri custom URI scheme + capability JSON). Defer full
   UI-mutation tier.
4. **IPC architecture:** Tauri-native. All React↔Rust via Tauri commands
   + Tauri events. No HTTP from frontend.

**Operating principle locked:** HTTP only where protocol mandates it.
External agent driver server stays HTTP. Internal MCP server (HTTP
localhost) stays — that's claude-code's MCP transport contract.
Everything else is Tauri IPC.

**What's preserved:** Entire Rust core (`src/agents/`, `src/core/`,
`src/policy/`, `src/storage/`, `src/signaling/`, `SignalingBridge`,
session permissions, sqlite schema, all 19+16 MCP tool implementations).
~12,000 LOC zero-delta. The 202 existing tests are the migration's
regression baseline.

**What's getting replaced:** ~6,700 LOC of Slint+view_model
(`ui/app.slint` + `src/ui/view_model.rs`) → ~3,000–5,000 LOC React
frontend + ~500–1,000 LOC thin Tauri command layer + new plugin module.

**Canonical blueprint:** `docs/plans/2026-05-26-tauri-v2-migration-design.md`
(committed `7d5d400` + `a9c0abf`). All five design sections (architecture
/ components / data flow / error handling / testing) user-validated
through structured `ask_user_choice` gates. Rain's 8 review flags all
incorporated as section content or addenda. Session brainstorm artifact
preserved as session doc `brainstorm-tauri-migration` (phase=investigate).

**Status:** Plan-phase output complete. Awaiting fresh-session
implementation handoff (worktree off main + `superpowers:writing-plans`
+ `superpowers:executing-plans`).

**Reference:** Elves (https://mvmcode.github.io/elves/) — Tauri v2 +
sqlite + PTY + AI agents, Homebrew-installable. Validates the exact
domain.

---

## 2026-05-24 — IPAV pills become document tabs (10-batch implementation)

User-requested redesign of the session view: the I/P/A/V pills no longer
advance the IPAV phase (agents do that via the `advance_phase` MCP tool —
two sources of truth was a latent bug). Instead the pills are document-
tab selectors driving a new right-pane DocumentPane in an always-visible
60/40 split (Chat left ~60%, Documents right ~40%). User-decided layout
over Brian+Rain's drawer-toggle recommendation.

**Data model**: `session_documents` gains a nullable `phase` TEXT column
(values `investigate`/`plan`/`apply`/`verify`) via `migrations/0008_
session_documents_phase.sql`. Existing rows pass through as NULL —
invisible to tabs + phase-filtered searches. The `session_doc_write` and
`session_doc_search` MCP tool descriptors gain optional `phase` enum
params + dispatch-layer validation. Agents tag plans/findings/etc. and
retrieve cross-phase context via `session_doc_search(phase="plan")`
instead of scrolling chat history. Hardcoded agent prompts updated in
`prompts.rs:72` + `general_rules.rs:63,83` so the pattern is discoverable.

**Apply tab — git diff path**: the in-memory `SessionHandle.session_
start_sha` (new field) captures `git rev-parse HEAD` via `spawn_blocking`
at session spawn. The view's `compute_apply_diff` runs `git diff --no-
color <sha>` (one-arg form covers committed + staged + unstaged in one
shot — `git diff HEAD` alone is empty right after commits land, which
is the moment the user wants to inspect what just shipped). Fallback
chain: SHA-diff → `git diff HEAD` with anchor-lost note → latest
`phase='apply'` session doc → empty state. No schema column for the SHA;
in-memory is enough since live session state already resets on app
restart.

**Slint changes**: `AppState.advance-phase` callback + the `on_advance_
phase` handler in `view_model.rs` fully stripped (Liars That Compile —
leaving dead callbacks invites future re-wiring that reintroduces the
bug). New `select-doc-tab` callback + `selected-doc-tab` property +
five `active-doc-*` properties (content/slug/updated-at/count/empty-msg).
PhasePill rewritten: top-border accent on selected tab (keeps per-phase
`tint` color), monochrome text. SessionView outer `VerticalLayout` now
wraps the chat + DocumentPane in a `HorizontalLayout` with `horizontal-
stretch: 1.5` / `1`. PhaseSelector relocated from session header to the
DocumentPane header. LabelChip remains the sole phase indicator.

**View-model wiring**: new `refresh_session_docs` async helper (called
both from the 500ms poll loop and the immediate tab-click handler);
new `compute_apply_diff` helper; new `current_selected_doc_tab_async`
+ `push_doc_pane_state` utility. "N more" chip surfaces in the
DocumentPane header when the active tab has >1 phase-tagged doc;
expansion UI deferred per YAGNI.

**Verification**: `cargo build` clean (dev + release), 202 tests pass
(was 196 → +6 from 2 storage phase tests + 3 MCP phase tests + 1 round-
trip). 11 files modified, 1 new migration. Diff stat: +714 / -113.
Visual smoke (60/40 split renders, tabs switch, doc loads from session_
documents, git diff appears in A tab after agent commits) is the user's
gate — bot-hq's desktop nature precludes automated UI testing.

---

## 2026-05-22 — Audit Round 4 cleanup (11 findings landed)

Brian + Rain adversarial sweep of the post-Round-2/3 codebase using
`~/.bot-hq/projects/slint-rust-docs/` Tier 1/2/3 as the Rust+Slint
reference. Two independent passes (session docs `findings-fresh-sweep-2026-05-22`
+ `findings-rain-sweep-2026-05-22`); 11 findings consolidated; all
shipped per the verified commit order.

**Landed (in order):**

- **C1 — `c54a8ea` (N7, real bug)** — main.rs's shutdown-signal
  `tokio::select!` had no `else` arm. If all three `signal()`
  registrations failed (non-Unix host, container without signal
  support) the select panics ("all branches disabled"). Added a
  `future::pending()` arm that parks the task — children still get
  reaped via the panic-hook path.
- **C2 — `99ffd62` (N8, lint)** — `panic_payload_string(&Box<dyn Any>)`
  → `&(dyn Any)`. clippy::borrowed_box. 2 lines.
- **C3 — `1c3a103` (N10, doc)** — PLAN.md said "165 tests passing";
  actual is 196.
- **C4 — `57d80e7` (N2, dispatch helper)** — `IpavPhase::parse + same
  error_hint format!` was triplicated across jsonrpc.rs:155, :174,
  external_jsonrpc.rs:387 — each with the same shape but different
  wire field names ("target" vs "phase"). Extracted
  `protocol::parse_phase_arg(field, value)` preserving the
  wire-compatible error string via the `field` param. Dropped the
  now-unused `IpavPhase` import in external_jsonrpc.rs. Net -3 LOC.
- **C5 — `303db61` (N1, response helper)** — `result_json(&json!({"ok":true}), "{}")`
  was repeated 6× in external_jsonrpc.rs as the standard "operation
  succeeded" payload. Extracted `ok_response()` next to `result_json`
  in response.rs.
- **C6 — `57fbd6d` (N3, error helper)** — F6 added file-private
  `internal_err(op, e)` to external_jsonrpc.rs but jsonrpc.rs still
  had 15× `map_err(|e| JsonRpcError::new(INTERNAL_ERROR, e.to_string()))`
  (no-op-prefix shape). Lifted `internal_err` into response.rs; added
  `internal_err_no_prefix` sibling; replaced all 15 sites with
  `.map_err(internal_err_no_prefix)?`.
- **C7 — `b9e1cb0` (N6, consistency)** — `on_set_session_permission`
  inlined a 4-line `weak.upgrade().map(...)` block instead of calling
  the existing `current_session_id(&weak)` helper (used by
  `on_advance_phase` + `on_broadcast`). Dropped the inline form.
- **C8 — `40f7868` (N5, bridge dedupe)** — `resolve_policy_for` and
  `audit_policy_files_for_session` had identical 12-line
  project→project_root resolution chains. Extracted private
  `resolve_project_and_root(data_dir, sid)` returning
  `(Option<String>, Option<PathBuf>)`. Both callers collapse to one
  line.
- **C9 — `0172ba4` (N4, bridge dedupe)** — `grant_session_permission`,
  `revoke_session_permission`, and `add_branch_to_session_grant` all
  replicated the same lock→entry-or-default→mutate→snapshot→drop→mirror
  sequence (~14 lines each). Extracted `mutate_session_permission(sid, FnOnce)`;
  each caller reduces to its one-line mutation closure. Side-effect:
  the mirror-write side can't be forgotten in a future variant.
- **C10 — `bea60bd` (R4-F1, real failure-mode fix)** — `catch_unwind`
  (ffi_safe) prevents Slint-callback panics from aborting, but does
  NOT clear a poisoned `Mutex`. Before C10: first panic inside e.g.
  `ANSWER_ACCUMULATOR.lock()` poisoned the mutex; every subsequent
  tray interaction re-locked → `.unwrap()` → panic → caught → toast-
  spam until session restart. Replaced `.lock().unwrap()` /
  `.lock().expect()` with `.lock().unwrap_or_else(|p| p.into_inner())`
  at all 8 sites (5× view_model.rs, 3× spawn.rs). This is the
  hardening pass logged in decisions.md (2026-05-22) as deferred.
- **C11 — `cd3a6b8` (R4-F2, paths dedupe)** — `directories::BaseDirs::new().context("locating user home dir")?.home_dir().to_path_buf()`
  was repeated 3× in paths.rs. Extracted `home_dir() -> Result<PathBuf>`;
  all three sites reduce to one line. Net -2 LOC.

**Wire compatibility:** every refactor preserved exact existing wire
error strings and tool-call result shapes. C1 + C10 are the only
commits with behavior changes (both eliminating failure modes that
previously aborted or toast-spammed the daemon).

**Round 4 metrics:** 11 commits, 12 files touched, ~41 duplication
sites collapsed into 6 new shared helpers (`parse_phase_arg`,
`ok_response`, `internal_err_no_prefix`, `resolve_project_and_root`,
`mutate_session_permission`, `home_dir`). Net +12 LOC because each
helper carries a load-bearing docstring; the raw repetition is gone.
196 tests still pass.

**Deferred from Round 2/3 still deferred:** view_model.rs (3,038 LOC),
bridge.rs (1,841 LOC), storage/mod.rs (960 LOC), app.slint (3,846 LOC)
splits — all organizational. Re-open when actively painful.

---

## 2026-05-22 — Audit Round 3 cleanup (S4, S1, S5 landed)

Acted on `findings-slint-rust-audit` (session doc) — Brian + Rain
adversarial audit of the codebase against
`~/.bot-hq/projects/slint-rust-docs/` Tier 1/2 reference docs.
Five findings produced; three actionable (S4 LOW, S1 HIGH, S5 LOW)
shipped; two (S2, S3) deferred organizationally per the same precedent
that deferred Round 2's F8/F9.

**Landed:**

- **S4 — `41ef278`** — `build.rs` was using Slint's default
  `std-widgets` style, which is platform-dependent (fluent on Windows,
  qt on Linux, native on macOS), so widget chrome (LineEdit /
  ScrollView / TextEdit focus rings, scrollbar handles, input borders)
  drifted across builds even though the rest of the app paints from
  the Theme global. Switched to `compile_with_config(..., with_style(
  "material"))` to match the app.slint header's stated "Material 3
  dark theme".
- **S1 — `a88bc0a`** — `view_model.rs` used the LLM anti-pattern
  `slint::invoke_from_event_loop(move || { if let Some(handle) =
  weak.upgrade() {...} })` at 38 call sites — exactly what
  `slint-rust-docs/patterns/weak-handle.md` calls out as duplicating
  what `Weak::upgrade_in_event_loop` packages. Migrated all 38 sites
  to the canonical primitive (closure receives the upgraded handle,
  silently skips if the component dropped). Two edge cases handled per
  the audit spec: TreeState init moved inside the new closure body;
  `current_session_id_async`'s oneshot dropped the explicit empty-send
  branch (the receiver's `rx.await.unwrap_or_default()` covers tx-drop
  identically). Also updated 4 doc/inline comments naming the old
  primitive. Net -126 LOC (view_model.rs: 2966 → 2840).
- **S5 — `bfbde16`** — `AppState` global in `ui/app.slint` defaulted
  `in-out property` for one-way Rust-pushed values. Per
  `slint-rust-docs/conventions/slint-syntax-for-rust.md` + 
  `patterns/globals.md`, `in-out` should be justified, not the default.
  Verified each property by grepping for `.slint`-side writes
  (`AppState.foo = ...`, `<=>` two-way binds). Converted 33 to
  `in property`; kept 27 as `in-out` where TextEdit/LineEdit `<=>`
  binds, UI click-toggles, modal state, or drag-resize legitimately
  write from the `.slint` side. Three audit-table corrections caught
  during the grep pass — `cl-dirty`, `cl-metadata-dirty`, and
  `external-mcp-token-revealed` ARE UI-written (initial table was
  wrong) and stayed `in-out`.

**Deferred:**

- **S2 — persistent `Rc<VecModel<ChatMsg>>` with incremental
  mutation.** Reference pattern in `slint-rust-docs/patterns/models.md`.
  Current behavior uses fresh `ModelRc::new(VecModel::from(rows))` per
  poll with a `MSG_FINGERPRINTS` cache to short-circuit identical
  refreshes. The fingerprint workaround is load-bearing and correct;
  the canonical pattern is perf+correctness polish, not a fix. Re-open
  if rebuild churn surfaces in profiling or selection-loss appears as
  a UX complaint.
- **S3 — split `ui/app.slint`** (3846 LOC) into conventional
  `ui/{theme,types,components/,views/,main}.slint` layout per
  `slint-rust-docs/conventions/project-structure.md`. Organizational,
  not correctness — same conclusion Round 2 reached for F8/F9. Re-open
  if the mono-file becomes painful to edit / merge.

**What was already correct (no change needed):** Tokio/Slint event-loop
boundary (multi-thread Tokio + Slint on main thread, matches "Fix (a)"
in `pitfalls/tokio-event-loop-conflict.md`), zero `clone_strong()`
usage, correct weak-handle capture in all callbacks, correct
`export component AppWindow` shape, no `set_X(format!()).into()`
allocation anti-patterns on hot paths.

---

## 2026-05-21 — Audit Round 2 cleanup (F12, F2, F1, F5, F11, F6, F13, F4 landed)

Acted on `~/.bot-hq/projects/bot-hq/investigations/audit-round-2-2026-05-21.md`
— the Brian+Rain adversarial codebase audit produced earlier in the
session. Seven findings shipped, one remains queued.

**Landed:**

- **F12 — `05249b8`** — `request_phase_advance` used a hardcoded
  `matches!()` against full names, rejecting chip-form targets while
  `advance_phase` accepted both via `IpavPhase::parse`. Real behavioral
  bug — `request_phase_advance(target="I")` returned INVALID_PARAMS.
  Same SSOT issue in `view_model.rs:250-255` (manual chip-to-phase
  reimplementation). Added `IpavPhase::error_hint()` so internal +
  external MCP dispatch quote the canonical
  `"I/P/A/V or Investigate/Plan/Apply/Verify"` string instead of three
  divergent ones. Two regression tests lock in chip-form acceptance.
- **F2 — `ac4db22`** — `PROTOCOL_VERSION` was duplicated in
  `external_jsonrpc.rs:21` alongside the public const in
  `protocol.rs:11`. Silent-desync risk on MCP version bumps. Deleted
  the local copy; imported the public const.
- **F1 — `39efd51`** — `result_json()` helper from `jsonrpc.rs:108`
  was never propagated to `external_jsonrpc.rs`, which inlined the same
  `serde_json::to_string(...).unwrap_or_default()` shape at 16 call
  sites. Lifted the helper into `signaling/response.rs` as
  `pub(super)`; replaced all 16 sites. Net -26 LOC. Intentional
  behavior diff: serialize failures now return `"{}"` instead of
  `""` — valid JSON shape, matches the existing internal pattern.
- **F5 — `5e46844`** — `Message → json!({...})` projection was
  copy-pasted 4× across `external_jsonrpc.rs`
  (`get_session_messages`, `get_emma_messages`, `wait_for_change`,
  `get_session_snapshot`). Extracted file-private
  `message_to_json(&Message) -> Value` near the top; all 4 sites
  collapsed to `.iter().map(message_to_json).collect()`. Switched
  `.into_iter()` → `.iter()` per-site after verifying none reuse the
  source vec. Internal `jsonrpc.rs` has zero matching sites — F5 is
  external-only. Net -22 LOC. Same 5-field shape preserved;
  `session_id` stays dropped (DB-only, not MCP view).
- **F11 — `6a423c9`** — `SignalingBridge` had 3 constructors
  (`new` / `with_violations_log` / `with_policy`) each copy-pasting
  the same 9-field `Arc::new(Self {...})` struct literal, differing
  only in `violations: Option<ViolationsLog>` and
  `data_dir: Option<PathBuf>`. Added private
  `new_with(Option, Option)` containing the single struct-literal
  build; collapsed the 3 public fns to thin wrappers. Zero call-site
  changes across the ~41 callers (1 prod in `main.rs:59`, ~40 in
  tests). Doc comments preserved on the public wrappers. Net -13 LOC.
- **F6 — `8ef5203`** — `JsonRpcError::new(INTERNAL_ERROR,
  format!("op: {e}"))` was repeated 16× across `external_jsonrpc.rs`
  (audit counted 8 single-line sites; rediscovered 8 more in 4-line
  rustfmt-wrapped form at deeper nesting). Added file-private
  `internal_err(op: &str, e: impl Display) -> JsonRpcError`. Each
  multi-line site collapses 4 lines → 1; single-line sites get
  shorter. Internal `jsonrpc.rs` uses a different shape
  (`e.to_string()`, no op prefix) — helper stays external-only. One
  static-message site (line 558, "violations log not configured...")
  left untouched as it doesn't fit the helper signature. Net -20 LOC.
- **F13 — `136e924`** — `tool_descriptors()` (19 internal tools,
  `protocol.rs`) and `external_tool_descriptors()` (16 external
  tools, `external_jsonrpc.rs`) rebuilt their full
  `Vec<ToolDescriptor>` — including all the `serde_json::json!`
  schema trees — on every MCP `tools/list` handshake. Wrapped each
  in `static LazyLock<Vec<ToolDescriptor>>`, returning
  `&'static [ToolDescriptor]`. Three caller sites updated (drop
  `: Vec<_>` annotation; slice serializes through `json!` the same
  as the owned Vec). Rain caught that `&TOOLS` would lean on a
  multi-step `Deref` coercion — switched to explicit `&*TOOLS`. Net
  +4 LOC; perf win is one alloc per process instead of per call.
- **F4 — `fb2deb0` (tests) + `fab33e9` (extract)** — both HTTP
  handlers (`signaling/server.rs::handle_request` and
  `external_server.rs::handle_request`) had identical body-collect →
  serde_json::from_slice → PARSE_ERROR-envelope blocks and identical
  dispatch-outcome match arms (~30 LOC each, copy-paste-divergent
  waiting to happen). Rain's gate required external HTTP smoke
  coverage of the paths first — `tests/external_mcp_test.rs` already
  exercised the full HTTP stack but neither parse-error nor
  202-ACCEPTED were covered explicitly. First commit (`fb2deb0`)
  added 4 tests pinning those contracts on both servers; second
  commit (`fab33e9`) extracted `decode_jsonrpc_body(Incoming) ->
  Result<JsonRpcRequest, Response>` and `dispatch_outcome_to_response
  (outcome, id_for_err) -> Response` into `signaling/response.rs`.
  Per-server pre-dispatch logic (path parse for internal; method +
  path + bearer auth for external) and debug log lines stay in the
  callers since they carry caller-specific fields. Net: each handler
  drops ~28 LOC; response.rs gains ~50; -6 LOC overall, but the more
  meaningful win is removing the last RPC-handling drift surface
  between the two servers.

**Rejected (recorded for future re-evaluation):** F3 (generic
`dispatch_jsonrpc<F>` extraction — async closure overhead exceeds
savings), F10 (per-table storage split — import sprawl without
discoverability gain). See the audit file for re-open triggers.

**Audit round 2 complete.** Last F-series code commit `fab33e9` (F4).
F8 / F9 (view_model.rs / bridge.rs splits) remain deferred — both are
organizational preference rather than duplication; defer until either
file is actively painful or the user requests the split. Audit file
`investigations/audit-round-2-2026-05-21.md` archived as the
source-of-truth for the round.

---

## 2026-05-20 — Session permission grants (in flight)

New module `src/policy/session_permissions.rs` plus integration across
the bridge, the duo, the spawn path, and the `pre-push` git hook.

**What changed:**
- `SessionPermissions { commit: GrantScope, push: GrantScope }` with
  `None` / `AllBranches` / `Specific { branches }` scopes.
- In-memory cache on `SignalingBridge` is the source of truth; mirrored
  to `<data_dir>/.local/session-permissions/<session_id>.json` so the
  `pre-push` git hook (separate subprocess) can read it.
- All mirror files purged on bot-hq startup; per-session file deleted
  on `close_session`.
- MCP tools added: `grant_session_permission(action, scope, branches?)`,
  `revoke_session_permission(action)`, `list_session_permissions()`.
  HANDS-only — Rain (EYES) cannot call them.
- `pre-push` hook checks the mirror before the static
  `policy.push_gate.remembered_approvals` list.

**Documentation cross-refs:** ARCHITECTURE.md → Session permissions
section; README.md → Internal MCP tools + Policy enforcement.

---

## 2026-05-19..05-20 — Doc refresh

Full rewrite of canonical docs (README, ARCHITECTURE, PLAN, PROGRESS,
CLAUDE) to reflect the post-rebuild state. Original rebuild design +
roadmap + Phase 0 research archived under `docs/rebuild-archive/`.

---

## 2026-05-15 — UI redesign

Substantive frontend pass triggered by user feedback ("the UI is really
bad").

- **Single chronological chat.** Replaced the two-pane Brian/Rain split
  with one chronological column where all messages interleave by
  `created_at`. User can now see their own messages clearly.
- **Design system.** Slint `Theme` global owns colors, typography,
  spacing, radii. 4-tier background hierarchy
  (canvas → surface → elevated → overlay), 4-step font scale, 4px-base
  spacing scale. Author color coding: brian=orange, rain=purple,
  emma=green, user=blue, system=muted grey.
- **Per-surface polish.** Topbar gains brand mark + tab underline +
  Emma button distinct treatment. Dashboard title block + primary
  `+ New session` CTA + elevated session tiles with `Need input` badge
  tinting border red. Session view: rich header (title + phase subtitle
  + back link + interactive PhaseSelector segmented control); banner
  uses author-rain purple (choice) vs attention red (awaiting). Emma
  overlay: dedicated header bar + close affordance + divider.
- **CL refresh.** New files in CL appear without app restart via a 2s
  periodic poll plus a manual ↻ refresh button. Directories sort before
  files in the tree.

Files touched: `ui/app.slint` (full rewrite: 796 → 1410 lines),
`src/ui/view_model.rs` (714 → 743 lines). Tests still 56-passing at the
time, release build clean.

---

## 2026-05-15 — Post-review fixes

Follow-ups after the autonomous rebuild's READY-FOR-REVIEW state.

1. Added `/target/` to `.gitignore` (was 6.7 GB; `git add .` without
   this would have committed all build artifacts).
2. **Emma auto-spawn at startup.** Extracted `spawn_session_handle`
   helper in `src/core/session.rs`; added `spawn_existing_session` for
   sessions whose row already exists. `AppState::ensure_session_started`
   is idempotent and called for `"emma"` in `main.rs` post-core
   construction. Failure is non-fatal.
3. **Settings save persists user edits.** Replaced inline LineEdit-in-
   for-loop pattern with `AgentConfigEditor` component owning per-row
   edit state via `in-out` properties bound via `text <=>`.
4. **Per-project rules migration from legacy CL.** Distilled
   operational rules from `~/.bot-hq/projects/<project>.yaml` into the
   new minimal CL. Per-project policy gates + disguise rules captured.

---

## 2026-05-14 — Rebuild milestone (v0.1.0)

From-scratch rebuild of bot-hq landed: single Rust + Slint binary
replacing the Go daemon + tmux + MCP hub + Emma forwarder + 29-tool
surface. Built autonomously across multiple claude-code sessions per
the original rebuild plan.

**Result:** 56 tests passing, release build clean, binary launches and
runs the UI loop cleanly, full agent lifecycle implemented (subprocess
spawn, stream-json IO, sqlite storage, internal MCP server with 2
initial tools, IPAV duo coordination, Slint UI with topbar +
dashboard + session view + Emma overlay).

**Phase A complete:** rebuilt minimal CL distilled into `~/.bot-hq-dev/`
(60K, 398 lines across general-rules + 3 agent startups + 5 projects of
conventions + notes). Replaces the 860-file legacy CL.

For full phase-by-phase progress + Phase 0 research findings + sub-
agent dispatch log + initial decisions, see
[`docs/rebuild-archive/PROGRESS-through-2026-05-15.md`](docs/rebuild-archive/PROGRESS-through-2026-05-15.md).

---

## Decisions made autonomously (across the build)

Things that diverged from the original PLAN / decision-doc and shipped
that way. Captured for future reference.

1. **MCP transport: in-process HTTP, not stdio + UDS bridge.** Original
   design sketched claude-code spawning bot-hq as an MCP child process
   and bridging back via Unix-domain socket. That's two subprocesses
   per agent + ~150 LOC of IPC framing. Ship version runs a single
   in-process HTTP MCP server, per-agent `mcp-config.json` files
   pointing at unique URLs. Direct AppState access, no IPC layer.
2. **MCP server: hand-rolled JSON-RPC, not `rmcp` crate.** Phase 0
   research recommended `rmcp` 1.7.0; orchestrator chose hand-roll
   (~300 LOC at `src/signaling/{jsonrpc,server,protocol}.rs`) for
   simpler in-process transport. Drop-in `rmcp` upgrade later is
   straightforward.
3. **`claude --append-system-prompt` is a string, not a file.** Plan
   said `--append-system-prompt-file`; CLI only accepts inline
   `--append-system-prompt <prompt>`. Concatenated text passed inline.
4. **`--verbose` required with `-p --output-format stream-json`.**
   Empirically discovered. Spawn command includes it.
5. **`--dangerously-skip-permissions` set on agent spawn.** bot-hq IS
   the policy layer; claude-code's own permission prompts would
   double-gate and hang. Enforcement provided by `src/policy/` + git
   hooks.
6. **Role prompts hardcoded in `src/agents/prompts.rs`.** Not CL-loaded.
   Reasoning: role boundary (Brian writes, Rain reviews) is structural
   and must survive CL edits + custom-instruction changes.
7. **System-prompt layering with policy block at the end.** Session
   spawn concatenates: hardcoded role → CL anchor → general-rules →
   custom-instruction → policy directives. Project conventions/notes
   NOT injected — agents use `cl_index_search` + `Read` on-demand.
8. **`HANDS_ONLY_TOOLS` enforced at the JSON-RPC dispatch layer.** Rain
   is structurally blocked from `ask_user_choice`, `mark_awaiting_user`,
   `request_approval`, `grant_session_permission`,
   `revoke_session_permission`. Returns a JSON-RPC error, not a
   convention.
9. **Two-layer policy enforcement.** MCP tool calls (probabilistic
   primary path, audited via `violations.jsonl`) PLUS git hooks
   (deterministic backstop). Per DeepSeek-V4-Pro's review during the
   policy module work — single-layer enforcement would fail when
   agents' context drifted.
10. **`BOT_HQ_SESSION_ID` env var injected into agent subprocesses** so
    git-hook subprocesses (spawned by git, separate from the agent's
    subprocess) can re-resolve session-scoped state (session
    permissions in particular).
11. **External MCP token at `<data_dir>/mcp-token`** auto-generated
    (UUIDv4, 0600). Constant-time comparison via `subtle` crate. Read
    once at startup, never re-read — rotation requires restart.
12. **Slint pin: `slint = "1.16"`** (resolves to 1.16.1). MSRV 1.92 per
    Phase 0.3 research.
13. **Sessions/agent_configs tables have CHECK constraints on
    `agent_name` ∈ `{'emma','brian','rain'}`** so a typo from Settings
    UI doesn't silently create a bogus row.
14. **First-run detection key: `cl-version.txt` existence** — not data-
    dir existence (test setup creates the data-dir before binary touches
    it).

---

## How to verify the human-driven parts

```bash
cd ~/Projects/bot-hq
cp .env.example .env             # already contains BOT_HQ_DATA_DIR=~/.bot-hq-dev/
cargo run --release

# In the window:
#   - Click "+ New session" on the Dashboard.
#   - Type a small task in the broadcast prompt bar.
#   - Watch Brian + Rain stream. Click the I/P/A/V chips to advance phase.
#   - If an agent calls ask_user_choice, choice buttons should appear inline.
#   - Toggle the Emma button (top-right) — half-pane chat slides in.

# Richer logs:
#   RUST_LOG=trace cargo run --release
```
