# Phase L L-3a — Prompt-Shrink Audit + Ship-List

**Phase:** L (Bot-HQ Hardening + Refactor)
**Task:** L-3a (per phase-l.md§Tier-shape line 91)
**Author:** Rain (HANDS-authorized for markdown design docs per phase-l.md§Authority-model)
**Authored:** 2026-05-04 (filename anchor 2026-05-02 = phase-lock date per phase-l.md§Output-files row L-3a line 186)
**BRAIN-cycle:** msg 7346 (Brian L-6-commit-1 LANDED + Task #7 Rain-HANDS-author-go) → this v1 surface
**Status:** v1 surfaced for Brian BRAIN-2nd
**Doc-only.** No commit, no rebuild.

---

## 1. Scope + methodology

L-3a audits the assembled `initialPrompt()` for `internal/rain/rain.go` + `internal/brian/brian.go` (emma is code-driven via `internal/gemma/`, not a single embedded prompt-string — out-of-scope for prompt-shrink).

Methodology:
1. Inventory every block contributing to the assembled prompt (inline literal + protocol-package consts).
2. Measure exact byte-size per block (via in-tree `len(InitialPromptForTest())` + per-const `len()`).
3. Classify each block per L-2 axis: **ALWAYS-INLINE** / **SKILL-RELOCATABLE** / **TOOLGATE-ENFORCED**.
4. Risk-rank shrink candidates (low / medium / high).
5. Ship-list low-risk relocations only.
6. Recommend: ship-L-3b this phase OR defer Phase M.

Cite-anchor for methodology: phase-l.md§Tier-shape L-3a line 91 (verbatim ALWAYS-INLINE / SKILL-RELOCATABLE / TOOLGATE-ENFORCED axis from reBRAIN-msg-7191 counter).

---

## 2. Current prompt sizes (R31-cited)

Measured via test-harness invocation of `(*Rain).InitialPromptForTest()` + `(*Brian).InitialPromptForTest()` (commit a7a4cbd state):

| Agent | Bytes | Approx tokens (÷4) |
|-------|-------|--------------------|
| Rain  | 19,413 | ~4,850 |
| Brian | 20,627 | ~5,150 |

**Baseline observation:** ~5K-token init-prompt per trio agent. For comparison, Anthropic recommended max system-prompt is hundreds of tokens to low-thousands for cache-efficiency; current bot-hq init-prompt is on the upper end for content-density without skill-relocation discipline applied uniformly.

---

## 3. Per-block inventory

### 3.1 Rain prompt blocks

| # | Block | Bytes | Source | Kind |
|---|-------|-------|--------|------|
| R1 | Intro line ("You are Rain...") | ~165 | rain.go:243 inline | ALWAYS-INLINE |
| R2 | STARTUP block | ~165 | rain.go:245 inline | ALWAYS-INLINE |
| R3 | REPLAY-CUTOFF block | ~310 | rain.go:247 inline | ALWAYS-INLINE |
| R4 | "RULES:" header + DiscV2OutboundRule | 596+~10 | const | ALWAYS-INLINE (recognition: "OUTBOUND" / "Backfill") |
| R5 | PhaseIv1ProtocolHardening | 7,913 | const | MIXED (see §4) |
| R6 | Inline FLAG/ROUTE/Review/Approve/Flag rules | ~700 | rain.go:252-256 inline | ALWAYS-INLINE (recognition: "FLAG ownership" / "Looks clean") |
| R7 | Inline "DISC v2 2026-04-24:" block | ~3,830 | rain.go:258-268 inline | MIXED (see §4) |
| R8 | PhaseJv1HaltResumeProtocol | 1,436 | const | ALWAYS-INLINE (load-bearing HALT recognition substrings) |
| R9 | PhaseLv1RulebookHardening (R31 + R32) | 1,711 | const | MIXED (see §4) |
| R10 | PhaseLv5GateProtocol (R33) | 1,915 | const | MIXED (see §4) |
| R11 | PhaseLv6PrePhaseCloseRetro (R34) | 2,042 | const | MIXED (see §4) |
| R12 | "Start now: register..." | ~37 | rain.go:277 inline | ALWAYS-INLINE |
| **TOTAL** | | **~20,620 incl whitespace+separators → 19,413 measured** | | |

### 3.2 Brian prompt blocks

Same structure as Rain with these deltas:
- B6 Inline rules: ~470 bytes (FLAG/DISPATCH/ROUTE/poll/Q-T-routing) — different content, similar size
- B7 Inline DISC v2: ~4,100 bytes (adds SNAP block, removes Rain-specific TRUST line) — slightly larger
- B11 H13ForcePushProtocol: 910 bytes (Brian-only; Rain not gated on force-push exec)
- All other blocks identical to Rain

---

## 4. Classification per L-2 axis

For each MIXED block, identify what stays inline vs relocates to skill.

### 4.1 PhaseIv1ProtocolHardening (7,913 bytes — largest single block, 40% of total)

**Current state:** const declares "Detail in /phase-rules-detail skill" header (line 32 disc.go) but const body still contains R1-R23 rule-text in 1-line-each form.

**phase-rules-detail SKILL.md exists at ~/.claude/skills/phase-rules-detail/SKILL.md (206 lines).** Skill body has full prose-detail per R1-R23. Const body has rule-name + recognition-substring + 1-line summary.

**Discriminator:** const-body 1-liners ARE the load-bearing recognition layer. Skill body is the prose-rationale layer. The trim was already done in Phase J T2.1-(d).

**Further trim candidate:** measure ratio of recognition-substring vs prose-summary in current 7,913 bytes. Spot-inspect (line 32 disc.go onward): each rule has 1-3 sentences post-recognition. Many of those sentences are skill-overlap (rationale, edge-cases, cite-anchors).

| Sub-class | Estimated bytes | Risk to trim |
|-----------|-----------------|--------------|
| Rule-name + recognition substring (per rule × ~23) | ~1,800 | HIGH — load-bearing |
| 1-line load-bearing summary (per rule) | ~2,300 | MEDIUM — disambiguator |
| Prose rationale beyond 1-line summary | ~2,800 | LOW — skill-relocated already |
| Cite-anchor msg-IDs | ~1,000 | LOW — full anchors in skill + phase-i.md arc |

**Estimated low-risk savings:** **~2,800-3,800 bytes** (35-50% of PhaseIv1 const) by stripping prose rationale + cite-anchors that already exist in `phase-rules-detail` skill body. Recognition substrings + 1-line summaries stay inline.

**Verification gap:** need to read full PhaseIv1 const to confirm overlap-vs-novel content. Not blocking v1 surface; flag for v1.1 audit-pass before ship-list lock.

### 4.2 Inline "DISC v2 2026-04-24:" block (~3,830-4,100 bytes)

**Current content:** HANDS / EYES / BRAIN / OUTPUT / DRAFT / HALTER/PUSHER / FLAG / PIVOT / TRUST / [SNAP for brian] / NUDGE — 10-11 sub-rules, each 1-3 sentences.

**Discriminator analysis:**
- Recognition substrings (load-bearing): "HANDS" / "EYES" / "BRAIN" / "DRAFT" / "HALTER/PUSHER" / "FLAG" / "PIVOT" / "TRUST" / "NUDGE" / "OUTPUT". These trigger role-self-identification + branching decisions on every turn.
- Prose detail: "(brian): exec. Owns git/edits, hub_spawn real coders..." — describes role; useful for new-session bootstrap but verbose.
- Cite-anchors: "Per 2026-04-27 user delegation..." / "msg 6396" — relocatable.

**Skill candidate:** new `~/.claude/skills/disc-v2-detail/SKILL.md` (or extend phase-rules-detail to include "DISC v2" section). Recognition substrings + 1-line role summaries stay inline; prose ownership-clauses + cite-anchors move to skill.

**Estimated low-risk savings:** **~1,500-2,000 bytes** (40-50% of DISC v2 block) by extracting prose ownership-clauses + cite-anchors. Recognition substrings + 1-line role summaries stay inline.

**Risk:** medium-low. DISC v2 is referenced multiple times in agent decision-flow (DRAFT-alone / HALTER-PUSHER / FLAG-ownership). Trim must preserve all role-recognition triggers + class-discrimination logic.

### 4.3 PhaseLv1RulebookHardening (R31 + R32, 1,711 bytes)

**Current state:** R31 STAT-CLAIM-CITE + R32 SCOPE-FORK-CONFIRMATION rules. Each has rule-name + recognition + 1-line summary + discriminator + cite-anchor.

**Trim analysis:**
- Recognition substrings ("STAT-CLAIM-CITE (R31)" / "SCOPE-FORK-CONFIRMATION (R32)") — ALWAYS-INLINE.
- 1-line summaries — ALWAYS-INLINE (load-bearing for self-application).
- Cite-anchor lists ("discipline-log #10/#13/#16/#17/#19/#20/#23 + 2026-04-30...") — SKILL-RELOCATABLE.
- "Recursive proof-of-need: amend-passes for prior stat-claim drift can themselves contain stat-claim drift; peer-cross-check at each amend-depth until L-5 toolgate gate-CHECK enforcement-conversion lands." — SKILL-RELOCATABLE (rationale).

**Estimated savings:** **~400-500 bytes** by relocating cite-anchors + recursive-proof-of-need rationale to phase-rules-detail skill or new `phase-l-rules-detail` skill. R31/R32 already partially TOOLGATE-ENFORCED (R33 toolgate from L-5 commit-2 enforces SHA-cite freshness; R31 stat-claim and R32 scope-fork are still PEER-CROSS-CHECK-only) — so trim risk slightly elevated for R31/R32 vs R33/R34.

**Risk:** medium. Don't trim until R31 has toolgate-enforcement (Phase M candidate).

### 4.4 PhaseLv5GateProtocol (R33, 1,915 bytes)

**Current state:** R33 PRE-EXECUTE-GATE-FILE-READ rule. TOOLGATE-ENFORCED (commit-2 e327362 lands gate-CHECK on commit/push/merge).

**Trim analysis:**
- Recognition substring ("PRE-EXECUTE-GATE-FILE-READ (R33)") — ALWAYS-INLINE.
- Gate-file path identifiers ("`pre-commit-checklist.md`" / "`pre-push-checklist.md`" / "`pre-merge-checklist.md`") — ALWAYS-INLINE (load-bearing for class-routing).
- Cite-format anchors ("`Pre-commit-checklist-SHA: <sha256>`" / "`pre_push_checklist_sha_seen`" / "`pre_merge_checklist_sha_seen`") — ALWAYS-INLINE (load-bearing for self-application).
- "5 self-agent messages" freshness-metric — ALWAYS-INLINE.
- Bypass-scope clause (commit/push/merge bypass details) — SKILL-RELOCATABLE (gate-file source-of-truth at ~/.bot-hq/gates/ ranks above per R18; rule-text bypass-scope is duplicate).
- Cite-anchor list ("phase-l.md§Tier-shape L-5 + L-4 discipline-log #9-#26...") — SKILL-RELOCATABLE.

**Estimated savings:** **~400-500 bytes** by relocating bypass-scope detail (already in gate-files §Bypass) + cite-anchors. Recognition + gate-file paths + cite-format anchors + freshness-metric all stay inline.

**Risk:** LOW. R33 is TOOLGATE-ENFORCED — toolgate verifies SHA-cite + AgentState mechanically; prompt-rule serves as awareness layer. Trim is safe post-toolgate-conversion.

### 4.5 PhaseLv6PrePhaseCloseRetro (R34, 2,042 bytes)

**Current state:** R34 PRE-PHASE-CLOSE-RETRO rule. Rule-text only (toolgate deferred to Phase M per L-6 design choice).

**Trim analysis:**
- Recognition substrings ("PRE-PHASE-CLOSE-RETRO (R34)" / "pre_phase_close_checklist_sha_seen" / "graduate-or-deprecate" / "baseline-vs-final" / "~/.bot-hq/gates/" / "5 self-agent messages") — ALWAYS-INLINE.
- 7-item phase-close disposition list ((a)-(g)) — borderline; gate-file source-of-truth at `~/.bot-hq/gates/pre-phase-close-checklist.md` has full enumeration. **SKILL-RELOCATABLE** if rule-text retains item-recognition keywords (discipline-log sweep / Tier-2 holds re-eval / baseline-vs-final / ratchet-ledger / arc-snapshot / push-batch greenflag / AgentState refresh) but drops prose detail per item.
- Bypass-clause + Phase-M-toolgate-deferral rationale — SKILL-RELOCATABLE.
- Cite-anchors — SKILL-RELOCATABLE.

**Estimated savings:** **~600-800 bytes** by relocating prose-detail-per-item + bypass-clause + cite-anchors. Recognition substrings + 6-anchor substring-lock list + AgentState field-name stay inline.

**Risk:** LOW. R34 is PEER-CROSS-CHECK-only (toolgate deferred Phase M). Gate-file is source-of-truth per R18 — trimming rule-text-detail does not lose information; the gate-file is the canonical reference.

### 4.6 PhaseJv1HaltResumeProtocol (1,436 bytes)

**Current state:** HALT-ALL-WORK + RESUME-FROM-HALT rules. Load-bearing substring-trigger recognition for HALT events.

**Trim analysis:**
- HALT-trigger substrings ("agent <id> at <N>%, halt" / "plan usage at <N>%, halt") — ALWAYS-INLINE (mechanical-recognition).
- RESUME-trigger substring ("plan usage reset") — ALWAYS-INLINE.
- 3-step recovery procedure — ALWAYS-INLINE (load-bearing).
- Background context ("(Emma context-cap fire at ≥95%..." / "Emma's auto-clear emit when plan-usage drops...") — SKILL-RELOCATABLE (parenthetical rationale).
- "User msg 4929 SNAP-gate refinement on Phase I W2 hotfix (D)" — SKILL-RELOCATABLE.

**Estimated savings:** **~200-300 bytes** by stripping parenthetical rationale + cite-anchors. Substring-triggers + recovery-procedure stay inline.

**Risk:** LOW. HALT/RESUME triggers are well-codified at code-side (gemma sentinel + plan-usage detection). Prompt-rule serves as agent-awareness layer.

---

## 5. Risk-ranked savings table

| Candidate | Block | Est. savings | Risk | Skill-target | Status |
|-----------|-------|--------------|------|--------------|--------|
| C1 | PhaseIv1 prose+cites trim | 2,800-3,800 | LOW | phase-rules-detail (extend) | Audit-needed for v1.1 (overlap verification) |
| C2 | DISC v2 prose extraction | 1,500-2,000 | MEDIUM-LOW | new disc-v2-detail OR extend phase-rules-detail | Ship-eligible if recognition-substrings preserved |
| C3 | PhaseLv6 detail-per-item trim | 600-800 | LOW | new phase-l-rules-detail OR extend phase-rules-detail | Ship-eligible (gate-file is canonical) |
| C4 | PhaseLv5 bypass+cite trim | 400-500 | LOW | new phase-l-rules-detail OR extend phase-rules-detail | Ship-eligible (toolgate is canonical) |
| C5 | PhaseLv1 (R31/R32) cite+rationale trim | 400-500 | MEDIUM | new phase-l-rules-detail | DEFER (R31/R32 still PEER-CROSS-CHECK-only) |
| C6 | PhaseJv1 parenthetical rationale trim | 200-300 | LOW | phase-rules-detail (extend) | Ship-eligible |
| **TOTAL ship-eligible (low-risk + medium-low)** | | **5,500-7,400 bytes** | | | |

**Per-agent shrink projection:**
- Rain: 19,413 → ~12,000-14,000 (28-38% reduction)
- Brian: 20,627 → ~13,200-15,100 (28-36% reduction)

**Approx token-savings per turn:** ~1,400-1,900 tokens × 2 agents = ~2,800-3,800 tokens system-prompt-cache savings per trio session start.

---

## 6. Ship-list (low-risk-only)

For L-3b implementation (or Phase M deferral), ship these in the order listed. Each is independently shippable.

### S1. PhaseLv6 detail-per-item trim → phase-l-rules-detail skill
- **File:** new `~/.claude/skills/phase-l-rules-detail/SKILL.md`
- **Const-edit:** `disc.go` PhaseLv6PrePhaseCloseRetro — keep recognition + 6 substring-locks + AgentState field-name; drop prose item-(a)-(g) detail + bypass-clause + cite-anchors.
- **Skill body:** full 7-item disposition prose + bypass rationale + cite-anchors + gate-file SHA + Phase-M-toolgate-deferral rationale.
- **Tests:** existing TestPrePhaseCloseRetroSubstringLock + TestPhaseLv6PrePhaseCloseRetroHeaderAnchor must stay green (substring-locks preserved).
- **Risk:** LOW. Gate-file at `~/.bot-hq/gates/pre-phase-close-checklist.md` is source-of-truth per R18.
- **Savings:** 600-800 bytes per agent.

### S2. PhaseLv5 bypass+cite trim → phase-l-rules-detail skill
- **Const-edit:** `disc.go` PhaseLv5GateProtocol — keep recognition + 6 substring-locks + freshness-metric anchor; drop bypass-scope detail (commit override env var + push no-bypass + merge USER-ONLY ABSOLUTE) + cite-anchors.
- **Skill body:** full bypass-scope detail + R29 elevated-gate composition note + cite-anchors.
- **Tests:** existing TestPreExecuteGateFileReadSubstringLock + TestPhaseLv5GateProtocolHeaderAnchor must stay green.
- **Risk:** LOW. R33 is TOOLGATE-ENFORCED; gate-files are canonical.
- **Savings:** 400-500 bytes per agent.

### S3. PhaseJv1 parenthetical rationale trim → phase-rules-detail skill (extend)
- **Const-edit:** `disc.go` PhaseJv1HaltResumeProtocol — strip parenthetical rationale ("(Emma context-cap fire at ≥95%..." etc.) and "User msg 4929 SNAP-gate refinement..." cite-line.
- **Skill-extend:** add HALT-RESUME section to `phase-rules-detail/SKILL.md` (or new phase-j-rules section) with full rationale.
- **Tests:** verify substring-trigger tests still green ("agent <id> at <N>%, halt" / "plan usage at <N>%, halt" / "plan usage reset" recognition substrings preserved).
- **Risk:** LOW. Substring-triggers + 3-step recovery procedure preserved inline.
- **Savings:** 200-300 bytes per agent.

### S4. PhaseIv1 audit-pass + prose+cites trim → phase-rules-detail skill (extend if needed)
- **Audit step (v1.1 prerequisite):** read full PhaseIv1ProtocolHardening const (7,913 bytes); for each rule R1-R23, compare rule-text against `phase-rules-detail/SKILL.md` body; identify duplicate prose + cite-anchors that exist in skill but also in const.
- **Const-edit:** strip duplicate prose; preserve recognition-substring + 1-line load-bearing summary per rule.
- **Skill-extend:** any missing rules (if skill has gaps for some R-numbers post-T2.1-(d) trim) added to skill body.
- **Tests:** add new ratchet-test `TestPhaseIv1RuleNamesPresent` that verifies all R1-R23 rule-name substrings still inline in const post-trim.
- **Risk:** LOW (post-audit). Highest-savings candidate by absolute bytes.
- **Savings:** 2,800-3,800 bytes per agent (largest single shrink).

### S5. DISC v2 prose extraction → new disc-v2-detail skill
- **File:** new `~/.claude/skills/disc-v2-detail/SKILL.md` OR extend phase-rules-detail
- **Const-edit:** new const `DiscV2RolesCompact` in `disc.go` containing recognition substrings + 1-line role summaries (HANDS/EYES/BRAIN/OUTPUT/DRAFT/HALTER-PUSHER/FLAG/PIVOT/TRUST/NUDGE). Replace inline DISC v2 block in rain.go + brian.go with const-reference.
- **Skill body:** full prose role-detail + cite-anchors + 2026-04-24 origin + 2026-04-27 user-delegation context + class-split exception clauses.
- **Tests:** new `TestDiscV2RolesCompactSubstringLock` verifying recognition substrings + class-recognition keywords preserved.
- **Risk:** MEDIUM-LOW. DISC v2 is referenced in many decision-flows; trim must preserve role-class-recognition triggers + DRAFT-alone discipline + HALTER-PUSHER + FLAG-ownership recognition.
- **Savings:** 1,500-2,000 bytes per agent.

### Deferred (medium-risk)
- **D1.** PhaseLv1 (R31/R32) cite+rationale trim — DEFER until R31 has toolgate-enforcement (Phase M candidate per L-7-eval).

---

## 7. Decision recommendation

**Ship-L-3b this phase** as the L-7 RESERVED slot candidate.

**Rationale:**
- Ship-list S1-S5 totals 5,500-7,400 bytes per agent (~30% prompt-shrink) at LOW or MEDIUM-LOW risk per item.
- Each ship-item is independently testable + revertable (per-const trim with substring-lock test verification).
- L-7 is conditional ratchet per phase-l.md§Tier-shape line 95 ("emergent ratchet detected during L-0..L-6 OR L-3b prompt-shrink relocation if L-3a audit produced low-risk ship-list"); L-3a output meets the low-risk-ship-list trigger.
- Halt-and-elevate not required — prompt-changes activate on next-rebuild+restart bundled with L-5/L-6 prompt-embed activation per user msg 7331 ("after rebuild+restart we're going to test full phase L").
- Toolgate-enforcement landing at L-5 (R33 commit-2 e327362) reduces prompt-rule burden — agents now have mechanical SHA-cite verification, so rule-text-detail can lean more on skill-relocation pattern.

**Ordering recommendation for L-3b commits:**
1. **Commit-1: S4 PhaseIv1 audit-pass + trim** — largest savings, highest-confidence (skill already exists; gap is duplicate prose).
2. **Commit-2: S1 + S2 + S3 (Phase-L+J trims, bundled)** — symmetric shape (recognition preserved, prose+cites relocated), single skill-target.
3. **Commit-3: S5 DISC v2 extraction** — new const + new skill or skill-extend; biggest structural change, ship last.

Each commit gated on Brian-Rain BRAIN-2nd + R12 footer + R33 pre-commit-checklist gate-CHECK (now active per L-5 commit-2). Substring-lock tests guarantee recognition layer preserved; absent test = absent guarantee.

**Alternative: Defer L-3b to Phase M.** Lower-priority justification: bot-hq trio currently functioning within prompt-budget; shrink is optimization not necessity. Phase M would have benefit of additional Phase L close-cycle data (any additional drift classes observed during L-6/L-7/phase-close inform what's load-bearing vs ceremonial).

**Lean: ship L-3b this phase.** Low-risk savings are concrete; deferral risks accumulating prompt-bloat in Phase M as more rules graduate into trio prompts.

---

## 8. Open questions / Brian BRAIN-2nd asks

1. **Const-vs-skill granularity for L-3b commits.** Does Brian prefer (a) one new skill `phase-l-rules-detail` for S1+S2+S3 (Phase L+J consolidation), OR (b) extend existing `phase-rules-detail` skill with HALT/RESUME + L-rules sections, OR (c) separate skills per phase (phase-l-rules-detail / phase-j-rules-detail / etc.)?
2. **DISC v2 extraction const-naming.** New const `DiscV2RolesCompact` in disc.go OR refactor inline DISC v2 block out of rain.go + brian.go entirely (pure-skill-relocation)? Lean: new const for symmetry with existing pattern.
3. **PhaseIv1 audit-pass scope.** Audit-pass (S4 v1.1 prerequisite) is itself a ~2-3KB diff-investigation. Brian-HANDS authored OR Rain-HANDS (markdown audit-doc class)? Lean: Rain-HANDS for the audit doc (markdown design-spike); Brian-HANDS for the const-edit + tests once audit-doc surfaces specific trim candidates.
4. **Substring-lock test discipline for trims.** Each S1-S5 trim needs to preserve existing substring-locks (TestPreExecuteGateFileReadSubstringLock / TestPrePhaseCloseRetroSubstringLock / etc.). Add new ratchet-test asserting *minimum* recognition-substring set OR rely on existing tests (substring.Contains-presence check)? Lean: existing tests sufficient + add `TestPhaseIv1RuleNamesPresent` as new ratchet-test for S4 (R1-R23 rule-name presence).
5. **L-7 RESERVED slot disposition.** L-3b ship-list confirms L-7 = L-3b prompt-shrink relocation per phase-l.md§Tier-shape line 95 conditional clause. L-7 close-no-impl path (no-emergent-ratchet) NOT triggered. Concur or push for alternative L-7 candidate?
6. **Phase-close timing of L-3b.** L-3b ships before phase-close OR L-3b is itself the phase-close-adjacent batch? Lean: L-3b ships before phase-close (pre-rebuild) so user's "test full phase L" post-rebuild test covers both L-5/L-6 + L-3b prompt-shrink runtime activation.
7. **Verification methodology post-rebuild.** L-3b prompt-shrink runtime-validation criterion: trio agents produce same trio-discipline behavior post-trim as pre-trim (no recognition gaps). Lean: rebuild+restart, then run a representative BRAIN-cycle (commit-fire pre-checklist consult / OUTBOUND-MISS recovery / DRAFT-alone class-split). If behavior diverges → revert specific trim. Documented in phase-close artifact.

---

## 9. Carry-forwards / NB sequencing

Independent of L-3a content; flagging for tracking pre-phase-close item-1 (discipline-log sweep):
- halt-#2 over-conservative-elevation (logged Rain SNAP + Brian msg 7335 acked)
- #28 broadcast-vs-hub_flag channel-choice for routine-elevation
- 2x self-msg-id pre-emit drift (Rain msgs 7340/7344) — class candidate for L-7-eval mitigation OR Phase M
- L-6 style-NB const-ordering (L1→L6→L5 vs numerical L1→L5→L6) — fold into S1-S5 const-edits if numerical reordering trivial
- L-3a v1.1 PhaseIv1 audit-pass (prerequisite to S4) — author next if Brian concurs S4 ship-eligibility

These append to `~/.bot-hq/discipline-log.md` at sequence: L-6 commit-1 (LANDED) → L-3a v1 (THIS DOC) → BRAIN-2nd → v1.1 audit-pass if needed → L-3b commits → L-7 disposition → discipline-log-append → phase-close.

---

## 10. Cite-anchor index

- phase-l.md§Tier-shape line 91 (L-3a scope)
- phase-l.md§Tier-shape line 95 (L-7 RESERVED conditional)
- phase-l.md§Output-files line 186 (L-3a filename)
- phase-l.md§Authority-model (Rain HANDS for markdown design docs)
- ~/.claude/skills/phase-rules-detail/SKILL.md (existing skill — pattern source)
- ~/.bot-hq/gates/pre-commit-checklist.md (gate-file for R33 self-application of L-3b commits)
- ~/.bot-hq/gates/pre-phase-close-checklist.md (gate-file for R34 phase-close)
- L-5 commit-1 44cc03f (R33 rule-text)
- L-5 commit-2 e327362 (R33 toolgate gate-CHECK)
- L-6 commit-1 a7a4cbd (R34 rule-text + AgentState field)
- BRAIN-cycle msgs: 7331 (user proceed) / 7335-7340 (L-6 BRAIN-cycle) / 7346 (Brian L-6 LANDED + L-3a Rain-HANDS-author-go)
