# Rule Locus Inventory

**Authored:** 2026-05-02 (Phase L L-2)
**Status:** v1.1 — post-Rain-BRAIN-2nd amend-pass-1; tight-feedback-loop with L-1 rulebook-tier-spec.md
**Purpose:** single source-of-truth enumeration of every rule in the bot-hq rulebook system, classified per L-1 tier × scope axes. Drives L-3a prompt-shrink classification + L-4 graduation queue + staleness-detection ratchet-test.
**Cite-anchor:** [phase-l.md§Tier-shape row L-2, NEW(BRAIN-cycle-msgs-7171-7173)+(reBRAIN-msg-7191-merge-with-classification-axis)]
**Source-pin (per Rain BRAIN-2nd F4 staleness-detection):** disc.go last-edit SHA = **86521b6** (K-22 R30 HR-tag; pre-Phase-L). Verify via `git log -1 --format=%H -- internal/protocol/disc.go`. If SHA changes without inventory update, this doc is stale-by-source-edit and must be reconciled before next Phase L commit.

---

## Inventory schema

Per L-1 rulebook-tier-spec.md§Per-rule classification axis. Each row:

| Column | Meaning |
| ------ | ------- |
| **rule-name** | Stable identifier (R-NN shorthand or const-name) |
| **locus** | Where authoritative text lives (`const` / `prompt` / `skill` / `file`) |
| **tier** | Durability axis (1/2/3/4) |
| **scope** | Project axis (trio-self / per-project-<id>) |
| **tests** | Ratchet-test names asserting recognition + behavior |
| **hooks** | toolgate hook names (empty = no toolgate enforcement) |
| **enforcement-status** | `DETECTION-ONLY` / `RULE-TEXT-ONLY` / `PEER-CROSS-CHECK` / `TOOLGATE-ENFORCED` |
| **recurrence-count** | Phase L baseline event count (per L-0 baseline) |
| **graduation-target** | L-4 sweep concrete next-step |

Recurrence-count ground-truth: hub.db query (`hub_read since_id=<phase-open> filter <substring>`) + L-0 baseline event-count snapshot.

---

## Trio-self-discipline rules

### Tier-1 always-resident (in agent prompts)

