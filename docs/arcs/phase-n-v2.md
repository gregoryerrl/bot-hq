# Phase N v2 — bot-hq trio meta-protocol arc snapshot

**Closed:** 2026-05-06
**Scope:** hook-class + test-infra + ID-sessions impl cluster (per (α-tighter) v2 cluster framing at `~/.bot-hq/phase/phase-n.md§v2`)
**Phase N v3 follow-up:** OQ-5/-6/-7 productionize-class + Tier-2 carry-forwards

---

## 1. Scope-lock recap

User absolute-greenlight at msg 8075 ("absolute-greenlight (everything including push-greenlight)") opened Phase N v2 scope covering commit + push class for the v1-deferred cluster. Bilateral PASS-2-FINAL at Brian msg 8080 + Rain BRAIN-2nd msg 8079 locked an 8-commit DAG: Tier-2-bundle (#1) → N-3 (#2) → N-2 (#3) → N-1(b)-A/B/C (#4-#6) → N-1(c) (#7) → close-composite (#8). Conditional split-trigger at heartbeat or context-cap >0.85; D2-split-elective fired at emma PRE-COMPACT-SNAP 0.90 (msg 8153) — 7/8 commits pushed pre-rebuild, #8 deferred to fresh session.

## 2. Phase N v2 commits (8 commits, 7 pre-rebuild + 1 post-rebuild close-composite)

| # | Commit | SHA | Subject | Class |
|---|---|---|---|---|
| 1 | N-T2-bundle | 9705621 | R36-sub HANDSHAKE-ACK-BLIND-SPOT + R31-sub FILESYSTEM-SIGNAL-CITE rule-text + ratchet | rule-text + ratchet (light) |
| 2 | N-3 | 590aa81 | R39 TEST-ISOLATION rule-text + ratchet | rule-text + ratchet (light-medium) |
| 3 | N-2 | e52ba87 | R40 VOICE-MIRROR-DISCIPLINE + voicemirror PreToolUse-hook (alert-only) + tests | hook-class (medium) |
| 4 | N-1(b)-A | e64d8b9 | sessions package — boundary-detector + manifest-author | impl (medium-heavy) |
| 5 | N-1(b)-B | d9bb05d | hub_session_load MCP tool + CLI + helpers | impl (medium) |
| 6 | N-1(b)-C | 92b8222 | index.md maintenance + hub_register auto-load | impl (light-medium) |
| 7 | N-1(c) | 55eb1f1 | integration tests + /id-sessions skill-pointer wiring | tests + skill (medium) |
| 8 | close | (this composite) | Phase N v2 close-composite — voicemirror install subcommand + arc-snapshot + ratchet-ledger update + discipline-log Joint entry + R34 reflexive-bootstrap 4th self-application | bundle |

## 3. Tier-2 re-eval (per Phase L/M precedent)

| Item | Disposition |
|---|---|
| N-T2-c R36 sub-clause / inline note | **GRADUATED** — landed as R36-sub HANDSHAKE-ACK-BLIND-SPOT in #1 N-T2-bundle |
| N-T2-d R31 filesystem-signal sub-class | **GRADUATED** — landed as R31-sub FILESYSTEM-SIGNAL-CITE in #1 N-T2-bundle |
| N-T2-h PreToolUse-hook-on-Write user-artifact paths | **GRADUATED** — merged with N-2 voice-mirror hook in #3 |
| N-T2-a R15 self-flag carve-out extension | Re-deferred Phase N v3 (preflight-CRITICAL-self-detection class candidate) |
| N-T2-b Toolgate-class estimate-band refinement | Re-deferred Phase N v3 (R37 sub-clause OR fixture-density modeling guidance) |
| N-T2-e Bilateral over-asking pattern formalization | Already pinned as `feedback_pivoted_context_authority_scope.md` HARD RULE; re-deferred (durable covers) |
| N-T2-f Force-push per-instance override pattern | Re-deferred (`feedback_bot_hq_push_gate_strictness.md` durable append candidate) |
| N-T2-g Trim-pre-flight skill-edit-traceability | Graduated-by-pattern-adoption (Phase M+N1 precedent); convention-class |
| N-T2-i Audit-doc-presence-check | Re-deferred Phase N v3 (Phase L Tier-2 hold extension) |

## 4. Baseline-vs-final event-count comparison

| Class | Baseline (Phase N v2 open) | Phase N v2 close | Delta | Notes |
|---|---|---|---|---|
| OUTBOUND-MISS notifications during Phase N v2 window | ~5-10 expected | TBD per hub.db query | n/a | R36 Stop-hook BLOCKING enforcement-conversion live since Phase M; pre-rebuild runtime had R36-sub absent → "." pendulum class observed (carry-forward #14) |
| Discipline-log carry-forward instances pending | 6 from v1 close + carry | 14 net (3 graduated to v2 ratchets; 14 new instances + Tier-2 carry surfaced this cycle) | +8 net | Includes pendulum-class 8-reversal trace (#11/#12) + minimal-hub_send-not-free (#14) — strongest empirical for R36-sub load-bearing-ness |
| Bilateral cross-check correction instances | ~3-5 expected | 5-7 during Phase N v2 (Rain Q2 OQ-1 split push-back + D2-split converge + bilateral pendulum settle + cite-precision msg-id-self-cite drift) | similar rate | Discipline holding |
| Test-claim overstatement instances post-N-4 | 0 expected | 0 observed | 0 | Confirms N-4 R31-sub-clause efficacy (Phase N v1 commit 1fe7baa) |
| ID-based-session creation events | 0 pre-#4 deploy | non-zero post-#5 deploy expected post-rebuild | n/a until trio post-rebuild test sweep | **Phase N v3a amendment (post-fire):** v2 close-composite framing of "non-zero post-#5 deploy expected" referred to LOAD-side delivery (`hub_session_load` MCP tool + `hub_register` auto-load via `MostRecentForProject` — wired in v2 #5/#6/#7). WRITER-side runtime integration was carry-forward to v3a (`hub_session_create` + `hub_register` calling `WriteManifest` + `WriteIndex` idempotently). See discipline-log §2026-05-06T(post-v2-close-pre-v3-open) joint entry R31 OVER-CLAIM phase-close-arc-snapshot-class sub-class anchor for the framing-class drift this amendment addresses. |
| session_id=NULL ratio | 100% pre-#4 | <50% expected post-#5 deploy | TBD | Measurement at next-session-cluster opening per N-1 (a) §6 step 5 design. **Phase N v3a amendment:** writer-side wired in v3a — measurement now empirically possible. |

## 5. Retrospective

### What worked
- **Absolute-greenlight pre-delegation working as designed** — user msg 8075 covered scope+commit+push class; Brian + Rain drove 7-commit batch end-to-end without per-commit re-asking (per `feedback_pivoted_context_authority_scope.md` analogue applied to bot-hq scope). Push-fire at msg 8207 (2a9097b..55eb1f1) without per-instance verbatim token gate.
- **D2-split-elective at heartbeat 0.90 trigger** — Phase N v2 scope-lock §Realistic-scope conditional split-trigger fired cleanly at emma PRE-COMPACT-SNAP 0.90 (msg 8153). Rain hub_flag elevated WARNING msg 8157 + Brian PASS-3 split-CONCUR msg 8158 + bilateral split-RATIFIED msg 8162. 7/8 split discipline working under load.
- **R37 BYTE-PROJECTION-CITE dual-stage discipline** — applied at staged-time across all 7 commits; drift envelope ±25% held for #1+#2+#5+#6+#7 (within band) + carry-forward observation for #3+#4 toolgate-class drift (Phase M precedent #38/#40 reinforced).
- **Self-recursive empirical-to-rule turnaround** — R36-sub HANDSHAKE-ACK-BLIND-SPOT shipped in #1 caught its own load-bearing motivation in the same cycle: bilateral 8-reversal pendulum (msgs 8210→8228) emerged pre-rebuild because R36-sub was not yet active in pre-rebuild runtime. Post-rebuild runtime activates R36-sub → terminates pendulum class by construction at first peer-most-recent-scan.

### What surfaced (carry-forward)
- **Pendulum-class under cross-in-flight without R36-sub-active runtime** (carry-forward #11/#12) — bilateral state-reversal 8-deep when handshake-terminator "." emit reflexively closes loop while peer's most-recent message carries substantive content. Empirically validated as R36-sub recursion-deactivation gap pre-rebuild; activated post-rebuild.
- **Minimal-hub_send-not-free on bare-"." ping cycle** (carry-forward #14) — Rain's minimal-ack pattern (msgs ~8240-8262) consumed ~5-10% plan-usage cumulatively across ~30+ ping cycles. User-CRITICAL flagged at msg 8264. Bilateral commitment: zero bare-"." emissions; true-silence preferred over minimal-ack (accept R36 stop-hook block on truly-silent turns). Recovery mechanism: substantive-content-only or true-silence; never reflexive minimal-ack.
- **Toolgate-class LOC-estimate drift continued** (#3 voicemirror hook + #4 sessions impl over-bound by similar pattern to Phase M #38/#40) — establishes class-distinguishing signature: hook-class + impl-class + test-fixture-density-modeling structurally over-bound estimates. Phase N v3 candidate: refine R37 sub-class OR estimate-band differentiation per commit-class.
- **Cross-restart-resume bootstrap composability** — R16 CROSS-RESTART-RESUME-OPERATIONAL bootstrap order (a)/(b)/(c)/(d) held cleanly across HALT-95% + plan-reset + rebuild+restart sequence. Preserved-working-tree carry intact through pendulum-storm settle. Strengthens R16 mechanism.

## 6. Phase N v3 carry-forward scope

**v3a amendment (post-fire):** id-sessions writer-flow runtime wiring (the (α) bilateral-locked carry from v2 close — `hub_session_create` + `hub_register` calling `sessions.WriteManifest` + `sessions.WriteIndex` idempotently with project param) **NOW DONE** in v3a. See `internal/mcp/tools.go` `hubSessionCreate` + `hubRegister` handlers post-v3a commit + `~/.bot-hq/discipline-log.md` §2026-05-06T(post-v2-close-pre-v3-open) joint entry for the framing-class anchor that prompted this amendment. The other v3 scope below (web UI + Clive + rules) is the bigger Phase N v3 cluster per `~/.bot-hq/phase/phase-n.md§v3`; remaining productionize items moved to Phase O.

**Deferred to Phase O (post-v3 amendment):**

- **OQ-5 retention policy + age-based pruning** — productionize-class for ID-sessions (`~/.bot-hq/sessions/` cleanup over time)
- **OQ-6 privacy + secrets-scan-on-manifest-author** — productionize-class
- **OQ-7 cross-session-search indexed lookup** — productionize-class (extends N-1(b)-C index.md prep)
- **N-T2-a R15 self-flag carve-out extension** (preflight-CRITICAL-self-detection class)
- **N-T2-b Toolgate-class estimate-band refinement** (R37 sub-clause OR fixture-density modeling)
- **N-T2-i Audit-doc-presence-check** (Phase L Tier-2 hold extension)
- **Phase N v2 15-item discipline-log carry-forward queue** (per §discipline-log Joint entry; #55 added post-staged-diff via Rain msg post-8286 R36-mechanical-block empirical)
- **Recursion-terminator candidate** — Brian-side post-bootstrap self-id verification (tmux session-name vs `BOT_HQ_AGENT_ID` env match) per discipline-log Phase N v3 cycle-open self-confabulation root-cause analysis

## 7. Cross-references

- Phase N v2 scope-lock: `~/.bot-hq/phase/phase-n.md§v2` (per R10 SCOPE-LOCK-BEFORE-IMPL)
- Phase N v1 arc-snapshot: `docs/arcs/phase-n-v1.md` (carry-from precedent)
- Phase M arc-snapshot: `docs/arcs/phase-m.md` (R34 reflexive-bootstrap precedent)
- Discipline-log Phase N v2 Joint entry: `~/.bot-hq/discipline-log.md` (15-item carry-forward queue)
- Ratchet-ledger Phase N v2 section: `~/.bot-hq/ratchets/active.md` (prepend per L-3a precedent)
- N-1 (a) design-spike: `docs/plans/2026-05-05-phase-n-N-1-id-based-sessions-design-spike.md` (RATIFIED leans Q-I=δ / Q-II=spec'd / Q-III=c / Q-IV=iii / Q-V=s)
- /id-sessions skill: `~/.claude/skills/id-sessions/SKILL.md` (154L)
- New packages: `internal/sessions/` (boundary-detector + manifest-author) + `internal/voicemirror/` (PreToolUse-Write hook + install subcommand)
- 4 NEW R-rules + 1 sub-pair: R36-sub HANDSHAKE-ACK-BLIND-SPOT + R31-sub FILESYSTEM-SIGNAL-CITE + R39 TEST-ISOLATION + R40 VOICE-MIRROR-DISCIPLINE + IdSessionsSkillPointer wiring
- User absolute-greenlight (msg 8075) + bilateral converge msgs 8076-8088 + push-fire msg 8207 + bilateral pendulum 8210→8228 + user CRITICAL ".-cycle" msg 8264 + post-rebuild RESUME msg 8274
- Pre-commit-checklist SHA: `d41e877d4b0176ba7acaa92441d2938b8b386401ad605b9a2014371661afa472` (R33 cite)
- Pre-phase-close-checklist SHA: `e17abd0acf9d5ebaaa6c77efb9a664ad8ff93217ccf398dd4625f7022dad3e56` (R34 reflexive-bootstrap 4th self-application)

---

(end Phase N v2 arc-snapshot v1; bilateral PASS-1 → Rain BRAIN-2nd → close-composite stages + commits)
