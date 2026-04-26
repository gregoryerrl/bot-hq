# Arc-pointer discipline

**Status:** convention (Phase H slice 2 H-11)
**Applies to:** all `docs/arcs/*.md` open arcs

## Rule

Deferred items in an open arc.md must cite a **named consumer event** as the unblock trigger — never a count-based heuristic.

## Rationale

Count-based heuristics ("revisit in 3 weeks", "after 50 dispatches", "once we've shipped 4 features") are unfalsifiable in practice. They drift past their numeric threshold without anyone noticing because no specific event surfaces the deadline. The Phase G slice-3-trigger (msg 3000-era) sat past its count-target until a named consumer event finally fired the deferral.

A **named consumer event** is something an outside observer can detect happening. When the named event fires, the deferred item moves from "deferred" to "ready". Observers, sentinels, or Rain's doc-drift watcher can detect the event mechanically; humans don't have to remember.

## Examples

✅ **Good — named events:**
- "deferred until first force-push hits production"
- "deferred until ≥3 sentinels in flight" (named scope-threshold; mechanical)
- "deferred until next phase opens (per phase-close ratchet)"
- "deferred until Emma calibration data exists post-tuning-gate"
- "deferred until hub message-id watermark passes 5000" (mechanical scope-threshold)
- "deferred until the next bcc-ad-manager bootstrap" (named consumer event — the bootstrap will fire when user wants to dispatch into that project)

❌ **Bad — count-based heuristics:**
- "deferred ~3 weeks" (count-based, no triggering observation)
- "revisit later" (no trigger at all)
- "TODO when we have time" (no trigger)
- "after some user feedback" (vague — not named)
- "if it becomes a problem" (only fires if someone notices and remembers)

## Edge cases

**Time-bounded deferrals are allowed if they reference a named time-anchored event:**
- ✅ "defer until next quarterly review" (named event — the review)
- ❌ "defer 90 days" (count-based)

**The line between count-based and named-scope-threshold:** a count-based heuristic relies on someone tracking the count and remembering. A named scope-threshold is mechanical (a sentinel, a CI check, or a build step can detect when N is reached). Both use numbers, but only the second is consumable by automation.

## Closure discipline interaction

Per `feedback_arc_closure_discipline.md`, closed arc.md files are append-only. H-11 applies to **open** arcs — once closed, refining deferred-pointer text retroactively is not allowed. Refinement happens at next-arc-open if the original text was suboptimal.

## Enforcement

V1: reviewer-side (Rain BRAIN-review on arc.md edits should flag count-based deferrals).
V2 candidate (slice 4): pre-commit hook scanning `Status: open` arcs for count-based deferral patterns.

## Refs

- arc: `docs/arcs/phase-h.md` (H-11 item)
- master design: `docs/plans/2026-04-26-phase-h-design.md`
- slice 2 design: `docs/plans/2026-04-27-phase-h-slice-2-design.md`
- Phase G slice-3-trigger lesson: hub msgs around the slice-3 deferral that sat past its count
