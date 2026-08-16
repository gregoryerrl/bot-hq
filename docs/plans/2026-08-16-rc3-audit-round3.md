# rc3 audit — round 3 (2026-08-16)

Two mandates in one session: finish **R6**, the piece round 2 measured and
deliberately left, and run a **third audit pass** — with a angle neither earlier
round had. This binary was built from round 2's fixes, so round 2 could be
checked against a *running system* rather than re-read.

Baseline `710b8be`, clean tree. Every number below came from a command run this
session.

> **The table below is the R6 + audit half only** (`710b8be` → `ffd33bf`). The
> D10 retirement that follows in the tail moved these again — final state there.

| gate | at `710b8be` | after R6 + audit |
|---|---|---|
| `cargo test` | 1174 lib + 60 integration + 2 doc = **1236** | **1236** — green; F7 removed one test, F12 added one |
| `npx vitest run` | 392 in 45 files | unchanged |
| `npx tsc --noEmit` | clean | clean |
| clippy (`^src/.*: warning:`) | **10** | **4** |
| `run_sequencer` | **714 lines** | **407** |
| `advance_turn` | **11 parameters** | **4** |

---

## 1. Live evidence — round 2, observed rather than re-read

The most useful thing available to a third round was that the second round's
fixes were *running*.

### R5 (migration 0059) — CONFIRMED LIVE

It applied at this app's boot. `select max(version) from _sqlx_migrations` → **59**,
row `59|rfc3339 stamp backfill|1`.

| column | round 2 measured | round 3 measured |
|---|---|---|
| `participant_deliveries.delivered_at` | 4011 zone-less / 14 RFC3339 | **4370 RFC3339, 0 zone-less** |
| `participant_cursors.updated_at` | 85 zone-less / 5 RFC3339 | **92 RFC3339, 0 zone-less** |

Both populations are clean **and both have grown** since round 2 measured them
(4025 → 4370, 90 → 92). That growth is the part a fixture cannot give you: every
row written by live traffic in between is well-formed, so the writers are
confirmed against production, not against a test.

### B3 (roster cap) — CONFIRMED

`MAX_SESSION_PARTICIPANTS = 8` now sits in `storage/participants.rs:126`, beside
the roster invariant it protects rather than in the command layer, and all three
creation paths are bounded: the dialog refuses (`tauri_cmd/sessions.rs:189`), the
seeder clamps (`participants.rs:1010`), the plugin wire clamps
(`plugin_api.rs:137`). The dialog/backend split is stated honestly in two places
(`Dashboard.tsx:31`, `ARCHITECTURE.md:45`) rather than being a silent divergence.

### E3 / task 14 — CONFIRMED, but it left claims behind

`src/core/` holds `pump.rs` and `sequencer.rs`; `router.rs` and `duo.rs` are
gone, and `.env:7` records `BOT_HQ_SEQUENCER` as no longer read. One turn engine.
**The deletion's residue is this round's largest finding — see F1.**

---

## 2. Findings

### F1 · A doc claimed a monitoring capability that does not exist · MEASURED · FIXED `b5510b4`

`core/watchdog.rs` — `run_stall_watchdog`'s doc said it

> "Also watches the peer-forward router (`router`): a dead router while agents
> are live is an anomaly (forwarding is down) — warn + emit a router-health event
> once."

`core/router.rs` was deleted by task 14. Grepping `src/` for
`router_health|RouterHealth|router-health` returns **exactly one hit — that
sentence.** The function takes no router handle, has no such branch and emits no
such event.

This is a step worse than the vocabulary rot the earlier rounds swept. It is a
**phantom capability in the file that documents the session's monitoring**: a
reader asking "is forwarding watched?" reads this and concludes yes. Nothing
watches forwarding, and with one turn engine there is no separate forwarder left
to watch.

Fixed by replacing the claim with what the loop does, **and by recording what the
sentence was**. Deleting it silently would leave the next reader to rediscover
the same question with no trail.

