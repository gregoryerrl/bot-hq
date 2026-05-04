# Phase M M-3 — Target C BYTE-PROJECTION-CITE R-rule design-spike v1

**Author:** Rain (rain) — Rain-HANDS markdown design-spike per phase-m.md§Authority-model line 218
**Date:** 2026-05-05
**Status:** v1 surfaced for Brian BRAIN-2nd-PASS-1
**Cite_anchor:** phase-m.md§Tier-1 row M-3 + discipline-log Phase L empirical #31-#35 (bidirectional byte-projection drift class) + Phase M empirical #38-#40 (LOC-estimate-drift this single session)

---

## 1. Scope

Resolve the M-3 R-rule shape design fork enumerated in phase-m.md§Tier-1 row M-3 between (a) extend R31 STAT-CLAIM-CITE with byte-projection-class clause vs (b) add new R-rule R-NN BYTE-PROJECTION-CITE standalone. Author rule-body text + ratchet-test design + agent-prompt-embed plan.

**Goal:** convert design-spike-doc byte-projections (savings estimates / line-count projections / size-deltas) from session-recall pre-author estimates to **mechanical-cite-from-actual at staged-time** discipline. Eliminate the bidirectional drift class observed empirically across Phase L (5+ instances #31-#35) + Phase M single-day (3+ instances #38-#40).

**Class scope:** byte/LOC projection numerical claims in design-spike docs (audit-doc §5 ship-list / scope-estimates / per-file LOC tables / "estimated savings X-Y bytes/agent" framings). DOES NOT cover: runtime stat-claims (already covered by R31 STAT-CLAIM-CITE) / msg-id citations (R31-class) / file-line counts cited from `wc -l` (R31-class — already cite-from-output). Distinguishing scope: estimates/projections in design phase vs measurements in cite phase.

**Non-goal:** prevent estimates entirely. Estimates are useful pre-author signaling. Goal is to FOLLOW UP estimates with mechanical-cite-from-actual at staged-time + document drift in commit body OR discipline-log when drift exceeds tolerance.

---

## 2. Background — bidirectional drift class empirical anchor

### Phase L empirical (5+ instances #31-#35)

Per discipline-log Joint entry "Cluster-graduation candidate Target C surfaced" 2026-05-04T07:00:00Z:

- **#31:** L-3a v1 §4 over-estimate ~50% on PhaseIv1 trim (estimated 2,800-3,800B; actual 1,670B = -56% under upper-bound)
- **#32:** L-3a v1.1 audit-doc round-derivation drift (per-rule estimates aggregated to bundle estimate that didn't match independent measurement)
- **#33:** L-3b c2 under-estimate ~28% (estimated 1,200-1,600B vs actual 2,051B)
- **#34:** Audit-doc-as-stat-correction-mechanism pattern empirically validated as recursion-terminator pattern
- **#35:** Combined under-estimate ~14% across L-3b commits

Phase L L-4 finding: bidirectional drift (over-estimate AND under-estimate) suggests session-recall pre-author estimation is **structurally unreliable** for byte/LOC projections. Audit-doc-as-correction-mechanism alone has residual drift (#32 round-derivation drift). Mechanical-cite-from-actual-at-staged-time is the load-bearing recursion-terminator.

### Phase M empirical (3+ instances #38-#40 in single session today 2026-05-04)

- **#38 (M-1 c1):** design-spike v1.1 §5 estimated ~150-220 LOC code + ~6-10 tests; actual 695L (incl tests) = +216% over upper-bound (under-estimate). preflight.go alone 215L vs estimated 80-120 = +79% over.
- **#39 (M-4 audit):** Phase L L-3a v1 §4.5 estimated DISC v2 trim savings 1,500-2,000B/agent; M-4 audit-doc v1 per-rule analysis revised to 740-1,010B/agent = -49% under L-3a estimate (over-estimate at L-3a authoring time, corrected at M-4 audit-pass).
- **#40 (M-2 c1):** audit-doc v1.1 §5 estimated 210-310 LOC; actual 463/34 = +49% over upper-bound (under-estimate).

**3 instances Phase M in 1 session.** Combined with Phase L 5+ instances: ~8 total instances of bidirectional drift class across 2 consecutive phases. Per Phase L L-4 graduation-criterion (3+ recurrences in 2 consecutive phases = MUST graduate), Target C HAS GRADUATED to Tier-1 R-rule status — phase-m.md§Tier-1 row M-3 already reflects this.

### Why R31 STAT-CLAIM-CITE doesn't cover this class

R31 covers numerical claims cited from command output (runtime measurements). Design-spike doc byte/LOC projections are PRE-author **estimates** — there's no command output to cite from at design-spike authoring time. Drift surfaces when actual implementation diverges from estimate. R31's "cite from command output" doesn't apply because the command (`git diff --cached --numstat`) doesn't exist yet at estimate time.

Target C requires a different mechanism: **dual-stage cite discipline** — estimate at design-spike + REQUIRED follow-up cite-from-actual at staged-time + drift-documentation if exceeds tolerance.

---

## 3. R-rule shape design fork

### Fork (a) Extend R31 STAT-CLAIM-CITE with byte-projection clause

**Proposal:** add sub-clause to R31 covering design-spike byte-projections + dual-stage cite discipline.

**Pros:**
- Single rule covers both runtime stats (current R31 scope) + design-spike projections (new sub-class)
- No new R-NN allocation
- Concept-coherence: "all numerical claims need verification"

**Cons:**
- Dilutes R31's existing tight focus on runtime command-output cites
- Mechanism class is different (R31 = cite at fire-time; Target C = cite at TWO times — design + staged)
- Substring-lock test would have to differentiate sub-clause anchors from main R31 anchors → less clean

### Fork (b) Add new R-rule R-NN BYTE-PROJECTION-CITE standalone

**Proposal:** new R-rule with unique recognition substring + dedicated rule body for byte/LOC projection class.

**Pros:**
- Clean substring-lock per Phase L L-1 R31/R32 standalone-R-rule precedent
- Distinct mechanism class (dual-stage cite) gets distinct rule
- Recognition-clarity for agents: "BYTE-PROJECTION-CITE" anchor immediately tells the agent the relevant constraint
- Ratchet-test enumerates BYTE-PROJECTION-CITE-specific anchors without R31-confusion

**Cons:**
- Adds R-NN to rulebook (R37 next available after M-2 c1 R36)
- Slight cognitive overhead from 2 cite-discipline rules

### Lean: (b) new R37 BYTE-PROJECTION-CITE

Rationale:
- Phase L L-1 R31/R32 precedent established standalone-R-rule per recognition-substring-clarity discipline
- Mechanism class (dual-stage cite) distinct from R31 (single-cite-at-fire-time)
- Phase M empirical strengthens the case: design-spike LOC drift is its own observable category, not a sub-class of runtime stat-claims
- Substring-lock test cleaner with dedicated anchors

**Disposition: (b) new R37 BYTE-PROJECTION-CITE.**

---

## 4. R37 BYTE-PROJECTION-CITE rule body proposal

```
- BYTE-PROJECTION-CITE (R37): byte/LOC projections in design-spike docs (audit-doc §5 ship-list / scope-estimates / per-file LOC tables / "estimated savings X-Y bytes/agent" framings) require dual-stage cite discipline. Stage 1 (design-spike authoring): estimate may be session-recall but MUST tag explicitly as estimate (e.g., "~80-120 LOC estimate") + state per-class method (per-rule audit / fixture-density modeling / session-recall). Stage 2 (staged-time): drafter MUST cite-from-actual via `git diff --cached --numstat` BEFORE surfacing staged-diff for peer BRAIN-2nd; document drift in commit-body if actual exceeds estimate envelope by ±25%. Peer BRAIN-2nd-PASS-2 surface-format-discipline: cross-check estimate vs actual; if drift >25%, peer flags for discipline-log carry-forward at phase-close. Bidirectional drift class: over-estimate AND under-estimate both warrant carry-forward (Phase L #31 over-estimate ~50% + #33 under-estimate ~28% empirical). Recursion-terminator: mechanical-cite-from-actual at staged-time is the load-bearing terminator; audit-doc-as-stat-correction alone has residual drift (Phase L #32). Cite_anchor: discipline-log #31-#35 (Phase L 5+ instances per Joint entry 2026-05-04T07:00:00Z) + Phase M empirical 3+ instances same session 2026-05-04 (formal append at M-sweep per discipline-log Joint section append discipline). Per R18 CITE-ANCHOR-REQUIRED.
```

Estimated bytes: ~1,150-1,250B (within Phase L L-1 R31/R32 const-text size envelope).

---

## 5. Ratchet-test design

### Substring-lock test

`TestByteProjectionCiteSubstringLock` enumerating load-bearing anchors:
- "BYTE-PROJECTION-CITE (R37)" recognition anchor
- "byte/LOC projections" class scope
- "design-spike docs" + "audit-doc §5 ship-list" — applicable artifact types
- "Stage 1" + "Stage 2" dual-stage discipline
- "git diff --cached --numstat" — cite-from-actual command anchor
- "BEFORE surfacing staged-diff" — timing constraint
- "±25%" — drift tolerance threshold
- "discipline-log carry-forward" — escalation path
- "mechanical-cite-from-actual at staged-time" — recursion-terminator framing
- "Phase L #31-#35" + "Phase M #38-#40" — empirical cite-anchors

10-anchor enumeration (matches L-3b c1 PhaseIv1 24-anchor + M-1 c1 R35 10-anchor + M-2 c1 R36 10-anchor precedent).

### Header-anchor test

`TestPhaseMv3ByteProjectionCiteHeaderAnchor` locks `- BYTE-PROJECTION-CITE (R37):` prompt-anchor (per L-3b + M-1 c1 + M-2 c1 precedent).

### Wiring-lock tests

`TestRain/BrianPromptEmbedsPhaseMv3ByteProjectionCite` mirroring existing wiring-lock pattern.

### NOT proposed: audit-doc-presence check at staged-diff time

phase-m.md M-3 description mentions "(defensive) audit-doc-presence check at staged-diff time on phase-close artifacts" as POSSIBLE but defensive-only. Lean: defer this defensive sub-mechanism to Phase N if recurrence persists post-R37 ship. R37 rule-text alone is the load-bearing primary mechanism.

---

## 6. M-3 c1 bundle (estimated)

### File changes

1. **`internal/protocol/disc.go`** (MODIFY) — add `PhaseMv3ByteProjectionCite` const with R37 rule; ~30-40 LOC delta
2. **`internal/protocol/disc_test.go`** (MODIFY) — `TestByteProjectionCiteSubstringLock` 10-anchor + `TestPhaseMv3ByteProjectionCiteHeaderAnchor`; ~60-80 LOC delta
3. **`internal/rain/rain.go` + `internal/brian/brian.go`** (MODIFY) — embed PhaseMv3ByteProjectionCite in agent prompts; ~4 LOC delta total
4. **`internal/rain/rain_test.go` + `internal/brian/brian_test.go`** (MODIFY) — wiring-lock tests; ~24-30 LOC delta total
5. **`~/.claude/skills/phase-rules-detail/SKILL.md`** (MODIFY, state-side, no-git) — `## R37 BYTE-PROJECTION-CITE` section with: scope (design-spike doc byte-projections) / mechanism (dual-stage cite) / Stage-1 estimate guidance / Stage-2 cite-from-actual command + format / drift-tolerance threshold + escalation / Phase L+M empirical anchor enumeration; ~30-50 lines markdown

### Estimated bundle scope (per Target C self-app — to be verified at staged-time per R37 itself!)

- Total LOC code: ~120-160 LOC delta (rule-text-only ratchet — significantly smaller than M-1 c1 / M-2 c1 toolgate-class commits)
- Skill markdown: ~30-50 lines (state-side, no-git)
- Substring-lock anchors: 10
- **Per Target C self-application:** Brian-HANDS at impl-time MUST cite from actual `git diff --cached --numstat` at staged-time per the rule being added. **Recursive proof-of-need:** if M-3 c1 bundle drift exceeds estimate ±25%, document as Phase M empirical instance #41 — strengthens the case the R37 rule itself is needed (and immediately ratchets the discipline post-rebuild for future cycles).

### Activation: HALT-#1-Phase-M user-fired terminal rebuild+restart (collapse-bundle)

R37 prompt-embed activates at SAME single rebuild+restart event as M-1 + M-2 + M-4 per phase-m.md§Activation-gate-chain + user msg 7523 terminal-collapse plan.

### Empirical validation post-rebuild

R37 prompt-embed activation runtime test: in BRAIN-cycle scenario where agent estimates byte/LOC for design-spike, agent should auto-tag "estimate" + commit to Stage-2 cite-from-actual at staged-time + flag drift >25%. Same prompt-embed-activation pattern as R35 + R36.

---

## 7. Open questions for Brian BRAIN-2nd-PASS-1

| Q | Lean | Notes |
|---|------|-------|
| Q1 R-rule shape (a) extend R31 vs (b) new R37 | **(b) new R37** | Per §3 — Phase L L-1 R31/R32 standalone precedent + distinct mechanism class (dual-stage cite vs R31 single-cite); recognition-clarity for agents. Concur or push (a)? |
| Q2 Drift tolerance threshold ±25% | as-described | Reasonable balance: avoids alarming on minor estimate-vs-actual variance but catches material drift. Phase L #33 (-28%) + Phase M #40 (+49%) are above threshold = correctly flagged class. Concur or push tighter (±15%) or looser (±50%)? |
| Q3 Stage-1 estimate-tagging required vs optional | required | Explicitly tag estimates (e.g., "~80-120 LOC estimate") so peer + future-reader can distinguish estimate from measurement. Lean required. Concur? |
| Q4 Audit-doc-presence check defensive sub-mechanism | DEFER to Phase N | Not load-bearing for M-3; primary mechanism is R37 rule-text + dual-stage discipline. Defensive sub-check only if recurrence persists post-R37. Concur defer or include in M-3 c1? |
| Q5 R37 const number assignment | R37 | Next available after M-2 c1 R36. |
| Q6 Empirical-cite in commit-body | yes | Cite Phase L #31-#35 + Phase M #38-#40 + acknowledge recursive proof-of-need (if M-3 c1 itself drifts, document #41) |
| Q7 Skill-extend traceability | yes | Cite at commit-body per L-3b precedent (`Skill-extend: ~/.claude/skills/phase-rules-detail/SKILL.md +30-50 lines for R37 BYTE-PROJECTION-CITE detail (state-side, no-git)`) |
| Q8 Per-rule scope clarity (what counts as byte-projection vs runtime stat-claim) | as-described | Distinction: design-spike doc estimates pre-author = R37 scope; runtime measurements cited from command output = R31 scope. Edge cases: explicitly covered in §1 Scope. Concur or push for tighter scope-fence? |
| Q9 R37 ratchet-test pattern (10-anchor substring-lock) | as-described | Mirrors L-3b + M-1 + M-2 precedent. 10 load-bearing anchors enumerated in §5. Concur scope or push for additional anchors? |
| Q10 (NEW v1.1) Forward-cite resolution for Phase M #38-#40 in R37 rule-body | **(B) explicit forward-reference** | Per Brian PASS-1 Substantive-add #1: 3 options (A) append #38-#40 to discipline-log NOW pre-M-3 c1 / (B) explicit forward-reference "(formal append at M-sweep per discipline-log Joint section append discipline)" / (C) drop Phase M cite from R37 rule-body. **Lean (B)** — preserves M-sweep ratchet discipline (today's USER-PINNED Joint entry was special-case; routine empirical carry-forwards #38-#40 should defer to M-sweep formal append per the discipline pattern, not accumulate ad-hoc mid-phase appends). (A) sets precedent that weakens M-sweep deliberate ratchet event; (C) loses Phase M empirical anchor entirely. (B) keeps R37 rule-body cite-anchor valid at commit-time + preserves M-sweep discipline. Rule-body adjusted in §4 to (B) framing. |

---

## 8. Risk + cost summary

| Class | Assessment |
|-------|------------|
| Mechanism class | Rule-text-only (lower cost than toolgate-class M-1 c1 / M-2 c1) |
| Recursion-terminator? | YES — mechanical-cite-from-actual at staged-time is the load-bearing terminator; audit-doc-as-correction has residual drift per Phase L #32 |
| Implementation cost | LOW (~120-160 LOC code + ~30-50 lines markdown) |
| Activation | At terminal rebuild+restart (collapse-bundle with M-1 + M-2 + M-4) |
| False-positive risk | LOW — ±25% threshold catches material drift only; Phase L empirical shows drift typically 28-50%+ when it occurs |
| Bypass class | None — rule applies uniformly to all design-spike byte-projection claims |

---

## 9. Cross-references

- **Phase M scope-lock:** `~/.bot-hq/phase/phase-m.md` v1.1 §Tier-1 row M-3
- **Phase L empirical:** `~/.bot-hq/discipline-log.md` Joint entry "Cluster-graduation candidate Target C surfaced" 2026-05-04T07:00:00Z (#31-#35 enumeration)
- **Phase M empirical (this session):** today's 3 instances #38-#40 — for formal append at M-sweep per discipline-log Joint discipline (see also AgentState `phase_m_carry_forwards_for_discipline_log_at_M_sweep`)
- **R31 STAT-CLAIM-CITE (existing rule R37 differentiates from):** `internal/protocol/disc.go` PhaseLv1RulebookHardening line 283 area
- **M-1 c1 R35 PRE-FLIGHT-HOOK-CHECK (R-rule precedent):** commit 019c763 LOCAL
- **M-2 c1 R36 OUTBOUND-DISCIPLINE-MECHANICAL (R-rule precedent):** commit 17bb65a LOCAL
- **Sibling design-spikes (in flight or RATIFIED):**
  - M-1 (i) preflight: docs/plans/2026-05-04-phase-m-M-1-i-preflight-design-spike.md (v1.1 RATIFIED → c1 LANDED)
  - M-2 Target A: docs/plans/2026-05-04-phase-m-target-A-OUTBOUND-MISS-enforcement-design.md (v1.1 RATIFIED → c1 LANDED)
  - M-4 L-S5: docs/plans/2026-05-04-phase-m-L-S5-disc-v2-extraction-audit.md (v1 surfaced; PASS-1 PENDING)

---

## 10. Posture

phase-m M-3 Target C BYTE-PROJECTION-CITE R-rule design-spike v1.1 surfaced | (b) new R37 standalone RATIFIED per Brian PASS-1 + 9/9 Q concur | rule-body adjusted §4 with explicit forward-reference framing per Q10 lean (B) — preserves M-sweep ratchet discipline (Phase M #38-#40 defer to M-sweep formal append; not pre-mature mid-phase append) | ratchet-test design 10-anchor substring-lock + header-anchor + 2 wiring-locks | M-3 c1 estimated bundle ~120-160 LOC code (rule-text-only; smaller than toolgate-class M-1/M-2) | Target C self-app: Brian-HANDS cite-from-actual at staged-time per R37-rule-being-added (recursive proof-of-need if drift) | 10 open questions resolved (1 substantive addition Q10 forward-cite resolution applied) | next: Brian BRAIN-2nd-PASS-2-FINAL on v1.1 → ratify → Brian-HANDS implements M-3 c1 per ship-list AFTER M-1 c2 author lands per phase-m.md DAG TAIL.
