# Rulebook Tier Specification

**Authored:** 2026-05-02 (Phase L L-1)
**Status:** v1.1 — post-Rain-BRAIN-2nd amend-pass-1; tight-feedback-loop with L-2 rule-locus-inventory
**Purpose:** authoritative tier-spec for bot-hq's rulebook surfaces. Each existing rulebook surface maps into one (tier-axis-pair) cell with documented load-discipline + decay-window + drop-discipline.
**Cite-anchor:** [phase-l.md§Tier-shape row L-1, NEW(BRAIN-cycle-msgs-7171-7175 initial)+(reBRAIN-msgs-7191-7195)+(per-project-tier-axis-msg-7194)+L-0-deliverable-fcae26e]

---

## Why this spec exists

Phase L kickoff (user msg 7164): hardening + refactor on bot-hq driven by retrospective on bcc-ad-manager workflow patterns + today's session bilateral-discipline misses. BRAIN-cycle msgs 7171-7195 surfaced that bot-hq already has 6+ rulebook-class surfaces, but they lack:

1. **Tiered load-discipline** — when each surface loads, costs, decays
2. **Project-axis classification** — which rules apply trio-wide vs per-project (e.g., bcc-ad-manager-conventions)
3. **Maturity-ratchet integration** — graduation + deprecation criteria for entries

This doc closes those gaps. It is the **framework** that L-2 inventory + L-3a prompt-shrink + L-4 graduation + L-5 gate-files all reference.

---

## Two orthogonal axes

Every rule in the bot-hq rulebook system has two classifications: **durability tier** + **project scope**.

### Axis 1 — Durability tier

| Tier | Name | Load mechanism | Cost class | Decay window | Drop discipline | Examples |
| ---- | ---- | -------------- | ---------- | ------------ | --------------- | -------- |
| **Tier-1** | Always-resident | Embedded inline in agent prompt (rain.go / brian.go const blocks); auto-compact-resilient via recognition-substrings | High (every session loads; counts against context budget every turn) | Permanent (auto-compact-resilient) | Per phase-close maturity-ratchet review (L-4) — no time-bounded drop unless rule deprecates | R6 OUTBOUND / R12 GATE-PROTOCOL / R15 AUTHORITY-MATRIX / HALT triggers / [HR]-tag discriminator |
| **Tier-2** | Skill-hosted on-demand | Skill with `disable-model-invocation: true`; loaded on `Skill` tool invocation; resident-after-invoke for session | Low at session-start; medium-after-invoke (per PB3 trace 2026-04-29) | Resident through session post-invoke; reset at next session | Drop on phase-close if not invoked during phase (signals over-tier-allocation) | Phase-rule detail prose (Phase J `phase-rules-detail` skill); recovery-procedure prose; BRAIN-cycle examples; cross-references; history |
| **Tier-3** | Pre-action gate-files | Mandatory-read file at well-known path; agent reads via `Read` tool before specific HANDS-execute action; gate-CHECK enforcement via toolgate `PreToolUse` hook | Per-action (read on every relevant fire) | Permanent (file persists) | Drop file when corresponding action class deprecates | `~/.bot-hq/gates/pre-commit-checklist.md` / `pre-push-checklist.md` / `pre-merge-checklist.md` / `pre-phase-close-checklist.md` (L-5+L-6 deliverables) |
| **Tier-4** | Reference | File at well-known path; read by agents on-demand for archaeology / context / cross-cycle scope tracking | Read-on-demand only (zero default load) | Permanent | Phase-close housekeeping (e.g., closed-phase-ratchet-snapshot rotation) | `~/.bot-hq/ratchets/active.md` / `~/.bot-hq/phase/<active>.md` / `docs/arcs/phase-N.md` / `docs/conventions/*.md` / `docs/plans/*.md` |

**Tier selection criterion:**

