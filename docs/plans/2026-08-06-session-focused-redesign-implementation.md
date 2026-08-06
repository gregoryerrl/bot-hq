# Session-Focused Redesign — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace bot-hq's compiled-in agent identities with session-owned
participants assigned user-defined roles, and replace the hidden peer-forward
pipe with a single auditable channel — without changing what the user sees,
except the deliberate serialisation of the turn cycle.

**Architecture:** A session owns N participants. A participant is a model plus
an invite-time snapshot of a user-owned Role (capabilities, participation mode,
composed prompt). Exactly one participant holds the turn; the cycle advances on
turn end and halts on unanimous done-votes or a parked question. All delivery
goes through one channel with per-participant cursors, so what is delivered is
what is recorded is what is displayed.

**Tech Stack:** Rust (tokio, sqlx/SQLite, hyper), React 18 + TypeScript +
Tailwind via Tauri v2.

**Design:** `docs/plans/2026-08-06-session-focused-redesign-design.md` — complete,
all four architectural forks validated by the user.

---

## Read this first: the two rules that make this plan safe

**1. The parity oracle comes BEFORE the migration.** The hard constraint is
"nothing changes client-side." That cannot be verified by care; it needs
executable tests that pin today's behaviour and still pass afterwards. **Batch
B0 exists solely to build that oracle, and no schema moves until it is green.**
If you skip B0, every later batch is unfalsifiable.

**2. B1 is irreversible.** `migrations/*.sql` is applied automatically at app
start, checksummed by sqlx at runtime, and protected by the immutable-artifact
pre-commit gate. There is no second attempt and no edit-after-apply. The draft
and its runbook already exist and are dry-run-proven; **follow the runbook, do
not improvise.**

**Gate suite — run in this order before EVERY commit** (order matters: the
binary embeds `frontend/dist`, so `npm run build` invalidates cargo's
fingerprint):

```bash
cargo test
cd frontend && npm test
npm run lint
npm run build
cd .. && cargo build --release
```

**Commit conventions:** imperative subject, no AI attribution, message via
`git commit -F /tmp/msg` (an inline heredoc puts the body in the command string,
where the Tool Gate matches quoted keywords — this has already bitten twice).
Call `check_open_findings` before every commit.

---

## Batch B0 — The parity oracle (NO schema, NO behaviour change)

**Why first:** every later batch claims "behaviour is unchanged." This batch is
what makes that claim checkable. It is pure test-writing against the CURRENT
code, so it can land immediately and cannot break anything.

### Task B0.1: Pin the capability boundary as it exists today ✅ DONE (`1f9d26c`)

**Correction to this plan, found while executing it:** the file cannot live in
`tests/`. `signaling::jsonrpc` is a **private** module (`mod jsonrpc;`), so an
integration test cannot reach `dispatch` or `CallerIdentity`. It landed as an
in-crate `#[cfg(test)] mod parity;` inside `signaling`, which compiles to
nothing in release. Apply the same check to B0.2 before writing it.

**Also worth copying into B0.2:** the tests pin *behaviour* (per-tool
accept/reject per agent), not the constant lists — asserting the constants would
not catch a tool being silently added to or removed from a gate. And a fourth
test pins the tools that are UNGATED today, because over-gating is as much a
parity break as under-gating and ships far more quietly.

**Verification standard set here — apply it to every later oracle:** a test that
passes on first run proves nothing. `halt` was removed from `HANDS_ONLY_TOOLS`
to confirm exactly one test fails (`halt must reject EYES`) while the rest stay
green, then `jsonrpc.rs` was restored. An oracle must be shown to discriminate.

**Files (as built):**
- Create: `src/signaling/parity.rs`
- Modify: `src/signaling/mod.rs` (add `#[cfg(test)] mod parity;`)

**Step 1: Write the failing test**

