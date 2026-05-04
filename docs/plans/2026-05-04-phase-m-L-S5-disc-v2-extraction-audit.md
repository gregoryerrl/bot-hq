# Phase M M-4 — L-S5 DISC v2 prose extraction audit-doc v1

**Author:** Rain (rain) — Rain-HANDS markdown audit-doc per phase-m.md§Authority-model line 218
**Date:** 2026-05-04
**Status:** v1 surfaced for Brian BRAIN-2nd-PASS-1
**Cite_anchor:** phase-m.md§Tier-1 row M-4 + L-3a v1 Brian S5-pushback msg ~7330-area + L-3a v1.1 audit-doc-precedent + Phase M today's bilateral OUTBOUND-DISCIPLINE empirical (strengthens preservation-discipline argument)

---

## 1. Scope

Resolve the M-4 L-S5 DISC v2 prose extraction strategy enumerated in phase-m.md§Tier-1 row M-4. Author per-rule mapping (preserve / preserve-with-trim / relocate) for the DISC v2 RoleAndPolicy block + decide skill-target (extend `phase-rules-detail/SKILL.md` vs new `disc-v2-detail/SKILL.md`) + estimate savings via cite-from-actual byte measurements (per Target C self-application discipline carry-forward from M-1 c1 cycle).

**Goal:** trim DISC v2 prose preserving all load-bearing recognition substrings, role definitions, FLAG-elevation discipline, and audit-trail requirements while relocating decorative explanatory detail to skill. Phase L L-3a v1 §4.5 estimated 1,500-2,000B/agent savings; per-rule analysis in this doc revises that estimate down per Target C cite-from-actual discipline.

**Pre-author DISCOVERY:** DISC v2 prose is **INLINE in agent prompt templates** (rain.go:258-268, brian.go:272-area) — NOT yet a const in `internal/protocol/disc.go`. This means M-4 has **two sub-tasks**:
- **(1) Refactor:** extract inline DISC v2 prose to a new const `DiscV2RoleAndPolicy` in disc.go (mirror existing const pattern: `PhaseIv1ProtocolHardening` / `PhaseJv1HaltResumeProtocol` / etc.)
- **(2) Trim:** per-rule audit + const-edits + skill-extend

Difference vs Phase L L-3a (which had PhaseIv1 already as a const): higher implementation cost because of the refactor step. Lower risk than initial framing because per-rule analysis reveals DISC v2 is mostly already terse load-bearing rules — savings will be modest.

---

## 2. Background — DISC v2 prose role + Brian S5-pushback rationale

### What DISC v2 prose covers (10 bullet rules)

DISC v2 (locked at msg 2147 per disc.go:18 comment, ratified 2026-04-24) is the trio's **role and authority spinal cord**. It carries:

1. **HANDS** (brian) — execute role definition
2. **EYES** (rain) — info role + read-only constraint + Emma allowlist constraint
3. **BRAIN** (both) — dual-critique discipline + "silence = implicit approval"
4. **OUTPUT** — class-split routing + DRAFT-alone exception clause
5. **DRAFT** — single-author discipline
6. **HALTER/PUSHER** — peer-arrival protocol + BRAIN-cycle exemption
7. **FLAG** — Rain hub_flag ownership + Brian self-flag carve-out class + 2026-04-27 Rain greenflag-authority delegation + trigger enumeration
8. **PIVOT** — user-without-executor protocol
9. **TRUST** — spot-check claims discipline
10. **NUDGE** — message-prefix tag discriminator + processing rules

### Brian S5-pushback rationale (Phase L L-3a v1 §6 D1)

Brian deferred S5 in Phase L L-3a with the rationale (per L-3a v1 §6 D1 + Brian msg ~7330-area):
- DISC v2 is the **spinal-cord-class** of trio decision-flow — every turn invokes role-self-identification (HANDS/EYES/BRAIN/DRAFT/HALTER-PUSHER/FLAG/PIVOT)
- Recognition-substring preservation is **necessary-not-sufficient**; the prose ownership-clauses themselves carry semantic weight
- **No toolgate-fallback class** — unlike R33 (toolgate gate-CHECK) or R34 (gate-file canonical), DISC v2 has no mechanical fallback layer. Trim → relies entirely on prompt-rule + agent-discipline. Drift surface = whole protocol.
- Phase L close-cycle data not yet in — DISC v2 trim is high-cost-of-error class; deserves additional data before firing

