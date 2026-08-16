# rc3 audit — round 2 (2026-08-16)

**Audited at** `632c163`. **Shipped through** `f339f6d` — 10 commits, 23 files,
+977/−185.

Round 1 is [`2026-08-15-rc3-audit.md`](2026-08-15-rc3-audit.md), executed in session
`s-2b866c4a`, and **all of it is in this binary**. Nothing here re-reports a round-1 finding.

**Baseline at the audited commit:** `cargo test` green — 1165 lib + 62 across the other
binaries, 1 ignored (`the_ring_runs_against_a_real_session`, needs `/tmp/smoke.db`).
`cargo clippy --all-targets` exit 0, **17** diagnostics (lib 10 + 7 unique in the test profile).
**At `f339f6d`:** cargo **1169** + 62 · vitest **392** in 45 files · `tsc --noEmit` clean ·
clippy **10**.

**Who measured what**, since this report's whole discipline is that a reader can tell:
the HEAD figures were run independently by both participants and agree — including clippy's
10, reached by the same arithmetic from two separate runs. The **17 baseline is measured by
one participant only**, at `632c163`; corroborating it needs a checkout, which is a tree
mutation the reviewer declined to make. So the delta's left-hand side is asserted, not
independently confirmed. `cargo build --release` was likewise run by one participant; the
reviewer confirmed its observable artifact (the 39,932-byte dist CSS) but not the build.

---

## 0. How to read the status column

Every finding carries one, and it is the load-bearing part of this report. Round 2 reported a
finding (G2) that a cut falsified inside the hour, and shipped a fix whose commit message
claimed a verification that did not hold. The distinction is therefore structural, not tonal:

| status | means |
|---|---|
| **MEASURED** | a command was run and its output is quoted here. The command is named so a reader can re-run the row. |
| **TRACED** | the chain is certain from the code, and was not executed |
| **HYPOTHESIS** | unmeasured; stated so it is not mistaken for either of the above |

---

## 1. Executive summary

Round 2's distinctive scope was the surface round 1 could not audit: **its own fix batch.**
`git diff f8127b0..HEAD --stat` at the audited commit was **42 commits, 62 files, +5466/−801**,
and round 1 audited the ring *before* it was rewritten. Two of the round's three most serious
findings live in a **+146-line** delta inside `src/policy` — the highest finding density of any
area, and code that did not exist when round 1 ran.

The other half of the value came from **re-testing round 1's own calls**, which its §5 declares
a blind spot. That is where the confirmed enforcement bypass came from.

Three findings are defects in enforcement or evidence machinery, all now fixed:

- **H1** — a staged non-UTF-8 file switched the forbidden-word commit gate off. Confirmed by
  probe. Four fail-open sites, one root cause; the post-commit backstop went blind on *exactly*
  the input that defeated pre-commit, so the two layers contributed zero redundancy.
- **G5** — the violations log lost **~60% of records** under concurrent hook activity. The
  writers are separate processes; the lock was in-process; a record was two `write_all` calls.
- **G1** — rotating the violations log made its history unreachable, so a rollover emptied the
  audit trail from every surface a user or driver has. Strictly worse than the no-rotation state
  it replaced.

A fourth, **A1**, is the round's cleanest instance of this codebase's recurring failure: a guard
that had never run, because its `#[test]` was written above the previous test's doc comment.
Both compilers warned, in every build, for as long as it shipped.

---

## 2. Findings

### Enforcement + evidence

**H1 · The forbidden-word commit gate could be switched off by a staged file · MEASURED · FIXED
`c4bf857`, pinned `c8951eb`**

`git_output` (`policy/hooks.rs`) returned `None` when `String::from_utf8(stdout)` failed, and
four callers read `None` as "nothing to scan":