```rust
//! Parity oracle: the tool-authorization boundary as it behaves BEFORE the
//! session-focused redesign. These assertions must hold identically after it.
//! A change here is either a bug or a decision that must be recorded in
//! docs/plans/2026-08-06-session-focused-redesign-design.md Constraint 0.

use bot_hq::signaling::bridge::SignalingBridge;
use bot_hq::signaling::jsonrpc::{dispatch, CallerIdentity};
use serde_json::json;

/// Every tool HANDS may call and EYES may not, as of 2026-08-06.
const HANDS_ONLY: &[&str] = &[
    "ask_user_choice", "mark_awaiting_user", "request_approval", "action_gate",
    "supersede_question", "disposition_finding", "override_reviewer_block",
    "halt", "declare_working", "terminal_exec",
];
/// Every tool EYES may call and HANDS may not.
const EYES_ONLY: &[&str] = &["eyes_flag", "approve_finding"];
/// CL-mutating tools EYES may not call.
const CL_MUTATE: &[&str] = &["cl_write_file", "cl_register_folder_description"];

#[tokio::test]
async fn hands_only_tools_reject_eyes() {
    let bridge = SignalingBridge::new();
    for tool in HANDS_ONLY {
        let caller = CallerIdentity { session_id: "s1".into(), agent: "rain".into() };
        let res = dispatch(
            /* build a tools/call request for `tool` */ todo!(),
            &caller, &bridge,
        ).await;
        assert!(
            format!("{res:?}").contains("reserved for the HANDS agent"),
            "{tool} must reject EYES"
        );
    }
}
```

Mirror it with `eyes_only_tools_reject_hands` and
`cl_mutating_tools_reject_eyes`.

**Step 2: Run to verify it fails**

```bash
cargo test --test parity_capability_boundary
```
Expected: FAIL — `todo!()` panics. Replace the `todo!()` by copying the request
construction from `src/signaling/jsonrpc.rs`'s existing tests
(`rain_rejected_from_hands_only_tools`), which already builds these.

**Step 3: Make it pass** — fill in the request builder. No production code changes.

**Step 4: Verify**
```bash
cargo test --test parity_capability_boundary
```
Expected: PASS, 3 tests.

**Step 5: Commit**
```bash
git add tests/parity_capability_boundary.rs
git commit -F /tmp/msg   # "test: pin today's tool-authorization boundary"
```

### Task B0.2: Pin the commit gate's behaviour ✅ DONE (`e55870f`)

Landed in `src/signaling/parity.rs` (same in-crate correction as B0.1), as three
tests: a blocking finding gates and the verdict names both the finding and how
to resolve it; an advisory finding never gates; both dispositions clear it,
`rebutted` included. The reviewer-down backstop is pinned **through
`check_open_findings`** rather than the private `reviewer_block_decision`, so it
survives however B6 refactors the predicate — healthy passes, dead blocks with
reviewer-gone wording, override opens it *visibly*, recovery clears it.

Regression-proven: dropping the `severity = 'blocking'` filter fails exactly the
advisory assertion and nothing else.

**Note for B6:** `check_open_findings` decides "is this a duo" by reading
`sessions.rain_enabled`. That read must become a roster lookup, and it is one of
the `sessions.{brian,rain}_*` reads the cutover grep audit must catch.

### Task B0.3: Inventory the router's encoded behaviour ✅ DONE (`e55870f`)

**Files:** `docs/plans/2026-08-06-router-behaviour-inventory.md`

All 20 behaviours have a verdict: **11 preserved** (each needs a green test in
the new model *before* the old one is deleted), **8 dissolved** (a fixed ring
cannot express the failure they guard), **2 dropped** with stated reasons.

Two findings worth carrying into B5:
- **#9 and #13 would plausibly have been lost in a rewrite** — an
  acked-but-substantive turn forwarding anyway (which exists because four real
  reviews were destroyed), and a convergence reset surviving a suppressed
  forward. Neither is visible in the code, only in the tests.
- **#6 is dropped deliberately and it is the redesign's whole point:** the
  delivery path asserting it never touches storage IS the invisibility the user
  reported. Its replacement asserts the opposite; the perf concern that
  motivated it is carried forward as a **benchmark in B5, not an assumption**.

**Definition of done for B0:** ✅ met — 8 parity tests green and
regression-proven, inventory complete with a verdict on every behaviour, no
production behaviour touched.

---

## Batch B1 — Schema (IRREVERSIBLE — follow the runbook)

**Files:**
- Move: `docs/plans/2026-08-06-session-participants-migration-DRAFT.sql`
  → `migrations/0044_session_participants.sql`
- Procedure: `docs/plans/2026-08-06-session-participants-runbook.md`

**Already done and proven:** the draft creates `roles` (seeded `hands`/`eyes`),
`session_participants` (with invite-time snapshot + turn state),
`participant_cursors`, `participant_deliveries`, and rebuilds `messages` with
`participant_id`/`origin`/`envelope`. Dry-run against the live DB: 2 roles, 764
participants all resolving a role, 382 sessions each with exactly one
position-0 participant, 0 unmapped rows, integrity + FK clean. Guards 2, 3 and 5
proven to abort on deliberately broken variants, each leaving the original table
intact.