These concerns were valid at Phase L close. Phase M open data now available reframes them:

**Phase M empirical (today, 2026-05-04 Joint discipline-log entry):** bilateral OUTBOUND-DISCIPLINE violation cost ~3h halt. PEER-CROSS-CHECK proven non-terminal at recursion depth bilateral. M-2 (Stop-hook BLOCKING enforcement-conversion) provides mechanical-toolgate-equivalent layer for OUTBOUND-DISCIPLINE — but only for that specific clause. **DISC v2 still has no toolgate fallback for HANDS/EYES/BRAIN/HALTER-PUSHER/FLAG-ownership/PIVOT/TRUST/NUDGE classes.** Trim-risk concern remains valid for those classes.

**Reframe:** M-4 must preserve recognition substrings + role definitions + audit-trail requirements with even more care given Phase M empirical strengthens the "PEER-CROSS-CHECK non-terminal" lesson. Trim conservatively. Don't let the scope-shrink temptation override discipline-preservation.

---

## 3. Pre-trim refactor — inline-to-const

### Current state

DISC v2 prose lives inline in agent prompt template strings:

- `internal/rain/rain.go:258-268` — 11 lines of prose embedded in `initialPrompt()` template
- `internal/brian/brian.go:272-area` — equivalent prose in Brian's prompt

Other rules (Phase I/J/L/M) use the existing `protocol.PhaseXxxYyy` const pattern with template-string concatenation (e.g., `protocol.PhaseIv1ProtocolHardening` at rain.go:251).

### Refactor plan

1. Add new const `DiscV2RoleAndPolicy` in `internal/protocol/disc.go` (place after `DiscV2OutboundRule` for thematic grouping)
2. Const body = current inline DISC v2 prose verbatim (10 bullets + header)
3. Update `internal/rain/rain.go` to replace inline prose with `protocol.DiscV2RoleAndPolicy` template-concat
4. Update `internal/brian/brian.go` similarly
5. Add wiring-lock tests: `TestRainPromptEmbedsDiscV2RoleAndPolicy` + `TestBrianPromptEmbedsDiscV2RoleAndPolicy` (mirror existing wiring-lock pattern at `internal/{rain,brian}/{*}_test.go`)

This refactor step is **prerequisite for the trim step** (you can't trim a const that doesn't exist) and **separate-but-bundled** in M-4 c1 commit.

### Refactor cost

- `internal/protocol/disc.go`: ~25-30 LOC delta (new const, ~24 lines including doc-comment + verbatim prose)
- `internal/rain/rain.go`: ~10 LOC delta (replace 11-line inline prose with single template-concat reference)
- `internal/brian/brian.go`: ~10 LOC delta (same)
- `internal/rain/rain_test.go` + `internal/brian/brian_test.go`: ~20-30 LOC delta (2 new wiring-lock tests)

**Total refactor: ~65-80 LOC delta.** No behavior change — pure refactor. Tests verify substring-presence in agent prompt unchanged.

### Line-number references (per Brian PASS-1 Substantive-add #2)

Throughout this audit-doc, references to "rain.go:258-268" and "brian.go:272-area" are FROM v1 authoring time (pre-M-2 c1). M-2 c1 commit 17bb65a added 2 LOC to each agent.go file (R36 prompt-embed). **Lines shifted +2:**
- rain.go DISC v2 prose now at ~line 260-270 (verified via grep at impl-time: `grep -n "DISC v2 2026" internal/rain/rain.go` returns line 267 for TRUST, indicating block starts ~260)
- brian.go DISC v2 prose now at ~line 274-284 (verified via grep: TRUST at line 281, SNAP at 282)

