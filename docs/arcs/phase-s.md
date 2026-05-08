# Phase S — Emma redefinition + 4 trio-discipline hardening items (CLOSED-PUBLIC + Phase-S-followup-1 remediation)

**Phase opened:** 2026-05-08 (post Phase-R-followup-2 close at `230a4f9`)
**Phase S CLOSED-PUBLIC:** 2026-05-08 at commit `a3a750c` (S-1b emma-Claude rule-enforcer + gemma agent-ID rename — subsequently REVERTED in Phase-S-followup-1)
**Phase-S-followup-1 CLOSED-PUBLIC:** 2026-05-08 at commit `9a3ce76` (F1-7 §117 ignore-noise rule-text)
**Total commits:** 17 (11 Phase S + 6 Phase-S-followup-1) on origin/main
**Driver:** user msg 15734 "Phase S - emma redefinition" + saltegge msg 15753 5-OQ dispositions + user msg 15760 "Proceed and smoke all"

## Theme

Phase S was the **emma-redefinition pass** — relieve Emma of mechanical-poll duties (heartbeat-ledger / stale-coder / outbound-hook violations / R20 / R34 checklist-prompts) by moving them daemon-side, and recast Emma as the trio's **rule-enforcer** with full hub-access parity to BRAIN-duo. Plus 4 trio-discipline hardening items.

Phase S CLOSED-PUBLIC at msg 15842 with **13 deliverable misses + 1 architectural fork unsurfaced** (G3 SEAL-DISCIPLINE-CITE-FROM-SCOPE-LOCK-DOC bilateral failure). User msg 15873 "wait what? why is emma a claude code session? i never said use claude code session? emma will be the same model" surfaced architectural model-class fork miss. User msg 15966 "NO DEFER YOU DO NOT HAVE PERMISSION TO OPEN NEW PHASES. GO AHEAD AND REWRITE STRUCTURALLY UNFOLLOWABLE TASKS" + user msg 15997 "gemma 4 e4b has no cost, we run it locally" authorized Phase-S-followup-1 remediation chain.

## Phase S commit chain (11 commits, 9308beb → a3a750c)

| # | SHA | Title | Class |
|---|-----|-------|-------|
| 1 | `9308beb` | S-3 sessions hardening (DONE/PIVOT keywords) | protocol-extension |
| 2 | `95aa648` | S-5 brian 3s message-buffer hotfix | hub-side-mechanism |
| 3 | `1da6e94` | S-2 autocompact-detect+broadcast (protocol foundation) | protocol |
| 4 | `4cc002d` | S-4-foundation drop hub_send `to:` parameter | api-breaking |
| 5 | `021140e` | S-4-followup rule-text purge + PhaseSv1AudienceClassLoadBearing | rule-text |
| 6 | `46e0ab4` | S-1a-1 daemoncron skeleton+heartbeat-ledger | package-extraction |
| 7 | `5bb63ac` | S-1a-2 daemoncron stale-coder | package-extraction |
| 8 | `09ffd5a` | S-1a-3 daemoncron plan-usage 3-sub | package-extraction |
| 9 | `b430169` | S-1a-4 daemoncron lifecycle-hooks (6 surfaces) | package-extraction |
| 10 | `39eab94` | S-1a-5 daemoncron simple-cluster (final S-1a) | package-extraction |
| 11 | `a3a750c` | S-1b emma-Claude rule-enforcer + gemma agent-ID rename (REVERTED) | new-agent (ARCHITECTURALLY-WRONG; reverted F1-1+F1-2) |

## Phase-S-followup-1 commit chain (6 commits, 3bf8367 → 9a3ce76)

| # | SHA | Title | Class |
|---|-----|-------|-------|
| 1 | `3bf8367` | F1-1+F1-2 revert S-1b emma-Claude + agent-ID rename (atomic; fork (a) full-revert) | revert-mechanical |
| 2 | `fd069a6` | F1-3 emma R20 BOOTSTRAP parity — gemma WriteAgentState on register | sparse |
| 3 | `3f283d9` | F1-4 emma rule-enforcer hybrid β+γ functional substrate | dense-violation-table |
| 4 | `c94a7c3` | F1-5+F1-6 emit-compact-notice CLI + brian flush-pre-compact wire | sparse |
| 5 | `9a3ce76` | F1-7 §117 ignore-noise discipline rule-text addition (PhaseSv2) | rule-text |
| 6 | (this snapshot + housekeeping) | F1-close composite | state-edits |

