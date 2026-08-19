# Overnight autonomous run — state, authorisations, queue

> **CLOSED — the run finished and its one remaining item (delete `core/router.rs`) shipped 2026-08-13.** This was the source of truth for a single overnight run; it is history now, and its authorisations do not carry forward. Banner added in round 12 (2026-08-19).

**This file is the source of truth for the overnight run.** It exists because a
context compaction would otherwise lose the queue and the user's authorisations.
**Re-read it at the top of every wakeup, and update the log at the bottom after
every landed unit.** If anything here disagrees with memory, this file wins.

User is asleep (from ~2026-08-11 22:40 PH). Work continuously, no input available.

---

## Authorisations given explicitly by the user, 2026-08-11 ~22:40 PH

1. **DELETE `core/router.rs` — B5 task 14 is AUTHORISED.** Previously withheld as
   irreversible; the user has now cleared it. Still walk the gate first: all 20
   inventory rows (**12 PRESERVED / 6 DISSOLVED / 2 DROPPED** — corrected count,
   see `2026-08-06-router-behaviour-inventory.md`). Every PRESERVED row needs a
   named green test; every DISSOLVED row needs the structure that makes it
   impossible; every DROPPED row needs its reason re-confirmed.
2. **Round cap is SETTLED — no longer an open question.** The user: *"just set it
   to my maximum run. I thought we settled the round-cap?"* They are right; D2
   settled the value at **500** and only the UNIT was open. Resolution:
   - **Unit = LAPS** (one full pass of the ring over active participants).
   - **Default 500 laps**, `0` = off, per-session override.
   - Why that is above their maximum run: measured 3,561 uninterrupted stretches,
     max **294 agent text messages**; at N=2 that is ~147 laps. 500 laps is ~3.4×
     their largest real run, so it cannot fire on legitimate work — which is what
     design §1b means by "high enough to be invisible in normal use".
   - Document the messages→laps conversion where the constant is defined. Do not
     claim a corpus-wide organic maximum: the available proxies are rain-only and
     undercount laps badly in the tail (brian 174/rain 10 in the largest).
3. **Latitude:** *"you can do whatever you want, I just want this finished by the
   time I wake up if possible."*

## Standing guardrails (NOT lifted)

- **No `git push`.** Commit on main, never `--no-verify`.
- **Do not leave bot-hq running.** If started to verify, stop it before the turn ends.
- **Migrations 0046/0047 exist**; 0046 is committed-but-unapplied. New ones start at **0048**.
- Every test **mutation-verified**. Never write an unverified claim into the repo.
- Worktrees are created ~95 commits stale — `git merge --ff-only main` in each before working.

## Baseline at handoff

`main`, tree clean, **14 commits** on 2026-08-11, suite **1148 lib + 66 integration** green,
app stopped. Binding docs: `2026-08-11-rc3-decisions.md` (8 user decisions),
`2026-08-11-design-drift-audit.md`, `2026-08-06-session-focused-redesign-design.md`.

## Queue, in order

1. **Two open reviewer findings.** (a) B8a's `update_role` `if changed == 0 { bail }`
   guard is untested — deleting it leaves the suite green. (b) B7a's two call-site
   args in `spawn_session_handle` (`brian_prose.as_deref()` / `rain_prose.as_deref()`)
   are killed by no mutation.
2. **A1 — the round cap**, per authorisation 2 above. Now unblocked.
3. **B8 FRONTEND — the Roles tab. THE USER'S TOP PRIORITY.** Backend landed
   (`src/tauri_cmd/roles.rs`). Per D8: **no Agents tab**; the Roles tab owns Default
   Model via `roles.default_model_id`; the New Session dialog overrides per
   participant via `session_participants.model_id` (both columns already exist).
   Add roles, edit free-text prose (`roles.description_prompt`), capabilities,
   participation mode. **Omit `on_demand` from the picker** (D1 — it needs
   `@mention`, which needs N-participant spawn first).
4. **B7 layer 2** — capability-derived rules GENERATED from `roles.capabilities`
   (D3: generate denials from ABSENT capabilities too); peer names generated from
   the live roster (D4).
5. **N-participant session create** — design §1: "how many agents, **default 1**".
   Today `ensure_session_roster` hardcodes two from 11 per-agent `sessions` columns.
6. **B5 task 14** — delete `core/router.rs`, gate-walked. Do this LAST: items 1–5
   are additive and reversible; this one is not.

---

## Log — append one line per landed unit (newest last)

- 2026-08-11 22:40 PH — handoff. Baseline 1148 lib + 66 integration, 14 commits.
- 2026-08-12 morning — **the overnight loop never fired.** The wakeup was
  re-armed to land after the 00:50 PH limit reset and did not come back. Nothing
  was lost: `main` is still `f18d6fb`, tree clean, suite unchanged. The queue
  below is untouched and resumes from item 1. Do not re-run the timer; the user
  is awake and driving.
- 2026-08-12 — phase 1 fan-out started: queue items 1, 2, 3 and 4 in four
  isolated worktrees, each adversarially verified before merge. Items 5 and 6
  follow after the merge, in that order, because both touch `core/session.rs`
  and item 6 is the irreversible one.
- 2026-08-12 — **queue items 1–5 are DONE and merged.** Suite `dc1202f`:
  **1208 lib + 66 integration + 222 frontend**, `tsc` clean, bindings in sync.
  Phase 1 landed the reviewer findings, the round cap, the Roles tab and layer 2;
  phase 2 landed capability enforcement, migration 0048, the findings sweep and
  N-participant session create. **Only item 6 (delete `core/router.rs`) remains**,
  and it is held for a live session run because flipping `BOT_HQ_SEQUENCER` from
  opt-in to the only path is the irreversible half of it.
- 2026-08-12 — **the reframe contract is now the governing doc**:
  `2026-08-12-rc3-reframe-contract.md`, written from the user's own framing
  (rc3 is an architecture reframe, not a redesign). It supersedes this file's
  framing wherever the two differ.

## Lesson from this batch — re-verify cross-branch claims AFTER the merge

Three defects reached `main` in this batch. **Every one was a claim that was true
on its own branch and false in combination**, and none was catchable by
reviewing either branch alone:

1. Migration 0048 removed `` `Edit`/`Write`/`NotebookEdit` `` from `RAIN_ROLE`
   because layer 2 regenerated it; the findings sweep then removed those names
   from layer 2 because native EYES was promised a tool it lacks. Both correct
   alone. Together, EYES was blocked from three tools the prompt no longer named
   — `NotebookEdit` appeared zero times in the whole 48 KB composed prompt. The
   sweep's own commit message asserted "the CLI role prose still names `Edit`,
   `Write`" — true when written, falsified minutes later.
2. The sweep added a UI notice branching on `roles.builtin`; 0048 set
   `builtin = 0` for every row, permanently. The notice always rendered its
   wrong branch, telling the user HANDS would join with no instruction when in
   fact clearing it restores the built-in prose.
3. `Dashboard.test.tsx` (N-participant) and the `has_builtin_prose` field
   (collision fix, branched pre-merge) never saw each other; `tsc` broke on a
   fixture missing a now-required field.

**Rule going forward:** after merging parallel worktree branches, re-run the
full suite AND re-verify any claim a commit message makes about another part of
the tree. The per-branch verifiers were rigorous and still could not see this —
the blind spot is structural, not a lapse.
