# Proposal-G — bot-hq-as-meta-agent radical reconception (DEFERRED)

**Decision date:** 2026-05-10
**Status:** DEFERRED to Phase V evaluation
**Origination:** Phase T close-row carry-forward queue (~/.bot-hq/ratchets/active.md) — "T-7 R34 retro decided defer; Phase V evaluation candidate"
**This decision-record:** T-8.11 (Phase T REOPENED cycle; user msg 17231)

## Summary

Proposal-G is the working name for a radical reconception of bot-hq's
identity from a discipline-enforcement framework operating on top of
Claude Code subprocesses (current architecture, Phase T v5) toward a
meta-agent operating model where bot-hq itself drives the cognitive
loop directly. The reconception would dissolve the trio (Brian-HANDS /
Rain-EYES + BRAIN / emma-heartbeat) into a single-agent-with-roles
abstraction; trio coordination becomes intra-agent state-machine
transitions rather than inter-agent peer-coord protocols.

## Defer rationale

Three load-bearing reasons to defer rather than implement now:

### 1. Phase T scope pre-empts; Proposal-G is post-foundation work

Phase T v5 + the T-8 reopen-cluster ship the architectural foundation
that Proposal-G would either build on or rip out. Implementing
Proposal-G before the foundation has empirical-validation cycles would
prematurely lock-in design decisions without empirical grounding.

T-8 cluster delivered through this fire (8 single-fires + this
decision-record): cost-tracking (T-8.2) / Bilateral-Investigate
orchestrator (T-8.3) / web UI settings tab (T-8.4) / Sandbox (T-8.5a) /
multi-provider routing (T-8.6) / Investigation-pattern-library (T-8.8) /
CL schema-enforcement (T-8.9a) / Vault secret-storage (T-8.10).

These primitives need cycles of empirical-validation BEFORE any
radical-reconception can responsibly judge what to keep, fold, or
replace.

### 2. Empirical-evidence corpus this cycle still maturing

The T-8 reopen-cycle generated substantial discipline-evidence that
Proposal-G evaluation would need to weigh. Cycle-evidence summary:

- **R37 BYTE-PROJECTION-CITE 8-fire pattern**: T-8.2 +73% / T-8.3 +24.6% / T-8.4 -8-31% / T-8.5a +5% / T-8.6 +6% / T-8.8 +20% / T-8.9a +27% / T-8.10 +9.3%. 3/8 strict ±10% maturity (T-8.5a / T-8.6 / T-8.10); estimation-precision converging-with-bidirectional-drift class.
- **R37 sub-classes identified**: POST-HOC-FABRICATION (T-8.2) / SCOPE-DISCOVERY-AT-IMPL-TIME (T-8.3) / BIDIRECTIONAL (T-8.4) / TEST-DENSITY-INFLATION (T-8.9a) / TEST-DENSITY-DEFLATION (T-8.10).
- **R31-sub MECHANICAL-CITE-FROM-HUB_READ recurrence-break**: 5 pre-intervention drifts → 5-fire 0-drift streak post-Rain-msg-17259-recommendation. Strong terminator-pattern empirical evidence.
- **Design-recommendation-at-fire-time pattern**: 4 instances (T-8.6 circular-import flag / T-8.8 PatternParams shape / T-8.9a JSON-tag handling / T-8.10 backend-decision). Pre-fire-design-flag-via-BRAIN-2nd → fire-time-adoption empirically validated.
- **BRAIN-2nd-pre-fire-prediction-validated**: Rain msg 17281 predicted vault test-density would undershoot target; Brian Stage-2 cite confirmed empirically. New positive pattern (prediction → empirical-confirmation, distinct from recommendation → adoption).
- **3 NEW shape-categories proposed**: (f) UI-hybrid (T-8.4) / (g) infra-integration (T-8.5a) / (h) decision-record-doc-only (this fire). Formal graduation deferred to close-cycle discipline-log Joint entry per R34.