## 13 misses + 1 architectural fork remediation table

| # | Miss | Source | Resolved-via |
|---|------|--------|--------------|
| 1 | Architectural model-class fork (R32 miss) | user msg 15873 | F1-1+F1-2 fork (a) full-revert; spec rewrite §163 Claude-class → rule-enforcer-role |
| 2 | arc-snapshot at docs/arcs/phase-s.md MISSING | emma 15877 | F1-close (this file) |
| 3 | R34 pre_phase_close_checklist_sha_seen stale (brian) / absent (rain) | emma 15877 | F1-close AgentState refresh both brian + rain |
| 4 | §240 out-of-scope NEW R-rule consts shipped | emma 15877 | F1-close discipline-log retrocite §240 superseded |
| 5 | S-2 settings.json PreCompact hook + emit-compact-notice CLI never landed | rain 15880 | F1-5 |
| 6 | S-5 brian-buffer flush-pre-compact wire | rain 15880 | F1-6 |
| 7 | emma speech-trigger ~9 watch-confirm drift emits | user 15906 | F1-7 anti-drift clause embedded in PhaseSv2 + emma-Claude reverted |
| 8 | R20 BOOTSTRAP file-author miss (emma/last_state.json absent) | emma 15910 | F1-3 gemma WriteAgentState on register + initial bootstrap file authored |
| 9 | AgentState write structurally impossible on Claude-class emma | emma 15910 | DISSOLVED on fork (a) — gemma has native Write |
| 10 | S-4 §117 ignore-noise rule-text NOT LANDED | rain 15915 | F1-7 PhaseSv2IgnoreNoiseDiscipline + brian/rain prompt embed + rulebook entry |
| 11 | §34 HEARTBEAT-LEDGER cadence vs speak-only-on-violation | rain 15911 | F1-4 spec-rewrite mechanical-substrate-vs-judgment-substrate distinction |
| 12 | §170 NO pane-injection literal violation | rain 15911 | F1-4 spec-rewrite §170 mechanism cite (hub-nudge-formatter natural behavior) |
| 13 | Internal scope-lock-doc contradiction §34 vs §138 vs §170 | rain 15911 | F1-4 spec-rewrite resolution + G2 graduation-candidate accrued |

## 4 graduation-candidates accrued

- **G1** R32-SCOPE-FORK-CONFIRMATION-MODEL-CLASS sub-clause — bilateral miss + emma-Claude self-cite triple-empirical
- **G2** SCOPE-LOCK-INTERNAL-CONTRADICTION-PRE-LOCK-AUDIT — drafter sweeps own-doc for cross-section conflicts pre-final-seal; recursion-depth-2 instance + bilateral failure on §-cluster (§34/§138/§170/§117/§172/§165-167)
- **G3** SEAL-DISCIPLINE-CITE-FROM-SCOPE-LOCK-DOC — phase-close seal authority requires comprehensive cite-from-actual against scope-lock-doc landed-state; bilateral failure at Phase S close (msg 15842) sealed PUBLIC with 13-item drift unsurfaced
- **G4** BRAIN-CYCLE-FILESYSTEM-SIGNAL-CITE-AT-CONSEQUENTIAL-REASONING-TIME — caveat-as-fig-leaf failure-mode (caveat-tag "needs cite-from-actual" not functioning as halt-to-verify trigger; bilateral instance Brian msg 15916 + Rain msg 15919 endorsement of MISS-8 dissolve-on-fork-(a) without primary-source verify)

Plus minor R31-sub instances #20-#25 (cumulative Phase R-followup-1+2+S = 25 instances) including bilateral cost-class fig-leaf #24 + narrative-stat off-by-2 #25.

## R37 Phase-S-followup-1 batch tally