### F2 · A coverage claim pointing at a deleted module · MEASURED · FIXED `b5510b4`

`core/pump.rs` — a test helper's doc said the forward/suppress/break decision
"is tested in `core::router`", directing a reader to coverage that cannot exist
in a module that does not. Round 2's own method note was "a test can pin a
falsehood"; this is its inverse — a doc asserting coverage that is absent.

### F3 · G4, open since round 2 — a duplicate parser beside its own import · MEASURED · FIXED `b5510b4`

`bridge/tray.rs` `gate_age_secs` hand-rolled the RFC3339-then-`%Y-%m-%d %H:%M:%S`
two-branch parse that `bridge/util.rs:279` `parse_tray_ts` already provides —
from the top of the same module, which `tray.rs:9` **already imports**. Identical
branches, identical fallback, identical `None`. Eight lines became one call.

Worth stating in the fix rather than deleting quietly: the duplicate is what
makes a *tolerance* change land in one of two places, and the tolerance is the
entire reason the pair exists.

### F4 · The round-2 report's own status headers were stale · MEASURED · FIXED

**B3** and **G3** were still headed `· OPEN` while their bodies and `PROGRESS.md`
both recorded them shipped (`0059b0b` + `a1aee95`; `5e43eaa`). A round that reads
headers picks up finished work.

Corrected **beside, not rewritten** — the round-2 rule that evidence is amended
next to itself. The traces stay as filed; they describe the tree they were filed
against.

### F5 · R6's design undercounted the ring state by two · MEASURED · FIXED

The recorded design listed **11** `let mut` ring-state locals in `run_sequencer`.
There are **13**: it omits `gate_seed_failed` and `open_gates`.

Those two are exactly the pair that most needed the struct. A `HashSet` cannot
express "I could not read the gates", so the flag carries that third state — and
the source **documents both in one comment paragraph while declaring them as two
separate locals**, with the set's paragraph sitting above the flag's declaration.
A `RingState` built to the recorded count would have left behind the two fields
whose relationship the type existed to hold.

Caught before step 1 was taken, which is the only reason it cost nothing.

### F6 · Seventeen functions exceed the seven-argument threshold · MEASURED · PARTLY FIXED

13 `#[allow(clippy::too_many_arguments)]` in `src/` plus 4 live warnings at
baseline. R6 removed one (`advance_turn`, 11 → 4). The remaining three warn:
`cl_write.rs:22` (8/7), `storage/plugins.rs:21` and `:84` (9/7).

The point is systemic rather than per-function: **a long parameter list is this
codebase's standing signal that a struct is missing**, and R6 is the worked
example of what it costs to answer one late. The clippy count is the only check
in the repo that tracks it.

### F7 · Two write paths with no readers · MEASURED · FIXED

`core/session.rs:758` states it outright — "Those columns are left in place and
UNREAD." Spawn reads the roster: `SpawnConfig` takes `session_effort: p.effort`
and `session_ultracode: p.ultracode` off the participant row.

Yet two storage methods existed only to write those columns:

- **`set_session_effort_config`** — four columns (`brian/rain_effort`,
  `brian/rain_ultracode`), one caller, nothing reading them. **Removed.**
- **`set_session_spawn_config`'s model half** — `brian_model_id` /
  `rain_model_id`, likewise unread. `ensure_session_roster` seeds model ids from
  `roles.default_model_id`, never from these (its own test says so at
  `participants.rs:4665`). **Narrowed to the one column that IS read**,
  `rain_enabled`, the solo/duo bookkeeping flag.

**The create path already said so.** Twenty lines under the effort write sits the
comment *"Spawn reads them off the participant rows now, so writing them only to
`sessions` above would be a picker that changes nothing."* The code documented
its own redundancy and the write stayed.

