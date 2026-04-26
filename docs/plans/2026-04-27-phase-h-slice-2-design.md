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

Slice 2 extends discipline already partly canonical in Brian+Rain initial prompts. The change vector is the existing DISC v2 blocks in:
- `internal/brian/brian.go:227` (`initialPrompt()`) — DISC v2 block at L240-254
- `internal/rain/rain.go:206` (`initialPrompt()`) — DISC v2 block at L219-228

`internal/protocol/` ships only `disc.go` / `constants.go` / `types.go` (single rules like `protocol.DiscV2OutboundRule` referenced by both prompts). Centralizing all prompt content in a `prompts.go` file is a structural refactor *out of scope* for slice 2 — slice 2 edits the existing prompt strings in place.

**Current canonical state (verified via grep):**
- DISC v2 baseline (HANDS/EYES/BRAIN/OUTPUT/DRAFT/FLAG/PIVOT/TRUST/SNAP/NUDGE) — ✅ already present in both Brian and Rain prompts
- Brian: "Messages arrive automatically. Don't poll hub_read in a loop." — ✅ already in `brian.go:237`
- Rain: "Then poll hub_read (no agent filter) every 5-10s." — ❌ still polling per `rain.go:209`
- Halter/pusher rule — ❌ NOT in either prompt (PIVOT block at brian.go:247 is a different scenario: user-w/o-executor 60s hold)
- FLAG self-flag carve-out enumeration — ❌ NOT in either prompt
- Greenflag-authority delegation (2026-04-27) — ❌ NOT in either prompt
- "Rain owns FLAG ownership" framing — ❌ current is symmetric (both say ALWAYS FLAG)

**Slice 2 edits (smaller than initial draft implied):**

1. **H-1 (halter/pusher addition):** add a single line after the existing `DRAFT:` line in each DISC v2 block:
   - Brian: `- HALTER/PUSHER: on peer-arrival, Rain halts, Brian pushes through. BRAIN-cycle exempt — DRAFT-alone discipline retains.`
   - Rain: same text mirrored. Mutual-halt deadlock impossible by construction (asymmetric rule).

2. **H-2 (FLAG governance asymmetry + carve-out + greenflag):** rewrite the existing `FLAG:` line in both prompts:
   - Asymmetric framing: "Rain owns flag-elevation decisions. Brian PMs Rain on flag-worthy events; Brian self-flags ONLY in enumerated carve-out cases."
   - Enumerate carve-out: `(push-failure | repo-corruption | auth-failure | hub-disconnect | git-state-unexpected-on-write-path)` AND `Rain unreachable >60s`
   - Audit tag: self-flags include `[self-flag-carve-out: <reason>]` prefix for ledger discipline
   - Greenflag delegation: "Per 2026-04-27 user delegation — Rain may pick joint defaults without flagging when user is not in the loop on the specific decision."

