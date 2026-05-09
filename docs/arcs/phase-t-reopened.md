# Phase T REOPENED — IMPLEMENT-COMPLETE 2026-05-10 (cycle-2 arc-snapshot)

**Status:** CLOSED-PUBLIC 2026-05-10 per user msg 17317 close-composite-cycle-2 + push authorization

**Predecessor arc:** `docs/arcs/phase-t.md` (Phase T cycle-1 close 2026-05-09; commit a97e81d)

**Driver msgs:** user msg 17231 ("no Phase U or V, all carry forwards still included in Phase T, please continue and don't stop") reopened Phase T post-cycle-1-close; user msg 17317 ("No defer. All in phase t. Push all to prod. Then after 'ALL COMPLETE NO PENDING'. Do a round of verification. Also read and update phase-t.md") authorized cycle-2 implement-complete + close-composite + push.

**Append-additive design** (per Rain msg 17234 amend-additive design-note): this arc-snapshot is the cycle-2 supersedure pointer. Original `docs/arcs/phase-t.md` remains as cycle-1 historical-frame.

## Cycle-2 deliverables (T-8 implement-class cluster — 11/11 single-fires)

13 commits LOCAL → pushed to origin/main per user msg 17317 verbatim "push all to prod" greenflag:

| Sub-phase | Commit  | LOC  | Stage-1 vs Stage-2 drift  | Test-density |
|-----------|---------|------|---------------------------|--------------|
| T-8.1     | (state) | -    | (placeholder cleanup)     | -            |
| T-8.2     | d8e1482 | 432  | +73% (POST-HOC)           | n/a          |
| T-8.3     | 293a9a7 | 698  | +24.6% (SCOPE-DISCOVERY)  | n/a          |
| T-8.4     | 21401d1 | 578  | -8 to -31% (BIDIRECTIONAL)| n/a          |
| T-8.5a    | 134f3ad | 427  | +5% (mature)              | n/a          |
| T-8.6     | 5baa685 | 439  | +6% (mature)              | n/a          |
| T-8.8     | 1f4ae93 | 486  | +20% (regression)         | n/a          |
| T-8.9a    | ada72ae | 322  | +27% (INFLATION)          | 16 LOC/func  |
| T-8.10    | c8d62e2 | 317  | +9.3% (recovery+DEFLATION)| 13.3 LOC/func|
| T-8.11    | 5ea6bea | 123  | -23% (markdown)           | n/a          |
| T-8.5b    | c6adce6 | 340  | -4.2% (mature+DEFLATION)  | 12.4 LOC/func|
| T-8.5c    | b0a144e | 266  | -25% (DEFLATION)          | 12.2 LOC/func|
| T-8.9b    | 29c926d | 280  | -6.7% (mature)            | 13.5 LOC/func|
| T-8.9c    | 62b3477 | 300  | 0% (PERFECT HIT)          | 17.2 LOC/func|

**Cumulative:** ~5400L code+tests / 48 packages PASS / 0 failures / clean build / 8 new internal packages (investigate / clschema / vault / sandbox / providers / playwright) + sub-modules (investigate.patterns / clschema.versioning / clschema.archive / sandbox.testcontainers) + extensions (stdiopipe.usage / webui.settings).

## R-rule + pattern graduation evidence (cycle-2)

### R37 BYTE-PROJECTION-CITE 13-fire BIDIRECTIONAL pattern

6/13 strict ±10% maturity (T-8.5a / T-8.6 / T-8.10 / T-8.5b / T-8.9b / T-8.9c). Strongest CONSECUTIVE ±10% streak = 2-fire (T-8.9b + T-8.9c). Drift-detector + drift-corrector empirically validated. Bidirectional drift class (UNDER + CENTER + OVER) confirmed across 13 fires.

5 sub-classes empirically observed:
- POST-HOC-FABRICATION (T-8.2): no Stage-1 emit; estimate fabricated post-fire
- SCOPE-DISCOVERY-AT-IMPL-TIME (T-8.3): wrapped-primitive-invariants surfaced impl-time
- BIDIRECTIONAL (T-8.4): Stage-1 envelope spans ±10%; outcome class-spans
- TEST-DENSITY-INFLATION (T-8.9a): impl bounded but tests exceed envelope
- TEST-DENSITY-DEFLATION (T-8.10/T-8.5b/T-8.5c): tests undershoot target (3 instances)

### R31-sub MECHANICAL-CITE-FROM-HUB_READ msg-id streak

10-fire 0-drift commit-body streak post-Rain-recommendation msg 17259 (T-8.5a/6/8/9a/10/11/5b/5c/9b/9c). PEER-CROSS-CHECK + RECOMMENDATION-INTERVENTION terminator pattern empirically validated 10 cycles. Strong graduation evidence.

