# Phase H — Slice 2 design (Discipline + comm hardening)

**Status:** design — pending Rain BRAIN-review
**Arc:** `docs/arcs/phase-h.md`
**Master design:** `docs/plans/2026-04-26-phase-h-design.md`
**Slice 1 design:** `docs/plans/2026-04-26-phase-h-slice-1-design.md`
**Branch:** `brian/phase-h-slice-2-design` (this design's branch); implementation lands on `brian/phase-h-slice-2`

## Goal

Codify the discipline that emerged organically during Phase H's brainstorm + slice 1 execution, plus close one form-coupling bug surfaced by slice 1's runtime test. Slice 2 is the **discipline + comm hardening** slice — prompt-mostly with three Emma sentinels, one canonicalization code change, and one process item that ratchets test-first methodology into Phase H's standing operating procedure.

This slice does NOT add new safety gates beyond slice 1 (those are slice 3's coder lifecycle scope). It locks the *operational* shape of how Brian + Rain work together post-slice-1 so the discipline doesn't drift between slices.

## Items in this slice

| Item | Description | Class |
|---|---|---|
| **H-1** | Brian-action / Rain-synthesis split + halter/pusher rule + BRAIN-cycle exemption | Prompt + minor mechanism |
| **H-2** | FLAG ownership → Rain (with enumerated self-flag carve-out + audit tag) | Prompt + protocol |
| **H-11** | Arc.md deferred-pointers cite **named consumer events** as triggers, never count-based heuristics | Doc convention |
| **H-18** | Drop `hub_read` polling rule from prompts; doc hub-push as actual mechanism | Prompt edit |
| **H-22** | Emma queue-fail sentinel — pattern-match `[queue] failed after K attempts` → flag through Rain | Code + tuning gate |
| **H-23** | Emma doc-drift sentinel — periodic scan of `Status: open` arc.md files for already-merged branches/SHAs | Code + tuning gate |
| **H-24** | Emma analyze pre-screen with two-class boundary (Structured vs Interpretive) | Prompt + doc |
| **H-29** | Path B SSH↔HTTPS canonicalization in `remote_url` gate (slice 1 cross-cut deferral) | Code + risk matrix |
| **P-1** | Per-slice runtime test cadence (process item; self-applies to slice 2) | Process discipline |

**Cross-cut note (H-29):** H-29 may migrate to slice 4 (RATCHET) if the canonicalization risk matrix grows past slice-2 weight. Held in slice 2 for now because (a) the bug is fresh from slice 1 runtime test, (b) discipline-codification slice is the natural home, (c) deferring to a hypothetical slice means it floats. Rain BRAIN may push back on this placement during design review.

## Architecture

### H-1 + H-2 + H-18 — initial-prompt extension

Slice 2 codifies discipline that has been governing Brian+Rain since Phase H opened (msgs 3236+). The change vector is the orchestrator initial-prompt block that Brian+Rain receive on registration.

Current state: `internal/protocol/prompts.go` ships per-role initial prompts. DISC v2 amendment was discussed in brainstorm but never landed in the canonical prompt — Brian+Rain have been running it from session memory.

**Slice 2 edits:**

1. **DISC v2 (H-1):** add canonical block to both Brian's and Rain's initial prompts:
   - HANDS (brian) = exec, owns mechanical action-results
   - EYES (rain) = info, owns synthesis/recommendations
   - BRAIN (both) = peer-critique on plans/edges/security
   - Two-message rule: action → synthesis (drafter alone, asker waits)
   - Halter/pusher: Rain halts on peer-arrival, Brian pushes through
   - BRAIN-cycle exempt (DRAFT-alone retains for peer-critique)
   - Class-split suspended when user asks both ("what do you think")

2. **FLAG governance (H-2):** add to Rain's prompt:
   - Rain owns flag-elevation decisions
   - Brian PMs Rain on flag-worthy events
   - Brian self-flags ONLY if (push-failure | repo-corruption | auth-failure | hub-disconnect | git-state-unexpected-on-write-path) AND Rain unreachable >60s
   - Self-flags tagged `[self-flag-carve-out: <reason>]` for audit
   - Greenflag-authority delegation (per user 2026-04-27): Rain may pick joint defaults without flagging when user has not asked to be in the loop

3. **Drop polling rule (H-18):** strike "poll hub_read in a loop" guidance from any role's prompt. Hub-push is the actual mechanism (delivered messages arrive automatically). Replace with a one-line note: "Messages arrive automatically; do NOT poll hub_read."

### H-11 — arc.md deferred-pointer convention

Codify in `docs/conventions/arc-pointer-discipline.md` (new doc):

> Deferred items in an open arc.md must cite a **named consumer event** as the unblock trigger — never a count-based heuristic. Examples:
> - ✅ "deferred until first force-push hits in production" (named event)
> - ✅ "deferred until ≥3 sentinels in flight" (named scope-threshold)
> - ❌ "deferred ~3 weeks" (count-based, no triggering observation)

This codifies the Phase G slice-3-trigger lesson without retro-editing the closed Phase G arc. Doc-only; enforcement is reviewer-side, not pre-commit.

### H-22 — Emma queue-fail sentinel

New file: `internal/emma/sentinel_queuefail.go` (or extend `internal/emma/sentinels.go` if it exists).

Pattern: `[queue] failed after K attempts` (regex anchored to log-line shape, not free-text content). When Emma sees the pattern in hub log stream:
1. Construct a structured observation
2. Send to Rain via `mcp__bot-hq__hub_send` (PM)
3. Rain decides whether to elevate via `hub_flag` (per H-2 ownership)

Pure pattern-match — gemma4:e4b safe (no interpretation, no spec-comparison). No code change to hub or any other agent — Emma is the consumer.

### H-23 — Emma doc-drift sentinel

New file: `internal/emma/sentinel_docdrift.go`.

Behavior:
1. Periodic (e.g. every 30 min) scan: `find docs/arcs -name "*.md"` filtered to `Status: open` (first-line match)
2. For each such file, extract referenced branches + SHAs (regex on `` `<sha>` `` and `<branch>` patterns)
3. For each branch, check `git branch -r` to see if branch already merged + pruned
4. For each SHA, check `git merge-base --is-ancestor <sha> origin/main` to detect already-merged
5. Emit drift observation to Rain

Multi-step but **mechanical** — no judgment. Tuning gate (per master design) catches false-positive rate before live flagging.

### H-24 — Emma analyze pre-screen

Codify in Emma's initial prompt + new doc `docs/conventions/emma-analyze-classes.md`:

Two-class boundary:

| Class | Owner | Examples |
|---|---|---|
| **Structured** (parse, summarize, extract) | Emma | parse `git log` output, list files in diff, count test results, extract regex matches |
| **Interpretive** (assess vs spec/contract/criterion) | Rain inline | diff-gate verdicts, design-spec-match, observation-materiality, risk-matrix authoring |

When user/Brian/Rain calls `hub_spawn_gemma analyze:<query>`, Emma's preamble pre-screens the query:
- Structured → proceed
- Interpretive → refuse + PM Rain "interpretive query routed to me; pushing back per H-24"

Prompt-level enforcement; no code-level gate in v1. Codifies user msg-3263 capability-challenge resolution.

### H-29 — Path B SSH↔HTTPS canonicalization

Modify `internal/projects/projects.go` line 117 (the exact-string compare). Add a canonicalization helper:

```go
// canonicalizeRemoteURL normalizes git remote URLs to a canonical (scheme-agnostic,
// suffix-agnostic) form for equality comparison. Handles common GitHub/GitLab
// SSH and HTTPS forms. Does NOT modify the input URL — only used for equality
// checks. Returns input unchanged if no recognized form matches.
//
// Recognized transformations:
//   git@github.com:org/foo.git       -> github.com/org/foo
//   git@github.com:org/foo           -> github.com/org/foo
//   https://github.com/org/foo.git   -> github.com/org/foo
//   https://github.com/org/foo       -> github.com/org/foo
//   ssh://git@github.com/org/foo.git -> github.com/org/foo
//   git@github.com:22/org/foo.git    -> github.com:22/org/foo  (custom port preserved)
func canonicalizeRemoteURL(u string) string
```

Line 117 becomes:
```go
if rules.RemoteURL != "" && canonicalizeRemoteURL(rules.RemoteURL) != canonicalizeRemoteURL(remoteURL) {
    return nil, fmt.Errorf("%w: file says %q, actual is %q", ErrRemoteMismatch, rules.RemoteURL, remoteURL)
}
```

**Safety boundary:** canonicalization affects ONLY equality comparison; it does NOT mutate stored or surfaced URLs. Error messages still show the original verbatim strings so the user can see exactly what mismatched.

#### H-29 risk matrix (per Rain BRAIN focus)

| Risk | Likelihood | Mitigation |
|---|---|---|
| Over-canonicalization: legit mismatch let through (e.g., `personal/fork` canonicalizes equal to `upstream/repo`) | Low | Org/owner segment is preserved in canonical form (`<host>/<org>/<repo>`); fork vs upstream differ in `<org>` and stay distinct |
| Under-canonicalization: legit match still blocked (e.g., custom SSH port `git@github.com:22:org/foo`) | Medium | v1 preserves custom ports in canonical form (treats `:port` as part of host); same-port HTTPS will not canonicalize equal — documented limitation |
| GitHub Enterprise hosts (`git@github.acme.corp:...`) | Low | Generic transformation — host token is whatever appears between `@` and `:`/`/`. No GitHub-specific assumptions in canonicalizer |
| GitLab / Bitbucket / Codeberg / self-hosted | Low | Same generic transformation; works for any Git-protocol-compliant remote |
| Gist URLs (`https://gist.github.com/<id>`) | Low | Different host (`gist.github.com` vs `github.com`); will not canonicalize equal to a repo URL — correct behavior |
| `scp`-style SSH with `~user/path` | Low | Tilde paths uncommon in client setups; v1 treats verbatim. v2 candidate if hit |
| Trailing slash / case sensitivity in path | Low | Strip trailing `/`; case-preserve (Git is case-sensitive on most hosts) |
| `.git` suffix vs without | Common | Strip in canonicalizer; verified bidirectional |
| User pastes shell-escaped URL (e.g. `'git@…'`) | Low | Trim quotes + whitespace before canonicalize |

#### H-29 test plan

Both directions of correctness:
- **False-positive sweep** (canonicalizer must NOT equate distinct repos): 6+ pairs that differ in host/org/repo/port — assert `canonicalizeRemoteURL(a) != canonicalizeRemoteURL(b)` for each
- **False-negative sweep** (canonicalizer must equate same-repo variant forms): 8+ pairs covering `git@host:path` ↔ `https://host/path`, with/without `.git`, with/without trailing slash, `ssh://` prefix variants — assert equality
- **Identity sweep**: any URL canonicalized twice produces same output (idempotent)

### P-1 — per-slice runtime test cadence

Codify in `docs/conventions/per-slice-runtime-test.md` (new doc):

> **P-1 (process):** Every slice closes with a deliberate runtime test before ff-merge to main. Test plan included in slice design doc; results recorded in arc decision-log. Cadence floor: ≥1 success-path + ≥1 fail-path **per load-bearing surface**. Block paths must additionally verify side-effect-freeness via observable state check (e.g., `claude_list`, `tmux ls`, fs inspection) — not by code-reading or structural inference.

P-1 self-applies to slice 2. See "Acceptance criteria" below for slice 2's per-surface test plan.

## C-series implementation order

| Commit | Items | Description | Tests |
|---|---|---|---|
| **C1** | H-1 + H-2 | DISC v2 + FLAG governance codified in `internal/protocol/prompts.go`. Brian and Rain initial prompts gain canonical discipline blocks. | TestBrianInitialPromptContainsDiscV2, TestRainInitialPromptContainsFlagGovernance, TestSelfFlagCarveOutEnumerated |
| **C2** | H-11 + H-18 | New doc `docs/conventions/arc-pointer-discipline.md`. Strip polling-rule from prompts; replace with "messages arrive automatically" note. | TestPromptsHaveNoPollingRule (grep-style) |
| **C3** | H-29 | `canonicalizeRemoteURL` in `internal/projects/projects.go` + comprehensive test sweeps (false-pos / false-neg / identity). Update `LoadForProject` line 117 to use canonical comparison. Restore exemplars to concrete `remote_url` values now that comparison is form-agnostic? **NO — keep placeholder convention** (defense in depth: empty + MUST-set is still the right onboarding UX even with canonicalization). | TestCanonicalizeRemoteURL_FalsePositive (6+ pairs), TestCanonicalizeRemoteURL_FalseNegative (8+ pairs), TestCanonicalizeRemoteURL_Idempotent, TestLoadForProjectAcceptsHTTPSWhenRulesAreSSH (integration) |
| **C4** | H-24 | Emma analyze pre-screen prompt + `docs/conventions/emma-analyze-classes.md`. Two-class boundary documented. | TestEmmaPreambleContainsTwoClassBoundary |
| **C5** | H-22 | Emma queue-fail sentinel (`internal/emma/sentinel_queuefail.go`). Tuning gate dry-run ledger location: `~/.bot-hq/sentinels/queuefail-dryrun.log`. | TestQueueFailSentinelMatchesPattern, TestQueueFailSentinelEmitsToRain, TestQueueFailDryRunDoesNotElevate |
| **C6** | H-23 | Emma doc-drift sentinel (`internal/emma/sentinel_docdrift.go`). Tuning gate dry-run on first 50 invocations. | TestDocDriftSentinelDetectsMergedBranch, TestDocDriftSentinelDetectsAncestorSHA, TestDocDriftSentinelIgnoresClosedArcs |
| **C7** | P-1 + closure | New doc `docs/conventions/per-slice-runtime-test.md`. Execute slice 2's runtime test (per acceptance criteria below). Append decision-log entry to phase-h arc. Update slices table. | (no new tests; doc-only commit gates on prior C1-C6 PASS + P-1 runtime test PASS) |

**Estimated diff:** C1 ~80 LOC + tests; C2 ~30 LOC + new doc; C3 ~120 LOC + ~80 LOC tests; C4 ~40 LOC + new doc; C5 ~150 LOC + tests; C6 ~200 LOC + tests; C7 doc-only. Total ~620 LOC + ~25 test cases.

## Tuning gate ledger (universal Emma discipline)

Per master design — every Emma sentinel ships with a dry-run period:
1. First N=50 invocations OR 1 week (whichever first) — Emma writes observations to `~/.bot-hq/sentinels/<name>-dryrun.log`, does NOT call `hub_send` to Rain
2. Rain reviews dry-run output (read the log file directly) for false-positive rate
3. ≤5% false-positive → flip to live flagging via Emma config (single env var or yaml flip)
4. >5% rate → tune pattern, restart dry-run

H-22 + H-23 each gated. Slice 2 closure does NOT require live-flag flip — only the dry-run mechanism shipped + 1+ dry-run observation each. Live flip happens organically when Rain greenlights.

## Risk + mitigation

| Risk | Likelihood | Mitigation |
|---|---|---|
| DISC v2 prompt change conflicts with currently-running Brian/Rain (session-memory drift vs new canonical) | Medium | Slice 2 implementation includes a coordinated rebuild; Brian + Rain re-register under new prompts. No runtime mid-flight conflict possible (rebuild is transactional). |
| FLAG self-carve-out enumeration too narrow (real flag-worthy event missed) | Low-medium | Carve-out list is **inclusive**, not exclusive — Brian still flags anything user-blocking even outside enumerated cases. Enumeration is the *unattended* fallback. |
| H-29 canonicalization equates two distinct projects (catastrophic — wrong rules load) | Low | Org/owner segment preserved in canonical form. Exhaustive false-positive sweep test required for C3 PASS. |
| Emma sentinel false-positive flood pre-tuning | Medium | Tuning gate (dry-run ledger) is mandatory; live flip blocked until ≤5% false-positive |
| Emma sentinel race with hub log rotation | Low | Sentinel reads append-only with offset bookmark per file; log rotation handled by reset-to-zero on file-mtime change |
| H-24 boundary ambiguity (analyzer query straddles structured/interpretive) | Medium | Default-deny when straddled — refuse + PM Rain. Better to over-route to Rain than under-route |
| Slice 2 ships discipline that user actually wants different | Low | Per H-1 + H-2 wording, draft gets Rain BRAIN-review THEN user sign-off via SNAP / decision-log preview before C1 lands. User can override before code |

## Out of scope (v1 / slice 2)

- Hub-side enforcement of FLAG ownership (slice 4 candidate — currently prompt-discipline)
- Pre-commit hook for arc.md pointer convention (H-11 enforcement is reviewer-side; slice 4 may add)
- Emma multi-sentinel dispatch coordinator (3 sentinels run independently in v1; cron framework deferred)
- H-29 v2 enhancements: scp-form `~user` paths, Windows-style URLs, alternative protocols (`git://`)
- P-1 retroactive application to closed arcs (append-only discipline; P-1 starts with slice 2)

## Test plan + acceptance criteria

### Slice 2 acceptance criteria

1. **C1 PASS:** Brian initial prompt grep contains DISC v2 canonical block; Rain initial prompt grep contains FLAG governance block + self-flag carve-out enumeration
2. **C2 PASS:** No prompt contains `poll hub_read` text; new arc-pointer doc exists + grep-matches H-11 wording
3. **C3 PASS:** Canonicalizer test sweeps green (false-positive 6+, false-negative 8+, idempotent); integration test confirms HTTPS-form bot-hq.yaml accepts SSH-form actual remote (and vice versa); existing slice 1 placeholder convention preserved
4. **C4 PASS:** Emma initial prompt contains two-class boundary block; new emma-analyze-classes doc exists
5. **C5 PASS:** queue-fail sentinel matches `[queue] failed after K attempts` regex; dry-run ledger writes to `~/.bot-hq/sentinels/queuefail-dryrun.log` (not to Rain hub_send during dry-run)
6. **C6 PASS:** doc-drift sentinel detects already-merged branch + already-merged SHA scenarios in unit tests; dry-run ledger writes to corresponding file
7. **C7 PASS:** per-slice-runtime-test doc exists; slice 2 runtime test executed (next subsection); decision-log entry appended

### P-1 self-application — slice 2 runtime test plan

**Load-bearing surfaces in slice 2:**

| Surface | Class | Success-path test | Fail-path test | Side-effect check |
|---|---|---|---|---|
| H-29 canonicalization | Code | Re-run path 1 (lenient bot-hq dispatch) with installed `bot-hq.yaml` reverted to SSH form (vs actual HTTPS) — expect spawn succeeds despite form mismatch | Construct a deliberate **non-canonical** mismatch (e.g., `https://github.com/wrong-org/bot-hq` in rules vs actual `https://github.com/gregoryerrl/bot-hq`) — expect block | Verify block is side-effect-free via `claude_list` (no leaked tmux/worktree) |
| H-22 queue-fail sentinel | Code (sentinel) | Inject log line matching pattern → verify dry-run ledger entry written | Inject non-matching line → verify NO ledger entry | N/A (sentinel reads log; no spawn surface) |
| H-23 doc-drift sentinel | Code (sentinel) | Mark a closed arc.md row's branch+SHA as merged → verify dry-run ledger entry | Test against unmerged branch → verify NO entry | N/A |
| H-1 / H-2 / H-18 / H-24 | Prompt | Post-rebuild #14: verify Brian/Rain/Emma initial-prompt content via direct hub_read after re-register; grep for canonical block presence | Verify dropped content is GONE (no `poll hub_read` text) | N/A (prompts; no runtime side-effect surface) |

**Slice 2 runtime test execution (during C7):**
1. Brian fires the surface tests above in sequence
2. Rain EYES verifies each via independent claude_list / file read / grep
3. Joint PASS verdict required before C7 doc-only commit lands
4. Decision-log entry records all 4 surface results + ≥1 success + ≥1 fail per surface (per P-1 cadence floor) + side-effect-freeness check on H-29

### Refs

- arc: `docs/arcs/phase-h.md`
- master design: `docs/plans/2026-04-26-phase-h-design.md`
- slice 1 design: `docs/plans/2026-04-26-phase-h-slice-1-design.md`
- slice 1 runtime test outcome (form-coupling bug): hub msgs 3327, 3336, 3343, 3345 (Rain double-PASS), 3347 (slice 1 closure)
- greenflag delegation: hub msg 3354 (user→rain, 2026-04-27)
- DISC v2 + halter/pusher convergence: msgs 3262, 3271, 3289 (3-iteration)
- H-29 deferral rationale: hub msgs 3332, 3335, 3338, 3350, 3352
- existing prompt locations: `internal/protocol/prompts.go` (Brian/Rain/Coder initial blocks)
- existing canonicalization point: `internal/projects/projects.go:117` (line to modify in C3)