And the comment justifying the other write — *"respawn_session reads them off the
row"* — names a reader that does not read them: `respawn_session` is four lines
and calls `ensure_session_started`. Same class as F1, one file away from a live
write path.

**The columns themselves stay.** Dropping them is SQLite's twelve-step table
rebuild; retiring a writer with no reader is not, and the two decisions are
separable.

### F8 · The one column default proven to fire is still installed · MEASURED · NOT FIXED, deliberately

`participant_cursors.updated_at TEXT NOT NULL DEFAULT (datetime('now'))` is still
in the live schema. R5 bound both INSERT sites and backfilled the 85 bad rows,
which fixes the data and the writers — but **round 2's stated reason for leaving
the other 15 defaults was that they have never fired, and this is the one that
did.** A new INSERT site that omits the bind re-contaminates silently, and the
guard only inspects a row it just wrote.

Not fixed here because dropping it needs the twelve-step rebuild, and shipping a
schema change in the middle of the most delicate refactor in the repo is bad
sequencing. Recorded with its cost so the next round starts from a decision
rather than a rediscovery — the same treatment R6 got from round 2, which is
what made R6 cheap to finish.

**One table, 92 rows.** That is a different calculus from round 2's "rebuild 13
tables", and the difference is the whole argument.

### F9 · CL `notes.md` named deleted code in the present tense — and carried one wrong STATUS · MEASURED · FIXED in the CL

`cl_stale_refs(bot-hq)` reports 27 claims naming code that is gone. **The report
is not a work order and most of it should not be actioned**: the connector
learnings already carry OBSOLETE banners, `decisions.md` / `issues.md` are
append-only by design, and several hits are correct by construction — the
`resolved_at` line exists *to say* the column is called `answered_at`, and
`SpectaFn` / `ExitWorktree` name a dependency internal and a harness tool, neither
of which is bot-hq source. Each was checked against the repo before anything was
touched.

Three were real, and one of those is the finding:

1. **The L2 volley-breaker block** described `core/router.rs` as live mechanics.
   Verified gone: `break_volley`, `last_forward`, `consecutive_short`,
   `user_silent_forwards`, `HEARTBEAT_LEADS` have **no occurrence in `src/`**.
   Banner added rather than deletion — the *reasoning* was inherited by the ring
   (`jaccard_similarity`, `spinning` in `sequencer.rs`), so it stays as reasoning
   and stops being a where-to-look pointer.
2. **`core/duo.rs` → `core/pump.rs`** in the "grep all `recv().await`" advice.
   Path corrected; the advice is still good.
3. **A note asserting a live defect that had been FIXED.** `notes.md` carried:
   *"`halt()` never clears `sessions.current_turn_participant_id` … Fix is
   threading `deps` into `halt`; **unfixed as of 2026-08-13**."* Re-read at
   `5decdcf`: `halt` takes `&SequencerDeps` and its last line is
   `deps.storage.set_current_turn(&deps.session_id, None).await`, with a comment
   naming this exact defect. **The prescribed fix is what shipped.**

That third one is the one that matters, and it is the *inverse* of every other
staleness in this round. F1 and F2 were docs claiming a capability that does not
exist. This was a note claiming a **defect** that no longer exists — which costs a
future session differently: it does not mislead about what works, it sends
someone to fix something already fixed, or worse, to design around a limitation
that was lifted. **A status word carries a date and needs a re-read; it cannot be
inherited.**

### F10 · A nested tuple where a name belonged · TRACED · FIXED `b5510b4`

`core/state.rs` — `Mutex<HashMap<String, (String, Vec<(String, String)>)>>`.
Named as `StagedResponse` / `StagedPick` aliases. An alias rather than a struct
on purpose: the tuple crosses three signatures and the frontend's hand-written
mirror, and what was wrong was that `Vec<(String, String)>` said nothing at any
call site — not that it was a tuple.

### F11 · Four trivial clippy items · MEASURED · FIXED `b5510b4`

