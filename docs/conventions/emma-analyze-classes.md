# Emma analyze-class boundary (H-24)

**Status:** convention (Phase H slice 2 H-24)
**Model:** gemma4:e4b (small, fast, no reasoning chain)
**Codified in:** `internal/gemma/gemma.go` (`canonicalEmmaBlock` const)

## Rule

When the orchestrator (Brian or Rain) dispatches an analyze task to Emma via `hub_spawn_gemma analyze:<query>`, the query must fall into the **Structured** class. Interpretive queries belong to Rain inline.

## Two-class boundary

| Class | Owner | Examples |
|---|---|---|
| **Structured** (parse, summarize, extract, count) | Emma | parse `git log` output, list files in diff, count test results, extract regex matches, summarize command stdout |
| **Interpretive** (assess vs spec / contract / criterion) | Rain inline | diff-gate verdicts, design-spec-match assessments, observation-materiality judgments, risk-matrix authoring, refactor-vs-feature classification |

## Why

gemma4:e4b is a small fast model. It is reliable for:
- Pulling structured tokens out of structured output
- Counting matches in a regex
- Summarizing log streams
- Pattern-matching against fixed strings

It is unreliable for:
- "Does this diff match the design intent?"
- "Is this observation material to the slice's acceptance criteria?"
- "Did the implementation honor the contract?"
- "Is this a refactor or a feature?"

These require holding the spec or contract in working memory, comparing against the artifact, and rendering a judgment. Rain handles those inline because they need a richer reasoning chain.

## Pre-screen behavior

Per `canonicalEmmaBlock` in Emma's prompt, when Emma sees an interpretive query:
1. Emma replies: `"interpretive query — routing back to Rain per H-24"`
2. Caller (Brian/Rain orchestrator) sees the refusal and re-routes the query to Rain inline

**Default-deny on straddled queries** — when the boundary is unclear, Emma refuses to Rain. Better to over-route (Rain handles a structured query) than under-route (Emma renders a brittle interpretive verdict).

## Examples

✅ **Structured (Emma OK):**
- "parse the test results from this `go test` output and count pass vs fail"
- "list every file path mentioned in this diff"
- "extract all SHAs that appear in this `git log` output"
- "summarize this stdout — flag any non-zero exit codes"
- "count occurrences of `panic` in this output"

❌ **Interpretive (refuse to Rain):**
- "is this diff faithful to the C2 design row?"
- "does this commit message match the imperative-mood convention?"
- "did the implementer follow the slice acceptance criteria?"
- "is this observation material to the diff-gate verdict?"
- "rate the risk of this refactor from 1-5"

⚠️ **Straddled (default-deny — refuse to Rain):**
- "summarize this diff" — could be structured (file list, line counts) or interpretive (intent rendering). Refuse + ask caller to be specific.
- "what changed in this commit?" — structured if "list the changed files", interpretive if "what was the change's purpose". Refuse.

## Calibration discipline

Per master design tuning gate, Emma's two-class boundary itself is monitored:
- If Emma refuses too often (e.g., >50% refusal rate), the boundary is too tight; relax via canonical-block edit
- If Emma renders brittle verdicts on interpretive queries (e.g., Rain catches >5% wrong-class verdicts in BRAIN-review), the boundary is too loose; tighten

Slice 2 ships the boundary at the conservative-deny baseline. Calibration data accumulates during slice 2/3 execution. H-26 (interpretive-class summarizer) was deferred at master-design scope-lock pending this calibration data.

## V1 enforcement

Prompt-level only — Emma's preamble contains the boundary; Emma's behavior depends on gemma4:e4b respecting the prompt. No code-level gate routes interpretive queries away before Emma sees them.

V2 candidate: caller-side (Brian / Rain orchestrator) pre-classifies queries before `hub_spawn_gemma` dispatch. Premature in v1; the prompt-level discipline produces the calibration data needed to design v2.

## Refs

- arc: `docs/arcs/phase-h.md` (H-24 item)
- master design: `docs/plans/2026-04-26-phase-h-design.md` (Slice 2 — Emma analyze pre-screen)
- slice 2 design: `docs/plans/2026-04-27-phase-h-slice-2-design.md` (§H-24)
- canonical block source: `internal/gemma/gemma.go` (`canonicalEmmaBlock` const)
- H-26 deferred: master design "Out of scope" / hub msg 3263 (capability challenge resolution)