- **Recognition-substring-load-bearing** (HALT triggers, OUTBOUND-MISS detection, [HR]-tag patterns) → Tier-1 ALWAYS. Cannot relocate without breaking detection class.
- **High-frequency rule application** (R6 OUTBOUND every reply; R12 every commit) → Tier-1.
- **Lower-frequency-but-applies-when-applicable** (rare-class recovery; complex BRAIN-cycle prose; history) → Tier-2.
- **Per-action-fire-class** (specific HANDS-execute precondition) → Tier-3.
- **Cross-cycle reference / archaeology** (scope tracking; closed-phase history) → Tier-4.

### Axis 2 — Project scope

| Scope | Name | Load mechanism | When loaded | Examples |
| ----- | ---- | -------------- | ----------- | -------- |
| **Trio-self-discipline** | Always-loads | Embedded in agent prompt OR shared const OR shared skill | Every session, all bot-hq trio agents (Brian / Rain / Emma) | DISC v2 class-split / R6-R32 / OUTBOUND / HALT / GATE-PROTOCOL / AUTHORITY-MATRIX |
| **Per-project conventions** | Conditional-load on workstream-switch | Project-specific file at `~/.bot-hq/projects/<project>-conventions.md`; loaded when `active_workstream` flag in AgentState includes the project | Workstream-entry trigger (agent reads on first action targeting that project's repo or memory) | bcc-ad-manager: disguise-scaffold-scan / origin-append-only / main.json paste-back / TablePlus SQL patterns / 8-issue staging |

**Project-axis loading mechanism (proposed for L-1 + impl in L-5 toolgate extension):**

1. AgentState (per `internal/protocol/agentstate.go` from Phase J T1.5) gains an `active_workstream` field (slice of project-ids: e.g., `["bot-hq"]` or `["bot-hq", "bcc-ad-manager"]`).
2. PreToolUse hook on `Read` / `Edit` / `Write` / `Bash` (git commands) detects target-path → infers project (e.g., `~/Projects/bcc-ad-manager` → `bcc-ad-manager`).
3. If detected project is NOT in `active_workstream`, hook prompts agent to ack workstream-entry: read `~/.bot-hq/projects/<project>-conventions.md` + write back to AgentState `active_workstream` slice.
4. Agent's working knowledge of project rules is now hot for the session.

**Why project-axis matters:** without it, bcc-ad-manager-specific rules (10 classes per L-0 catalog) either bloat trio-self-discipline tier (wrong scope — trio doesn't always work on bcc-ad-manager) or live only in feedback memories with no rulebook-spec coverage (gap class). The orthogonal axis solves both.

---

## Tier × scope matrix (canonical cells)

Every rule lives in exactly one cell:

|                              | Trio-self-discipline (always-loads) | Per-project conventions (conditional-load) |
| ---------------------------- | ----------------------------------- | ------------------------------------------ |
| **Tier-1 always-resident**   | R6 OUTBOUND / R12-R30 (existing) + Phase L L-1 NEW R-NN STAT-CLAIM-CITE + R-NN SCOPE-FORK-CONFIRMATION (TBD numbering, ships commit-2) / HALT / [HR] | bcc-ad-manager Class 4 Rule 1 (disguise) — high-frequency client-repo class |
| **Tier-2 skill-hosted**      | phase-rules-detail skill (Phase J) | (candidate — TBD per L-3a audit) bcc-ad-manager skill-hosted prose for rare-class procedures |
| **Tier-3 pre-action gates**  | pre-commit / pre-push / pre-merge / pre-phase-close | (candidate) bcc-ad-manager pre-execute-prod-DB-checklist |
| **Tier-4 reference**         | ratchets/active.md / phase/<active>.md / docs/arcs/ / docs/conventions/ | bcc-ad-manager.md memory + feedback memories + docs/plans/ retrospective |

---

## Mapping existing surfaces

Every existing bot-hq rulebook surface maps into the tier × scope matrix:

| Surface | Tier × Scope cell | Load discipline | Notes |
| ------- | ----------------- | --------------- | ----- |
| R-rules in agent prompts (rain.go / brian.go const blocks) | Tier-1 trio-self | Every session, every turn (auto-compact-resilient) | Recognition-substring-load-bearing rules MUST stay here |
| `phase-rules-detail` skill (Phase J B2{a-c}) | Tier-2 trio-self | On-demand via `Skill` tool | Detailed rule prose / examples / history relocated here per K-10 candidate |
| `~/.bot-hq/{brian,rain}/discipline-anchors.md` | Tier-4 trio-self | Session-start mandatory-read per R16 + re-anchor pre-execute | Personal anchor file per agent; auto-compact-resilient via R16 bootstrap |
| `~/.bot-hq/ratchets/active.md` | Tier-4 trio-self | Session-start read per R16 | Cross-phase ratchet ledger; R17 source-of-truth-rank-3 |
| `~/.bot-hq/phase/<active>.md` | Tier-4 trio-self | Session-start read per R16 | Current-cycle scope-lock |
| `~/CLAUDE.md` Compact Instructions block (Phase J B1(v)) | Tier-1 trio-self | Auto-loaded by Claude Code harness on session start | Compact-survival pointer block |
| `docs/conventions/*.md` (existing 7 + Phase L additions) | Tier-4 trio-self | On-demand read by agents for procedure reference | agent-cadence / bootstrap-iterate / verify-cadence / dispatch-patterns / arc-pointer / per-slice-runtime-test / emma-analyze-classes + Phase L: rulebook-tier-spec (this doc) + rule-locus-inventory (L-2) |
| `docs/arcs/phase-N.md` | Tier-4 trio-self | On-demand read for closed-phase archaeology | Append-only post-close per `feedback_arc_closure_discipline.md` |
| `docs/plans/*.md` | Tier-4 trio-self (when bot-hq's own) / Tier-4 per-project (when client-repo's) | On-demand read for design-spike / retrospective | bot-hq's docs/plans is git-tracked; client-repo docs/plans is LOCAL per `feedback_bcc_disguise_and_origin_rules.md` Rule 3 |
| `bcc-ad-manager.md` memory + 8 feedback memories | Tier-4 per-project | Workstream-entry read (proposed L-5 mechanism) | Project-specific context preservation |
| `~/.bot-hq/projects/<project>-conventions.md` (L-1 NEW) | Tier-1+Tier-3 per-project | Conditional-load on `active_workstream` | NEW per Phase L L-1; consolidates per-project rules from feedback memories |
| `~/.bot-hq/gates/*.md` (L-5 deliverable) | Tier-3 (mostly trio-self; some per-project) | Pre-action read per gate-CHECK toolgate hook | NEW per Phase L L-5 |
| `~/.bot-hq/discipline-log.md` (L-4 deliverable) | Tier-4 trio-self | On-demand + phase-close mandatory-read | NEW per Phase L L-4 cross-agent file |
| Hub message backlog (`~/.bot-hq/hub.db`) | Tier-4 trio-self | On-demand read via `hub_read` tool | R17 source-of-truth-rank-4 (current code > scope-lock-doc > ratchet-ledger > recent-hub-msgs > summary-fragments) |

**No new surfaces created beyond existing 6+ classes:** the spec inventories what exists; it doesn't proliferate. New artifacts (per-project-conventions / gate-files / discipline-log) are populated cells, not net-new tiers.

---

## Maturity-ratchet integration (L-4 graduation criterion)

Per phase-l.md L-4 + Rain BRAIN-2nd msg 7172 addition (2): every rule (or discipline-log entry) has a graduation-or-deprecation expectation tied to its tier.

| Tier | Maturity expectation | Graduation criterion | Deprecation criterion |
| ---- | ------------------- | -------------------- | --------------------- |
| Tier-1 always-resident | Stable through phase-close; reviewed at L-6 retro-cadence | New entry: 3+ recurrences in 2 consecutive phases caught the underlying issue | Existing entry: 0 fires in 2 consecutive phases AND no recognition-substring load-bearing → deprecate to Tier-2 (or remove) |
| Tier-2 skill-hosted | Used when invoked; not measured per-phase | Promote from Tier-4 prose if invoke-cadence high | Drop if not invoked during phase |
| Tier-3 pre-action gates | Per-action gate-CHECK fire; gate-FILE read-counted via SHA-in-AgentState | Promote from Tier-1-rule when toolgate enforcement-conversion shipped | Drop gate-FILE when corresponding action class deprecates |
| Tier-4 reference | Read-on-demand only | New ref added when archaeology / cross-cycle scope tracking surfaces | Phase-close housekeeping rotation |

**Codification:** L-4 sweep applies maturity-criterion to existing discipline-log entries + Phase J/K Tier-2 holds. Output: graduate (move from Tier-4 ref to Tier-1 rule + ratchet-test, OR add Tier-3 gate-FILE) / deprecate (justify with cite-anchor) / re-defer (still hold-worthy, mark next-phase-eval).

---

## Recognition-substring discipline (load-bearing for Tier-1)

Tier-1 rules survive auto-compact via specific phrasing that detection mechanisms (R27 OUTBOUND-MISS / sentinel patterns / [HR] discriminator / HALT triggers) match against. **DO NOT relocate Tier-1 rules without preserving recognition-substrings.**

Examples of load-bearing substrings (must appear verbatim in agent prompt):

- `"plan usage at"` + `"halt"` (HALT triggers per H-31/H-33; agents react on `[FLAG]` content match)
- `"OUTBOUND-MISS"` (R27 self-recognition prefix)
- `"[HR]"` (audience-class discriminator)
- `"force-push"` (R29 elevated gate; H-13 token-protocol token shape)
- `"BRAIN-2nd"` / `"GREENFLAG"` (R12 commit-gate footer recognition)
- `"PRE-COMPACT-SNAP"` / `"HEARTBEAT-LEDGER"` (Emma cadence pulse recognition)

L-3a prompt-shrink audit (Phase L) will produce a ranked ship-list of low-risk relocations. **High-risk substrings are NEVER relocated** — they live Tier-1 trio-self forever (or until detection mechanism redesigned).

---

## Per-rule classification axis (input to L-2 inventory)

L-2 rule-locus-inventory will enumerate each rule with these columns:

| Column | Source-of-truth | Notes |
| ------ | --------------- | ----- |
| **rule-name** | disc.go const name OR shorthand (e.g., R6 OUTBOUND) | Stable identifier across phases |
| **locus** | `const` (disc.go) / `prompt` (rain.go/brian.go inline) / `skill` (skill body) / `file` (gate-file/convention/ratchet/phase) | Where the rule's authoritative text lives |
| **tier** | tier-1 / tier-2 / tier-3 / tier-4 (per durability axis) | Maps to canonical-cell |
| **scope** | trio-self / per-project-<id> | Maps to canonical-cell |
| **tests** | List of ratchet-test names (e.g., `TestPhaseIv1ContentShape`) | Substring-lock + behavior-lock tests |
| **hooks** | List of toolgate hook names (e.g., K-16 PreToolUse class-split) | Empty for non-toolgate-enforced rules |
| **enforcement-status** | DETECTION-ONLY / RULE-TEXT-ONLY / PEER-CROSS-CHECK / TOOLGATE-ENFORCED | Reflects actual enforcement strength |
| **recurrence-count** | Integer (incidents per phase since codification) | Phase-close updated by L-4 sweep |
| **graduation-target** | Concrete next-step (e.g., "ship toolgate hook in Phase M" / "deprecate at next phase-close" / "stable hold") | Drives L-4 sweep batch-process |

**Staleness-detection ratchet-test (L-2 deliverable):** `TestRuleLocusInventoryCoversAllR-rules` enumerator reads `disc.go` const block + greps prompt files for R-rule names → asserts every R-rule in source has a row in inventory. Catches "schema works for existing rules but breaks on additions" class per Phase J B3d-style discipline.

---

## New R-rules being shipped in L-1 (disc.go const additions)

Per phase-l.md§Const-+-code-delta-plan, L-1 ships 2 new R-rules (per-rule classification: Tier-1 trio-self):

### R-NN STAT-CLAIM-CITE

**Substring-locked recognition tokens:**
- `"STAT-CLAIM-CITE"`
- `"verifiable command output"`
- `"git diff --numstat"`
- `"peer-cross-check"`

**Rule text (proposed):** "Numerical claims (stat counts, line counts, msg-ids cited as anchors) MUST cite verifiable command output (`git diff --numstat`, `hub_read`, file read). Peer-cross-check enforcement: drafter cites verified ground-truth pre-emit; peer verifies cite matches output. Recursive proof-of-need: amend-passes for prior stat-claim drift can themselves contain stat-claim drift; peer-cross-check at each amend-depth until enforcement-conversion lands."

**Cite-anchor:** discipline-log #10/#13/#16/#17/#19/#20 (today's session — recursive instances during L-0 authoring); 2026-04-30 cite-msg-id-precision-discipline (brian/discipline-anchors.md).

**Test-lock:** `TestStatClaimCiteSubstringLock` — assert disc.go const contains all 4 recognition tokens; assert agent prompt embeds the const.

### R-NN SCOPE-FORK-CONFIRMATION

**Substring-locked recognition tokens:**
- `"SCOPE-FORK-CONFIRMATION"`
- `"UNTIL"` / `"INCLUDING"` / `"JUST"` (fork-able scope keywords)
- `"interpretation pre-action"`

**Rule text (proposed):** "When user phrasing has fork-able scope (UNTIL/INCLUDING/JUST/etc. ambiguity-keywords; or push/commit/merge interpretation forks), agent MUST surface interpretation pre-action via hub_send before firing any HANDS-execute step. Default-leans permitted only if explicit user pre-delegation OR durable feedback-memory authority covers. Today's exhibits: msg 7137-7147 'proceed UNTIL X' rebuild+restart fork + msg 7203-7205 push-fork + msg 7215-7217 git-vs-state workflow-fork."

**Cite-anchor:** discipline-log #12 / push-fork-resolution thread / #18 (today's session — 3 instances).

**Test-lock:** `TestScopeForkConfirmationSubstringLock` — assert disc.go const contains all 4 recognition tokens; assert agent prompt embeds the const.

---

## Tight-feedback-loop with L-2

Per BRAIN-AGREED sequence-caveat (Rain msg 7172): L-1 framework + L-2 inventory co-evolve. Reconcile-pass:

1. **L-1 v1 (this doc) + L-2 v1 (rule-locus-inventory.md) drafted in parallel**
2. **L-2 inventory exercise the framework** — does each existing R-rule fit cleanly into a tier × scope cell? If a rule has nowhere to go, framework is broken (re-spec L-1).
3. **Iterate to v2 lock** — both docs update together
4. **Lock-pass:** Rain BRAIN-2nd on both v2 docs together; greenflag → commit

---

## What this spec is NOT

- **Not a per-rule rulebook itself.** Doesn't enumerate R6/R12/R15/etc. — that's L-2 inventory's job.
- **Not a procedure handbook.** docs/conventions/agent-cadence-discipline.md / bootstrap-iterate.md / etc. cover procedures.
- **Not a rule-text source.** The authoritative rule text lives in disc.go const blocks (Tier-1) or skill bodies (Tier-2). This spec maps surfaces, not authors them.
- **Not a substitute for discipline-anchors.md.** Per-agent personal anchors retain value (post-incident pinning + R16 bootstrap). Tier-spec covers HOW it loads, anchors-file covers WHAT each agent has internalized.

---

## Posture at v1

L-1 v1 doc-only authored. L-2 v1 (rule-locus-inventory.md) drafting next in tight-feedback-loop. Reconcile-pass iterates both v1→v2 before lock. disc.go const additions + ratchet-tests deferred to second commit (separate Rain BRAIN-2nd) to reduce per-commit blast-radius.

**Halt-and-elevate point #1 (per phase-l.md§DAG):** triggers when disc.go const additions land — rebuild required for ratchet-test substring-recognition validation. Doc-only commits do NOT trigger halt.