| rule-name | locus | tests | hooks | enforcement-status | recurrence-count | graduation-target |
| --------- | ----- | ----- | ----- | ------------------ | ---------------- | ----------------- |
| **R6 OUTBOUND** (DiscV2OutboundRule) | const disc.go:21; embedded rain.go + brian.go prompt | `TestRainPromptContainsOutboundRule` / `TestBrianPromptContainsOutboundRule` (substring lock) | none — would need PostToolUse hook to detect missing hub_send post-turn-end | DETECTION-ONLY (R27 detects after-the-fact); RULE-TEXT-ONLY otherwise | OBM bilateral 8 today (Brian: 3 / Rain: 5) | **MUST graduate** — L-5 toolgate PostToolUse hook on session-turn-end verifying hub_send fired; OR settings.json hook |
| **R12 GATE-PROTOCOL** (push/merge user-only ABSOLUTE) | const PhaseIv1ProtocolHardening (disc.go:32 onward inline rule); embedded both prompts | `TestPhaseIv1ContentShape` substring lock | K-13 toolgate PreToolUse (toolgate.VerifyCommit) on `git commit` | TOOLGATE-ENFORCED (commit-gate); RULE-TEXT-ONLY (push/merge gates) | 0 push-gate violations today (push-fork resolved (b)); 0 commit-gate violations | **stable hold** — toolgate-enforced for commit-class; consider extending to push-class L-5 |
| **R13 cite-anchor verify-fail-without-cite** (R18 today's numbering) | const PhaseIv1ProtocolHardening | `TestPhaseIv1ContentShape` | none | RULE-TEXT-ONLY + PEER-CROSS-CHECK | 3 cite-anchor msg-id miscites today (#16/#17/2026-04-30) | **MUST graduate** — extend with hub_read-pre-cite discipline; L-1 R-NN STAT-CLAIM-CITE adjacent class |
| **R14 95%-plan-usage SNAP halt** | const PhaseIv1ProtocolHardening | substring lock | runtime in plan_usage.go (Emma-driven, not toolgate) | DETECTION-FIRES-RUNTIME | N/A (not yet fired Phase L) | stable hold |
| **R15 AGENT-AUTHORITY-MATRIX** | const PhaseIv1ProtocolHardening | substring lock | K-16 toolgate PreToolUse class-split gate (rain blocked from HANDS) | TOOLGATE-ENFORCED (class-split); RULE-TEXT-ONLY (delegated authorities) | 0 violations today | **stable hold** |
| **R16 CROSS-RESTART-RESUME-OPERATIONAL** | const PhaseIv1ProtocolHardening | `TestRainPromptContainsR16Bootstrap` / `TestBrianPromptContainsR16Bootstrap` substring lock | none | RULE-TEXT-ONLY | 0 bootstrap-skip events today (post-restart bootstrap held cleanly) | stable hold |
| **R17 SOURCE-OF-TRUTH-HIERARCHY** | const PhaseIv1ProtocolHardening | substring lock | none | RULE-TEXT-ONLY + PEER-CROSS-CHECK | 1 violation today (#12 Rain phrase-parsing missed cite-anchors) | partial-graduate via L-1 R-NN SCOPE-FORK-CONFIRMATION (covers fork-able-phrase sub-class) |
| **R18 CITE-ANCHOR-REQUIRED** | const PhaseIv1ProtocolHardening | substring lock | K-13 toolgate VerifyCommit (footer cite verification) | TOOLGATE-ENFORCED for commit-footer-cite; PEER-CROSS-CHECK otherwise | 3 miscites today (#16/#17/2026-04-30) | **MUST graduate** — adjacent to R-NN STAT-CLAIM-CITE; extend toolgate gate-CHECK to non-commit cite-anchors |
| **R19 CYCLE-CLOSE-USER-BLOCKING** | const PhaseIv1ProtocolHardening | substring lock | none | RULE-TEXT-ONLY | 0 violations today | stable hold |
| **R20 BOOTSTRAP-ON-CONVERSATION-RESUME** | const PhaseIv1ProtocolHardening | `TestPhaseIv1ContentShape`; AgentState helpers in agentstate.go | none (state-side-only — write detected via file-mtime) | RULE-TEXT-ONLY + state-write-cadence (heartbeat-driven) | 4 AgentState writes today (post-restart + 3 heartbeat opportunistic) | stable hold |
| **R21 MSG-TYPE-TAXONOMY** | const PhaseIv1ProtocolHardening | `TestMessageTypeTaxonomy` (types.go) | none | RULE-TEXT-ONLY | minor drift (msg-types observed: response/update/result; not all 6 taxonomy ones in use) | stable hold; K-6 §5.3-MsgUpdate→Result migration deferred |
| **R22 PRE-COMPACT-SNAP** | const PhaseIv1ProtocolHardening + plan_usage.go runtime | substring lock | Emma plan_usage.go emit (runtime, not toolgate) | DETECTION-FIRES-RUNTIME | ≥6 PRE-COMPACT-SNAP events today | stable hold |
| **R23 HEARTBEAT-LEDGER** | const PhaseIv1ProtocolHardening + plan_usage.go runtime | `TestPlanUsageHeartbeatLedger` / `TestPlanUsageHeartbeatLedgerCadence` | Emma runtime emit | DETECTION-FIRES-RUNTIME | 4 HEARTBEAT-LEDGER events today (post-restart 7158/7183/7210 + later) | stable hold |
| **R24 MUTUAL-HALT-PROTOCOL** | const R24MutualHaltProtocol (disc.go:129) | substring lock | none — would need PreToolUse pattern detection | RULE-TEXT-ONLY | 0 fires today | stable hold |
| **R25 PM-VS-BROADCAST-AUTHORIZATION** | const R25PMVsBroadcastAuthorization (disc.go:262) | substring lock | none | RULE-TEXT-ONLY | 0 violations today | stable hold |
| **R26 R12-COMMIT-GREENFLAG-FOOTER** | const R26R12CommitGreenflagFooter (disc.go:239) | `TestVerifyCommit*` (toolgate r12.go) | K-13 toolgate.VerifyCommit | TOOLGATE-ENFORCED | 2 commits today (c02f2be + fcae26e); both passed gate | stable hold |
| **R27 OUTBOUND-MISS-SELF-RECOGNITION** | const R27OutboundMissSelfRecognition (disc.go:260) | substring lock + `IsOutboundMissNotification` helper | none | DETECTION-ONLY (helper enables agent self-detection); no enforcement | 8 OBM events today | **MUST graduate** — adjacent to R6 graduation target; toolgate PostToolUse hub_send-verify |
| **R28 PER-INSTANCE-FIRE-GREENFLAG** | const R28PerInstanceFireGreenflag (disc.go:218) | substring lock | partial via K-13 commit-gate (R26 footer covers per-instance for commit) | TOOLGATE-ENFORCED for commit; RULE-TEXT-ONLY for push/merge/gh-pr-create | 0 violations today (BRAIN-AGREED-bypass active per Phase L user msg 7199) | stable hold |
| **R29 FORCE-PUSH-ELEVATED-GATE** | const R29ForcePushElevatedGate (disc.go:192) | `TestVerifyCommit*` covers force-push detection | K-13 toolgate `IsForcePushPattern` | TOOLGATE-ENFORCED | 0 force-pushes today | stable hold |
| **R30 HR-TAG-DISCRIMINATOR** | const R30HRTagDiscriminator (disc.go:162) | substring lock | none | RULE-TEXT-ONLY + PEER-CROSS-CHECK | minor — both peers have used [HR] correctly today on user-decision-affecting msgs | stable hold |
| **R27-pane-only-sub-class** (was "R32 pane-only" — renamed per Rain F2 to remove R-NN numbering collision) | inline (referenced in agent prompts; not separate const; sub-class of R27 OUTBOUND-MISS-SELF-RECOGNITION) | none yet | none | DETECTION-ONLY (covered by OUTBOUND-MISS broadcasts) | ≥3 today (Rain pane-text emits) | partial-graduate via L-5 toolgate hub_send-mirror enforcement |
| **DISC v2 class-split** (HANDS / EYES / BRAIN) | const PhaseIv1ProtocolHardening (inline) | `TestRainCannotExecuteHANDSPattern` (toolgate gate.go) | K-16 toolgate PreToolUse class-split gate | TOOLGATE-ENFORCED | 0 violations today | stable hold |
| **HALT-ALL-WORK (H-31, H-33)** | const PhaseJv1HaltResumeProtocol (disc.go:89) | `TestRainPromptContainsHaltAllWork` / `TestBrianPromptContainsHaltAllWork` substring lock | runtime in plan_usage.go (Emma-driven) | DETECTION-FIRES-RUNTIME | 0 fires today (post-rebuild 5h-only-gate gate now controls) | stable hold |
| **RESUME-FROM-HALT** | const PhaseJv1HaltResumeProtocol | substring lock | runtime emit | DETECTION-FIRES-RUNTIME | 0 fires today | stable hold |
| **H-13 FORCE-PUSH TOKEN PROTOCOL** | const H13ForcePushProtocol (disc.go:108; brian-only embed) | substring lock | K-13 toolgate `IsForcePushPattern` (overlaps R29) | RULE-TEXT-ONLY (token-protocol); TOOLGATE-ENFORCED (force-push detection) | 0 fires today | stable hold |
| **HANDSHAKE-TERMINATOR** ('.' compact) | inline rule in agent prompts | substring lock | none | RULE-TEXT-ONLY | ~6 terminator pairs today (cycle-closes) | stable hold |
| **SCOPE-LOCK-BEFORE-IMPL** | inline rule | none | none | RULE-TEXT-ONLY | held cleanly today (phase-l.md scope-lock pre-L-0) | stable hold |
| **AUDIENCE-CLASS-DISCRIMINATOR** | inline rule + R30 | substring lock | none | RULE-TEXT-ONLY + PEER-CROSS-CHECK | 0 violations today | stable hold |

### Tier-1 always-resident — Phase L additions (NEW)

| rule-name | locus | tests | hooks | enforcement-status | recurrence-count | graduation-target |
| --------- | ----- | ----- | ----- | ------------------ | ---------------- | ----------------- |
| **R-NN STAT-CLAIM-CITE** (Phase L L-1 NEW; numbering deferred to commit-2 disc.go authoring per Q1 lean (i)) | const disc.go (TBD const-name + R-NN number) — to ship in commit-2 of L-1+L-2 | `TestStatClaimCiteSubstringLock` (TBD) — assert disc.go has 4 recognition tokens + agent prompts embed | none yet (PEER-CROSS-CHECK by design pre-L-5) | RULE-TEXT-ONLY + PEER-CROSS-CHECK | 7 stat-claim drifts today (#10/#13/#16/#17/#19/#20/#23 — recursive proof-of-need during L-0+L-2 authoring) | **codify Phase L L-1**; stretch-graduate L-5 toolgate gate-CHECK on numerical claims (open question — regex-detection FP-prone per Rain msg 7202 F1) |
| **R-NN SCOPE-FORK-CONFIRMATION** (Phase L L-1 NEW; numbering deferred to commit-2) | const disc.go (TBD const-name + R-NN number) | `TestScopeForkConfirmationSubstringLock` (TBD) | none — stays RULE-TEXT-ONLY by design | RULE-TEXT-ONLY | 3 scope-fork drifts today (#12 + push-fork + #18 git-vs-state) | codify Phase L L-1; remains RULE-TEXT-ONLY (proactive-surface discipline; toolgate detection FP-prone) |

### Tier-1 always-resident — Phase L generalize-trio additions (from L-0 catalog)

| rule-name | locus | tests | hooks | enforcement-status | recurrence-count | graduation-target |
| --------- | ----- | ----- | ----- | ------------------ | ---------------- | ----------------- |
| **R-NN ORIGIN-APPEND-ONLY** (L-0 Class 4 Rule 2 generalize) | const disc.go (TBD) — sub-rule of R29 OR new const | TBD substring lock | partial via R29 force-push gate | RULE-TEXT-ONLY; partial TOOLGATE-ENFORCED via R29 | 0 violations today | codify Phase L L-1 (sub-rule of R29) OR L-5 pre-push-checklist content |
| **R-NN STAGING-TEST-DEPTH-D2-D3-DISCRIMINATOR** (L-0 Class 8 generalize) | docs/conventions or const — TBD per L-1 spec lean | TBD | none | RULE-TEXT-ONLY | held cleanly today (D3-analogue cycle) | codify Phase L L-1 (generalize trio-class) OR docs/conventions reference |
| **R-NN READ-ONLY-ROLE-GIT-INSPECTION** (L-0 Class 9 generalize) | sub-rule of DISC v2 class-split | TBD | extend K-16 toolgate PreToolUse on `git checkout` with role-discriminator | RULE-TEXT-ONLY → TOOLGATE-ENFORCED post-L-5 | 0 violations today (1 historical 2026-04-26 Rain) | partial-graduate via L-5 K-16 extension; OR fold into DISC v2 sub-rule |
| **R-NN POST-SKILL-HUB-SEND-CONFIRM** (L-0 Class 10 generalize) | sub-rule of R6 OUTBOUND | TBD | extend L-5 toolgate PostToolUse on `Skill` tool — verify hub_send fired post-skill | RULE-TEXT-ONLY → TOOLGATE-ENFORCED post-L-5 | held cleanly today | partial-graduate via L-5 PostToolUse extension; OR fold into R6 sub-rule |

### Tier-2 skill-hosted

| rule-name | locus | tests | hooks | enforcement-status | recurrence-count | graduation-target |
| --------- | ----- | ----- | ----- | ------------------ | ---------------- | ----------------- |
| **phase-rules-detail skill** (Phase J B2{a-c}) | `~/.claude/skills/phase-rules-detail/SKILL.md` (disable-model-invocation:true) | none direct | none | RULE-TEXT-ONLY (on invoke) | not invoked today | stable hold; L-3a may add more tier-2 candidates |

### Tier-3 pre-action gate-files

| rule-name | locus | tests | hooks | enforcement-status | recurrence-count | graduation-target |
| --------- | ----- | ----- | ----- | ------------------ | ---------------- | ----------------- |
| **R-NN PRE-EXECUTE-GATE-FILE-READ** (L-5 NEW) | const disc.go (TBD) + gate-FILE content at `~/.bot-hq/gates/pre-{commit,push,merge}-checklist.md` | TBD test-locks | L-5 toolgate PreToolUse gate-CHECK on git commit/push/merge — verify checklist SHA in commit-footer or AgentState | TOOLGATE-ENFORCED (post-L-5) | held cleanly pre-L-5 | codify Phase L L-5 (gate-FILE + gate-CHECK) |
| **R-NN PRE-PHASE-CLOSE-RETRO** (L-6 NEW) | const disc.go (TBD) + gate-FILE at `~/.bot-hq/gates/pre-phase-close-checklist.md` | TBD test-locks | L-6 may not have toolgate (cycle-rhythm artifact, not per-action) | RULE-TEXT-ONLY + cycle-rhythm | not yet exercised (Phase L close fires it) | codify Phase L L-6 |

### Tier-4 reference

| rule-name | locus | tests | hooks | enforcement-status | recurrence-count | graduation-target |
| --------- | ----- | ----- | ----- | ------------------ | ---------------- | ----------------- |
| **`~/.bot-hq/{brian,rain}/discipline-anchors.md`** | per-agent files | none | none — agent-discipline only | RULE-TEXT-ONLY | session-start mandatory-read held cleanly | stable hold |
| **`~/.bot-hq/ratchets/active.md`** | shared file | none | none | RULE-TEXT-ONLY (R17 source-of-truth-rank-3) | session-start read held cleanly | stable hold; rotation at phase-close |
| **`~/.bot-hq/phase/<active>.md`** | shared file | none | none | RULE-TEXT-ONLY (current scope-lock) | held cleanly today (Phase L state-side, archived to docs/arcs at close) | stable hold |
| **`~/CLAUDE.md` Compact Instructions** | user-global file | none | none | DETECTION-FIRES-AT-COMPACT (auto-loaded) | held cleanly today (post-restart bootstrap) | stable hold |
| **`~/.bot-hq/discipline-log.md`** (L-4 NEW cross-agent) | shared file | none | none | RULE-TEXT-ONLY (cross-agent ledger) | not yet authored | codify Phase L L-4 |
| **`docs/conventions/*.md`** (existing 7 + L-1+L-2 NEW = 9) | shared files | TBD: TestConventionsCoverageAllRules (staleness detector — proposed L-2 ratchet-test) | none | RULE-TEXT-ONLY | held cleanly today | stable hold; tier-spec NEW (L-1) + rule-locus-inventory NEW (L-2) |
| **`docs/arcs/phase-N.md`** | shared files | none | none | RULE-TEXT-ONLY (closed-phase reference) | held cleanly; phase-k.md missing per phase-l.md L-6 housekeeping | L-6 housekeeping |
| **`docs/plans/*.md`** | shared (bot-hq own) / LOCAL (client-repo) | none | none | RULE-TEXT-ONLY | held cleanly today (L-0 fcae26e) | stable hold |
| **Hub message backlog (`~/.bot-hq/hub.db`)** | sqlite db | none direct | `hub_read` MCP tool | RULE-TEXT-ONLY (R17 source-of-truth-rank-4) | held cleanly today (used for cite-anchor verification) | stable hold |

---

## Per-project conventions

### Per-project bcc-ad-manager (Tier-1 + Tier-3 conditional-load)

NEW per Phase L L-1 mechanism: `~/.bot-hq/projects/bcc-ad-manager-conventions.md` to be authored consolidating per-project rules from feedback memories. Conditional-load on `active_workstream` flag.

| rule-name (proposed) | source memory | tier | tests | hooks | enforcement-status | graduation-target |
| -------------------- | ------------- | ---- | ----- | ----- | ------------------ | ----------------- |
| **bcc-disguise-scaffold-scan** | `feedback_disguise_scaffold_scan.md` | Tier-1 per-project | TBD substring lock | L-5 toolgate PreToolUse on `git add` for client-repo files — verify scan-SHA in commit-footer | RULE-TEXT-ONLY → TOOLGATE-ENFORCED post-L-5 | codify Phase L L-1 + L-5 |
| **bcc-bot-hq-invisibility (Rule 1)** | `feedback_bcc_disguise_and_origin_rules.md` Rule 1 | Tier-1 per-project | TBD | partial via scaffold-scan toolgate | RULE-TEXT-ONLY + PEER-CROSS-CHECK | codify Phase L L-1 |
| **bcc-origin-append-only (Rule 2)** | `feedback_bcc_disguise_and_origin_rules.md` Rule 2 | Tier-1 trio-self (generalize) | TBD | R29 force-push gate covers force-rewrite | RULE-TEXT-ONLY + TOOLGATE-ENFORCED via R29 | codify Phase L L-1 (generalize trio-class) |
| **bcc-docs-plans-LOCAL (Rule 3)** | `feedback_bcc_disguise_and_origin_rules.md` Rule 3 | Tier-1 per-project | TBD | none | RULE-TEXT-ONLY | codify Phase L L-1 |
| **bcc-main-json-paste-back** | `feedback_bcc_main_json_paste_back.md` | Tier-1 per-project | none | none | RULE-TEXT-ONLY | codify Phase L L-1 |
| **bcc-tableplus-sql-patterns** | `feedback_tableplus_sql_paste_robust_patterns.md` | Tier-1 per-project | none | none | RULE-TEXT-ONLY | codify Phase L L-1 |
| **bcc-tom-context-preservation** | `bcc-ad-manager.md` | Tier-4 per-project | none | none — workstream-entry mandatory-read mechanism | RULE-TEXT-ONLY | codify Phase L L-1 (workstream-entry mandatory-read) |
| **bcc-prod-db-ungated-risk** | `feedback_staging_test_depth.md` D2/D3 + `bcc-ad-manager.md` | Tier-1+Tier-3 per-project | TBD | L-5 toolgate PreToolUse on prod-DB-ops | RULE-TEXT-ONLY → TOOLGATE-ENFORCED post-L-5 | codify Phase L L-1 + L-5 |
| **bcc-issue-staging-discipline** | `bcc-ad-manager.md` (cycle outcomes 8-issue Tom-bundle) | Tier-1 per-project | none — covered by R28 PER-INSTANCE-FIRE-GREENFLAG | RULE-TEXT-ONLY (R28 covers fire-class) | already covered by R28; no new ratchet needed | stable hold |
| **bcc-staging-test-depth-D2-D3** | `feedback_staging_test_depth.md` | Tier-1 trio-self (generalize) | TBD | none | RULE-TEXT-ONLY | codify Phase L L-1 (generalize trio-class) |

---

## Aggregate counts

| Tier | Trio-self count | Per-project count | Total |
| ---- | --------------- | ----------------- | ----- |
| Tier-1 always-resident (existing) | **28** (math: R6 + R12-R30 = 20 R-rules + 8 more (R27-pane-only-sub-class + DISC v2 class-split + HALT-ALL-WORK + RESUME-FROM-HALT + H-13 force-push + HANDSHAKE-TERMINATOR + SCOPE-LOCK-BEFORE-IMPL + AUDIENCE-CLASS-DISCRIMINATOR) = 28) — corrected per Rain BRAIN-2nd A1 catch (#23 stat-claim drift recursive) | 0 (none yet) | **28 existing** |
| Tier-1 always-resident (Phase L NEW) | 6 (R31/R32/ORIGIN-APPEND-ONLY/STAGING-TEST-DEPTH/READ-ONLY-ROLE/POST-SKILL-HUB-SEND) — net +6 (or +4 after consolidation Class 9→DISC sub-rule + Class 10→R6 sub-rule) | 8 (bcc-disguise-scaffold-scan + bcc-bot-hq-invisibility + bcc-docs-plans-LOCAL + bcc-main-json-paste-back + bcc-tableplus-sql-patterns + bcc-tom-context + bcc-prod-db-ungated + bcc-issue-staging) | **14 NEW (or 12 after consolidation)** |
| Tier-2 skill-hosted | 1 (phase-rules-detail) | 0 | 1 |
| Tier-3 pre-action gate-files (Phase L NEW) | 4 (pre-commit / pre-push / pre-merge / pre-phase-close) | candidate (bcc-prod-db-ops gate) | **4 trio + 1 per-project candidate** |
| Tier-4 reference | 9 (anchors-files-2 + ratchets + phase + CLAUDE.md + discipline-log NEW + docs/conventions + docs/arcs + docs/plans + hub.db) | 1+ (bcc-ad-manager.md + 8 feedback memories) | **9 trio + 9+ per-project** |

**Net new R-rules disc.go const additions for Phase L:** 6 (R31/R32/ORIGIN-APPEND-ONLY/STAGING-TEST-DEPTH/READ-ONLY-ROLE/POST-SKILL-HUB-SEND) — likely consolidated to 4 net new (R31/R32/ORIGIN-APPEND-ONLY/STAGING-TEST-DEPTH) with Class 9→DISC v2 sub-rule + Class 10→R6 sub-rule per L-0 lean.

**Net new gate-files for Phase L:** 4 trio-self (L-5 + L-6) + 1 per-project candidate (bcc-prod-db).

**Net new per-project files for Phase L:** 1 (`~/.bot-hq/projects/bcc-ad-manager-conventions.md` — consolidates ~8 bcc-specific rules from feedback memories).

---

## Staleness-detection ratchet-test (L-2 deliverable)

`TestRuleLocusInventoryCoversAllR-rules` (proposed; ships in L-1+L-2 commit-2 with disc.go const additions):

```go
func TestRuleLocusInventoryCoversAllR-rules(t *testing.T) {
    // 1. Grep disc.go for `const R\d+...Rule|Protocol|Discriminator|Gate|Footer|Recognition|Authorization`
    // 2. Read docs/conventions/rule-locus-inventory.md, parse table rows
    // 3. Assert every disc.go R-rule const has a row in inventory
    // 4. Optional: assert every inventory row's `tests` column references real test functions
    // 5. Optional: assert every inventory row's `hooks` column references real toolgate hooks
}
```

Catches "schema works for existing rules but breaks on additions" class per Phase J B3d-style discipline. Detects when a new R-rule is added to disc.go without inventory update.

---

## Tight-feedback-loop validation (L-1 framework reconcile)

This inventory exercises the L-1 framework. Findings:

1. **Tier × scope cells all populated:** every existing rule fits cleanly into a (tier, scope) cell. No "rule has nowhere to go" gap.
2. **Per-project tier validated:** 8 bcc-specific rules consolidate cleanly in Tier-1 per-project bcc-ad-manager block; without project-axis they would bloat trio-self-discipline (wrong scope).
3. **Tier-2 sparse but valid:** only 1 existing skill-hosted rule (phase-rules-detail). L-3a audit may surface candidates. Sparse ≠ broken — Tier-2 is intentionally on-demand.
4. **Maturity-ratchet integration confirmed:** every row has a graduation-target column; L-4 sweep can batch-process by tier+scope+enforcement-status.
5. **Recognition-substring-load-bearing rules identified:** R27/R30/HALT/[HR]/H-31-H-33/SCOPE-LOCK pattern recognition cannot relocate to Tier-2/Tier-3 without breaking detection. L-3a audit must mark these ALWAYS-INLINE.

**No L-1 framework reshape needed** based on inventory exercise. v1 holds.

---

## Open questions / unresolved

- **Q1: R-NN numbering vs sub-class fold (RESOLVED v1.1 lean (i)).** Phase K shipped through R30. Phase L adds 2 R-rules. Lean (i) per Rain BRAIN-2nd A2: defer R-NN numbering commitment to commit-2 disc.go authoring; use R-NN placeholder consistently in v1.1 docs. Pane-only confirmed as R27 sub-class (no separate const) per F2 rename — see line 56 "R27-pane-only-sub-class". Final R-numbers (likely R31/R32) commit in commit-2.
- **Q2: Class 9 + Class 10 consolidation strategy.** Inventory lists both as separate R-rules with potential fold-into existing rule sub-class. L-1 commit-2 decides: ship as standalone consts OR fold as sub-rule prose. Lean: fold to minimize disc.go const-block growth.
- **Q3: bcc-tom-context tier classification.** Listed as Tier-4 per-project (workstream-entry mandatory-read) — but workstream-entry mechanism doesn't exist yet (L-1 proposal). Does mandatory-read make it Tier-3 (gate-FILE-class)? Lean: stays Tier-4 because it's reference-class content, not pre-action-gate content. Mandatory-read mechanism is a Tier-4-with-load-trigger sub-pattern.

---

## Posture at v1

L-2 v1 doc-only authored. Tight-feedback-loop with L-1 v1 (rulebook-tier-spec.md) — both v1 surfaces produced. Reconcile-pass before lock + Rain BRAIN-2nd. disc.go const additions + ratchet-tests deferred to commit-2 (separate Rain BRAIN-2nd) per blast-radius discipline.

**No framework gaps surfaced** during inventory exercise. v1 framework holds. Ready for Rain BRAIN-2nd on framework-correctness + classification-precision + tier × scope cell mapping.