**Steps, in order — do not reorder:**

1. **STOP the app.** A running instance applies migrations at start and holds
   the single-instance lock.
2. **Back up and verify the backup opens** (runbook §Pre-flight). A backup
   nobody opened is not a backup.
3. **Re-run the dry run** against a fresh copy — the DB has grown since; the
   guards assert equality at run time.
4. **App-boot check** against a copied data dir. This is the one outstanding
   unknown: SQL guards cannot catch a sqlx compile-time query mismatch, and
   every `messages` query in the codebase currently selects `author`. **Expect
   this to fail** — that failure IS batch B3's task list.
5. Only when B3's storage layer compiles against the new schema: `git mv` the
   draft into `migrations/`, strip the DRAFT banner, start the app, re-verify.

**Note the ordering consequence:** B1's *file* is ready, but arming it requires
B3 to exist. Sequence the work B0 → B2 → B3 → arm B1 → B4…, not B1 first. This
contradicts "big-bang schema up front" in the literal sense; big-bang refers to
*one migration doing all the schema at once*, which it still does.

---

## Batch B2 — Types (no schema dependency; can land before B1 is armed)

### Task B2.1: The `Capability` enum

**Files:**
- Create: `src/agents/capability.rs`
- Modify: `src/agents/mod.rs` (add `pub mod capability;`)
- Test: in-file `#[cfg(test)] mod tests`

**Step 1: Write the failing test**

```rust
#[test]
fn hands_preset_matches_todays_hands_only_tools() {
    // Parity: the seeded HANDS role must grant exactly the tools
    // HANDS_ONLY_TOOLS lists today, no more and no less.
    let hands = CapabilitySet::preset_hands();
    for tool in ["ask_user_choice", "mark_awaiting_user", "request_approval",
                 "action_gate", "supersede_question", "disposition_finding",
                 "override_reviewer_block", "halt", "declare_working",
                 "terminal_exec"] {
        assert!(hands.allows_tool(tool), "HANDS must still allow {tool}");
    }
    for tool in ["eyes_flag", "approve_finding"] {
        assert!(!hands.allows_tool(tool), "HANDS must not gain {tool}");
    }
}

#[test]
fn capability_dependencies_are_enforced() {
    // GatedBash without RunBash is incoherent: you cannot gate a command you
    // cannot run. CloseSession without ReadChannel closes what it cannot see.
    let bad = CapabilitySet::from_slugs(&["gated_bash"]);
    assert!(bad.validate().is_err());
    let bad2 = CapabilitySet::from_slugs(&["close_session"]);
    assert!(bad2.validate().is_err());
}

#[test]
fn session_policy_is_the_ceiling() {
    // A role may be MORE restricted than the session allows, never less.
    let role = CapabilitySet::from_slugs(&["run_bash", "gated_bash", "edit_files"]);
    let ceiling = CapabilitySet::from_slugs(&["run_bash", "gated_bash"]);
    let effective = role.intersect(&ceiling);
    assert!(!effective.contains(Capability::EditFiles));
    assert!(effective.contains(Capability::RunBash));
}
```

**Step 2: Run to verify it fails** — `cargo test --lib capability` → FAIL,
module does not exist.

**Step 3: Implement** `Capability` (16 variants per the design),
`CapabilitySet` (grants only), `required_for(tool) -> Option<Capability>`,
`validate()` (dependency table), `intersect()`, and the two presets.

**Step 4: Verify** — `cargo test --lib capability` → PASS.

**Step 5: Commit** — "feat: add the capability model".

### Task B2.2: `ParticipantId` + `Role`/`Participant` structs

Mirror B2.1's shape. `Author` is NOT deleted here — it is demoted to a legacy
read-mapping in B4. Deleting it now would cascade 147 call sites into one
unreviewable commit.

---

## Batch B3 — Storage layer

### B3a — new tables ✅ DONE (`edf59aa`)

`src/storage/participants.rs`: roles, participants, cursors, deliveries, plus
the turn helpers (`next_active_participant`, `set_done_vote`,
`clear_done_votes`, `all_active_voted_done`). 7 tests.

**Test-harness discovery worth keeping:** `sqlx::migrate!` embeds `migrations/`
at **compile time**, so these tables do not exist in a stock `Storage::memory()`
until 0044 is armed — and arming it before the runbook's backup + boot check is
exactly what the runbook forbids. The tests therefore apply the reviewed draft
to the in-memory pool via `sqlx::raw_sql`. Side benefit: **the draft is now
proven to apply through sqlx, not just the `sqlite3` CLI** — which is the path
the real migration takes, and something the CLI dry-run could not establish.
Delete the `storage_with_0044()` helper once 0044 is applied.