`is_none_or` in the tray staleness check — where the double negative was hiding
that an **unparseable timestamp reads as stale**, which is the deliberate side to
fail on — and three doc-list warnings fixed by one blank `//!` line so the plugin
catalog's closing paragraph stops parsing as a list continuation.

### F12 · A safety choice pinned by nothing — found by verifying F11 · MEASURED · FIXED

The one finding this round produced by **checking its own work**, and the one
worth copying.

F11 looked like a formatting change. Mutating it back in Verify —
`is_none_or(|a| a > MAX)` → `is_some_and(|a| a > MAX)` — left the suite
**completely green: 1174 passed, 0 failed.**

Those two predicates differ on **exactly one input**: an `asked_at` that does not
parse. Every other case was pinned; the one that decides what happens when a
gated command's age is *unknowable* was pinned by nothing. And it is not a
detail — `stale` is what makes an approve require a confirm step, so the untested
branch is the one that decides whether a row of unknown age gets waved through
with one click.

The comment F11 added said which way it fails and why. **A safety choice with no
test is a comment**, and round 2 had already written the general form of this —
"say what a guard proves, in the guard" — one level up.

Fixed with `a_gate_whose_age_cannot_be_read_is_stale`, three rows for the three
branches, and the test is itself mutation-verified: red under exactly the flip
that found the gap, with its own message, green on restore.

**The transferable part is the order.** The gap was invisible while reading the
predicate — both spellings read as correct — and invisible to clippy, which
suggested the change. It appeared only when a *cut* was made. Round 2 learned
"unnamed ≠ unpinned" the hard way, by filing four functions as unpinned that a cut
proved were fine. This is the same rule paying out in the other direction:
**a cut is the only pinning measurement, and it is worth making on your own
changes, not only on the code you are auditing.**

---

## 3. R6 — done, in the three recorded steps

The round-2 design was followed, with F5's correction and one stated deviation.

**Step 1 — `RingState`, pure rename.** Thirteen fields, each keeping its original
doc comment verbatim (those comments are the module's reasoning about why each
piece of state is per-cycle; losing them was the whole risk). The gate set and its
seed-failed flag are constructed **together**, as one struct literal rather than
`Default` + field assignment — which is also what keeps
`field_reassign_with_default` quiet without an `#[allow]`.

Driven by the **compiler**, not by search-and-replace: collapse the declarations,
then let `E0425` name every site with a column. Comments and string literals
mention these words constantly and produce no error, so they could not be touched
by accident. 98 references, one pass, **zero test files edited**.

**Step 2 — `advance_turn` 11 → 4 parameters**, and its `too many arguments (11/7)`
warning with it: **clippy 10 → 9**, which is why the design insisted this be its
own commit. The body is untouched, because binding through `&mut RingState` with
a struct pattern gives every name back *at exactly the type it had as a
parameter*. Only the two that were passed by value differ.

`reseed_gates_if_needed` became a method: it is the one place that reads and
writes both halves of the gate pair, and its old signature was the shape that let
them be passed apart.

*Stated deviation:* the design also said to convert the other helpers.
**Not done.** `unwind_wedged_turn`, `pass_empty_turn`, `start_turn`,
`deliver_backlog`, `release_held` and `spinning` are 2–5 parameters, at most three
of them ring fields. Widening those to the whole struct would trade precise field
borrows — which say what each function touches and let the compiler enforce it —
for a vaguer signature, and the design's own stated aim was parameter counts going
**down**. The one clippy tripped over is the one that changed.

**Step 3 — the `TurnComplete` arm becomes `RingState::on_turn_complete`.**
`run_sequencer` **714 → 407 lines**.

The lift is **verified verbatim rather than asserted**: taking the arm out of the
previous commit, applying the same four mechanical substitutions and comparing
byte-for-byte against the method body gives an exact match across all 212 lines.