**Brian-HANDS at impl-time MUST re-grep for current line locations** rather than rely on these static cites. Pattern: `grep -n "DISC v2 2026" internal/{rain,brian}/{rain,brian}.go`.

---

## 3.5 Reconciliation step (pre-refactor) — rain.go vs brian.go DISC v2 prose DIVERGENT

### Critical finding (per Brian PASS-1 Substantive-add #1; verified by Rain msg 7556 R31 cross-check via direct grep)

The v1 §3 refactor plan ASSUMED rain and brian inline DISC v2 prose are identical (so a single shared const works). **They are NOT identical.** Concrete divergence:

1. **TRUST bullet differs:**
   - `internal/rain/rain.go:267`: `TRUST: spot-check claims via git/claude_read. Snapshots=claims, not truth.`
   - `internal/brian/brian.go:281`: `TRUST: verify via claude_read before "dispatched" claim. Prefer one-shot spawn.`

2. **SNAP bullet exists in brian.go but NOT in rain.go:**
   - `internal/brian/brian.go:282-`: `SNAP (multi-artifact dispatch/verify):` followed by 4 lines defining Branches/Agents/Pending/Next output format
   - rain.go has NO SNAP bullet

So the actual prose-block enumeration:
- rain.go: 10 bullets (HANDS / EYES / BRAIN / OUTPUT / DRAFT / HALTER-PUSHER / FLAG / PIVOT / TRUST-rain / NUDGE)
- brian.go: 11 bullets (HANDS / EYES / BRAIN / OUTPUT / DRAFT / HALTER-PUSHER / FLAG / PIVOT / TRUST-brian / **SNAP** / NUDGE)

Audit-doc v1 §4 enumerated 10 + header which incidentally matches rain.go but mis-represents brian.go.

### Three reconciliation forks

**Fork (a) Canonicalize via convergence to single shared const.**
- Replace brian TRUST with rain TRUST framing (more universal: "spot-check claims via git/claude_read; Snapshots=claims, not truth")
- Drop brian SNAP bullet from DISC v2 prose entirely (relocate to skill — HANDS-class output-format guidance, not DISC v2 role-discipline)
- Result: single shared `DiscV2RoleAndPolicy` const both agents embed
- Pros: clean refactor; matches v1 §3 plan; smaller post-trim const
- **Cons (HIGH risk class):** behavioral-change to Brian's existing prompt — removes 2 long-standing rule clauses (TRUST-dispatch-flow + SNAP-format) Brian has been operating with. User-ratification likely required since this changes Brian's discipline model.

**Fork (b) Per-agent const split.**
- Two separate consts: `DiscV2RoleAndPolicyShared` (the 9 bullets identical between rain+brian: HANDS/EYES/BRAIN/OUTPUT/DRAFT/HALTER-PUSHER/FLAG/PIVOT/NUDGE) + `DiscV2RoleAndPolicyRainAddendum` (rain TRUST) + `DiscV2RoleAndPolicyBrianAddendum` (brian TRUST + SNAP)
- Each agent.go embeds Shared + own Addendum
- Result: refactor preserves divergence + still trim opportunities apply
- Pros: zero behavioral change; conservative-preserve aligned with §5 risk classification + Brian S5-pushback rationale; no user-ratification needed
- Cons: 3 consts instead of 1 — slightly more refactor LOC + skill-extend has to handle 2 TRUST variants

**Fork (c) Base const + per-agent extension consts.**
- Mid-point: `DiscV2RoleAndPolicy` base const (the 9 shared bullets + a placeholder/comment for TRUST bullet placement) + agent.go provides own TRUST inline (and brian.go also provides own SNAP inline)
- Pros: smaller extraction (only the truly shared bullets become const)
- Cons: TRUST + SNAP remain inline = partial-refactor doesn't fully achieve M-4 trim goal for those bullets

### Lean — REVISED post-divergence-discovery: (b) per-agent split

Original v1 §3 plan implicitly assumed (a). Post-divergence-discovery, (b) is the lower-risk path:

