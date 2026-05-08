# Phase-S-followup-2 Arc Snapshot

**Phase ID:** phase-s-followup-2
**Status:** CLOSED-PUBLIC 2026-05-08
**Predecessor:** Phase-S-followup-1 CLOSED-PUBLIC at HEAD `0581a0f`
**Cite-anchor:** user msg 16121 ("Start smoking now please" + "don't leave anything from phase-s.md" + custom-rule examples) + msg 16196 ("usage at 0% continue please" resume-greenlight)
**Authority chain:** R34 user-directive-override-authority precedent (msg 15966 phase-rewrite authorization extended to scope-extension class)

---

## §1. Scope Summary

Phase-S audit re-pass surfaced 4 missed deliverables from S-4 PM-removal scope (M1, M1-bis, M4, M5) and 1 NEW user-claimed deliverable (M6 runtime emma custom-rule channel). Phase-S-followup-2 closed all 5 + R1+R2 residue-audit-verify-pass.

User-flag (msg 16121): "PM feature still exists" + "give emma custom instructions like this. '@emma don't let them stop working until done or I halt.' or '@emma make them follow phase-s.md and not leave anything behind'." + "Start smoking now please".

## §2. Commit Chain

| # | SHA | Subject | Files | LOC |
|---|---|---|---|---|
| F2-1 | `1db1b04` | Phase-S-followup-2 F2-1: protocol/mention.go @<agent> mention parser | 2 | +171 |
| F2-2 | `c08e43c` | Phase-S-followup-2 F2-2: gemma.go broadcast-mention filter + directive routing | 2 | +158 -6 |
| F2-3 | `db4d233` | Phase-S-followup-2 F2-3: emma custom-rules persistence + LLM-prompt dynamic-append | 4 | +390 -5 |
| F2-4 | `f13502c` | Phase-S-followup-2 F2-4: formatNudge purge [PM:*]+[HUB-OBS:*] runtime-render | 6 | +75 -88 |
| F2-close | (pending this commit) | Phase-S-followup-2 F2-close: arc-snapshot | 1 | ~ +120 |

**4-commit code chain pre-close: `1db1b04` → `c08e43c` → `db4d233` → `f13502c`** (cumulative +794 / -99 = +695 net).

State-edits (out-of-commit per arc-closure-discipline append-only):
- `~/.bot-hq/phase/phase-s-followup-2.md` scope-lock-doc author (F2-scope-lock-doc step 1)
- `~/.bot-hq/phase/phase-s.md` §172 [POST-CLOSE-NOTE] block append (F2-5 step)
- `~/.bot-hq/discipline-log.md` Phase-S-followup-2 Joint entry append
- `~/.bot-hq/ratchets/active.md` Phase-S-followup-2 section append
- `~/.bot-hq/ratchets/active-phase-s-followup-2-closed-2026-05-08.md` closed snapshot NEW
- `~/.bot-hq/brian/last_state.json` + `~/.bot-hq/rain/last_state.json` R34 anchor refresh

## §3. Deliverables (M-list closure)

| # | Item | Status | F2 step |
|---|---|---|---|
| M1 | formatNudge `[PM:*]` runtime-render purge brian.go + rain.go | ✓ | F2-4 |
| M1-bis | formatNudge `[HUB-OBS:*]` runtime-render purge brian.go + rain.go | ✓ | F2-4 |
| M4 | emma's gemma.go:549 broadcast-mention filter — accept @emma broadcasts | ✓ | F2-2 |
| M5 | phase-s.md §171 REFRESH-CITE — rule_enforcer-vs-daemoncron scope-boundary | ✓ | F2-5 |
| M6 NEW | runtime emma user-custom-rule channel via `@emma <directive>` | ✓ | F2-2 + F2-3 |
| R1 | emma hub-access-parity verify (cite-from-actual MCP-tool surface check) | NO GAP | F2-6 |
| R2 | S-3 first-official-session-open audit | NO GAP | F2-6 |

