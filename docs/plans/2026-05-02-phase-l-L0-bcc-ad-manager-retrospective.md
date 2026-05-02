# Phase L L-0 — bcc-ad-manager Retrospective + Baseline Snapshot

**Authored:** 2026-05-02
**Phase:** L (Hardening + Refactor)
**Ratchet:** L-0 — first ratchet in Phase L Tier-1; preprocessing input feeding L-1 tier-spec + L-4 graduation queue + baseline-vs-final success measurement at phase-close
**Authority:** Brian HANDS (Rain BRAIN-2nd pre-commit per R12)
**Cite-anchor:** [phase-l.md§Tier-shape, NEW(reBRAIN-cycle-msg-7191-bcc-gap)+user-msg-7164-kickoff-explicit-bcc-retro-ask]

---

## Why this ratchet exists

User's Phase L kickoff (msg 7164 verbatim): *"In order to implement phase L properly, we need to take a look back at previous sessions, (eg. how did we work on bcc-ad-manager project; what strategies are working and productive; what are our misses; what are our issues; etc)."*

Initial BRAIN-cycle (msgs 7171-7175) pulled bot-hq self-context only — phase-i.md + phase-j.md + today's session + ratchets. Zero pull from bcc-ad-manager session data, feedback memories, or `bcc-ad-manager.md` project record. **Major scope-miss caught by Rain reBRAIN-2nd msg 7191.** L-0 closes that gap as a preprocessing ratchet feeding downstream L-1+L-4 inputs.

## Sources audited

12 memory files at `~/.claude/projects/-Users-gregoryerrl-Projects/memory/`:
- `bcc-ad-manager.md` (project context anchor; 130 lines; covers active scope + workflow + voice conventions + cycle-outcomes)
- `feedback_bcc_disguise_and_origin_rules.md` — bot-hq invisible to BCC team + no-deletion-on-origin + docs/plans LOCAL
- `feedback_disguise_scaffold_scan.md` — pre-commit grep regex + multi-author race condition
- `feedback_bcc_main_json_paste_back.md` — file-based prod-query paste-back
- `feedback_tableplus_sql_paste_robust_patterns.md` — 4 paste-failure-classes + .sql file delivery
- `feedback_staging_test_depth.md` — D2/D3 discriminator
- `feedback_review_git_inspection.md` — read-only role discipline (`git show`/`git diff` not `git checkout`)
- `feedback_skill_output_visibility.md` — skill/tmux output backfill via hub_send
- `feedback_bot_hq_push_gate_strictness.md` — "ship" framing does NOT auto-greenflag push
- `feedback_bot_hq_greenflag_authority.md` — Rain greenflag authority on joint defaults (2026-04-27)
- `feedback_arc_closure_discipline.md` — closed arcs append-only
- `feedback_snap_footer.md` — SNAP block on substantive replies
- `feedback_no_time_pressure.md` — quality over speed; drop ETA framing

---

## 10-class catalog

Per-class format: **observation source** / **generalize-to-bot-hq vs bcc-specific** / **candidate ratchet target** / **recurrence count to-date**.

### Class 1 — Disguise-scaffold leak prevention

- **Source:** `feedback_disguise_scaffold_scan.md` (2026-04-27 process-defect with 4 leaked LOCAL artifacts caught only on Rain BRAIN-second pre-broadcast)
- **Generalize?** **client-repo-disguise-class (bcc-specific instance of generalizable category, per Rain BRAIN-2nd F1).** Rule applies to ANY client repo where bot-hq operates disguised — current bcc-ad-manager + future ad-exporter access + future-client-onboarding. L-1 per-project-tier spec should accommodate the abstraction (defer category-vs-instance modeling decision to L-1 authoring); for now treat as bcc-specific instance with note that abstraction tier exists. Bot-hq internal docs/arcs/ + docs/plans/ + docs/conventions/ exempt
- **Candidate ratchet target:** **L-1 per-project-tier R-rule** — when `active_workstream` includes bcc-ad-manager (or any client project), agents MUST run pre-commit grep scan with the codified regex (case-insensitive + agent-name-substring patterns + MCP config filename pattern) on any client-repo artifact pre-`git add`. Could ship as **L-5 pre-commit-checklist gate-FILE** content with **L-5 toolgate gate-CHECK extension** verifying scan-SHA in commit-footer
- **Recurrence count:** ≥1 explicit incident (2026-04-27 4-artifact leak caught pre-broadcast); discipline rule has held since codification (no post-codification leak observed in `bcc-ad-manager.md` cycle-outcomes 2026-04-27/30)