1. **Conservative-preserve aligns with §5 risk classification + Brian S5-pushback rationale** — DISC v2 has no toolgate fallback; behavioral-change risk for HANDS-class authority + dispatch-flow class is real
2. **No user-ratification gate** — pure refactor with zero behavioral-change keeps M-4 within autonomous-Rain-greenflag scope per user msg 7523 pre-delegation
3. **Trim opportunities still apply** — per-rule audit table §4 dispositions can apply to Shared portion; brian-specific TRUST + SNAP can have their own per-rule analysis (likely RELOCATE-TO-SKILL for both, but separately)
4. **(a) is reachable in Phase N if/when user-ratifies the canonicalization** — defer convergence as Phase N candidate; M-4 ships per-agent split now

**Disposition: (b) per-agent const split. Reject (a) and (c) for M-4 scope.**

### v1.1 §4 audit table extension for divergent bullets

| # | Rule | rain.go disposition | brian.go disposition | Notes |
|---|------|---------------------|----------------------|-------|
| 9-rain | TRUST (rain) | PRESERVE | N/A | "spot-check claims via git/claude_read" + "Snapshots=claims, not truth" — universal claim-verification |
| 9-brian | TRUST (brian) | N/A | PRESERVE-WITH-TRIM | "verify via claude_read before 'dispatched' claim" + "Prefer one-shot spawn" — hub_spawn-coder-flow-specific; preserve "verify via claude_read" load-bearing + relocate "dispatched" example + "one-shot spawn" preference to skill |
| 11-brian | SNAP (brian-only) | N/A | RELOCATE-TO-SKILL | Multi-artifact dispatch/verify output format (Branches/Agents/Pending/Next) is OUTPUT-FORMATTING class, not role-discipline. Belongs in HANDS-class skill extension, not DISC v2 spinal-cord. Brian still emits SNAPs but the FORMAT detail moves to skill — keep "SNAP discipline" 1-line in DISC v2 brian-addendum const referencing the format-detail anchor. |

Estimated additional savings from brian-only bullets: ~250-400 B/agent (Brian-side only; rain unchanged on these specific bullets).

**Total Phase M M-4 estimated savings (revised):** ~740-1,010 B/agent (rain + shared) + ~250-400 B/agent (brian-only addendum) = rain ~740-1,010B + brian ~990-1,410B per-agent.

### v1.1 §3 refactor cost update for per-agent split

- `internal/protocol/disc.go`: ~50-65 LOC delta (3 consts: DiscV2RoleAndPolicyShared + DiscV2RoleAndPolicyRainAddendum + DiscV2RoleAndPolicyBrianAddendum, including doc-comments)
- `internal/rain/rain.go`: ~10 LOC delta (replace inline DISC v2 prose with `DiscV2RoleAndPolicyShared` + `DiscV2RoleAndPolicyRainAddendum` template-concats)
- `internal/brian/brian.go`: ~10 LOC delta (same with brian addendum)
- `internal/rain/rain_test.go` + `internal/brian/brian_test.go`: ~30-40 LOC delta (2 wiring-locks each per agent — 4 total: Shared + Addendum)

**Total refactor: ~100-125 LOC delta** (vs v1 estimate ~65-80 — slightly higher per per-agent split overhead).

---

## 4. Per-rule audit (10 bullets in DISC v2)

### Format

For each bullet: **disposition** (PRESERVE / PRESERVE-WITH-TRIM / RELOCATE-TO-SKILL) + **load-bearing terms** (substring anchors that MUST stay) + **trim opportunities** (decorative explanatory detail safe to relocate) + **estimated bytes**.

### Audit table