Two of those substitutions had to be written narrowly, and both wrong versions
compiled far enough to look plausible: `&mut state` must not match
`&mut state.holder`, and `&deps` must not match `&deps.session_id`. The compiler
caught each — **but only because the resulting types differed.** A substitution
that changed meaning without changing a type would have passed silently, which is
the argument for the byte-comparison rather than for trusting the build.

---

## 4. Method — what round 3 adds

**A round that runs on the previous round's binary can check claims instead of
re-reading them.** R5's confirmation is not "the code looks right" but "4,370 rows
are well-formed and 345 of them were written after the fix, by live traffic."
Every audit from here should ask what the last one shipped and what would be
observable if it worked.

**The deletion, not the addition, is what leaves phantoms.** Rounds 1 and 2 swept
citation rot in `file.rs:NNN` form and closed it. F1 and F2 are the *next* layer:
prose that names a subsystem rather than a line, which no line-number check can
see and which reads as authoritative precisely because it is discursive. Task 14
deleted `router.rs` and left **120 mentions** across `src/`; most are legitimate
history, two claimed live behaviour. **After deleting a module, grep its name and
sort the hits into record and claim.**

**"Nothing reads this" is a claim to verify, not to infer.** F7 held up because
three independent things agreed: the source said so (`session.rs:758`), the
field-access grep found only `*_model_at_spawn`, and `SpawnConfig` visibly takes
its values off the participant row. Any one alone would have been the
Cognotify shape — two sources agreeing because one was copied from the other.

**A plan measured once should be re-measured before it is executed.** R6's design
was careful, correct in its reasoning, and wrong in its count. It cost nothing
because the count was checked at the start of step 1 instead of discovered in the
middle of it.

---

## 5. Stated bounds

- **The frontend was not audited this round.** Its gates were run (392 tests,
  `tsc` clean) and F10's alias was chosen not to disturb its hand-written mirror,
  but no frontend render logic was read. Round 2 left it "spot-read, not
  cleared"; **it is still not cleared.**
- **F8 is open by decision, not oversight** — cost stated above.
- **The remaining three `too many arguments` warnings are untouched**
  (`cl_write.rs`, `storage/plugins.rs` ×2). Each is a `RingState`-shaped question
  and none is in the turn path, so none was worth opening in the same session as
  R6.
- **`notes.md`'s ten stale references (F9) are reported, and corrected in the CL
  itself** — not in this repo. `cl_stale_refs` reports; it never edits.
- **No live multi-participant session was run.** R6's proof is the suite (1174,
  green at every step, zero test files touched) plus the verbatim byte-comparison
  — not a live turn through the new `on_turn_complete`.


---

# Round-3 tail — the D10 hard retirement (2026-08-16/17)

The user took **every** scope option and added one: *"hard retire brian and rain,
it might cause issues (maybe hallucinations that bot-hq still has brian and rain),
or maybe context corruption. Settle all your pending tasks here, no deferrals."*

That reason reframed the work. The schema was never the point — **the Context
Library is**, because it is what loads into an agent's context window by design.

## What shipped

| batch | commit | what |
|---|---|---|
| 1 | `83d17a5` | `Author` enum deleted; **F13** — agent phase requests stop being filed as the user |
| 2 | `95a9124` | the activity wire names turn slots; 42 role-shaped identifiers |
| 3 | `891b807` | **migration 0060** — 15 columns, the last CHECK, and F8's proven-fired DEFAULT |
| 4 | `7623a09` | the create-session wire; the driver exemption pinned by its own test |
| 5 | this | docs, the CL sweep, and the acceptance number |

**Acceptance: `cl_stale_refs(bot-hq)` 27 → 25**, measured before and after —
*down* two despite 0060 renaming or dropping fifteen columns.

## F13 · A phase request was recorded, and rendered, as the user's own words

