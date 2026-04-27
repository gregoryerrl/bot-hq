# Verify cadence ratchet (H-9)

**Status:** convention (Phase H slice 3 H-9)
**Origin:** Slice 2 P-1 (per-slice runtime test) + slice 3 R2 (gold/silver acceptance taxonomy) + slice 3 C5/C6 self-incident discipline (msg 3725)
**Codified in:** slice 3 completion design `docs/plans/2026-04-27-phase-h-slice-3-completion-design.md` §C8

## Rule

> **H-9 (process):** Verification is mandatory at three cadence layers. None is skippable as a fatigue concession. Skipping silver-class on a risk-bearing surface is a regression of slice 2's hard-won quality gate — silver acceptance is for *genuinely* mechanical/doc items only.
>
> 1. **Per-commit, pre-Rain-handoff (Brian self-check):** Brian inspects diff + builds + runs targeted tests before sending a coder commit OR HANDS-direct commit to Rain for diff-gate. Self-check catches obvious-mechanical regressions without consuming Rain queue depth or token churn. Brian states GREEN claim explicitly when forwarding to Rain.
> 2. **Per-commit, Rain diff-gate:** Rain reviews each commit independently — code shape, test coverage, scope discipline. Uses `git show` / `git diff` / `git log -p` only — never `git checkout origin/branch -- files` (per `feedback_review_git_inspection.md`). Trusts Brian's GREEN claim for test execution; verifies code-claim independently.
> 3. **Per-slice, pre-closure (P-1 runtime test):** every slice closes with deliberate runtime test before slice closure declaration. Test plan in slice design doc; results in arc decision-log. Cadence floor: ≥1 success-path + ≥1 fail-path per load-bearing surface. Block paths verify side-effect-freeness via observable state check (`claude_list`, `tmux ls`, fs inspection) — not by code-reading or structural inference.

## Why three layers

Each layer catches a different failure class:

- **Layer 1 (Brian self-check)** catches typos, broken builds, missing imports, regressions in obvious-mechanical commits. Cheap to run; saves Rain from re-discovering the same. The slice 3 C6 M1 silent-error-swallow finding (msg 3725) would have been a Brian self-check catch had the discipline been ratcheted earlier — that incident is the load-bearing motivation for codifying Layer 1 here.
- **Layer 2 (Rain diff-gate)** catches design-doc deviations, scope creep, non-obvious correctness issues, test-coverage gaps. Rain operates with fresh eyes on each commit; Brian's self-check is necessarily biased toward what was just written.
- **Layer 3 (P-1 runtime test)** catches binary-vs-source divergence, prompt-class regressions, MCP wiring gaps, side-effect leaks not reachable from in-process unit tests. Slice 1's form-coupling bug (rules SSH-form vs actual HTTPS-form) is the canonical example — code passed unit tests; runtime test caught it in <30s.

## Acceptance class taxonomy (slice 3 R2 lock)

Mark each surface's class in the design doc + closure decision-log:

- **Gold:** live cross-process runtime exercise. Required for risk-bearing surfaces (correctness-class, state-machine-class, security-class).
- **Silver:** unit-test direct-invocation under FS-redirect or mocked deps. Acceptable for genuinely mechanical/doc items only.
- **Silver-with-transitive-gold:** silver coverage on a surface whose shared infrastructure is independently gold-verified. The transitive-gold inheritance must be explicitly named in the closure decision-log.

Silver alone is weaker than gold. Never accept silver on a risk-bearing surface as a fatigue concession.

## Workflow per cadence layer

### Layer 1 (Brian self-check)

Before any `hub_send` to Rain announcing a commit ready for diff-gate:

1. `git diff --cached --stat` — verify expected files only
2. `go build ./...` — verify clean build
3. `go test ./<package>/...` for the touched package(s) — verify GREEN
4. (For HANDS-direct on small surfaces) re-read the diff one final time for typos / discarded errors / missing log calls
5. State GREEN claim explicitly when forwarding: "Full bot-hq test suite GREEN" or scoped equivalent

### Layer 2 (Rain diff-gate)

For each commit Brian forwards:

1. `git show <sha>` — full diff inspection; do NOT checkout
2. Verify scope matches design doc (no out-of-scope additions)
3. Verify test coverage matches class (gold = real cross-process; silver = mechanical with transitive-gold path explicit)
4. Verify Brian's GREEN claim is internally consistent (e.g., new test names match commit body claims)
5. Post PASS / fold-needed verdict with line-numbered findings
6. On fold-needed: Brian decides (a) HANDS-fold inline, (b) PM coder for revisions, (c) defer to slice closure backlog

### Layer 3 (P-1 runtime test, slice closure)

See `docs/conventions/per-slice-runtime-test.md` for the full P-1 workflow. H-9 ratchets the discipline; the SOP itself is unchanged.

## Anti-patterns

- "Just push it; the build is GREEN" without diff inspection (Layer 1 skip)
- "Trust the coder + skip diff-gate this once because it's a small fix" (Layer 2 skip)
- "Skip runtime test; we've gold-tested all the surfaces individually" (Layer 3 skip — misses prompt-class binary-vs-source divergence)
- "Silver-acceptable as a fatigue concession on this risk-bearing surface" (R2 violation)

Each anti-pattern represents a regression of slice 2's hard-won gate. Discipline costs less than re-fixing.

## Refs

- Slice 2 P-1 doc: `docs/conventions/per-slice-runtime-test.md`
- Slice 3 C1 cadence: `docs/conventions/agent-cadence-discipline.md`
- Slice 3 acceptance taxonomy: `docs/plans/2026-04-27-phase-h-slice-3-design.md` §R2
- Slice 3 C5/C6 self-incident motivation: msgs 3725 (Rain C6 M1 finding) + 3727 (Brian M1 fold)
- Memory: `feedback_review_git_inspection.md` (use git show, not checkout)