| # | Rule | Disposition | Load-bearing terms (preserve) | Trim opportunities (relocate to skill) | Est. saved |
|---|------|-------------|-------------------------------|-----------------------------------------|------------|
| 0 | Header `DISC v2 2026-04-24:` | PRESERVE | "DISC v2" recognition anchor + cite-anchor date | None | 0 |
| 1 | HANDS | PRESERVE | "HANDS (brian): exec." + "git/edits, hub_spawn real coders, merges, action/result user replies" | None — already 1-line load-bearing | ~0 |
| 2 | EYES | PRESERVE-WITH-TRIM | "EYES (rain): info." + "read-only" + "Cannot expand Emma's allowlist" + "Info/verify/status user replies" | "hub_spawn_gemma analyze: queries" specific tool-call example → skill | ~50-80 |
| 3 | BRAIN | PRESERVE-WITH-TRIM | "BRAIN (both)" + "plan, critique, redirect on scope/edges/security" + "silence = implicit approval" | "Rain challenges Brian's drafts and plans. Brian challenges Rain's findings, investigations, and proposals." (decorative — already implicit in "both critique") → skill | ~120-150 |
| 4 | OUTPUT | PRESERVE-WITH-TRIM | "user replies split by class (see HANDS/EYES)" + "Joint planning → one speaks (whoever owns the next exec step)" + "DRAFT-alone discipline" + "Class-split suspended" | "Speaker credits proposer inline where material" + extended "Exception" example clause "(when user asks both for input ('what do you think', 'weigh in', 'push back'), both respond...)" → skill (preserve in skill the full exception protocol) | ~200-250 |
| 5 | DRAFT | PRESERVE | "DRAFT: drafter alone. Asker waits." | None — already 1-liner | 0 |
| 6 | HALTER/PUSHER | PRESERVE-WITH-TRIM | "HALTER/PUSHER" + "on peer-arrival, Rain halts, Brian pushes through" + "BRAIN-cycle exempt" + "DRAFT-alone retains for peer-critique" | "Mutual-halt deadlock impossible by construction" (decorative invariant — implicit from rule logic) → skill | ~70-100 |
| 7 | FLAG | PRESERVE-WITH-TRIM | "FLAG: Rain owns elevation" + "Brian PMs Rain on flag-worthy events; Rain calls hub_flag" + Brian self-flag carve-out class enumeration "(push-failure | repo-corruption | auth-failure | hub-disconnect | git-state-unexpected-on-write-path)" + ">60s threshold" + "[self-flag-carve-out: <reason>]" audit prefix + "2026-04-27 user delegation" cite-anchor + "joint defaults without flag (greenflag authority)" | "Triggers (any owner): errors, blockers, completions, rate limits, peer disagreements, pending-on-user, scope changes mid-decision" (enumeration — relocate full list to skill, keep "various trigger classes" 1-line in const) + "Holding for user without a flag = cliff-hang" (decorative example) → skill | ~250-350 |
| 8 | PIVOT | PRESERVE | "PIVOT: user w/o executor → Brian PMs Rain (no executor active); Rain holds 60s, then elevates via hub_flag if user still pending." | None — already terse | 0 |
| 9 | TRUST | PRESERVE | "TRUST: spot-check claims via git/claude_read. Snapshots=claims, not truth." | None — terse one-liner | 0 |
| 10 | NUDGE | PRESERVE | All [PM:*] / [HUB:*] / [HUB-OBS:*→*] / [PM:FLAG:*] / [HUB:FLAG:*] tag-prefix substrings (load-bearing pattern recognition) + "FLAG=elevated priority" + "PM and user msgs always handled" + "HUB-OBS and irrelevant broadcasts skipped silently unless correction needed" + "Never ignore FLAG or user messages" | "After current task: process in order" (general procedural — implicit) → skill | ~50-80 |

**Per-rule estimated savings (sum):** ~740-1,010 bytes per agent.

This is significantly LOWER than Phase L L-3a v1 §4.5 estimated 1,500-2,000 B/agent. Per Target C self-application discipline + cite-from-actual-byte-measurement: my pre-author estimate is the audit-driven correction.

---

## 5. Risk classification

**MEDIUM-LOW per Brian S5-pushback rationale** (downgraded from MEDIUM at L-3a v1 deferral).