`Author::parse(&agent).unwrap_or(Author::User)`. `parse` knew `user`/`brian`/
`rain`, so for **every rc3 role slug it already returned `None`** and the
fallback filed the row `origin = "user", participant_id = NULL`.
`ChatMessage.tsx` has no case for `Text`, so it rendered as ordinary authored
prose under the user's name — and agents read the transcript back.

**The system was manufacturing a user utterance.** That is the mechanical form of
the fabricated-authorization failure the general rules exist to prevent, and it
is closer to the user's stated worry than any column. A rename would have
preserved it exactly: renaming does not teach a parser to see role slugs.

## F14 · The acceptance metric penalises the correct way to retire a claim

`cl_stale_refs`' retirement detection is **per line** (`RETIREMENT_MARKERS`,
`cl_staleness.rs:77`). A retirement *banner* spans lines, so the symbol names
land on continuation lines carrying no marker — and get reported as fresh
staleness.

Measured, on this round's own work: a banner saying *"Verified gone: `break_volley`,
`last_forward`, `consecutive_short`, `user_silent_forwards` … have no occurrence
anywhere in `src/`"* **raised the count by 3.** Rewriting it so every naming line
carries its own marker dropped it by 5.

So the number moved 27 → 30 → 25 without the repo changing in between. **A metric
that punishes the documented remedy will train sessions to delete history instead
of banner it** — which is the opposite of "evidence corrected beside, never
edited". Recommended fix: scope the marker check to the markdown block (bullet or
blockquote), not the line. Not done here: changing an audit tool while it serves
as that audit's acceptance metric is the same bad sequencing that kept F8 out of
round 2, and the user has twice accepted that argument.

## Method — the finding this round actually produced

**A word-boundary rename cannot tell a name in use from a name being talked
about**, and this task was almost entirely the latter. It over-reached **five**
times:

1. `activity.rs` — *"B4b: took `brian_busy, rain_busy`"* → `slot0_busy`, falsifying what it took.
2. The same line again, on the second pass — my own restored text → `hands_busy`.
3. A doc that said *"`sessions.rain_enabled` **was** a cached count"* → *"`sessions.multi_participant` was"*, which **shipped into generated `bindings.ts`** before it was caught.
4. `wire::MODEL_ARGS` — the **published driver API**, the one documented D10 exemption. A breaking change for every client.
5. EYES' own two-line verification script, whose guard aborted on `Ok(session.into())` matching twice.

The fifth is the one that settles it: it happened to the reviewer, on a script
written *while reviewing this exact hazard*. **The defect belongs to the
technique, not to whoever is driving it.** Every instance was caught by a
different mechanism — the suite, a human read, `tsc`, a test, a self-guard — and
none by the rename itself.

Corollary, three instances (`e829b048`, `a6ea28ff`, `657c7e04`): **changing a
parameter does not prompt a re-read of the comment above it.** All three were
justifying asides one or two lines from an edited line, showing in `git diff` as
context.

And **a test that reads the thing it is proving cannot catch that thing
changing** — `create_session_accepts_per_agent_model_ids` asserted against
`wire::MODEL_ARGS` and went green on the rewritten constant. It now spells the
literals out.

## The completeness claim, measured

**Zero live code identifiers** in `src/`, `tests/` or `frontend/src/` name them.
Verified by sweep, not asserted. They survive in exactly five places, each by an
explicit rule:

1. **12 immutable migration files** — hook-blocked, sqlx-checksummed.
2. **The driver wire** — D10's recorded exemption, now pinned by a test.
3. **Append-only evidence** — `PROGRESS.md` 144, `issues.md` 14, `decisions.md` 14.
4. **`conventions.md:5`** — which names them in order to retire them.
5. **Legacy-data fixtures** — where the retired name IS the subject, e.g. the
   test proving a row authored `brian` does not resolve against a role-derived
   roster. Sweeping that one made its assertion vacuous, and the suite caught it.

"Hard retire" cannot mean zero occurrences in this repo. It means no live code,
schema, or CL guidance — and round 4 can check that sentence against the tree.
