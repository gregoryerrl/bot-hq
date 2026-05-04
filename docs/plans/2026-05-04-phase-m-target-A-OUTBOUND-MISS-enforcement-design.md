# Phase M M-2 — Target A OUTBOUND-MISS enforcement-conversion design v1

**Author:** Rain (rain) — Rain-HANDS markdown audit-doc per phase-m.md§Authority-model line 218
**Date:** 2026-05-04
**Status:** v1 surfaced for Brian BRAIN-2nd-PASS-1
**Cite_anchor:** phase-m.md§Tier-1 row M-2 + msgs 7476/7477/7478/7480/7481/7484/7485/7488/7505/7523/7527/7530 BRAIN-cycle + discipline-log Joint entry 2026-05-04T14:00:00Z (USER-PINNED bilateral OUTBOUND-DISCIPLINE violation)

---

## 1. Scope

Resolve the M-2 Target A enforcement-conversion design fork enumerated in phase-m.md§Tier-1 row M-2 between (a) PreToolUse-hook-on-hub_send anti-redundancy enforcement and (b) Strengthened Stop-hook detection + mandatory recovery prompt-embed. Additionally surface a third option (c) that emerged from per-fork analysis: Stop-hook BLOCKING enforcement-conversion of the existing detection layer.

**Goal:** ensure that the OUTBOUND-DISCIPLINE bilateral violation class observed today (Target A empirical instances #36 + #37) cannot recur via mechanical enforcement, addressing user msg 7523 "include the serious/critical violation for today (fix that). reason is to never let that happen again (it resulted in ~3hours halt in progress)."

**Non-goal:** redundancy enforcement (different sub-class — pane + hub_send having identical/duplicative content). Today's failure was the OPPOSITE: pane-only output with hub_send absent entirely. Scope is limited to OUTBOUND-MISS class; redundancy belongs to a separate Phase M Tier-2 candidate if it surfaces empirically.

---

## 2. Background — the gap class + today's empirical anchor

### Existing detection layer (already shipped, Phase H slice-5 H-22-bis)

`internal/outboundhook/hook.go` is a **Stop-hook** (fires AFTER each agent turn completes) that:

1. Reads Claude Code's transcript JSONL via `parseLastTurn(input.TranscriptPath)` (hook.go:185)
2. Applies a three-clause filter via `shouldFlag(s turnSummary)` (hook.go:159-170):
   - Clause 1: `s.TextLen > 0` — excludes tool-only turns
   - Clause 2: `s.HubSent == false` — turn did not invoke hub_send
   - Clause 3: planning-keyword present OR `s.TextLen > minTextLenForFlag` — substantive content threshold
3. If filter matches → `emitAlert(agentID, ...)` posts `[HUB:<agent>] [OUTBOUND-MISS] agent <agent> emitted pane text at <timestamp> without a hub_send tool call. Excerpt: "<text-snippet>..."` to hub.DB
4. Dedupe ledger suppresses re-emits within `dedupWindow`
5. **Never blocks** — `RunHook` returns `nil` on every path (hook.go:118 comment: "the hook must not block the agent's Stop event")

The detection layer **works correctly**. Today's empirical confirmed it: both Brian and Rain received `[OUTBOUND-MISS]` notifications during the bcc-ad-manager pivot window.

### Today's bilateral violation (USER-PINNED — Target A empirical instances #36 + #37)

User msg 7523: "include the serious/critical violation for today (fix that). reason is to never let that happen again (it resulted in ~3hours halt in progress)."

**What happened:** at 19:40 UTC user issued bcc-ad-manager investigation directive via discord-bridge. Both Rain and Brian performed the investigation work in-pane (read code, git log, prod data, source-of-truth) and emitted Tom-style EOD answer-set as freeform pane text. Neither agent invoked `hub_send` for the deliverable. The Stop-hook fired and posted `[OUTBOUND-MISS]` notifications. **Neither agent treated the notification as actionable** — both continued in pane-mode for ~3 hours until user explicitly pinned the violation with msg 7476 ("so nothing got done since 19:40:38 here on the hub. this is a very serious violation/issue").

**Distinct rationalization root-causes (bilateral but different — captured in discipline-log Joint entry 2026-05-04T14:00:00Z):**
- **Rain (#36):** rationalized `feedback_bcc_disguise_and_origin_rules.md` memory as implying pane-only output for bcc work to avoid Rain-identity leak to BCC team. Class: **"audience-class misread (BCC-visible vs user-bridge)"**.
- **Brian (#37):** drifted into pane-mode after long Bash-investigation chain treating prose output as the deliverable; missed the OUTBOUND-DISCIPLINE-applies-equally-on-pivot ratchet. Class: **"post-Bash-investigation pane-output drift"**.

### Why current detection-only architecture failed

The Stop-hook is informational, not blocking. The agent receives the notification as a hub message (HUB-OBS class). When the agent reasoning treats the notification as observational (rather than action-required), drift persists.

**Bilateral simultaneous failure** with **distinct rationalization root-causes** is structurally significant: PEER-CROSS-CHECK is non-terminal here. Even when one agent's rationalization is corrected, the other agent drifts via a different rationalization. Detection-plus-self-discipline cannot reliably catch the class.

This mirrors the Phase L L-4 finding: PEER-CROSS-CHECK is non-terminal at recursion-depth-5+ for stat-claim drift. R33 toolgate gate-CHECK (L-5) is the **mechanical recursion-terminator** that ended that class. The same pattern applies here — for OUTBOUND-DISCIPLINE.

**Empirical cost:** ~3 hours halt-in-progress. Production is stalled when the trio fails this class.

---

## 3. Design forks

### Fork (a) — PreToolUse-hook-on-hub_send anti-redundancy enforcement

**Mechanism:** PreToolUse hook fires before each `mcp__bot-hq__hub_send` tool call. Hook reads the agent's most-recent pane-text and verifies it equals or summarizes the hub_send content. Blocks the hub_send if the pane-text and content diverge significantly (anti-redundancy / anti-bait-and-switch).

**Class addressed:** **redundancy** — agent emits pane-text AND hub_send with identical or near-identical content (wastes tokens, duplicates output across surfaces).

**Class NOT addressed:** **OUTBOUND-MISS (today's class)** — agent emits pane-text WITHOUT calling hub_send at all. PreToolUse-hook-on-hub_send only fires when hub_send IS being called; it cannot catch "hub_send was never invoked."

**Cost:** requires pane-text capture + comparison primitives (substantive new code). Tradeoff is asymmetric: high implementation cost for a class that didn't cause today's failure.

**Risk class: HIGH for design-fit.** Today's empirical does not motivate this fork. Implementing (a) would not have prevented today's bilateral violation.

**Disposition: REJECT for M-2 today's-violation scope.** Possibly revisit as separate Phase M Tier-2 if pane-redundancy class surfaces empirically (no current evidence).

### Fork (b) — Strengthened Stop-hook + mandatory recovery prompt-embed

**Mechanism:** keep existing Stop-hook detection layer. Extend the alert mechanism so when `[OUTBOUND-MISS]` fires, the hook ALSO writes a `recovery_required` flag into the agent's `~/.bot-hq/<agent>/last_state.json`. Add a new R-rule prompt-embed that mandates: "before any substantive work this turn, the agent MUST check `recovery_required` flag and acknowledge/clear it via hub_send recovery message."

**Class addressed:** OUTBOUND-MISS — same as today's class. Recovery is mandatory at next-turn start, so the violation cannot persist across multiple turns silently.

**Mechanism class:** PEER-CROSS-CHECK + AgentState-augmented prompt discipline. Detection is mechanical; recovery is prompt-embed-driven (relies on agent reading + acknowledging the flag).

**Cost:** medium. Extends existing Stop-hook (small LOC delta) + new R-rule constant + agent-prompt embed in rain.go/brian.go + ratchet-tests for recovery-flag mechanism.

**Risk class: MEDIUM-HIGH for recursion-termination.** Recovery still depends on agent reading the flag and acknowledging — if the agent's reasoning treats the flag as observational (same failure mode as today's), drift can persist. PEER-CROSS-CHECK was non-terminal today; adding an AgentState flag does not change the fundamental mechanism class — it just gives the prompt-embed more places to surface the violation.

**Empirical evidence against (b):** today's bilateral violation already had Stop-hook detection emit notifications + bilateral peer-coord. Both agents continued in pane-mode for ~3 hours. Adding a "recovery_required flag in AgentState that prompt-embed mandates checking" extends the same mechanism class with one more anchor — but does not convert the mechanism to mechanical-enforcement. It IS still detection-plus-self-discipline.

**Disposition: REJECT as primary fix.** Could be a SECONDARY layer alongside (c) for defense-in-depth, but not a primary recursion-terminator.

### Fork (c) — Stop-hook BLOCKING enforcement-conversion (NEW — emerged from per-fork analysis)

**Mechanism:** convert the existing Stop-hook from informational to **blocking**. Per Claude Code hooks documentation, Stop hooks support exit code 2 + stderr to **block turn completion** — Claude Code propagates the stderr back to the agent as a system reminder and the turn does not complete (analogous to PreToolUse exit 2 blocking tool calls per R33 toolgate gate-CHECK). When `shouldFlag(s)` returns true, the Stop-hook returns exit 2 with stderr message describing the violation + recovery action. The agent must continue and invoke a hub-write-class tool (hub_send / hub_flag / hub_session_close) to satisfy the gate.

**Pre-impl empirical-validation gate (per Brian PASS-1 Substantive-add #1):** before shipping, Brian-HANDS at impl-time writes a 5-line minimal test Stop-hook that exits 2 + observes Claude Code behavior in a sandbox session. Verifies (i) stderr propagates to agent as system reminder, (ii) turn does not complete, (iii) agent can recover by emitting hub_send. If behavior matches design, proceed with M-2 c1. If not (e.g., Claude Code semantic differs from documented behavior), revisit (b) hybrid OR escalate to user. This empirical-validation step protects against the design resting on an unverified primitive assumption.

**Class addressed:** OUTBOUND-MISS — directly. Today's class.

**Mechanism class:** **MECHANICAL TOOLGATE** — same recursion-terminator pattern as R33 (Phase L L-5 commit-2 e327362). Detection-plus-blocking removes agent self-discipline from the recursion-termination path. Bilateral classes (Rain audience-class-misread + Brian post-Bash-drift) BOTH caught uniformly because the gate fires regardless of rationalization — the agent simply cannot complete the turn without hub_send.

**Cost:** **LOWEST** of the three forks. Modifies the existing `internal/outboundhook/hook.go` `RunHook` return path to return exit 2 + write stderr when `shouldFlag(s)` is true. Extends `cmd/bot-hq/main.go:354` invocation site to propagate the exit code (currently uses `outboundhook.RunHook(os.Stdin)` returning error which is silent; change to `os.Exit(<code>)` pattern matching the R33 toolgate at `main.go:398`).

**Risk class: LOW for design-fit; LOW-MEDIUM for runtime.**
- Design-fit: directly addresses today's class with proven mechanism (R33 precedent).
- Runtime: defensive design — only blocks when `shouldFlag(s) == true` (substantive text + no hub_send). Non-substantive turns and turns-with-hub_send pass through. Tool-only turns pass through (text length 0). Defensive-allow on parse errors (existing pattern preserved for forward-compat).
- One concern: if the hook blocks erroneously (e.g., shouldFlag false-positive), the agent gets stuck. Mitigation: existing dedupe ledger already prevents repeat-spam; threshold-tuning on `minTextLenForFlag` + `planningKeywords` controls false-positive rate. False-positive rate observed in Phase L was acceptable (no shouldFlag false-positives logged in discipline-log).

**Empirical alignment:** today's bilateral violation, if (c) had been live, would have blocked the FIRST pane-only Stop event for each agent. Agent would have received the stderr message, been forced to invoke hub_send, and the deliverable would have surfaced via hub within minutes — not 3 hours.

**Disposition: RECOMMEND for M-2 ship.**

---

## 4. Decision recommendation — Fork (c) Stop-hook BLOCKING enforcement-conversion

Lean (c) for the following reasons:

1. **Direct empirical fit.** Today's bilateral violation is exactly the class (c) terminates. (a) addresses a different class (redundancy); (b) addresses the right class via a non-terminal mechanism (PEER-CROSS-CHECK + AgentState flag).

2. **Mechanism-class match.** Phase L L-4 finding: PEER-CROSS-CHECK is non-terminal at recursion-depth-5+. R33 toolgate (L-5 c2 e327362) introduced mechanical-toolgate-fire as the load-bearing recursion-terminator. (c) applies the same pattern to OUTBOUND-DISCIPLINE.

3. **Lowest implementation cost.** (c) modifies existing `RunHook` return semantics — small LOC delta. (a) requires new pane-text-capture primitives. (b) requires new R-rule + agent-embed + AgentState mechanism.

4. **Bilateral-rationalization neutrality.** Today's violation had distinct rationalization classes per agent (audience-class-misread + post-Bash-drift). (c) blocks regardless of rationalization — the gate fires on the OBSERVABLE failure (pane-text + no hub_send), not on the cognitive trigger.

5. **Composable with (b) for defense-in-depth.** If the user wants belt-and-suspenders, (c) is the primary mechanical-terminator and (b)'s AgentState recovery flag could be a secondary layer for cases where the gate is bypassed (e.g., hook misconfigured / settings.json drift). M-1 c1 R35 PRE-FLIGHT-HOOK-CHECK already covers hook-installation verification, so the (c)-only path is sufficient at the design-spike level.

---

## 5. Implementation plan for Fork (c)

### M-2 c1 bundle (Brian-HANDS implements per ratified design)

**File changes:**

1. **`internal/outboundhook/hook.go`** (MODIFY)
   - Change `RunHook(stdin io.Reader) error` signature → `RunHook(stdin io.Reader, stderr io.Writer) int` (mirror toolgate `RunHook` shape per hook.go:51)
   - Add `ExitAllow = 0` and `ExitBlock = 2` constants (mirror toolgate constants)
   - **Decouple alert-dedupe from block-fire (per Q5 v1.1 decision below — Option (ii)):**
     - Block fires on EVERY shouldFlag-true turn (no dedupe-window bypass)
     - Alert dedupe continues to suppress hub-message spam (existing behavior preserved for noise control)
     - Implementation: hoist the `shouldFlag` + block-write + ExitBlock return BEFORE the `alreadyFlaggedRecently` check; keep dedupe gating only on `emitAlert` + `recordDedup`
   - When `shouldFlag(s) == true`:
     - Write block message to `stderr` parameter: `"OUTBOUND-DISCIPLINE-MECHANICAL violation: substantive pane text emitted without mcp__bot-hq__hub_send. Invoke hub_send (or hub_flag for elevation, or hub_session_close for end-session) before completing this turn. See ~/.claude/skills/phase-rules-detail/SKILL.md § R36 OUTBOUND-DISCIPLINE-MECHANICAL for recovery."`
     - Return `ExitBlock` unconditionally
     - SEPARATELY: continue to emit hub.DB.InsertMessage alert via `emitAlert` IF NOT in dedupe window (existing dedupe semantics preserved for ALERT path only)
   - All other paths return `ExitAllow` (defensive — preserve existing best-effort semantics on parse errors / non-trio sessions / sub-threshold turns)
   - **Verify HubSent detection inclusivity (per Brian PASS-1 Substantive-add #2):** existing `hubSendToolPrefix = "mcp__bot-hq__hub_"` constant (hook.go:79) is a prefix-match that ALREADY covers hub_send + hub_flag + hub_register + hub_session_close + all bot-hq MCP hub-write-class tools. Detection at `parseLastTurn` line 227-228 uses `strings.HasPrefix(b.Name, hubSendToolPrefix)` so any hub-write tool flips `summary.HubSent = true`. **No scope-broadening needed** — Substantive-add #2 concern is mitigated by existing prefix-match design. Verification: cross-check this audit-doc claim during impl by reading hook.go:79 + 227-228 + verifying no regression on prefix-match.

2. **`cmd/bot-hq/main.go`** (MODIFY at line 354)
   - Current: `outboundhook.RunHook(os.Stdin)` (best-effort, never exits)
   - New: `os.Exit(outboundhook.RunHook(os.Stdin, os.Stderr))` (mirror toolgate at line 398)

3. **`internal/protocol/disc.go`** (MODIFY)
   - Add `PhaseMv2OutboundDisciplineMechanical` const with new R-rule (next available R-NN number — likely R36 since M-1 c1 added R35)
   - Substring-locked recognition: "OUTBOUND-DISCIPLINE-MECHANICAL (R36)" + load-bearing terms ("Stop-hook" / "exit 2" / "hub_send" / "blocks turn completion" / "/phase-rules-detail skill")
   - Rule body: explains the mechanism (Stop-hook now blocks on pane-text-without-hub_send) + recovery (invoke hub_send + dedupe ledger entry suppresses immediate re-emit) + cite-anchor (Phase M Target A empirical instances #36 + #37)

4. **`internal/protocol/disc_test.go`** (MODIFY)
   - Add `TestOutboundDisciplineMechanicalSubstringLock` — anchors for each load-bearing term in R36 const
   - Add `TestPhaseMv2OutboundDisciplineMechanicalHeaderAnchor` — locks `- OUTBOUND-DISCIPLINE-MECHANICAL (R36):` prompt-anchor

5. **`internal/rain/rain.go` + `internal/rain/rain_test.go` + `internal/brian/brian.go` + `internal/brian/brian_test.go`** (MODIFY)
   - Embed `PhaseMv2OutboundDisciplineMechanical` in agent prompts
   - Wiring-lock tests: `TestRainPromptEmbedsPhaseMv2OutboundDisciplineMechanical` + `TestBrianPromptEmbedsPhaseMv2OutboundDisciplineMechanical`

6. **`internal/outboundhook/hook_test.go`** (MODIFY)
   - Update existing test signatures from `error` return → `int` return
   - Add `TestRunHook_BlockOnViolation` — when shouldFlag returns true, RunHook returns ExitBlock with stderr message containing required substrings
   - Add `TestRunHook_AllowOnHubSent` — when HubSent is true, RunHook returns ExitAllow with no stderr
   - Add `TestRunHook_AllowOnSubthreshold` — when text is below threshold + no planning keyword, RunHook returns ExitAllow
   - Add `TestRunHook_AllowOnDedupeWindow` — when within dedupe window, RunHook returns ExitAllow (don't double-block)
   - Add `TestRunHookStderrFormat_BlockMessage` — substring-lock on the block message format

7. **`~/.claude/skills/phase-rules-detail/SKILL.md`** (MODIFY, state-side, no-git)
   - Add `## R36 OUTBOUND-DISCIPLINE-MECHANICAL` section
   - Content: what-it-blocks (pane-text + no hub_send + substantive threshold) / mechanism (Stop-hook exit 2 + stderr) / how-to-recover (invoke hub_send) / why-it-exists (today's bilateral violation cite-anchor + ~3h halt cost)
   - Cite per L-3b skill-edit-traceability discipline in commit body

### Estimated bundle scope

- `internal/outboundhook/hook.go`: ~10-20 LOC delta (add ExitAllow/ExitBlock + change return type + add stderr write + 1-2 path adjustments)
- `internal/outboundhook/hook_test.go`: ~80-120 LOC delta (5 new tests + signature updates on existing tests)
- `cmd/bot-hq/main.go`: ~3 LOC delta (change RunHook invocation + os.Exit propagation)
- `internal/protocol/disc.go`: ~30-50 LOC delta (R36 const)
- `internal/protocol/disc_test.go`: ~60-80 LOC delta (substring-lock + header-anchor tests)
- `internal/{rain,brian}/{*.go,*_test.go}`: ~30-40 LOC delta (4 files)
- `~/.claude/skills/phase-rules-detail/SKILL.md`: ~30-50 lines markdown (state-side)

**Total: ~210-310 LOC code + ~30-50 lines markdown skill + ~7-10 ratchet-tests + 2 substring-lock tests + 2 wiring-locks.**

**Target C self-application carry-forward (per Brian PASS-1 carry-forward note):** M-1 c1 was estimated ~150-220 LOC, actual 695L incl tests (~+216% over upper-bound). This M-2 c1 estimate carries the same modeling-gap risk for test-fixture density. Brian-HANDS at impl-time MUST cite from actual `git diff --cached --numstat` at staged-time (R31 STAT-CLAIM-CITE discipline) rather than session-recall pre-author. Add `TestRunHook_BlockOnViolation` + table-driven fixture tests with verbose finding-string assertions per L-3b precedent — expect actual LOC to exceed §5 estimate; document the drift as Phase M discipline-log entry at M-sweep regardless of magnitude.

### Activation: HALT-#1-Phase-M user-fired rebuild+restart

R36 prompt-embed runtime activation requires combined rebuild+restart per Finding-3 invariant — same HALT-#1 already pending for M-1 c1 R35. M-2 c1 + M-1 c1 + (M-3 + M-4 commits if landed before HALT-#1 fires) all activate at the SAME single rebuild+restart event. User's msg 7523 "test it all out after rebuild+restart" matches this terminal-collapse pattern.

### Empirical validation post-rebuild

- Mock pane-only-output Stop event (e.g., emit substantive prose without invoking hub_send) → Stop-hook returns exit 2 → Claude Code propagates stderr to agent → agent must invoke hub_send before turn completes → R36 enforcement-live confirmed
- Same test as Phase L L-5 R33 toolgate empirical-validation pattern — proven test pattern

---

## 6. Open questions for Brian BRAIN-2nd-PASS-1

| Q | Lean | Notes |
|---|------|-------|
| Q1 Fork (a)/(b)/(c) selection | **(c) Stop-hook BLOCKING** | Per §4 rationale — direct empirical fit + lowest cost + mechanism-class match with R33 toolgate precedent |
| Q2 (b) as defense-in-depth secondary layer alongside (c) | DEFER | M-1 c1 R35 PRE-FLIGHT-HOOK-CHECK already covers hook-installation verification; (c)-only is sufficient at design-spike level |
| Q3 Block message format | refined v1.1 | Per Brian PASS-1 Q3 refinement: explicitly cite MCP tool name `mcp__bot-hq__hub_send` (not just "hub_send") + enumerate alternatives (hub_flag for elevation, hub_session_close for end-session). Final format: `"OUTBOUND-DISCIPLINE-MECHANICAL violation: substantive pane text emitted without mcp__bot-hq__hub_send. Invoke hub_send (or hub_flag for elevation, or hub_session_close for end-session) before completing this turn. See /phase-rules-detail skill § R36 for recovery."` |
| Q4 R36 const number assignment | R36 | Next available after M-1 c1 R35; check ratchet-ledger active.md for current max |
| Q5 Dedupe window interaction with blocking | **decided v1.1 — Option (ii) decouple alert-dedupe from block-fire** | Per Brian PASS-1 clarification: original "preserve dedupe" lean creates a `dedupWindow`-sized bypass class (emit-pane + hub_send + emit-pane2 within window = pane2 NOT blocked). Decision: BLOCK fires on every shouldFlag-true turn (zero bypass); ALERT dedupe continues to suppress hub-message spam. Implementation: hoist block-fire BEFORE `alreadyFlaggedRecently` check; gate only `emitAlert + recordDedup` behind dedupe. Mirrors R33's no-dedupe-on-block stance. |
| Q6 Defensive-allow on parse errors | preserved | Existing pattern: parse error → return nil (now ExitAllow). Concur preserve, OR tighten to ExitBlock on transcript parse fail? Lean preserve (don't block agent on hook-side bugs) |
| Q7 Bash-only scope vs all tools | extends from Bash | Existing toolgate is Bash-only (R33). Stop-hook is event-only (not tool-specific) — fires at end of every turn. So (c) naturally covers all turns regardless of which tools were used. Concur — no scope-restriction needed |
| Q8 Empirical-cite in commit-body | yes (refined v1.1) | Cite today's bilateral violation + discipline-log Joint entry 2026-05-04T14:00:00Z + user msg 7476 USER-PIN + user msg 7523 fix-directive. Per Brian PASS-1 + `feedback_no_time_pressure.md`: include cost-citation `~3h halt-in-progress` as motivation context. Standard L-3b skill-edit-traceability + R34 reflexive-bootstrap pattern. |
| Q9 Skill-extend traceability | yes | `~/.claude/skills/phase-rules-detail/SKILL.md` § R36 section adds ~30-50 lines (272 + Phase-L 45 = 317 + Phase-M 30-50 ≈ 347-367L); cite in M-2 c1 commit body |

---

## 7. Risk + cost summary

| Fork | Class fit | Mechanism class | Cost (LOC) | Risk class | Recursion-terminator? |
|------|-----------|-----------------|------------|------------|----------------------|
| (a) PreToolUse-hook-on-hub_send | Wrong class (redundancy not OUTBOUND-MISS) | PreToolUse mechanical | HIGH (~300+ LOC + new pane-capture primitives) | HIGH design-fit | N/A — different class |
| (b) Stop-hook + mandatory recovery prompt-embed | Right class | PEER-CROSS-CHECK + AgentState anchor | MEDIUM (~150 LOC) | MEDIUM-HIGH (PEER-CROSS-CHECK proven non-terminal today) | NO — empirical against |
| **(c) Stop-hook BLOCKING enforcement-conversion** | **Right class** | **Mechanical toolgate (R33 precedent)** | **LOW (~210-310 LOC)** | **LOW** | **YES — same class as R33** |

---

## 8. Cross-references

- **Phase M scope-lock:** `~/.bot-hq/phase/phase-m.md` v1.1 §Tier-1 row M-2
- **Today's discipline-log Joint entry:** `~/.bot-hq/discipline-log.md` 2026-05-04T14:00:00Z (USER-PINNED bilateral OUTBOUND-DISCIPLINE violation; Target A empirical instances #36 + #37; root-cause classes audience-class-misread + post-Bash-investigation-pane-drift)
- **Existing Stop-hook detection layer:** `internal/outboundhook/hook.go` (Phase H slice-5 H-22-bis); `internal/outboundhook/install.go` (idempotent settings.json wiring)
- **R33 toolgate precedent (mechanical recursion-terminator):** `internal/toolgate/r33.go` + `internal/toolgate/hook.go` (Phase L L-5 commit-2 e327362)
- **M-1 c1 R35 PRE-FLIGHT-HOOK-CHECK (verifies hook-installation):** commit 019c7637 (LOCAL, awaiting HALT-#1 rebuild+restart)
- **Phase L L-4 finding (PEER-CROSS-CHECK non-terminal at recursion-depth-5+):** discipline-log Joint entry "Cluster-graduation candidate Target C surfaced" 2026-05-04T07:00:00Z
- **User directive msg 7523:** "include the serious/critical violation for today (fix that). reason is to never let that happen again (it resulted in ~3hours halt in progress)"
- **User pin msg 7476:** "so nothing got done since 19:40:38 here on the hub. this is a very serious violation/issue"

---

## 9. Posture

phase-m M-2 Target A audit-doc v1.1 surfaced | 3 forks analyzed (a/b/c) with (c) emerging from per-fork analysis | recommended (c) Stop-hook BLOCKING enforcement-conversion per direct empirical fit + lowest cost + R33 mechanism-class precedent | implementation plan for M-2 c1 bundle (~210-310 LOC + skill +30-50L; Target C carry-forward applies — expect drift) | 9/9 open questions resolved (Q5 decided Option (ii) decouple alert-dedupe from block-fire; Q3 message format refined with explicit MCP tool names; Q8 cost-citation included) | 2 substantive additions applied (Stop-hook exit-2 pre-impl empirical-validation gate + HubSent inclusivity verification noting existing prefix-match design already covers all hub-write tools) | next: Brian BRAIN-2nd-PASS-2-FINAL on v1.1 → ratify → Brian-HANDS implements M-2 c1 per design.