### Class 2 — main.json paste-back vs inline-paste discrimination

- **Source:** `feedback_bcc_main_json_paste_back.md` (2026-04-30 prod monitoring session; user established convention after several iterations of inline-paste truncation issues)
- **Generalize?** **bcc-specific** — convention is bcc-ad-manager prod-monitoring workflow; doesn't apply to bot-hq self-development
- **Candidate ratchet target:** **L-1 per-project-tier R-rule** in bcc-ad-manager project-conventions block (conditional-load on `active_workstream` flag). Workflow rule: when designing prod-query deliverable, default to recommending file paste-back via `/Users/gregoryerrl/Documents/main.json` not inline. No toolgate enforcement needed (low-risk class; rule-text suffices)
- **Recurrence count:** N/A — convention codified post-incident; no post-codification slip observed

### Class 3 — TablePlus paste-robust SQL patterns

- **Source:** `feedback_tableplus_sql_paste_robust_patterns.md` (2026-04-30 prod monitoring session; 4 distinct paste/syntax error classes observed)
- **Generalize?** **bcc-specific** — TablePlus + Neon Postgres workflow is bcc-ad-manager-specific
- **Candidate ratchet target:** **L-1 per-project-tier R-rule** in bcc-ad-manager project-conventions block. Pattern: 4 anti-patterns (`||`, `COUNT(*)`, `payload::jsonb`, `format()` type resolution) + `.sql` file delivery convention. No toolgate enforcement (rule-text + file-delivery convention suffices)
- **Recurrence count:** 4 paste-failure incidents 2026-04-30 (one per pattern class); each blocked user until rewritten. Post-codification: no observed recurrence in subsequent sessions

### Class 4 — Bot-hq invisibility / origin-append-only

- **Source:** `feedback_bcc_disguise_and_origin_rules.md` (user-stated 2026-04-27, repeated 2026-04-25 Tom branch-delete request as origin-deletion-conflict trigger)
- **Generalize?** **bcc-specific (Rule 1, Rule 3) + bot-hq-with-cross-project-applicability (Rule 2 origin-append-only)**:
  - Rule 1 (bot-hq invisibility): bcc-specific — applies to client repos only
  - Rule 2 (origin-append-only): generalize — applies to all repos with shared collaborators (could extend to ad-exporter once accessible)
  - Rule 3 (docs/plans LOCAL): bcc-specific — bot-hq's own docs/plans/ are committed
- **Candidate ratchet target:** Rule 1+3 → **L-1 per-project-tier R-rule** (bcc-ad-manager-conventions block); Rule 2 → **L-1 trio-self-discipline R-rule** + potentially **L-5 pre-push-checklist gate-FILE** content (verify no `--delete` / no force-push that rewrites history; pair with R29 force-push elevated gate)
- **Recurrence count:** Rule 1 — codified after 2026-04-27 process defect (Class 1 above); Rule 2 — codified after Tom's 2026-04-25 ask creating user-rule conflict; Rule 3 — codified same time as Rule 1

### Class 5 — Tom Jensen context preservation