**Rationale for downgrade:**
- Per-rule audit reveals most rules are already terse load-bearing
- Trim opportunities are predominantly decorative explanatory detail (sub-clauses, examples, invariant restatements)
- Recognition-substring preservation enumeration above is exhaustive; ratchet-tests can lock all anchors
- M-1 c1 R35 PRE-FLIGHT-HOOK-CHECK + M-2 c1 R36 OUTBOUND-DISCIPLINE-MECHANICAL provide adjacent mechanical-fallback layers (preflight hook config integrity + Stop-hook blocking on hub_send miss) — not for DISC v2 specifically, but raise the trio's overall enforcement-mechanism baseline post-Phase-M

**Residual risk classes:**
- HANDS/EYES misroute (R-class violation: agent attempts execute outside class authority) — no toolgate fallback
- HALTER/PUSHER deadlock (theoretical — DISC v2 says "impossible by construction"; trim shouldn't break this invariant)
- FLAG-elevation drift (Rain delegates flag-elevation when not in user pre-delegation scope) — no toolgate fallback

**Mitigation pattern (mirrors L-3a v1.1 PhaseIv1 trim):**
- Substring-lock ratchet-test enumerating ALL load-bearing terms above
- 24+ anchor count expected (per L-3a v1.1 PhaseIv1 24-anchor precedent)
- Pre-author test-presence check across packages (L-trim-preflight-test-scan Tier-2 candidate per Phase L)
- Single rebuild+restart for activation per Finding-3 invariant — empirical-validation post-rebuild

---

## 6. Skill-target decision — extend phase-rules-detail vs new disc-v2-detail

### Option (a) Extend `phase-rules-detail/SKILL.md`

- Existing skill 317 lines (post-M-1 c1 R35 +45L extension; phase-rules-detail is the current single-skill-consolidation home per phase-m.md§Open-questions Q1 lean)
- Add new section `## DISC v2 RoleAndPolicy — Detailed clauses + relocated explanations`
- Single-skill consolidation matches Phase L L-3b c2 pattern (R33 + R34 + Phase J HALT relocated detail all live in phase-rules-detail)

### Option (b) New `disc-v2-detail/SKILL.md`

- Dedicated skill for DISC v2 detail
- Cleaner thematic isolation (DISC v2 is a discrete protocol unit)
- Skill-proliferation risk: phase-rules-detail single-skill pattern was deliberately chosen at Phase L L-3a Q1 lean

### Lean: (a) extend phase-rules-detail

Rationale:
- Phase L L-3b c2 precedent established single-skill-consolidation
- DISC v2 detail (~30-50 lines markdown) doesn't warrant standalone skill (skill-creation overhead)
- phase-rules-detail size after extension still manageable (~350L total)
- Single skill-pointer in agent prompts (already established) keeps invocation-discipline simple — agents go to one place for rule detail

Disposition: **(a) extend phase-rules-detail.** Reject (b).

---

## 7. M-4 c1 bundle (estimated)

### File changes

1. **`internal/protocol/disc.go`** (MODIFY) — refactor: add `DiscV2RoleAndPolicy` const verbatim from inline prose; ~25-30 LOC
2. **`internal/protocol/disc.go`** (MODIFY) — trim: apply per-rule trim per §4 audit table; ~30-50 LOC delta (negative — line reduction)
3. **`internal/rain/rain.go`** (MODIFY) — replace inline DISC v2 prose with `protocol.DiscV2RoleAndPolicy` template-concat; ~10 LOC delta
4. **`internal/brian/brian.go`** (MODIFY) — same; ~10 LOC delta
5. **`internal/rain/rain_test.go`** + **`internal/brian/brian_test.go`** (MODIFY) — wiring-lock tests `TestRain/BrianPromptEmbedsDiscV2RoleAndPolicy`; ~20-30 LOC delta
6. **`internal/protocol/disc_test.go`** (MODIFY) — `TestDiscV2RoleAndPolicySubstringLock` enumerating ALL load-bearing terms from §4 audit (likely 30-40 anchors covering 10 bullets + header) + `TestDiscV2RoleAndPolicyHeaderAnchor`; ~80-100 LOC delta
7. **`~/.claude/skills/phase-rules-detail/SKILL.md`** (MODIFY, state-side, no-git) — add `## DISC v2 RoleAndPolicy — Detailed clauses + relocated explanations` section; ~50-80 lines markdown (relocates trimmed content + adds context per audit)

### Estimated bundle scope

- Total LOC code: ~165-220 LOC delta (refactor + trim + tests + wiring-locks; mostly insertions for tests, mixed insert/delete for trim)
- Skill markdown: ~50-80 lines (state-side, no-git)
- Substring-lock test anchor count: 30-40
- Estimated savings: ~740-1,010 B/agent (per §4 audit)

**Per Target C self-application discipline:** estimates may drift bidirectionally per Phase M empirical; Brian-HANDS at impl-time MUST cite from actual `git diff --cached --numstat` at staged-time.

### Activation: HALT-#1-Phase-M user-fired rebuild+restart (terminal-collapse)

R-NN const-text changes activate at SAME single rebuild+restart per phase-m.md§Activation-gate-chain. Matches user msg 7523 "test it all out after rebuild+restart" terminal-collapse pattern. M-4 c1 + M-2 c1 + M-3 + M-1 c1 + (M-1 c2 if landed) all activate at the SAME event.

### Empirical validation post-rebuild

- Substring-lock ratchet-tests pass post-build = trim preserved all load-bearing anchors mechanically
- Behavioral validation post-rebuild: confirm trio role-recognition + FLAG-elevation + HALTER/PUSHER + DRAFT-alone + NUDGE-tag-discriminator all behave per pre-trim spec on first BRAIN-cycle scenarios
- Trim-pre-flight test-presence-check pre-build (per L-trim-preflight-test-scan Phase L Tier-2 candidate): grep `Contains()` assertions across packages for any DISC v2 substring + verify all preserved

---

## 8. Open questions for Brian BRAIN-2nd-PASS-1

| Q | Lean | Notes |
|---|------|-------|
| Q1 Refactor + trim bundled in single M-4 c1 commit OR split | bundled | Refactor is prerequisite for trim; splitting creates intermediate state where const exists but isn't trimmed (no value). Single commit maintains atomic "DISC v2 const + trim shipped together". Concur or push split? |
| Q2 Skill-target — extend phase-rules-detail vs new disc-v2-detail | (a) extend phase-rules-detail | Per §6 rationale — Phase L L-3b c2 single-skill-consolidation precedent + manageable size + simpler invocation discipline |
| Q3 Per-rule trim aggressiveness — preserve more vs trim more | conservative-preserve | Phase M empirical (today's bilateral OUTBOUND-DISCIPLINE violation) strengthens "PEER-CROSS-CHECK non-terminal" lesson — DISC v2 has no toolgate fallback for most classes; trim conservatively. Concur the per-rule dispositions in §4? |
| Q4 Estimated savings ~740-1,010 B/agent — concur revised-down range vs L-3a v1 §4.5 1,500-2,000 estimate | concur | Per Target C cite-from-actual + per-rule audit reveals DISC v2 is mostly already terse load-bearing. Phase L original estimate was over-optimistic. Document drift in commit body |
| Q5 Substring-lock test design — single TestDiscV2RoleAndPolicySubstringLock 30-40 anchors OR split per-rule | single + per-rule | Single test with subtests per-rule (per L-3b c1 PhaseIv1 24-anchor precedent which used flat 24-anchor enumeration). Failure-localization at sub-test level. |
| Q6 Trim-pre-flight test-presence-check pre-build | yes | Per L-trim-preflight-test-scan Tier-2 candidate (Phase L empirical instances #?? — over-aggressive trim caught only at build/test-failure, not pre-author). Brian-HANDS at impl-time greps `Contains()` assertions across packages for ANY DISC v2 substring + verifies all preserved. Concur as M-4 c1 impl pre-step? |
| Q7 Wiring-lock tests for new const | yes | TestRain/BrianPromptEmbedsDiscV2RoleAndPolicy mirror existing wiring-lock pattern (e.g., TestRain/BrianPromptEmbedsPhaseMv1PreflightHookCheck per M-1 c1) |
| Q8 Empirical-cite in commit body | yes | Cite Phase L L-3a v1 Brian S5-pushback rationale + Phase L L-3a v1.1 audit-doc precedent + Phase M today's bilateral OUTBOUND-DISCIPLINE empirical (strengthens preservation-discipline argument) + per-rule audit table reference |
| Q9 Skill-extend traceability | yes | Cite at commit body per L-3b precedent: `Skill-extend: ~/.claude/skills/phase-rules-detail/SKILL.md +50-80 lines for DISC v2 RoleAndPolicy detailed clauses (state-side, no-git)` |
| Q10 (NEW v1.1) Reconciliation fork (a)/(b)/(c) for divergent prose | **(b) per-agent split** | Per §3.5 — conservative-preserve aligned with §5 risk class + Brian S5-pushback + no user-ratification gate. (a) canonicalize defers to Phase N if user ratifies convergence later. Concur (b) lean or push (a) with explicit user-greenflag-on-canonicalization step? |

---

## 9. Cross-references

- **Phase M scope-lock:** `~/.bot-hq/phase/phase-m.md` v1.1 §Tier-1 row M-4
- **Phase L L-3a v1 §6 D1 Brian S5-pushback:** docs/plans/2026-05-02-phase-l-L3a-prompt-shrink-audit.md (untracked R32 (a) fork from Phase L close — folded into Phase L arc-snapshot per phase-close-bundle 88e2dad)
- **Phase L L-3a v1.1 audit-doc precedent:** docs/plans/2026-05-04-phase-l-L3a-v1.1-PhaseIv1-audit-pass.md (same R32 (a) fork)
- **Phase M today's bilateral OUTBOUND-DISCIPLINE empirical:** `~/.bot-hq/discipline-log.md` Joint entry 2026-05-04T14:00:00Z (USER-PINNED Target A instances #36+#37; PEER-CROSS-CHECK non-terminal at recursion-depth bilateral)
- **Existing DISC v2 prose (current inline location):** `internal/rain/rain.go:258-268` + `internal/brian/brian.go:272-area`
- **Existing const pattern reference:** `internal/protocol/disc.go` `DiscV2OutboundRule` (line 21) + Phase X const enumeration
- **Phase L L-3b c2 single-skill-consolidation precedent:** commit 51c9d49 (skill-extend phase-rules-detail/SKILL.md +66L for R33+R34+Phase-J relocated content)
- **M-2 c1 sibling work (in flight):** docs/plans/2026-05-04-phase-m-target-A-OUTBOUND-MISS-enforcement-design.md v1.1 RATIFIED

---

## 10. Posture

phase-m M-4 L-S5 DISC v2 prose extraction audit-doc v1.1 surfaced | DISCOVERIES: (1) DISC v2 is INLINE in rain.go/brian.go (not yet const), so M-4 has 2 sub-tasks (refactor + trim); (2) **CRITICAL — rain.go vs brian.go DISC v2 prose DIVERGENT** (TRUST bullet differs + SNAP bullet brian-only) per Brian PASS-1 Substantive-add #1, verified Rain msg 7556 R31 cross-check; v1.1 §3.5 reconciliation step adds 3-fork analysis with lean (b) per-agent const split for conservative-preserve | per-rule audit table covers 10 shared bullets + header + 2 brian-only bullets (TRUST-brian + SNAP) with PRESERVE / PRESERVE-WITH-TRIM / RELOCATE-TO-SKILL dispositions | estimated savings rain ~740-1,010 B + brian ~990-1,410 B per-agent (revised post-divergence-discovery) | 10 open questions for Brian BRAIN-2nd-PASS-2-FINAL on v1.1 | line-number references re-grep at impl-time per Brian PASS-1 Substantive-add #2 (M-2 c1 added 2 LOC each agent.go) | next: Brian PASS-2-FINAL → ratify → Brian-HANDS implements M-4 c1 per per-agent split ship-list AFTER M-1 c2 lands (terminal-collapse activation at single rebuild+restart per phase-m.md§DAG + user msg 7523).
