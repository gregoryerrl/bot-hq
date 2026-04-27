# Per-slice runtime test cadence (P-1)

**Status:** convention (Phase H slice 2 P-1)
**Origin:** Phase H slice 1 runtime test methodology (msgs 3327, 3336, 3343, 3345)
**Codified in:** slice 2 design `docs/plans/2026-04-27-phase-h-slice-2-design.md` §P-1

## Rule

> **P-1 (process):** Every slice closes with a deliberate runtime test before slice closure declaration (the final cap ff-merge). Test plan included in slice design doc; results recorded in arc decision-log. Cadence floor: ≥1 success-path + ≥1 fail-path **per load-bearing surface**. Block paths must additionally verify side-effect-freeness via observable state check (e.g., `claude_list`, `tmux ls`, fs inspection) — not by code-reading or structural inference.

## Why

Unit tests verify code correctness in isolation. Runtime tests verify the binary actually exhibits the intended behavior under real conditions: bootstrapped session state, configured filesystem, MCP wiring, model weights, and the prompts that landed during the slice's commits.

Slice 1 (msgs 3327→3347) demonstrated the value: code-passing-unit-tests still surfaced a real form-coupling bug in <30s of runtime exercise (rules SSH-form vs actual HTTPS-form). Catching this *before* slice 2 design opened meant the hot-fix landed orthogonal to subsequent scope. P-1 ratchets that discipline into Phase H standing operating procedure.

## Workflow

For each slice:

1. **During design:** identify the load-bearing surfaces of the slice. List them in the slice design doc with a per-surface runtime-test plan covering success-path, fail-path, and (for block paths) side-effect-freeness check method.

2. **During implementation (C-series):** prompt-class surfaces verified at commit-time via file-grep / unit tests (the file IS the runtime artifact — no separate post-rebuild step needed). Code-class surfaces ratchet at unit-test layer for in-process invariants.

3. **Pre-closure (post-final-implementation-commit):**
   - ff-merge C-series implementation commits to main
   - User fires rebuild to deploy the slice's binary changes
   - Brian executes the runtime test plan: ≥1 success-path + ≥1 fail-path per load-bearing surface
   - For each block path, Brian (or Rain) verifies side-effect-freeness via observable state check (`claude_list`, `tmux ls`, fs inspection)
   - Rain EYES independently verifies each surface's verdict

4. **Closure declaration:** results recorded in arc decision-log. Joint Brian+Rain PASS verdict required before C7b (the final closure-cap commit) ff-merges as the slice closure declaration. Slice's status row in the arc's slices table flips to CLOSED.

## What is a "load-bearing surface"

A surface is **load-bearing** if a runtime regression in it would invalidate the slice's promise to the user. Per slice 1's experience:

- Each gate (slice 1's H-14 had 5 paths: rules-located, remote-compared, mismatch-blocks, missing-rules-bootstraps, all-match-emits-conditional-preamble) → each path is a surface
- Each prompt block whose presence/absence changes agent behavior → one surface per block
- Each new code path that produces externally-observable state (file writes, hub messages, tmux sessions) → one surface
- Each sentinel pattern → one surface (success = pattern matches; fail = non-match doesn't trigger)

Non-load-bearing surfaces (NOT required by P-1):
- Refactors with no behavior change
- Internal helpers with full unit-test coverage
- Doc-only changes
- Pure deletion of dead code

## Cadence floor

**≥1 success-path + ≥1 fail-path per load-bearing surface.** Floor — slices may exceed it. Slices with high blast-radius surfaces should add more cases.

The "≥1 success" + "≥1 fail" pairing is critical: a passing success-path alone proves the happy path works, but doesn't prove the failure path correctly fails (a permissive bug could pass everything). Both directions verify the gate's discrimination.

## Block-path side-effect-freeness

For surfaces that **block** an action (e.g., gates, error returns, refusals), the success-of-blocking must also verify no partial side effects leaked. Examples:

- A blocked `hub_spawn` must not create a tmux session, a worktree, or an MCP registration.
- A blocked file write must not create the file (or must clean up partial writes).
- A refused tool call must not log noise or perturb queue state.

**Verification must be empirical** — observable state check via `claude_list`, `tmux ls`, fs inspection, hub message-log query, etc. Code-reading or structural inference is NOT sufficient. The implementation can be argued to not have side effects; the runtime check confirms.

This refinement was added per Rain BRAIN-review msg 3354: bare "verified empirically" was tightenable to closed-loop observable-state lookups so future-Rain-or-Brian can't claim "verified empirically" by just reading the source.

## Suggested test methodology

For code-surface tests post-rebuild:

```
Brian:
  1. Identify the surface from slice design's surface table
  2. Construct minimal success-path input (deliberate happy case)
  3. Execute via the actual MCP/CLI/hub path the user would use
  4. Capture the observed outcome
  5. Repeat with fail-path input
  6. For block paths: capture observable state before + after + diff = side-effect free?

Rain (independent verify):
  1. Re-run the same test from a different context (gemma analyze, direct file read, etc.)
  2. Confirm the outcome matches Brian's claim
  3. For block paths: spot-check the observable state independently
  4. Render verdict: surface PASS or surface FAIL
```

Joint PASS = both Brian and Rain confirm. Either FAIL = hot-fix before closure.

## Decision-log recording

Slice closure-cap commit (C-final, doc-only) appends an entry to the arc's decision-log:

```
- 2026-MM-DD — Slice N runtime test PASS (joint Brian+Rain). Surfaces verified live:
  - <surface 1>: success-path ..., fail-path ..., side-effect check ...
  - <surface 2>: ...
  Slice N CLOSED at <SHA>.
```

This becomes the audit trail for which surfaces were exercised and how. Future-self reviewing the arc can reconstruct what "PASS" meant at slice close.

## Self-application

P-1 self-applies to slice 2. The slice 2 design's §"P-1 self-application" section enumerates the 7 load-bearing surfaces (4 prompt + 3 code) with per-surface success/fail plans. Prompt surfaces verified at C1/C2/C4 commit-time; code surfaces (H-29 / H-22 / H-23) verified post-rebuild #14 before C7b cap.

## Acceptance class taxonomy (gold vs silver)

Runtime tests fall into two classes by the path they exercise. Both are valid P-1 acceptance evidence; the distinction matters for closure transparency and for tracking when later slices need to ratchet a surface's coverage.

### Gold (live cross-process)

Exercises the full production runtime path including any cross-process or cross-session boundary the surface depends on. For a sentinel surface, gold means: hub_send (from a different process than the monitor) → DB row insert → in-process polling/callback fires → SentinelMatch → dispatch → ledger or hub-emit. The test is run against the actual installed binary in a representative invocation context, not via `go test`.

Gold-class tests are the strongest acceptance signal because they catch wiring defects that unit tests can miss by construction (e.g., process-local callback lists not firing for cross-process inserts — slice 2's H-22 acceptance gap, msg 3424).

### Silver (unit-test direct-invocation)

Exercises the surface at the function-level under filesystem and environment isolation (e.g., `t.Setenv("BOT_HQ_HOME", ...)` redirects the ledger path so production state is not touched). The production code paths run; only the I/O surface is redirected for test hygiene.

Silver is the right class when:
- The surface has no live periodic invoker yet (e.g., slice 2's H-23 docdrift sentinel deferred its periodic invoker to slice-3 backlog #6 by design — so the only gate at slice 2 is "ledger-emit when invoked").
- The surface's invoker is itself the gold-tested infrastructure (transitive confidence — see below).

Silver alone is weaker than gold; silver paired with gold-class verification of the surrounding infrastructure is **transitive-gold-equivalent**.

### Transitive confidence

Silver-class unit tests gain transitive gold-confidence when adjacent gold-class runtime tests exercise a shared path or related sentinel surface. Pattern:

- Silver test A exercises function-level correctness for surface X under FS-redirect
- Gold test B exercises the same dispatcher / ledger primitive end-to-end for surface Y
- Joint signal: surface X inherits transitive confidence in the shared infrastructure being correct, leaving only X's function-level logic as the new acceptance gate

Slice 2 worked example: H-23 (silver) inherits transitive confidence from H-22 (gold) because both share the `SentinelMatch` dispatcher, the `dispatchSentinelHit` shape, the `AppendToDryRunLedger` filesystem semantics, and the dryRun-gate primitive. Tests 1/2/4 (gold) end-to-end-verify that dispatch chain; tests 5/6 (silver) only need to verify H-23's function-level scan logic on top.

When a later slice adds a periodic invoker for a previously-silver surface, the gold-equivalent gate re-arms at that closure. Document the planned re-arm in the originating slice's closure decision-log entry.

### Decision-log shape

Slice closure decision-log entries should mark each surface's class explicitly:

```
- 2026-MM-DD — Slice N runtime test PASS (joint Brian+Rain).
  - Surface 1 (gold): success-path ..., fail-path ..., side-effect check ...
  - Surface 2 (silver): unit-test runtime via go test ...; transitive gold via Surface 3
  - Surface 3 (gold): ...
  Slice N CLOSED at <SHA>.
```

This audit trail lets a future reader reconstruct not just "which surfaces were exercised" but "by what class of evidence" — and identify which silver-class surfaces still owe a gold-equivalent gate at a future slice's closure.

## Refs

- arc: `docs/arcs/phase-h.md` (P-1 item)
- slice 2 design: `docs/plans/2026-04-27-phase-h-slice-2-design.md` (§P-1 self-application)
- methodology origin: hub msgs 3304 (Rain test-first rec), 3327 (slice 1 path 1 result), 3336 (slice 1 path 2 result), 3343 (path 1 retry GREEN), 3345 (Rain double-PASS slice 1 verdict)
- closure-discipline interaction: `feedback_arc_closure_discipline.md` (closed arcs append-only; this convention applies to open-arc slice closures)