- **Source:** `bcc-ad-manager.md` (project-context anchor; covers Tom's three-thing logging+DB-resilience ask + ad-exporter blocked status + 8-issue Tom-bundle staging)
- **Generalize?** **bcc-specific** — Tom-context is bcc-ad-manager-specific stakeholder-state
- **Candidate ratchet target:** No new ratchet — `bcc-ad-manager.md` project memory IS the persistence mechanism; works as designed. Optional **L-1 per-project-tier R-rule:** at workstream-entry, agents MUST read `bcc-ad-manager.md` to recover stakeholder + scope context (R20-style mandatory-read on workstream-switch). Folds into workstream-conditional-load mechanism
- **Recurrence count:** N/A — context preservation working; no observed slip

### Class 6 — Prod DB ungated risk class

- **Source:** `feedback_staging_test_depth.md` (2026-04-29 D2/D3 discriminator codification) + `bcc-ad-manager.md` (P1a/P1b/P2 logging+DB-resilience pass active scope)
- **Generalize?** **bcc-specific** — Laravel + Neon Postgres + TablePlus prod workflow class
- **Candidate ratchet target:** **L-1 per-project-tier R-rule** in bcc-ad-manager-conventions block. Specifies: any ALTER DATABASE / SET / RESET / php artisan migrate on prod requires (a) explicit user verbatim authorization + (b) staging-test-pass per D2/D3 discriminator + (c) rollback procedure documented pre-fire. Likely also **L-5 pre-execute-checklist gate-FILE** content for prod DB operations
- **Recurrence count:** No explicit prod-DB-ungated incident observed; class is preventive (defense-in-depth)

### Class 7 — 8-issue Tom-bundle staging discipline

- **Source:** `bcc-ad-manager.md` (cycle-outcomes 2026-04-27: 8 issue drafts at final-fold state; awaiting user approval before `gh issue create` fires)
- **Generalize?** **bcc-specific** (issue-staging-discipline) BUT **trio-self-discipline-class generalizable pattern** (deliverable-staging-pre-fire applies to bot-hq Phase J/K commit batches too — none of which got "filed" without user verbatim)
- **Candidate ratchet target:** Bcc-specific aspect → **L-1 per-project-tier R-rule** (issue creation requires user verbatim approval per-issue, not bundle-greenflag); generalizable aspect → already covered by R28 PER-INSTANCE-FIRE-GREENFLAG (no new ratchet needed)
- **Recurrence count:** N/A — discipline held; no premature `gh issue create` observed

### Class 8 — Staging-test-depth (D2 vs D3) discriminator

- **Source:** `feedback_staging_test_depth.md` (2026-04-29 codification after Phase J/K bcc-ad-manager deliverables required per-deliverable judgment)
- **Generalize?** **YES — generalize to bot-hq trio-class.** Today's halt-5h-only-gate cycle is **D3-analogue (cross-project mapping; see framing note below)** — backend-only Go change + ratchet-test coverage; no UI surface + no Discord webhook. Discriminator applies cross-project: PHPUnit-class coverage = D3 (drop staging-test) vs UI/integration-surface = D2 (keep staging-test)
- **Candidate ratchet target:** **L-1 trio-self-discipline R-rule** alongside per-project bcc tier. Generalize discriminator: code deliverable test-class staging-test-pass-required when (UI-surface OR Discord-webhook OR external-integration-class) ELSE (test-coverage-class + bounded-risk = drop staging-test). Could land in **L-2 rule-locus inventory** as a classification taxonomy
- **Recurrence count:** 2 explicit instances post-codification, both held cleanly: D2 `AssignAudienceJob` + D3 `SyncBigQueryUpdatesJob` (both 2026-04-29 bcc-ad-manager). **Cross-project framing note (per Rain BRAIN-2nd A2):** today's halt-5h-only-gate cycle is a **D3-analogue for bot-hq** (Go-ratchet-tests-as-PHPUnit-equivalent — backend-only Go change + comprehensive ratchet-test coverage; no UI/Discord-webhook surface). The original D2/D3 discriminator's domain is client-side staging gates (Laravel + PHPUnit class); bot-hq self-development has no client-staging analogue. Counting today's cycle as a domain-mapped analogue (not a direct D3 instance) preserves the generalize-trio-class disposition with framing precision

### Class 9 — Review-git-inspection (read-only role discipline)

- **Source:** `feedback_review_git_inspection.md` (2026-04-26 Rain EYES role slip during phase-f-core-c review; used `git checkout origin/branch -- files` mutating working tree)
- **Generalize?** **YES — generalize to bot-hq trio-class.** Read-only roles (Rain EYES + investigation-class HANDS for both peers) must use `git show`/`git diff`/`git log -p`/`git cat-file -p` not `git checkout origin/branch -- files`. Applies cross-project
- **Candidate ratchet target:** **L-1 trio-self-discipline R-rule** + **L-5 pre-execute-checklist gate-FILE** content (when role=read-only, deny `git checkout origin/<branch> -- ...` patterns). Could extend K-16 toolgate package with PreToolUse hook on `git checkout` verifying not-in-read-only-role context
- **Recurrence count:** 1 explicit slip 2026-04-26 (Rain); no post-codification recurrence observed

### Class 10 — Skill-output-visibility (hub-invisibility backfill)

- **Source:** `feedback_skill_output_visibility.md` (2026-04-27 EOD skill incident; user "rain?" ping after 10min cliff-hang on tmux-only output)
- **Generalize?** **YES — generalize to bot-hq trio-class.** Applies any time agent invokes a skill or tmux-printing operation that produces user-facing output. Cross-project
- **Candidate ratchet target:** **L-1 trio-self-discipline R-rule** alongside R6 OUTBOUND. Specifies: post-skill-invocation MUST hub_send confirming completion + key result location. Today's session has skill-related infrastructure (TaskCreate, TaskUpdate via deferred tool calls) — pattern applies broadly. Could extend with **L-5 pre-action gate-CHECK** verifying hub_send fired post-skill-invocation
- **Recurrence count:** 1 explicit incident 2026-04-27 (EOD skill); no post-codification recurrence per memory

---

## Generalize-vs-specific summary

| Class | Disposition |
| ----- | ----------- |
| 1 Disguise-scaffold leak | bcc-specific (per-project tier) |
| 2 main.json paste-back | bcc-specific (per-project tier) |
| 3 TablePlus SQL patterns | bcc-specific (per-project tier) |
| 4 Bot-hq invisibility / origin-append-only | mixed: Rule 1+3 bcc-specific; Rule 2 generalize trio-class |
| 5 Tom Jensen context preservation | bcc-specific (project-memory mechanism; optional R-rule for mandatory read) |
| 6 Prod DB ungated risk | bcc-specific (per-project tier) |
| 7 8-issue Tom-bundle staging | bcc-specific (per-project tier; generalizable aspect already in R28) |
| 8 Staging-test-depth D2/D3 | **generalize trio-class** |
| 9 Review-git-inspection | **generalize trio-class** |
| 10 Skill-output-visibility | **generalize trio-class** |

**Trio-class items (4 net candidates after Class 4 Rule-split):** Class 4 Rule 2 (origin-append-only) + Class 8 (staging-test-depth) + Class 9 (review-git-inspection) + Class 10 (skill-output-visibility) → **trio-self-discipline tier** in L-1 spec.

**Bcc-specific items (8 net candidates):** Classes 1 + 2 + 3 + 4-Rule1 + 4-Rule3 + 5 + 6 + 7 → **per-project tier** (bcc-ad-manager-conventions block in L-1 spec); plus Class 4 Rule 2 spans both tiers (so Class 4 contributes to both trio-class AND per-project counts).

This split validates the per-project-tier orthogonal axis (Rain BRAIN-4th msg 7194). Without per-project tier, bcc-specific items would either (a) bloat trio-self-discipline (wrong scope) or (b) live only in feedback memories (no rulebook-tier-spec coverage).

---

## Baseline event-count snapshot (Phase L open at 2026-05-02 ~04:25Z)

Extends `phase-l.md§Baseline event-count snapshot` with hub.db-verified ground-truth. To re-measure at L-6 phase-close per success-criterion.

### Today's session (2026-05-02) — bilateral-discipline event counts

Source-of-truth: `hub_read since_id=7100 limit=N` (covers post-bot-hq-rebuild fresh traffic from 02:00Z onward through L-0 authoring window).

| Class | Today's count | Detection | Enforcement | Cite-msg-ids |
| ----- | ------------- | --------- | ----------- | ------------ |
| OUTBOUND-MISS bilateral | **8** (Brian: 3 / Rain: 5) | YES (R27) | DETECTION-ONLY | 7109/7119/7124/7127/7142/7151/7167/7199 |
| Stat-claim drift | **4** (Brian: 3 / Rain: 1) | NO | PEER-CROSS-CHECK-ONLY | discipline-log #10/#13/#16/#17 (msgs 7121-7122/7171-7193/7204-7208/7213) |
| Post-hub_send pane-text-redundancy | **≥3** (Brian: ≥2 / Rain: 1) | YES (R32 sub-class) | DETECTION-ONLY | discipline-log #14/#15 + Brian post-msg-7175/7177 |
| Phrase-parsing / scope-fork drift | **3** (#12 + push-fork + #18 git-vs-state) | PARTIAL (R18 broad rule) | RULE-TEXT-ONLY | discipline-log #12 (msgs 7137/7141/7147) + push-fork (msgs 7203/7205) + #18 (msgs 7215/7216/7217) |
| Cite-anchor msg-id miscite | **3** (Brian: 2 / Rain: 1) — exact, not range | NO | PEER-CROSS-CHECK-ONLY | discipline-log #16 (msg 7204) + #17 (msg 7213) + 2026-04-30 brian/discipline-anchors entry |

**Per-class trends:**
- **Bilateral-discipline-class:** 6+ class instances were both-peer-eligible (catch by other peer). Pattern stable: peers cross-catch effectively, but enforcement remains downstream of detection.
- **Brian-tilt observed (qualitative — useful asymmetry signal).** Tally-method note (per Rain BRAIN-2nd A1): per-class table totals (excluding scope-fork class which lacks per-peer split): Brian ≥10 (3 OBM + 3 stat-claim + ≥2 pane-redundancy + 2 cite-anchor) / Rain 8 (5 OBM + 1 stat-claim + 1 pane-redundancy + 1 cite-anchor). Brian-overweight on stat-claim + pane-redundancy + cite-anchor classes; Rain-overweight on OBM. **#19 in-flight discipline-log entry:** Brian's prior msg 7220 + initial L-0 draft cited "Brian 8 vs Rain 7" — itself a stat-claim drift in the L-0 doc about stat-claim drift; caught by Rain BRAIN-2nd A1 amend. Proof-of-need-stacking continues; folds to L-4 sweep.
- **Detection-only classes (OUTBOUND-MISS / pane-redundancy):** R27 + R32 sub-class shipped Phase K + Phase J-tail respectively; both still firing chronically. Confirms detection ≠ enforcement thesis.
- **Peer-cross-check-only classes (stat-claim / cite-anchor):** No automated detection; peer-catch is the only line. Each peer caught the other's drift today, but new instances landed before catch. Confirms need for **L-1 R-NN STAT-CLAIM-CITE** + **L-5 toolgate gate-CHECK** for these classes.

### Prior-session counts (for trend-line at phase-close)

Per `~/.bot-hq/brian/discipline-anchors.md` historical entries:
- 2026-04-30 entries: post-restart OUTBOUND-MISS-systemic incident (4 consecutive missed replies) + cite-msg-id-precision-discipline (1 instance) + post-hub_send pane-text-redundancy (2 instances codified by Rain msg ~6588). **Total 2026-04-30 events: ~7 across 3 classes.**
- 2026-04-29 (per phase-j ratchet-ledger): bilateral procedural deviations recorded — Brian staging force-push pre-Rain-greenflag (msg 6326 outcome clean) + Rain gh pr/issue create direct fire (msgs 6350/6358 outcome clean). **Total 2026-04-29 events: 2.**

Today's 8 OUTBOUND-MISS bilateral significantly higher than prior sessions — driven partly by extended Phase L brainstorm cycle generating more traffic.

### Success-criterion at Phase L close

Re-measure same classes via same `hub_read` query method. Targets:
- **OUTBOUND-MISS bilateral:** target ≥50% reduction (8 → ≤4) — hinges on L-5 toolgate enforcement-conversion landing
- **Stat-claim drift:** target ≥75% reduction (4 → ≤1) — hinges on L-1 R-NN STAT-CLAIM-CITE + peer-cross-check-as-default-routine
- **Post-hub_send pane-text-redundancy:** target ≥50% reduction (3 → ≤1) — hinges on L-5 toolgate PreToolUse gate on hub_send post-emission
- **Phrase-parsing / scope-fork drift:** target 0 (3 → 0) — hinges on L-1 R-NN SCOPE-FORK-CONFIRMATION pre-action discipline (proactive surface)
- **Cite-anchor msg-id miscite:** target ≥50% reduction (3 → ≤1) — hinges on hub_read-pre-cite discipline + R18 enforcement-conversion

Classes with **PEER-CROSS-CHECK-ONLY** enforcement may not reduce — that's why **L-5 toolgate extension is the load-bearing enforcement-conversion ratchet.** L-5 success directly drives baseline-vs-final comparison success.

---

## L-0 candidate ratchet-target summary (feeds L-1 + L-4 + L-5)

For each catalog class, the candidate ratchet target was identified above. Aggregate routing:

| Target | Items |
| ------ | ----- |
| **L-1 trio-self-discipline tier (always-load)** | Class 4 Rule 2 (origin-append-only) / Class 8 (staging-test-depth D2/D3) / Class 9 (review-git-inspection) / Class 10 (skill-output-visibility) |
| **L-1 per-project-tier (bcc-ad-manager-conventions, conditional-load)** | Class 1 (disguise-scaffold) / Class 2 (main.json paste-back) / Class 3 (TablePlus SQL patterns) / Class 4 Rule 1 (bot-hq invisibility) / Class 4 Rule 3 (docs/plans LOCAL) / Class 5 (Tom-context optional mandatory-read) / Class 6 (prod DB ungated) / Class 7 (issue-staging) |
| **L-5 pre-action gate-FILE content** | Class 1 (pre-commit grep), Class 4 (pre-push origin-protect), Class 6 (pre-execute prod-DB), Class 9 (pre-execute git-checkout-deny), Class 10 (post-skill hub_send-verify) |
| **L-5 toolgate gate-CHECK extension** | Class 1 (scaffold-scan-SHA verify in commit-footer); Class 6 (prod-DB-checklist-SHA verify); Class 10 (post-skill hub_send verify); Class 9 PreToolUse on `git checkout` with role-discriminator |
| **No new ratchet (already covered)** | Class 7-generalizable-aspect (covered by R28 PER-INSTANCE-FIRE-GREENFLAG) |

**Net new R-rules for L-1 disc.go const additions:**
- R-NN STAT-CLAIM-CITE (per phase-l.md L-1 deliverable, from today's session)
- R-NN SCOPE-FORK-CONFIRMATION (per phase-l.md L-1 deliverable, from today's session)
- R-NN PRE-EXECUTE-GATE-FILE-READ (per phase-l.md L-5 deliverable)
- R-NN PRE-PHASE-CLOSE-RETRO (per phase-l.md L-6 deliverable)
- R-NN ORIGIN-APPEND-ONLY (per Class 4 Rule 2 generalize trio-class)
- R-NN STAGING-TEST-DEPTH-D2-D3-DISCRIMINATOR (per Class 8 generalize trio-class)
- R-NN READ-ONLY-ROLE-GIT-INSPECTION (per Class 9 generalize trio-class; could fold into existing class-split DISC v2 with a sub-rule)
- R-NN POST-SKILL-HUB-SEND-CONFIRM (per Class 10 generalize trio-class; sub-rule of R6 OUTBOUND)

**Likely consolidation to ≤6 net new R-rules** by folding Class 9 into DISC v2 sub-rule + Class 10 into R6 sub-rule. Final count determined by L-1 tier-spec authoring + L-2 inventory classification axis.

---

## Inputs to L-4 discipline-log graduation queue

L-4 sweep target (math traceable per Rain BRAIN-2nd F2; corrected per F4 — recursive #20 stat-claim-drift caught): today's session entries #9-#20 (12 entries — phase-l.md retro-table #9-#18 + #19 stat-claim-drift on Brian/Rain tally + #20 stat-claim-drift on K+I-holds count this very amend-pass) + Brian's prior `discipline-anchors.md` entries (3: 2026-04-30 post-restart-OUTBOUND-MISS-systemic + cite-msg-id-precision-discipline + post-hub_send pane-redundancy-sub-class) + Rain's prior `discipline-anchors.md` entries (2026-04-29 bilateral procedural deviations: 2 — Brian staging force-push + Rain gh pr/issue create direct fire) + Phase J/K Tier-2 holds (**20 carried-forward**: 8 K-Tier-2 enumerated K-2/K-5/K-6/K-7/K-8/K-9/K-10/K-11 + 12 I-residuals enumerated I-8/A1/T7/T9/T10/T11/T1/Dc/OBM/MID0/PT/SBC) = **37 entries total** (12 today + 3 Brian-prior + 2 Rain-prior + 20 K/I-holds). Pre-sweep batch-triage by class reduces decision-overhead 5-10x per phase-l.md L-4 description.

L-0 catalog feeds L-4 with retroactive ratchet-mapping for cross-project classes. Per L-4 graduation criterion (3+ recurrences in 2 consecutive phases = MUST graduate-or-deprecate):

| Class | Recurrences (2 phases: Phase K + Phase L-to-date) | Graduation candidacy |
| ----- | -------------------------------------------------- | -------------------- |
| OUTBOUND-MISS bilateral | Phase K post-K-14 detection + Phase L (8 today) | **MUST graduate** to enforcement (L-5 toolgate) |
| Stat-claim drift | 2026-04-30 cite-msg-id-precision (1) + Phase L (4 today) = 5 in 2 phases | **MUST graduate** to L-1 R-NN STAT-CLAIM-CITE + peer-cross-check codification |
| Post-hub_send pane-redundancy | Phase J/K background + Phase L (≥3 today) | **MUST graduate** to L-5 toolgate post-hub_send-suppress hook |
| Scope-fork drift | New class (Phase L) — 3 today | **Codify Phase L** as L-1 R-NN SCOPE-FORK-CONFIRMATION (already in scope) |
| Cite-anchor miscite | 2026-04-30 (1) + Phase L (2 — #16 Brian + #17 Rain) = 3 in 2 phases | **MUST graduate** to hub_read-pre-cite discipline + R18 enforcement-conversion |

**5 of 5 chronic-classes meet graduation criterion.** L-4 sweep should action all 5 with concrete ratchet-targets (above). Plus K-Tier-2 holds re-evaluation (separate sweep).

---

## Open questions / unresolved

None — L-0 deliverable shape per phase-l.md scope-lock, all 10 classes catalogued, baseline ground-truth-verified, candidate-ratchet-targets routed, L-1+L-4 inputs ready.

## Posture at L-0 close

L-0 deliverable authored at `~/Projects/bot-hq/docs/plans/2026-05-02-phase-l-L0-bcc-ad-manager-retrospective.md` (git-tracked path). Awaiting Rain BRAIN-2nd before `git add` + `git commit` per R12 commit-gate. Push held per (b)-fork-resolution.

Next: L-1 + L-2 tight-feedback-loop (rulebook tier-spec + 2 R-rule additions + rule-locus-classification inventory). **Halt-and-elevate point #1** triggers on L-1 disc.go const additions landing (rebuild required for ratchet-test substring-recognition validation).