| site | shape | layer |
|---|---|---|
| `:203` | `.unwrap_or_default()` | pre-commit forbidden-word |
| `:302` | `else { return 0 }` | pre-commit immutable-artifact / migrations |
| `:331` | `.unwrap_or_default()` | post-commit message scan |
| `:332` | `.unwrap_or_default()` on `git show` | post-commit **backstop** diff |

Git calls a file binary only on a NUL in the first 8 KB, so a NUL-free latin-1 file has its raw
bytes emitted into `git diff --cached`.

*Measured* — a throwaway repo, one staged latin-1 file containing a forbidden term:
```
 note.txt | 2 ++          ← TEXT: no "Bin" marker, raw bytes in the diff
INVALID utf-8 -> String::from_utf8 fails -> git_output returns None
1                         ← the term IS in the diff, there to be caught
```
→ `unwrap_or_default()` → `""` → `first_forbidden_word("")` → `None` → **`Ok(0)`, the commit
passes.** `:332` is why the fix belongs in the helper: the post-commit verifier exists to catch
what pre-commit missed and reads the same helper.

**Fix:** `String::from_utf8_lossy`. U+FFFD replaces only the invalid bytes, so a term on an
untouched line still matches — where blocking would refuse the commit without naming what
tripped it. After it, `None` means git genuinely failed.
**Re-run:** `cargo test --lib a_staged_non_utf8`; cut `git_output_in`'s decode back to strict and
that test alone fails.

**G5 · The violations log lost most records under concurrent hooks · MEASURED · FIXED `fa67241`**

`append_blocking` wrote a record as **two** `write_all` calls. `write_lock` is a per-instance
`std::sync::Mutex`, which serializes appends within one process — and this log's writers are
separate **processes**: five `ViolationsLog::new` sites across the git hooks (each its own
`bot-hq` subprocess) plus the app's own. Two writers interleave to `{j1}{j2}\n\n`: one
unparseable line, both records skipped.

*Measured* — `cargo test --lib concurrent_writers_never_merge`, 8 threads × 50 records:

| | records recovered |
|---|---|
| one `write_all` | **400 / 400**, 6/6 green (reviewer's independent run) |
| two `write_all` | **102–197 / 400**, 9/9 red across both runs |

`O_APPEND` makes a single `write` atomic with respect to the file offset — which is exactly why
a record taking two of them was the bug. **Honest bound:** 8 threads flat-out is far more
contention than real hook activity, so the production loss rate is much lower than 60%. What the
measurement establishes is that the mechanism is real and the recovery is total.

**G1 · A rotated violations log became unreadable · MEASURED · FIXED `2dd0a26`**

`rotate_if_oversized` renames the log to `.jsonl.1` at 4 MiB; `read_all` read `self.path` alone,
and nothing in the tree ever opened the rolled file. Both consumers go through `read_all` — the
Violations panel (`tauri_cmd/policy.rs`) and the external driver (`external_jsonrpc.rs`) — so a
rollover emptied the trail from every surface that exists. The doc promised the opposite:
*"keeps one generation of history … a rotation mid-incident does not lose the incident."*

The guard witnessed rather than guarded: it asserted the rolled file's bytes with
`std::fs::read_to_string`, a path no consumer has.

Ordering was checked before landing, because prepending to a list someone slices from the head
would have made the panel worse in a new way: `external_jsonrpc` reverses *then* truncates to
`limit`, `ViolationsPanel.tsx` reverses the whole list — both put older records where a cap
drops them. Cost: a panel open parses up to `2 × ROTATE_BYTES`, bounded by rotation.
**Re-run:** `cargo test --lib policy::violations`; delete the rolled-generation read and that
test alone fails.

**A1 · A guard that had never run · MEASURED · FIXED `e1d3683`**

`no_tool_description_an_agent_reads_names_an_agent` (`signaling/protocol.rs`) had no `#[test]`.
Its attribute was written above the *previous* test's doc comment, so Rust merged both doc
blocks and both attributes onto `every_registered_tool_is_documented`.

*Measured* — `cargo test --lib no_tool_description` → `running 0 tests … 1166 filtered out`.

**Why it was invisible, which is the better finding:** the duplicate attribute registered
`every_registered_tool_is_documented` **twice** — `cargo test --lib -- --list` at the parent
commit prints that name on two consecutive lines. The suite total was therefore *identical*
before and after the fix. **No count-based check could have caught this; the defect conserved
the count.** Both compilers said so in every build (`duplicated attribute`, `function is never
used`).

*Measured* — injecting `the duo` into a live tool description reddens it, naming `peer_ack`. It
guards, not merely runs. This seals round 1's audit C1-2 fix, unsealed since it shipped.

### Coverage

**R2-1 · 14 of 16 session drains were unpinned · MEASURED · FIXED `8f80524`**

`unregister_session` has sixteen drain lines. `session_phase` was pinned here; `session_sequencer`
turned out to be pinned from `sequencer.rs`'s own module. The reviewer cut the other fourteen in
one go, with `session_phase` left in the cut set as a **positive control**, and
`cargo test --lib` reported `1164 passed; 1 failed` — the control. Fourteen deleted green,
including `session_attention`, whose absence was a user-visible bug batch 1 had just fixed.

Two guards now, because the function fails two ways:
- **behavioural** — seeds all sixteen for *two* sessions and reports (map, holds-doomed,
  holds-survivor). The survivor is the point: `clear()` where `remove()` was meant empties the
  map for every live session too, and no length check tells those apart.
- **structural** — derived from the struct's own `HashMap` fields, so the only way to pass is to
  agree with the declaration. **No exemption list**; a sanctioned-absence list is what rots into
  permission for the next one.

*Measured*, four ways, each naming the offending map: delete the `session_attention` drain → both
red · `remove()`→`clear()` → *"dropped an UNRELATED live session"* · add a 17th `HashMap` field →
*"no line in unregister_session"* · **and the reviewer's**: delete the drain but leave a comment
containing `self.session_attention` → the structural guard passes, the behavioural one catches
it. **Only the pair is the guarantee, neither half is**, and the `self.` prefix on the needle is
load-bearing — a bare-name needle is already satisfied by that function's own backticked
comments.

**R2-2 · Two tests outran their names · MEASURED · FIXED `9fe6733`**

`oversize_files_are_refused_by_the_limit` wrote a 16-byte file and asserted
`MAX_VIEWABLE_BYTES >= 1024 * 1024` — two compile-time constants. It never entered the branch it
names. `..._and_the_forward_is_unchanged` bound a `ring_rx` it never read, promising a pairing
the router's deletion removed.

### Framing

**B1/B2 · Rendered strings still named a pair · MEASURED · FIXED `b093341`**

Round 1 swept `GENERAL_RULES`, the public site, README, INSTALL, ARCHITECTURE, CLAUDE.md. Three
survivors, all **rendered text rather than doc prose**, which is exactly why a prose sweep went
past them: `Settings.tsx`'s Archived Sessions body, and the NEEDS DIRECTION tooltip duplicated
verbatim in `SessionView.tsx` and `SessionTile.tsx`.

Tooltip pair is now one declaration in `lib/attention.ts` (the `phase.ts` shape). `lib/framing.ts`
+ `framing.test.ts` seal the class with a sweep over every source file, word-boundary matched
with `_` as a word char so `session.rain_enabled` is not a false hit. Three exemptions, each a
property of the file, **all stated** — `*.test.*` (never rendered), `bindings.ts` (generated at
launch; no human can author a regression into it), `framing.ts` (defines the pattern; building
the regex from fragments to dodge its own check would be rewording around the gate).

*Measured* — reintroduce "the duo" into the shared tooltip, or a bare `Brian` into Settings JSX,
and the sweep names file, line and word. Reviewer independently injected a probe const into a
real component and it fired.

**C1 · A dead pump discriminant · TRACED · FIXED `b093341`** — `PumpConfig.author` was
constructed per pump, per session (`if slot == 0 { Author::Brian } else { Rain }`), documented as
router-only, and read by nothing after task 14 deleted the router. `Author::parse` keeps
`"brian"`/`"rain"` for legacy message rows — that half **is** load-bearing.

**E3 · `core/duo.rs` → `core/pump.rs` · FIXED `90bec09`** — the module's own first line always
said "per-agent event pump". `DuoConfig` → `PumpConfig`, 31 references.

**G6 · `file.rs:NNN` citations cannot be audited, only converted · MEASURED · FIXED `f339f6d` +
follow-up**

`sequencer.rs`'s module doc cited line locations for the system-origin writers, and they had
rotted. **My first measurement of this was wrong twice and the reviewer caught both**, which is
why the conclusion below is the strong one:

1. *Wrong count.* My regex was `` `[a-z_/]+\.rs:[0-9]+` `` and missed every **ranged** form
   (`state.rs:878-882`, `state.rs:743-744`). Real total: **14**, of which 3 are generic examples
   in doc prose (`file.rs:12`, `src/foo.rs:133`, `src/signaling/bridge/util.rs:133`).
2. *Wrong verdict.* I called the rest "spot-check accurate" — but I had checked whether the cited
   line looked *plausible*, not whether it matched the **claim**. Checking the claim:

| citation | claims | line actually holds | |
|---|---|---|---|
| `watchdog.rs:364` | idle nudge writes TWO rows | `// TWO rows, because…` | ✓ |
| `watchdog.rs:379` | NUDGE posted as `"system"` | `// Host-authored, so origin='system'` | ✓ |
| `storage/messages.rs:60` | author → origin mapping | `Author::User => ("user", None)` | ✓ |
| `state.rs:878-882` ×2 | `advance_phase` writes `Author::User` | `if let Some(archive) = archive {` | ✗ (real: `:1487`) |
| `bridge/tray.rs:916` | `request_phase_advance` fallback | `}` | ✗ (real: `:1212`) |
| `state.rs:737-740` | "a user message is the steer" | `?activity,` | ✗ |
| `state.rs:743-744` | "a phase self-advance is not a user message" | `decision` | ✗ (real: `:1468`) |
| `general_rules.rs:166` | "no user click needed" | the IPAV heading | ✗ (real: `:183`) |

**Six of nine live citations were stale — two thirds, not "mostly healthy".**

**And the class has a second, independent failure mode** (reviewer): `src/` holds **13 duplicated
basenames** — `feedback.rs`, `findings.rs`, `messages.rs`, `mod.rs`, `models.rs`, `plugins.rs`,
`protocol.rs`, `session_docs.rs`, `sessions.rs`, `terminal.rs`, `tool_gate.rs`, `tray.rs`,
`updates.rs` — and `tray.rs` has **three** copies (361 / 482 / 2619 lines). So a bare
`tray.rs:916` names no file, and two of the three candidates do not even reach line 916. Today's
citations happen to name unique basenames, but the convention invites the ambiguity.

**Conclusion, stated as a convention rather than a round-3 note:** a `file.rs:NNN` citation rots
silently on the next edit to the file it names *and* may not identify a file at all. It cannot be
audited — only converted. **Cite a symbol; when a location is genuinely needed, cite a
repo-relative path.** A symbol decays loudly, because a reader greps and finds nothing. All nine
live citations in `sequencer.rs` are now symbol-anchored, including the three that were accurate —
they would have rotted next.

### Open — need a decision, not a fix

**B3 · The plugin wire expresses roster size as a boolean · TRACED · OPEN**

`plugin_api.rs:396` `duo: bool` → `dispatch_session_inner(rain_override)` → `sessions.rs:744`
`unwrap_or(false)` → `sessions.rain_enabled` → `session.rs:721`
`ensure_session_roster(id, rain_enabled == 0)` → `participants.rs:970` `first_role_only`.

`first_role_only = false` seeds **every** active non-`on_mention` role, **uncapped** — and that is
deliberate and pinned (`participants.rs:4611` asserts a third role joins). The storage layer is
correct rc3 behaviour; **the lie is at the wire.** `duo:true` promises "a Brian+Rain duo" and
delivers N. The live DB has 3 roles (one `on_mention`), so it seeds 2 today and *looks* right;
add one active role in Settings → Roles and a plugin's consented "duo" silently becomes 3. The
dialog's cap of 4 lives in `resolve_participant_picks`, not in the seeder.

Same shape on the external driver: `session.rs:355` `rain_enabled = if req.solo {0} else {1}`.
**Both non-dialog creation paths express roster size as a boolean.**

A rename is *worse than cosmetic* — it would make the wire read honest while `duo:true` still
means "all of them". Real fix: an optional `participants` array mirroring `options.participants`,
**plus the cap**, with `duo` kept as a documented legacy alias. Renaming the key breaks installed
plugins; adding an optional array does not. **The user's call.**

**E2 · `rain_*` columns — CORRECTED mid-round · TRACED · OPEN (naming only)**

Reported first as six superseded dead columns. **That was wrong**, and the reviewer traced it:
only `rain_claude_session_id` is dead. `rain_enabled` is B3's control input; `rain_model_id` /
`rain_effort` / `rain_ultracode` are live on the rosterless path (`sessions.rs:586-631`, the
legacy caller's only way to name a slot's model); `rain_busy` is a live event field reaching the
frontend. **If E2 lands in a "dead code deletion, zero behaviour change" batch it breaks roster
seeding.**

**G3 · The RFC3339 fix normalized the writer, not the data · MEASURED · OPEN**

`1a575e8` made `commit_delivery` bind `now_utc()` and shipped a guard. No backfill.
*Measured* — `sqlite3 ~/.bot-hq/.local/bot-hq.db "select … from participant_deliveries"`:

| shape | rows (measured 2026-08-16) |
|---|---|
| RFC3339-Z | **14** |
| zone-less | **4011** (`2026-08-12 14:12:08` → `2026-08-16 04:05:22`) |

The ratio is **dated on purpose.** Measured twice hours apart, the zone-less figure stayed exactly
4011 while the total moved 4025 → 4277: every one of the 252 rows written during this session was
well-formed, which confirms the writer against live traffic in a way no fixture can — and is why a
bare ratio would read as current while quietly ceasing to be.

The guard only inspects a row it just wrote, so it could not see the legacy population.
**Latent, not live:** `delivered_at` has no prod comparison — INSERT and a test SELECT, nothing
else. **FIXED** in `5e43eaa` (migration 0059), together with a second contaminated column the
pre-flight found: `participant_cursors.updated_at`, 85 of 90 rows, from a live-firing column
default the seeding INSERT omitted.

*Systemic version:* 17 columns still default to `datetime('now')` / `CURRENT_TIMESTAMP`, with
three lexicographic SQL windows over them (`messages.rs:80`, `:108`, `retrieval_events.rs:84`).
This class has been fixed at four sites already. **Reviewer's constraint, adopted:** dropping the
defaults converts a silent-wrong into a runtime INSERT failure and needs a new migration (applied
ones are immutable), so it must be paired with a query proving no live writer omits the column
*before* the migration, not after.

**G4 · Three timestamp parsers, one a duplicate of an already-imported helper · TRACED · OPEN**
`bridge/util.rs:280` `parse_tray_ts` (tolerant) · `bridge/tray.rs:23-29` `gate_age_secs` hand-rolls
the identical two-branch parse **while `tray.rs:9` already imports `parse_tray_ts`** ·
`cl_facade.rs:22` RFC3339-only. The third is **not** a defect — the reviewer confirmed both shapes
`cl_index.updated_at` holds are valid RFC3339, and `disk_is_newer` compares them parsed.

**E1 · `sequencer.rs` · TRACED · OPEN — deliberately NOT STARTED, with the design below** — 9781 lines, **3653 prod**: a 680-line module doc, then
`run_sequencer` **714 lines**, `advance_turn` **372**, `deliver_backlog` **285** — three functions
= 38% of the prod file, 20 top-level fns. Round 1's B1-F1 argued the *doc*; the **function**
decomposition was never on the table.

### R6 — why it was not attempted, and what it needs

The user authorised all of B3/R5/R6. B3 and R5 shipped; **R6 was measured and deliberately left**,
because the shape of the work makes a partial attempt worse than none.

*Measured:* `run_sequencer` is 714 lines whose body is one `match` over `SequencerCommand` with
ten-plus arms; the `TurnComplete` arm alone is **225 lines**. `run_sequencer` declares **11
`let mut` ring-state locals** (`holder`, `epoch`, `deferred`, `paused`, `held`, `spin`, `laps`,
`summons`, `spoke_this_lap`, `staged_pending`, `halted_pending_user`), and that one arm touches at
least **eight** of them.

So extracting an arm as a plain function means roughly ten `&mut` parameters — which is precisely
why `advance_turn` already trips clippy's `too many arguments (11/7)`. Adding a second
ten-parameter function would make the module measurably worse while looking like progress.

**The real shape is a `RingState` struct** holding those eleven locals, with the arms becoming
`impl RingState` methods taking `&mut self` plus `&SequencerDeps`. That also retires the
`too_many_arguments` allow rather than adding another. It is a genuine design change threading
through a 3653-line module and its 99 tests, and it is the most delicate code in the repo — the
thing that deals turns, where a half-finished state does not fail loudly, it wedges sessions.

**Sequencing for whoever takes it**, in the order that keeps the suite meaningful throughout:

1. Introduce `RingState` with the eleven fields and `Default`, constructed at the top of
   `run_sequencer`; replace the locals with `state.field` in place. No behaviour, no extraction —
   the suite must stay green on a pure rename.
2. Convert the existing extracted helpers (`advance_turn`, `deliver_backlog`, `unwind_wedged_turn`,
   `pass_empty_turn`, `reseed_gates_if_needed`) to take `&mut RingState` instead of their current
   parameter lists. Each conversion is independently verifiable, and `advance_turn`'s
   `#[allow(clippy::too_many_arguments)]` should come off as its own commit so the warning count
   proves the parameter list actually shrank.
3. Only then extract the match arms, largest first (`TurnComplete`, 225 lines).

**Do not skip step 1**, and do not extract an arm while the state is still eleven separate locals:
that is the version that adds a ten-parameter function. The clippy count is the progress signal
here — it should fall, never rise, and this round already recorded one case where a displaced
attribute was invisible to everything except that number.

**Batch-9 scope, now numbered.** Of **100** `#[tauri::command]` fns, **94** take `tauri::State`,
and **9 of them hold 13 inline `return Err(…)` refusal guards** unreachable from a unit test:
`cl_rename_project` (4), `read_workspace_file` (2), `cl_set_agent_visibility`,
`cl_register_project`, `cl_create_project`, `cl_delete_project`, `summarize_session_doc`,
`set_agent_feedback_status`, `rename_session`. One `State`-free-inner-fn convention, decided once.
`cl_rename_project` is the one to look at first — four guards, and it is the operation that
retargets open CL tabs.

---

## 3. Method — what round 2 learned about auditing

The most transferable results are corrections to how this round worked, two of them to **my own**
method.

**Name-search is not a pinning measurement — only a cut is.** I reported four batch-1 functions as
unpinned because no test named them; the reviewer cut all four and the suite went red on every
one. They are exercised *through* the ring loop, invisible to a name search. **Unnamed ≠
unpinned**, and the substantive result was the inverse of what I filed: batch 1's headline fixes
are well pinned.

**A guard's own fix needs the same discipline.** `c4bf857` shipped H1 with a test that called the
extracted helper, so the audited line stayed revertible with the suite green — and its message
claimed "mutation-verified". I had mutated the helper's body, a line the audit never named. The
reviewer measured it and filed it blocking.

**Extraction fixes the assertion; threading fixes the wire.** An untestable seam does not produce
*no* test — it produces a test that pins whatever *was* reachable, which reads as coverage.
Extracting a helper moves the boundary of "whatever was reachable" without removing it, so the
residue changes shape (from a degenerate assertion to a deletable call) rather than going away.
Only threading a path through to the real entry point makes a guard uninstallable-with-a-red-suite.
`files.rs` in this round is the half-remedy; `hooks.rs` is the full one.

**Evidence is corrected beside, never edited.** A change log and a commit message are both
evidence, so the fix for a wrong one is an adjacent right one. `c4bf857`'s false verification
claim stands with its correction in `c8951eb`; PROGRESS.md's 12 rename-sweep lines were reverted
because entries describe the tree as it was on their date.

**Say what a guard proves, in the guard.** The structural drain guard's message claimed "no line
in `unregister_session`" when the check is a source-text search for a *mention* — the reviewer
deleted a drain, left a comment carrying the needle, and it passed. Not a defect (the pair is
sound), but the gap between a message and its property is this codebase's most-repeated failure.

**Fixture shape decides what a test can see.** G1's assertion could not be re-pointed through
`read_all` until its 4 MiB filler was newline-terminated — and that repair is what surfaced G5.
The old substring check could not see a malformed line; a real parse failed instantly.

---

## 4. Checked and clean — do not re-derive

- **Migration gap 0055 → 0057 is benign.** `0056_user_actions.sql` was added by `14be4b6` and
  removed by its revert `d48df9a`; the live DB has no version-56 row. No boot hazard.
- **`src/plugins` and `src/claude_config` are byte-identical** to what round 1 audited — no
  round-2 pass needed, and "plugin sandbox uncovered" is withdrawn as a gap.
- **No dead frontend modules**; no storage N+1; zero `TODO`/`FIXME`/`HACK` in `src/` or
  `frontend/src/`.
- **`tests/codebase_map_test.rs` is a real guard** — it caught both new frontend files before I
  did, and its `.test.tsx`→`.tsx` fold is why `framing.ts` exists separately from its test.
- **C1-1 internal-MCP auth is closed** within its stated scope (fresh uuid per spawn, constant-
  time compare, bounded fail-open) — the reviewer's leak hypothesis was refuted.
- **`paths.rs`'s `LEGACY_*_CUSTOM_INSTRUCTION` must not be edited.**
  `migrate_agent_custom_instructions` compares `body.trim() != legacy_template.trim()`, so any
  character change makes an untouched seed read as real user content. Same class as an applied
  migration.

## 5. Stated bounds

- **The tool-description guard's phrase sweep enumerates six phrasings.** "the pair", "your
  counterpart", "the other agent", "both of you" all pass. Inherent to phrase-matching, not
  fixable by enumeration — and the enumeration weakness its own doc block criticizes two
  paragraphs earlier. The *name* half is general.
- **The framing sweep exempts three classes**, all stated in the test.
- **Frontend render logic was spot-read, not cleared.** The +452 there is dominated by
  `ContextLibraryEditor.tsx`'s tab-switch fix, which is heavily pinned (+182 test lines); I found
  no defect and did not run the app.
- **`duo` survives in `src/` deliberately:** the comment layer (the `146736d` convention — a
  comment describing a 2026-07-10 incident in that day's vocabulary is a record), the `LEGACY_*`
  templates, `cl_write.rs`'s test specimen, the two guards' own phrase lists, and
  `plugin_api.rs`'s wire field (B3).