### B3b — the `messages` sweep ⛔ NOT DONE, and it is the real blocker

`messages` reads/writes still select `author`. Switching them to
`participant_id`/`origin` changes `insert_message(author: Author)`'s signature,
which cascades into **`Author`'s 147 call sites** — that is B4's Author
demotion, not a query sweep. So:

**Revised sequencing:** B3b and B4 are one unit of work and must land together.
Arming 0044 comes after them, not after B3.

**Parity checkpoint (still required):** after B3b/B4, re-run B0's oracle — it
must be green, and `activity_events` must still record a row per transition.

---

## Batch B4 — Runtime rekey

`SessionHandle { brian, rain: Option }` → `participants: HashMap<ParticipantId,
AgentHandle>`; `ActivityTracker { brian_busy, rain_busy }` → per-participant
map; health/liveness/context-meter rekeyed (already keyed by
`(session, agent)` strings — a narrower change than it looks). `Author` demoted
to legacy read-mapping here.

**Parity checkpoint:** B0's oracle green, and `activity_events` still records a
row per transition.

---

## Batch B5 — Channel transport + turn sequencer

The largest batch, and the one that deletes `core/router.rs`.

**Order within the batch matters:**
1. Build the channel (cursors, delivery records, `PersistedMessage` newtype).
2. Build the turn sequencer (fixed ring, done-votes, consensus halt, parked-question
   preemption).
3. Port each router behaviour the B0.3 inventory marked **preserved**, with its
   test, BEFORE deleting anything.
4. Delete `core/router.rs` and the hold/flush family only when the inventory has
   a green test or a written "consciously dropped" verdict for every entry.

**The invariant, enforced by types not discipline:** a participant's input
sender is private; the only public entry takes a `PersistedMessage` whose
constructor is private to storage and produced solely by the row insert. "Wire
without a row" must fail to compile. Add the `system` participant here so the
six invisible injections (peer prefix, phase envelope, Apply-entry nudge,
reconcile directive, idle nudge, spawn prompt) become rows.

**This batch changes client-visible behaviour once** — serialisation. Expect
B0's oracle to still pass (it pins authorization, not concurrency); add one new
test asserting the cycle is serial.

---

## Batch B6 — Policy

Collapse `jsonrpc.rs:208-224`'s three name-equality gates into
`participant.capabilities.contains(required_for(tool))`. Add the relationship
constraints: `disposition_finding` requires `finding.author != caller`;
`approve_finding` requires `finding.author == caller`; `override_reviewer_block`
requires every `FileFinding` holder to be dead/stalled/absent. `close_session`
gains its capability, closing the ungated gap (CL issues #5).

**Parity checkpoint:** B0.1 and B0.2 must pass **unchanged** — this batch is
where a subtle authorization regression would hide.

---

## Batch B7 — Spawn derivation

`spawn.rs`'s two hand-written branches become derivations from capabilities:
`EditFiles` absent → deny-list; `GatedBash` → inject the PreToolUse hook;
`mcp_scope` → which servers. Prompt composed as core rules +
capability-derived rules + role description, then snapshotted onto the
participant.

---

## Batch B8 — UI: roster + Roles tab

Roster: per participant — name, model, turn position, whether it holds the turn,
cursor lag, capability set, last vote. Roles tab: capability editor with
`requires`/`conflicts`, the three guards (completeness, self-review, "explain
what this configuration means"). Session create: N agents (default 1), role per
agent, warning past ~5.

312 frontend agent-name occurrences make this a rewrite of the session view, not
a patch.

---

## Honest scoping note

B0–B3 are specified at task granularity because they are well understood and
their risk is concentrated. **B4–B8 are specified at batch granularity on
purpose:** writing exact code for them now would be fiction — their call sites
depend on types that do not exist yet, and a plan that invents them would be
confidently wrong, which is worse than a plan that says so. Re-run
`superpowers:writing-plans` for each of those batches when its predecessor
lands.

## Definition of done for the whole redesign

1. B0's parity oracle green, unchanged, at every checkpoint.
2. The router inventory has a verdict on all 822 lines of encoded behaviour.
3. A default session behaves as today's duo — same tools, gates, commit cycle,
   surfaces — with serialisation the single recorded exception.
4. A grep audit proves every read of `sessions.{brian,rain}_*` is dead.
5. The five gates green on every commit.