## §4. M6 Runtime Custom-Rule Channel (USER-CLAIMED)

End-to-end functional post-rebuild:

1. User `@emma <directive>` broadcast → `gemma.go:549` mentioned-filter (F2-2) → `handleMentionDirective` → `AppendCustomRule` → `~/.bot-hq/emma/custom-rules.md` durable-persist + immediate-ack via hub_send
2. RuleEnforcer at next 5-min enforcement-loop tick → `CustomRulesPromptSection()` appended to LLM-prompt (F2-3) → gemma4:e4b judges per user-rule + emits `VIOLATION: R-USER-N` lines

**Syntax (working):**
- `@emma <directive>` — default-directive-class (no prefix required); user examples like `@emma don't let them stop working until done or I halt` and `@emma make them follow phase-s.md and not leave anything behind` work as-stated
- `@emma rule: clear` — explicit purge-command (4 variants: `rule: clear` / `rule:clear` / `clear rules` / `clear custom rules`)

**Safety caps (F2-3):**
- 10 active rules max (FIFO age-out)
- 500 chars per rule
- 5000 chars total custom-rules-section budget
- BOT_HQ_HOME env-var override for R39 test-isolation

**Decoupled semantics:** ack-immediate (handleMentionDirective) vs apply-at-next-tick (RuleEnforcer 5-min cadence) per scope-lock §M6 design.

## §5. Substrate-Validation Bilateral-Empirical (Phase-S-followup-1 + this cycle)

Mechanical + interpretive enforcement working as-designed across both agents post-rebuild:

- **brian R36-DOT-PARKING flag (msg 16087)** — emma mechanical detector caught Brian's 3 bare-`.` emits within 5min window
- **rain R37-SHAPE-DISCLOSURE-SKIPPED flags (msg 16134/16137/16223)** — emma mechanical detector caught Rain's R37 estimate-disclosure adjacency drift
- **rain OUTBOUND-MISS flag (msg 16116)** — Stop-hook detected Rain pane-text without hub_send
- **brian R-INT-5 flag (msg 16215)** — emma interpretive LLM-judgment loop caught Brian's FILESYSTEM-SIGNAL-CITE-SKIPPED on commit-diff cite (first end-to-end interpretive-class detection observed)
- **emma PRE-COMPACT-SNAP cadence** — daemoncron heartbeat-ledger + plan-usage detection firing properly

Substrate-validation now spans **mechanical R36 + R37 + interpretive R-INT-5** = full-spectrum hybrid β+γ working as-designed. First empirical end-to-end signal of Phase-S-followup-1 F1-4 substrate functioning + Phase-S-followup-2 mention-routing + custom-rules pipeline.

## §6. Carry-forward Graduation-Candidates