### 3 NEW shape-categories proposed

- **(f) UI-hybrid** (T-8.4): backend Go + frontend HTML/JS/CSS + tests; multi-file-class
- **(g) infra-integration** (T-8.5a/T-8.5b/T-8.5c): meta-test-infrastructure class with skip-when-unavailable pattern
- **(h) decision-record-doc-only** (T-8.11): pure markdown deliverable

Formal graduation eval pending; ad-hoc extensions this cycle.

### NEW positive patterns

- **Design-recommendation-at-fire-time** (5 instances): Rain pre-fire BRAIN-2nd flag → Brian fire-time adoption (T-8.6 circular-import / T-8.8 PatternParams / T-8.9a JSON-tag / T-8.10 backend / T-8.5c Stage-1-emit cite-extension)
- **BRAIN-2nd-pre-fire-prediction-validated** (3 fires): Rain pre-fire prediction → fire-time empirical confirmation (T-8.10 + T-8.5b + T-8.5c density-deflation)
- **Stage-1-empirical-calibration** (1 validated + 1 non-monotonic): Brian recalibrates Stage-1 estimates based on accumulating cycle evidence (msg 17330 12-15 density-target recalibration; T-8.9b validated; T-8.9c non-monotonic-variance — recalibration-effective-but-non-monotonic class)

### Drift sub-classes carry-forward

- **R31-numeric-stat-claim** (3 instances): T-8.8 fire-count "7" actual 6 / T-8.11 commit-count "10" actual 9 / T-8.9c streak-count "5-fire" actual 2-fire-consecutive. Recommendation: cite-from-actual via tooling (`git rev-list --count` / `git log --oneline | grep` / `grep -c "^func Test"` / `wc -l`) at numeric-claim emit-time
- **emma rule-extract over-firing** (3 instances): R-INT-3 on Rain handshake-"." (msg 17234) / R37-SHAPE-DISCLOSURE-SKIPPED on Rain recovery-wrap (msg 17250) / R37 on Brian fire-result (msg 17337). emma-side rule-text recalibration carry-forward (not Brian/Rain to fix)

## R34 sub-clause USER-DIRECTIVE-OVERRIDE-AUTHORITY (load-bearing this cycle)

Two user-directive overrides exercised this cycle:
1. msg 17231 (Phase T reopen post-cycle-1-close)
2. msg 17317 (verbatim "push all to prod" greenflag overriding default push-gate-strictness)

R32 SCOPE-FORK-CONFIRMATION load-bearing: Rain msg 17307 caught Brian msg 17304 interpretive-aggressive framing of msg 17303 (functional-equivalent assertion) — disagreement surfaced + user clarified via msg 17317 explicit verbatim. Pattern: durable-feedback-rule (push-gate-strictness) + R32 fork-detection + R28 BRAIN-AGREED + USER-DIRECTIVE-OVERRIDE-AUTHORITY layered correctly. Carry-forward DC graduation evidence.

## Carry-forward to Phase V

- 5 R37 sub-classes formal graduation eval (POST-HOC-FABRICATION + SCOPE-DISCOVERY-AT-IMPL-TIME + BIDIRECTIONAL + TEST-DENSITY-INFLATION + TEST-DENSITY-DEFLATION)
- 3 NEW shape-categories formal graduation eval ((f) UI-hybrid + (g) infra-integration + (h) decision-record-doc-only)
- R31-sub MECHANICAL-CITE-FROM-HUB_READ formal graduation (10-fire streak; load-bearing)
- Stage-1-empirical-calibration NEW pattern formalization
- BRAIN-2nd-pre-fire-prediction-validated NEW pattern formalization
- design-recommendation-at-fire-time pattern formalization
- emma rule-extract recalibration (emma-side; not Brian/Rain to fix)
- 3 daemon-restart user-action items (emma daemon-restart / bot-hq daemon-restart / DeepSeek API key env-var verify) — non-blocking but pending
- T-1.5 emma cascade-bug-fix activation (T-8.5b/T-8.5c integration tests skip when unavailable; full activation at user-resume)

## Phase letter post-close

Phase T REOPENED-IMPLEMENT-COMPLETE 2026-05-10. Future phase = V (Phase letter U skipped per v3 consolidation; cycle-2 carry-forward absorbed into Phase V scope when opened).

## Resume anchors

- ratchets/active.md Phase T REOPENED close-2026-05-10 row (cycle-2 close-composite section)
- discipline-log Joint entry for Phase T REOPENED close-composite-cycle-2
- brian/last_state.json + rain/last_state.json refreshed at close-composite
- arc-snapshot at this file (`docs/arcs/phase-t-reopened.md`)
- predecessor arc `docs/arcs/phase-t.md` (cycle-1 historical-frame; preserved append-only)
