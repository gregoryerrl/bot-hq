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

## Refs

- arc: `docs/arcs/phase-h.md` (P-1 item)
- slice 2 design: `docs/plans/2026-04-27-phase-h-slice-2-design.md` (§P-1 self-application)
- methodology origin: hub msgs 3304 (Rain test-first rec), 3327 (slice 1 path 1 result), 3336 (slice 1 path 2 result), 3343 (path 1 retry GREEN), 3345 (Rain double-PASS slice 1 verdict)
- closure-discipline interaction: `feedback_arc_closure_discipline.md` (closed arcs append-only; this convention applies to open-arc slice closures)