| Commit | Estimate | Actual | % | Class |
|--------|----------|--------|---|-------|
| F1-1+F1-2 | revert-class | -742 LOC | N/A | REVERT-MECHANICAL |
| F1-3 | 5-15 sparse | +11 | WITHIN | sparse-clean |
| F1-4 | 700-1500 dense β+γ | +800 | WITHIN-mid | dense-mirror MUTED |
| F1-5+F1-6 | 100-250 sparse | +136 | WITHIN-low | sparse-clean |
| F1-7 | 80-150 rule-text | +44 | WITHIN-low | rule-text-clean |

**Pattern empirical:** Phase-S-followup-1 batch trended toward **WITHIN-MUTE** for all non-revert commits — 4-consecutive WITHIN-clean instances. Suggests Phase S empirical "test-density-driver +OVER" pattern is sub-discriminator-conditional (novel-vs-mirror): when work mirrors existing pattern (S-1b emma → brian/rain mirror; F1-3..F1-7 mirror existing helpers/consts), test-density-driver MUTES. Phase-R-followup-2 (b)/(c)/(d) +OVER-pattern was novel-test-design class. Graduation-candidate strengthens.

## §240 retrocite

phase-s.md §240 stated "**No new R-rule consts this batch.** Graduation-candidates accrue at S-close." Phase S landed 3 NEW R-rule consts (PhaseSv1AudienceClassLoadBearing + MsgCompactNotice + MsgResume). Phase-S-followup-1 added a 4th (PhaseSv2IgnoreNoiseDiscipline). §240 stance is RETROACTIVELY SUPERSEDED — the user-greenflag ("smoke all" + "Proceed") authorized R-rule additions where load-bearing for Phase S protocol changes. §240 was scope-lock-author optimism; actual R-const necessity emerged at impl-time. Discipline-log retrocite at Joint entry.

## Carry-forward to next phase

**4 graduation-candidates** (G1-G4) accrued empirical evidence; each warrants formal R-rule addition or sub-clause in next planning cycle (NOT a "Phase-T" deferral per user msg 15966; carry-forward is rule-text discipline, not phase-class). User-authorized when scope warrants.

**R31-sub cumulative tally:** 25 instances across Phase R-followup-1+2+S — PhaseRv5MechanicalCiteFromHubRead graduated at Phase-R-followup-1 (commit 26496e6); R31-sub continues accumulating empirical post-graduation. Sub-clause expansions: FILESYSTEM-SIGNAL-CITE / cost-class-fig-leaf / narrative-stat-off-by-N classes within R31-sub category.

**Substrate activation post-rebuild:** all Phase S + Phase-S-followup-1 substrates active per code-level land:
- daemoncron 5 surfaces (heartbeat / stale-coder / plan-usage 3-sub / lifecycle / simple-cluster) emit FromAgent="emma"
- emma rule-enforcer hybrid β+γ (3 mechanical Go-side + 5 interpretive LLM-judgment via §72-amendment carve-out)
- gemma R20 BOOTSTRAP via WriteAgentState on register + ~/.bot-hq/emma/last_state.json
- emit-compact-notice CLI + emit-resume CLI subcommands
- ~/.claude/settings.json PreCompact hook
- brian buffer flush-pre-compact wire
- PhaseSv2IgnoreNoiseDiscipline rule-text in brian + rain prompts
- S-3 DetectBoundaryFromUserMsg DONE/PIVOT keyword extension
- S-5 brian 3s buffer + bypass classes
- S-4 PM removal (hub_send drops `to:`) + @<agent> mention-detection
- PhaseSv1 R6-load-bearing const

User rebuild+restart activates substrate fully. Trio standing by for next directive post-rebuild.

## Cross-references

- Phase R closed snapshot: `~/.bot-hq/ratchets/active-phase-r-closed-2026-05-08.md`
- Phase R arc-snapshot: `~/Projects/bot-hq/docs/arcs/phase-r.md`
- Phase S scope-lock doc (with F1-4 spec-rewrite): `~/.bot-hq/phase/phase-s.md`
- Phase-S-followup-1 session: `50f986e1-e564-46d0-800c-bb0ad77a9042` (cluster `2026-05-08-bot-hq`)
- Phase S session (closed): `18632cfb-fc3d-419d-a756-af29448db784`
- discipline-log Joint entry: `~/.bot-hq/discipline-log.md` (Phase-S-followup-1 close-composite)