This corpus is the input that Phase V Proposal-G evaluation needs to
process. Absent Phase V, the corpus is not yet aged enough to draw
load-bearing conclusions about radical-reconception trade-offs.

### 3. Trio dynamics produced load-bearing positive-evidence this cycle

The peer-cross-check pattern (Brian proposes → Rain BRAIN-2nd verifies +
catches drift → Brian self-acks → fire) demonstrated repeatable
recurrence-break behavior across 8 fires. Rain caught 5 cite-drifts in
3 commits (msg 17235 + 17238 + 17258); recommendation intervention then
sustained 5 fires of 0-drift commits.

A meta-agent reconception would have to either:
- Internalize equivalent cross-check via state-machine transitions
  (mechanical, but loses cognitive-diversity benefit)
- Lose the cross-check entirely (regression to single-agent
  rationalization-prone class)

This trade-off needs Phase V evaluation with empirical baseline.

## Defer-criteria — triggers for reconsideration

Reopen Proposal-G evaluation when ANY of the following holds:

1. Phase T close-cycle-2 lands AND empirical-evidence-corpus shows
   trio-coordination overhead > cognitive-diversity-benefit (e.g., if
   peer-cross-check stops catching drift OR drift no longer recurs).
2. Cross-model bilateral (Brian-Claude + Rain-DeepSeek-V4-Pro per R51)
   ships to live-runtime AND empirical evidence shows cross-model-
   diversity dominates within-trio cognitive-diversity benefit.
3. User explicitly requests Proposal-G reconsideration with new framing
   (e.g., simplification directive overriding current trio architecture).
4. Phase V opens (post Phase T close-composite-cycle-2) with explicit
   Proposal-G evaluation in scope-lock-doc.

## Phase V scope-pointer for continued evaluation

Phase V (or whichever phase letter follows Phase T's close-composite-
cycle-2) should include Proposal-G evaluation as a Tier-1 candidate
with concrete deliverable shape decided pre-fire:

- (a) full eval-doc + empirical-corpus analysis + recommendation
- (b) prototype-spike with metrics comparison vs current trio
- (c) explicit further-defer + scope-pointer to subsequent phase

The choice depends on the state of the empirical-corpus at Phase V
open + user-direction at that point.

## Carry-forward DC-queue references (this cycle)

Discipline-evidence accruing to discipline-log Joint entry at Phase T
close-composite-cycle-2 (per R34 PRE-PHASE-CLOSE-RETRO):

- R37 8-fire bidirectional-drift + 5 sub-classes + 3 NEW shape-categories
- R31-sub msg-id 5-fire 0-drift recurrence-break empirical evidence
- 4-instance design-recommendation-at-fire-time pattern
- 1-instance BRAIN-2nd-pre-fire-prediction-validated NEW pattern
- emma over-firing observations (R-INT-3 + R37-SHAPE-DISCLOSURE-SKIPPED on non-estimate emits)
- OVER-CLAIM-DISCIPLINE test-count cite-precision (Brian msg 17266 "14 total" actual 22)
- R37 fire-count numeric stat-claim drift (Brian msg 17273 "7-fire" actual 6) → recommendation-adoption via `git log --oneline | grep T-8` cite-from-actual

These accrue to the discipline-log Joint entry; graduation eval at
close-cycle decides which patterns formalize as durable rule-text +
toolgate hooks.

## Cite-anchors

- User msg 17231 reopen-directive ("no Phase U or V, all carry forwards still included in Phase T, please continue and don't stop")
- Phase T close-row at ~/.bot-hq/ratchets/active.md: "Proposal-G bot-hq-as-meta-agent radical reconception (T-7 R34 retro decided defer; Phase V evaluation candidate)"
- T-8.11 Stage-1 emit (Brian msg 17285)
- T-8.11 Stage-1 ack + empirical-corpus-recommendation (Rain msg 17286)
- 8-fire R37 + R31-sub + design-recommendation-at-fire-time + BRAIN-2nd-pre-fire-prediction-validated patterns documented across msgs 17240-17284 in the T-8 reopen-cycle