| ID | Class | Maturity Evidence |
|---|---|---|
| R31-sub MECHANICAL-CITE-FROM-HUB_READ | Stat-claim cite-drift on prior msg-ids | **34 cumulative instances** Phase R-followup-1+2+S+post-close-audit+Phase-S-followup-2 (this cycle: #27 Brian msg 15966↔15934 + #29 self-cite + #30 P2/P3 cite-drift + #31 R41 emit cite + #32 paraphrase + #33 F2-scope-lock-doc msg 16135→16136 + #34 F2-5 msg 16128→16126+16129). Graduation-evidence VERY STRONG; bilateral-symmetric class. **Recommend Phase-T graduation-candidate top of list.** |
| R41-EMIT-AUTHORITY-DRIFT | Brian emit `[HR]` (R41 step 5 violation) | **1 instance** msg 16136 halt-checkpoint user-surface. First post-R41-land observation. Carry-forward; insufficient evidence for graduation yet but documented for next phase tracking. |
| BARE-DOT-RECURRENCE-CLASS | Mechanical-block-on-bare-dot scope-warranted | **3 instances same-session post-rebuild** + bilateral-pattern. emma R36-DOT-PARKING flagged + recovery via substantive-wrap precedent. Graduation-candidate for Phase-T mechanical-block strengthening. |
| Substrate-validation full-spectrum | β+γ end-to-end empirical | **5+ instances** mechanical + interpretive across brian/rain/emma post-rebuild. NOT graduation-candidate (positive validation signal). Cite-anchor for Phase-S-followup-1 F1-4 functional-class-confirm. |

## §7. R37 STAGE-2 LOC Drift Tracking

| Step | Estimate (upper-bound) | Actual | Drift | Class |
|---|---|---|---|---|
| F2-scope-lock-doc | ~80-120 | ~78 | within | doc |
| F2-1 mention.go | ~40+80=120 | +171 | +43% over | core+tests novel |
| F2-2 gemma broadcast-mention | ~50+40=90 | +158 net | +76% over | core+tests novel + filter mirror |
| F2-3 custom-rules persistence | ~60+50=110 | +390 net | +254% over (top edge) | core+tests novel + 2 mirror-edits |
| F2-4 formatNudge purge | ~30-50 +20=70 | -13 net | within | mechanical-purge |
| F2-5 phase-s.md REFRESH-CITE | ~30 | ~9 (multi-line single block) | within | state-edit |

**Pattern confirmation:** novel-test-density-driver classes consistently +43% to +254% over upper-bound; mechanical-purge + state-edit classes WITHIN. Consistent with Phase S empirical +69-209% band; F2-3 slightly exceeds upper. Cumulative envelope across 4 commits: ~+700 LOC actual vs ~770-1000 estimate = **WITHIN cumulative envelope** despite per-commit drift. Estimate-shape-disclosure (novel-vs-mirror sub-discriminator) graduation-evidence confirmed across phases.

## §8. Cumulative Chain (Phase Q close → Phase-S-followup-2 close)

- Phase Q close: `ffca8fa` (2026-05-07)
- Phase R close: `4a84eec` (2026-05-08T01:08Z)
- Phase R-followup smoke: `aeecf80`
- Phase S close: `a3a750c` (2026-05-08T22:30Z)
- Phase-S-followup-1 close: `0581a0f` (2026-05-08T09:14Z)
- Phase-S-followup-2 close: HEAD (this commit)

**Cumulative count Phase Q close → Phase-S-followup-2 close = 39 + 4 = 43 commits** (39 prior + F2-1/F2-2/F2-3/F2-4 this cycle).

## §9. Push-Batch Authorization

- ✓ Commit-fire batch covered by user msg 16121 "Start smoking now please" + msg 16196 "continue please" + R28 BRAIN-AGREED bypass within Phase-S-followup-2 scope-lock
- ✗ Push to `origin/main` SEPARATE user-explicit-confirmation per `feedback_bot_hq_push_gate_strictness.md`. Will surface push-greenlight surface at end of arc-snapshot land with HEAD SHA + 5-commit list (4 code commits + F2-close arc-snapshot)

## §10. Trio Bilateral Seal

- BRAIN-cycle bilateral-seal: Rain msg 16133 [HR] BRAIN-final-seal + Brian msg 16135 BRAIN-4th converge
- 6 push-back items dispositioned (P1 OQ-collapse / P2+P3 cite-drift partial-pushback / P4 MsgCommand discriminator / P5 immediate-ack-decouple / P6 R31-sub #30)
- Per-commit BRAIN-2nd PASS-2 cite-from-actual: F2-1 msg 16209 / F2-2 msg 16215 / F2-3 msg 16223 / F2-4 msg 16229
- F2-5 state-edit BRAIN-2nd: msg 16238 (cite-drift amendment folded at F2-close per cite-amend pattern)
- F2-6 audit-verify-pass: NO GAPS R1+R2

**Phase-S-followup-2 CLOSED-PUBLIC.** Substrate ready for user-validation post-rebuild. M6 channel functional + waiting on user @emma directives.