3. **H-18 (Rain polling rule drop):** edit `rain.go:209` — replace `"Then poll hub_read (no agent filter) every 5-10s."` with `"Messages arrive automatically; do NOT poll hub_read."` (matching Brian's existing phrasing). Brian's prompt is already correct — no edit needed there.

### H-11 — arc.md deferred-pointer convention

Codify in `docs/conventions/arc-pointer-discipline.md` (new doc):

> Deferred items in an open arc.md must cite a **named consumer event** as the unblock trigger — never a count-based heuristic. Examples:
> - ✅ "deferred until first force-push hits in production" (named event)
> - ✅ "deferred until ≥3 sentinels in flight" (named scope-threshold)
> - ❌ "deferred ~3 weeks" (count-based, no triggering observation)

This codifies the Phase G slice-3-trigger lesson without retro-editing the closed Phase G arc. Doc-only; enforcement is reviewer-side, not pre-commit.

### H-22 — Emma queue-fail sentinel

**Note:** Emma's package name is `gemma` (agent ID is "emma"; package retains historical naming). Sentinel framework already exists at `internal/gemma/sentinel.go` with `preFilterPatterns` + `alwaysFlagPatterns` regex slices + rate-cap/hysteresis via `Gemma.shouldFlag`. H-22 + H-23 extend this.

H-22 implementation (smaller than initial draft — leverages existing framework):
- Add regex `(?i)\[queue\]\s+failed\s+after\s+\d+\s+attempts?` to `preFilterPatterns` (and to `alwaysFlagPatterns` if Rain greenlights post-tuning-gate)
- Pattern is gemma4:e4b safe — pure regex shape, no interpretation
- Tuning gate (dry-run ledger, see Tuning gate section) gates the `alwaysFlagPatterns` promotion
- Existing `Gemma.shouldFlag` discipline (rate-cap, hysteresis) applies unchanged

### H-23 — Emma doc-drift sentinel

New file: `internal/gemma/sentinel_docdrift.go` (separate from `sentinel.go` because it's a periodic-scan class, not the hub-message-reactive class).

Behavior:
1. Periodic (e.g. every 30 min) scan: `find docs/arcs -name "*.md"` filtered to `Status: open` (first-line match)
2. For each such file, extract referenced branches + SHAs (regex on `` `<sha>` `` and `<branch>` patterns)
3. For each branch, check `git branch -r` to see if branch already merged + pruned
4. For each SHA, check `git merge-base --is-ancestor <sha> origin/main` to detect already-merged
5. Emit drift observation to dry-run ledger (then to Rain after tuning-gate flip)

Multi-step but **mechanical** — no judgment. Tuning gate (per master design) catches false-positive rate before live flagging.

### H-24 — Emma analyze pre-screen

Emma currently has no canonical "You are Emma" initial-prompt block — only a per-task analyze prompt at `internal/gemma/gemma.go:292`. H-24 ADDS the canonical block (not edits an existing one) plus a new doc `docs/conventions/emma-analyze-classes.md`:

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
| `http://` vs `https://` of same repo | N/A | Deliberate equivalence — canonicalizer strips scheme. Transport-security choice is out-of-band; the gate concerns project identity, not transport. Documented in canonicalizer godoc. |

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
| **C1** | H-1 + H-2 | Edit DISC v2 blocks in `internal/brian/brian.go` (L240-254) and `internal/rain/rain.go` (L219-228): add HALTER/PUSHER line; rewrite FLAG line for asymmetric ownership + carve-out enumeration + greenflag delegation. Tests assert SPECIFIC NEW substrings (not generic "DISC v2 present" — that already passes). | TestBrianPromptContainsHalterPusher, TestRainPromptContainsHalterPusher, TestBrianPromptContainsCarveOutEnumeration, TestRainPromptContainsCarveOutEnumeration, TestBrianPromptContainsGreenflagDelegation |
| **C2** | H-11 + H-18 | New doc `docs/conventions/arc-pointer-discipline.md`. Edit `rain.go:209` only — replace `"poll hub_read (no agent filter) every 5-10s"` with `"Messages arrive automatically; do NOT poll hub_read."` (matches Brian's existing phrasing at `brian.go:237`). Brian's prompt needs no edit. | TestRainPromptHasNoPollingRule (grep `poll hub_read every` returns no match) |
| **C3** | H-29 | `canonicalizeRemoteURL` in `internal/projects/projects.go` + comprehensive test sweeps (false-pos / false-neg / identity). Update `LoadForProject` line 117 to use canonical comparison. Restore exemplars to concrete `remote_url` values now that comparison is form-agnostic? **NO — keep placeholder convention** (defense in depth: empty + MUST-set is still the right onboarding UX even with canonicalization). | TestCanonicalizeRemoteURL_FalsePositive (6+ pairs), TestCanonicalizeRemoteURL_FalseNegative (8+ pairs), TestCanonicalizeRemoteURL_Idempotent, TestLoadForProjectAcceptsHTTPSWhenRulesAreSSH (integration) |
| **C4** | H-24 | ADD canonical "You are Emma" block to `internal/gemma/gemma.go` (Emma has no canonical block today — only per-task analyze prompt at L292). Block contains two-class boundary. + new doc `docs/conventions/emma-analyze-classes.md`. | TestEmmaCanonicalBlockExists, TestEmmaPromptContainsTwoClassBoundary |
| **C5** | H-22 | Add `(?i)\[queue\]\s+failed\s+after\s+\d+\s+attempts?` regex to `internal/gemma/sentinel.go` `preFilterPatterns`. Dry-run ledger location: `~/.bot-hq/sentinels/queuefail-dryrun.log`. Promotion to `alwaysFlagPatterns` blocked until tuning-gate green. | TestQueueFailRegexMatches, TestQueueFailNotInAlwaysFlagPreTuning, TestSentinelDryRunWritesToLedger |
| **C6** | H-23 | New file `internal/gemma/sentinel_docdrift.go` (periodic scan, distinct from hub-message-reactive `sentinel.go`). Tuning gate dry-run on first 50 invocations. | TestDocDriftSentinelDetectsMergedBranch, TestDocDriftSentinelDetectsAncestorSHA, TestDocDriftSentinelIgnoresClosedArcs |
| **C7** | P-1 + closure | New doc `docs/conventions/per-slice-runtime-test.md`. Append decision-log entry to phase-h arc. Update slices table. **Note:** code-surface runtime tests (H-29 / H-22 / H-23) require rebuild #14 to deploy slice 2 binary; user fires rebuild after C6 lands + before C7 ff-merge. Prompt-surface tests already covered at unit-test layer at C1/C2/C4. | (no new tests; doc-only commit gates on prior C1-C6 PASS + post-rebuild code-surface runtime test PASS) |

**Estimated diff:** C1 ~30 LOC + ~50 LOC tests (smaller — DISC v2 baseline already present, only delta lines + carve-out + greenflag added); C2 ~10 LOC edit + ~30 LOC new doc; C3 ~120 LOC + ~80 LOC tests; C4 ~50 LOC + new doc (Emma block ADDED, not edited); C5 ~30 LOC (regex + ledger plumbing — leverages existing `internal/gemma/sentinel.go` framework); C6 ~200 LOC + tests (new periodic-scan file); C7 doc-only. Total ~470 LOC + ~25 test cases (smaller than first draft due to existing scaffolding).

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

1. **C1 PASS:** Brian + Rain initial prompts each grep-match the new `HALTER/PUSHER:` marker, `self-flag-carve-out` enumeration substring, and `greenflag` delegation substring (NOT generic "DISC v2 canonical block" — that already passes today). Asymmetric FLAG framing replaces symmetric `ALWAYS FLAG. When in doubt, flag` line.
2. **C2 PASS:** Rain prompt grep `poll hub_read (no agent filter) every 5-10s` returns NO match; rain prompt grep `Messages arrive automatically; do NOT poll` returns 1 match. New `docs/conventions/arc-pointer-discipline.md` exists + grep-matches H-11 named-consumer-event wording.
3. **C3 PASS:** Canonicalizer test sweeps green (false-positive 6+, false-negative 8+, idempotent); integration test confirms HTTPS-form bot-hq.yaml accepts SSH-form actual remote (and vice versa); existing slice 1 placeholder convention preserved
4. **C4 PASS:** Emma initial prompt contains two-class boundary block; new emma-analyze-classes doc exists
5. **C5 PASS:** queue-fail sentinel matches `[queue] failed after K attempts` regex; dry-run ledger writes to `~/.bot-hq/sentinels/queuefail-dryrun.log` (not to Rain hub_send during dry-run)
6. **C6 PASS:** doc-drift sentinel detects already-merged branch + already-merged SHA scenarios in unit tests; dry-run ledger writes to corresponding file
7. **C7 PASS:** per-slice-runtime-test doc exists; slice 2 runtime test executed (next subsection); decision-log entry appended

### P-1 self-application — slice 2 runtime test plan

**P-1 ordering note:** prompts are data — the file IS the runtime artifact. So for prompt surfaces (H-1 / H-2 / H-18 / H-24), file-grep at unit-test layer at commit time IS the runtime-test analog; no post-rebuild step needed. Post-rebuild runtime test is restricted to **CODE surfaces** where runtime ≠ file (H-29 canonicalizer, H-22 + H-23 sentinels).

**Load-bearing surfaces in slice 2 — per-block, not bundled:**

| Surface | Class | Success-path | Fail-path | Side-effect check | When verified |
|---|---|---|---|---|---|
| H-1 (halter/pusher) | Prompt | Grep brian.go + rain.go for `HALTER/PUSHER` marker — present in both | (n/a — additive) | N/A | C1 commit-time |
| H-2 (FLAG asymmetry + carve-out + greenflag) | Prompt | Grep both prompts for `self-flag-carve-out` substring + `greenflag` substring | Grep both prompts for symmetric `ALWAYS FLAG. When in doubt, flag` framing — should be GONE / replaced | N/A | C1 commit-time |
| H-18 (Rain polling drop) | Prompt | Grep rain.go for `Messages arrive automatically; do NOT poll` | Grep rain.go for `poll hub_read (no agent filter) every 5-10s` — should return NO match | N/A | C2 commit-time |
| H-24 (Emma two-class) | Prompt | Grep gemma.go for `two-class` + `Structured` + `Interpretive` substrings | (n/a — additive) | N/A | C4 commit-time |
| H-29 canonicalization | Code | Re-run path 1 (lenient bot-hq dispatch) with installed `bot-hq.yaml` reverted to SSH form (vs actual HTTPS) — expect spawn succeeds despite form mismatch | Construct deliberate **non-canonical** mismatch (e.g., `https://github.com/wrong-org/bot-hq` in rules vs actual `https://github.com/gregoryerrl/bot-hq`) — expect block | Verify block is side-effect-free via `claude_list` (no leaked tmux/worktree) | C7 post-rebuild |
| H-22 queue-fail sentinel | Code (sentinel) | Inject log line matching pattern → verify dry-run ledger entry written | Inject non-matching line → verify NO ledger entry | (sentinel reads log; no spawn surface — no side-effect check applicable) | C7 post-rebuild |
| H-23 doc-drift sentinel | Code (sentinel) | Mark a closed arc.md row's branch+SHA as merged → verify dry-run ledger entry | Test against unmerged branch → verify NO entry | (no spawn surface) | C7 post-rebuild |

**Slice 2 runtime test execution (during C7 closure):**
1. **Prompt surfaces:** verified at C1/C2/C4 commit-time via test asserts (already in C-series tests above)
2. **Code surfaces (post-rebuild):** Brian fires H-29 + H-22 + H-23 surface tests in sequence after rebuild #14
3. Rain EYES verifies each code surface via independent claude_list / file read / log inspection
4. Joint PASS verdict required before C7 doc-only commit lands
5. Decision-log entry records all 7 surface results + ≥1 success + ≥1 fail per surface (per P-1 cadence floor) + side-effect-freeness check on H-29

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
