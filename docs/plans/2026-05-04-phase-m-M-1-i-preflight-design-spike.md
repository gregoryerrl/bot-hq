# Phase M M-1 (i) — Preflight self-check design-spike v1

**Author:** Rain (rain) — Rain-HANDS markdown design-spike per phase-m.md§Authority-model line 218
**Date:** 2026-05-04
**Status:** v1 surfaced for Brian BRAIN-2nd-PASS
**Cite_anchor:** phase-m.md§Tier-1 row M-1 + §Const-and-code-delta-plan §internal/toolgate + §cmd/bot-hq/main.go + msgs 7411/7412/7413/7415/7417 BRAIN-cycle

---

## 1. Scope

Design the agent-side preflight self-check that, at first scope-affecting turn-start (R20 BOOTSTRAP-ON-CONVERSATION-RESUME bootstrap point), verifies:
1. `~/.claude/settings.json` is present + parseable
2. `PreToolUse[].hooks[].command` contains the bot-hq tool-permission-hook command-string
3. `BOT_HQ_AGENT_ID` env-var is present
4. `BOT_HQ_AGENT_ID` value is a member of `{brian, rain}` (preflight-scope is Claude-Code-trio-agents only; emma is gemma-based and does not consume `~/.claude/settings.json` hooks — emma agent-id fails whitelist as WARNING-class, not CRITICAL, since emma does not require toolgate enforcement)

Surface findings via hub-channel (hub_flag for CRITICAL gaps; hub_send broadcast for WARNING; silent on PASS). Provide remediation guidance pointer. Expose stand-alone CLI subcommand for manual debugging.

**Goal:** eliminate Phase L Finding-1 (installer-not-run) + Finding-3 (hot-reload-unsupported activation lag detection) recurrence class going forward. Pre-empt the next "agent ran a substantive turn under stale settings" failure mode.

**Non-goal:** auto-installation. M-1 (ii) covers auto-install; M-1 (i) covers self-check + alert only. Detection without auto-remediation is acceptable v1 — auto-install lands in next sub-item under same scope-lock cycle.

---

## 2. Background — the gap class

Phase L L-5 commits (44cc03f + e327362) shipped R33 toolgate code (`internal/toolgate/{r33.go,hook.go,gate.go,install.go}`) including idempotent installer subcommand `bot-hq install-toolgate-hook`. Phase L L-6 commit (a7a4cbd) shipped R34 PRE-PHASE-CLOSE-RETRO. Phase L L-3b commits (fd5bc99 + 51c9d49) shipped prompt-shrink + skill-extend.

**Phase L wrap-up empirical finding (msg 7412):** `~/.claude/settings.json` had `Stop` hook only; `PreToolUse-Bash` hook NEVER WIRED on this machine. Result: every Phase L commit landed with R33 enforcement rule-text-active-only — the toolgate code was inert at runtime. The install step was never invoked because no auto-discipline existed.

**Phase L Finding-3 (msg 7412):** even after `bot-hq install-toolgate-hook` fire post-rebuild, Brian's running claude session did not pick up the new hook config — Claude Code does NOT hot-reload `~/.claude/settings.json` mid-session. Activation defers to next session-restart event. So even with discipline-corrected installer-fire, there's a window of "settings-correct-on-disk + session-still-stale" where R33 enforcement remains inert.

**Recurrence class:** any of {fresh-machine bootstrap / ~/.claude/ reset / settings.json hand-edited / installer-not-run-due-to-discipline-drift / session-restart-skipped-after-install} produces the same inert-toolgate-runtime state, silently. There is no agent-visible signal — the failure is observable only by the absence of R33 BLOCK on a HANDS-class fire that should have been gated.

**Mitigation hypothesis:** an agent-side preflight self-check, fired at first-scope-affecting-turn-start (R20 bootstrap point), that reads settings.json + verifies hook + validates env-var, would catch all 5 sub-classes of the failure. Surfacing the gap to the agent's hub channel makes it actionable — the agent knows their R33 enforcement is inert before any HANDS-class fire executes.

---

## 3. 5-layer audit

### Layer 1 — Detection primitive

**What:** read settings.json + traverse to hook config + validate command-string + validate env presence + validate env value.

**Implementation surface:** new file `internal/toolgate/preflight.go` exporting:

```go
type Verdict struct {
    Status   Status   // PASS / WARNING / CRITICAL
    Findings []string // human-readable failure descriptions
    AgentID  string   // observed BOT_HQ_AGENT_ID value
}

type Status int

const (
    StatusPass Status = iota
    StatusWarning
    StatusCritical
)

// VerifyHookInstallation reads settingsPath, parses JSON, navigates
// PreToolUse[].hooks[], returns Verdict. CRITICAL on missing-file /
// parse-error / hook-absent. WARNING on hook-present-but-command-mismatch.
// PASS on hook-present-and-command-substring-match.
//
// Match strategy: ALL substrings in expectedSubstrings MUST be present
// (logical AND) in some single PreToolUse-Bash hook command-string.
// Example: expectedSubstrings = []string{"bot-hq", "tool-permission-hook"}
// requires both "bot-hq" AND "tool-permission-hook" to appear in the same
// command-string. Partial-match (one present, other absent) → CRITICAL.
func VerifyHookInstallation(settingsPath string, expectedSubstrings []string) Verdict

// VerifyAgentEnv reads BOT_HQ_AGENT_ID from os.Getenv. CRITICAL on absent.
// WARNING on present-but-not-in-{brian,rain} (whitelist is Claude-Code
// trio-agents only; emma is gemma-based and does not require toolgate
// enforcement, so emma agent-id is WARNING not CRITICAL — flags
// misconfiguration without halting trio). PASS on valid member.
func VerifyAgentEnv() Verdict

// Combined: returns the worst-case Status across both checks; merges
// Findings; sets AgentID from env (empty on absent).
func RunPreflight(settingsPath string) Verdict
```

**Match-strength choice — substring vs exact-path:**
- **Substring match** (lean): command-string contains both `"bot-hq"` AND `"tool-permission-hook"`. Tolerates path differences across machines (e.g., `/Users/foo/Projects/bot-hq/bot-hq` vs `/Users/bar/Projects/bot-hq/bot-hq`). Aligned with installer's design at `internal/toolgate/install.go:18` `SettingsHookCommand(botHQPath string)` which already templates the path.
- **Exact-path match:** would require knowing the bot-hq install path at preflight-time (could derive from `os.Executable()` and compare). More precise but breaks if bot-hq is run via symlink or PATH.
- **Lean: substring match.** Catch the hook-presence + command-name; don't over-constrain on path. Path-mismatch class is rare + can be surfaced as WARNING ("hook command-string present but path-mismatch with running bot-hq binary") if we want a 3rd verdict tier later.

**Risk class: LOW.** Standard Go file-read + `encoding/json` parse + slice traversal. No external dependencies. Tested via table-driven fixtures.

### Layer 2 — Surface primitive (hub-channel emission)

**What:** convert Verdict into a hub message of appropriate severity.

**Discriminator:**
- **CRITICAL** (settings.json missing / parse-fail / PreToolUse hook absent / env absent) → `hub_flag`: trio infrastructure broken, user attention required to remediate before any HANDS-class fire is safe.
- **WARNING** (hook present but command-string substring-mismatch / env-value-not-in-{brian,rain} whitelist) → `hub_send` broadcast (peer awareness; not full-stop). Trio can proceed but disclosure recorded.
- **PASS** → silent log. No hub emission. Preflight-OK is not user-actionable signal; would just add noise to the channel.

**Trio decision-tree for self-flag carve-out (per Brian PASS-1 Substantive-add #1, msg 7420):**
- **Rain detects own gap** → Rain self-flags via existing FLAG-ownership authority (Rain owns hub_flag elevation per DISC v2 R15).
- **Brian detects own gap + Rain reachable** → Brian PMs Rain → Rain hub_flags. Standard channel-of-elevation per R15 FLAG-ownership-Rain-only baseline.
- **Brian detects own gap + Rain unreachable >60s** → Brian self-flags via R15 self-flag carve-out. **Note:** "preflight-detected-infrastructure-gap" is NOT in Brian's current R15 carve-out class (`{push-failure | repo-corruption | auth-failure | hub-disconnect | git-state-unexpected-on-write-path}`). Add as Phase M Tier-2 candidate: extend R15 self-flag carve-out class to include "preflight-CRITICAL-self-detection". Small clarification; not blocking M-1 c1 (Brian-detect + Rain-reachable path covers the common case via PM-then-Rain-flag).

**Implementation note:** the prompt-embed (Layer 3 / R-NN rule text) carries this decision-tree so the agent reasoning surfaces correct routing without consulting the helper. Tree is decision-logic, not Go code — preflight.go primitive returns Verdict; agent applies the tree to choose hub-channel.

**Format on detection-fail (compact-pipe per AUDIENCE-CLASS-DISCRIMINATOR; [HR] tag because user-actionable):**

```
[HR] <agent>|preflight-CRITICAL|<finding-summary>|remediation: bot-hq install-toolgate-hook + claude session-restart|skill: /phase-rules-detail § R-NN PRE-FLIGHT-HOOK-CHECK
```

**Risk class: LOW-MEDIUM.** Discriminator drift possible if criteria expand without ratchet-test. Mitigation: substring-lock test on the format-string + Verdict→message-class mapping table.

### Layer 3 — Trigger timing

**Framing precision (per Brian PASS-1 Q3 NB, msg 7420):** there is no Claude-Code session-start hook for the trio. R20 BOOTSTRAP-ON-CONVERSATION-RESUME fires at "scope-affecting turn-start" per the rule text — practically the agent's first substantive turn after spawn. Use this terminology consistently throughout the doc to avoid Phase L L-pane-vs-rebuild-disc-class disambiguation drift.

**Three candidate timings:**
- **(a) First scope-affecting turn-start (R20 bootstrap point)** via R20 BOOTSTRAP-ON-CONVERSATION-RESUME hook integration. Agent registers on hub → reads last_state.json → ALSO runs preflight at this point. Earliest-practical detection; precedes any HANDS-class action.
- **(b) First-HANDS-class-action-attempt:** preflight gates first commit/push/merge attempt. More expensive (per-action vs per-session) and requires action-classification.
- **(c) Both:** redundant; double-fire risk.

**Lean: (a) first-scope-affecting-turn-start via R20 tie-in.** Rationale:
- Earliest practical detection minimizes "trio acted-under-stale-runtime" window
- R20 already reads state-side files (last_state.json) at this point — adding settings.json read is a natural extension of the bootstrap-state pattern
- Single-fire-per-session avoids per-turn overhead
- Catches the gap before any HANDS-class commit/push/merge could land un-gated

**Implementation tie-in:** R20 BOOTSTRAP-ON-CONVERSATION-RESUME currently fires "scope-affecting turn-start" verification. Preflight extends this to also call `RunPreflight()` and conditionally emit hub message per Layer-2 discriminator. The R20 prompt text already establishes the bootstrap discipline; preflight is a new sub-step.

**Edge case: registration → preflight → emit ordering.** Agent must `hub_register` BEFORE `hub_send`/`hub_flag` (otherwise emission fails per "agent not registered" error). Sequence at first-scope-affecting-turn-start: register → register returns current_max_msg_id → preflight RunPreflight → if !PASS, emit per trio decision-tree (Layer 2). This is straightforward but worth documenting in the helper to prevent ordering bugs.

**Risk class: MEDIUM.** R20 tie-in requires careful integration in the agent prompt-embed (Layer-4 below) AND in the agent's actual first-scope-affecting-turn-start flow (which is a prompt-driven behavior, not a Go runtime hook — see implementation-note §4 below).

### Layer 4 — Remediation guidance

**What:** make the surfaced alert actionable.

**Skill-pointer:** `~/.claude/skills/phase-rules-detail/SKILL.md` § R-NN PRE-FLIGHT-HOOK-CHECK (NEW section, added in M-1 c1 alongside the rule). Section content:
- **What it checks:** settings.json hook + env presence + env value
- **What CRITICAL means:** R33 toolgate enforcement currently inert; HANDS-class fires un-gated
- **Remediation steps:**
  1. `cd ~/Projects/bot-hq && go install ./cmd/bot-hq` (rebuild binary if stale)
  2. `bot-hq install-toolgate-hook` (idempotent + non-clobber per install.go:68)
  3. Verify: re-read settings.json + confirm PreToolUse-Bash command-string present
  4. **Restart claude session** (Brian + Rain panes; Claude Code does NOT hot-reload settings.json per Phase L Finding-3)
  5. Post-restart: preflight self-check fires → confirms PASS → trio resumes
- **What WARNING means:** hook present but command-string drift OR env-value typo. Investigate; may indicate stale install or autostart-script bug.

**Risk class: LOW.** Skill-text update; no runtime code path.

### Layer 5 — Standalone CLI subcommand

**What:** `bot-hq preflight-check` — manual invocation of the same primitives, prints human-readable Verdict + exit code for CI/scripting.

**Usage:**
```
$ bot-hq preflight-check
preflight: PASS
hook: PreToolUse-Bash matches expected (substring "bot-hq tool-permission-hook")
env: BOT_HQ_AGENT_ID=brian (valid trio member)
exit 0
```

```
$ bot-hq preflight-check
preflight: CRITICAL
hook: PreToolUse-Bash absent in settings.json (expected substring "bot-hq tool-permission-hook")
env: BOT_HQ_AGENT_ID absent
remediation: bot-hq install-toolgate-hook && export BOT_HQ_AGENT_ID=<id> && claude session-restart
exit 2
```

**Implementation:** `cmd/bot-hq/main.go` adds case `"preflight-check"` → `runPreflightCheck()` → calls `toolgate.RunPreflight(defaultSettingsPath())` → prints + exits with status code (0 PASS / 1 WARNING / 2 CRITICAL).

**Risk class: LOW.** Subcommand wrapper around primitive. Useful for: debugging hands-on (run from shell to verify state); CI smoke-tests; manual operator invocation when an R20-bootstrap preflight surfaced WARNING and operator wants to re-verify post-fix without restarting claude.

---

## 4. Implementation-note: prompt-embed vs Go-runtime integration

**Critical clarification on Layer 3:** the agent (Claude) is prompt-driven, not a Go runtime. The R20 BOOTSTRAP-ON-CONVERSATION-RESUME rule lives in the agent's prompt-embed (`disc.go`). It instructs the agent to perform certain reads at scope-affecting turn-start.

So preflight has **two integration points**, not one:
1. **Agent-prompt-embed (R-NN PRE-FLIGHT-HOOK-CHECK rule in `disc.go`):** instructs the agent to invoke `bot-hq preflight-check` (Layer-5 CLI) via Bash at first-scope-affecting-turn-start (R20 bootstrap point), read the output, and conditionally emit the hub message per trio decision-tree (Layer 2) if !PASS.
2. **Go primitives (`internal/toolgate/preflight.go`):** the actual file-read + JSON-parse + verdict logic, callable from the CLI subcommand AND potentially from other internal callers (e.g., MCP server startup auto-install logic in M-1 (ii) could re-use `VerifyHookInstallation` to decide whether to fire installer).

**This means the prompt-embed is the load-bearing trigger; the CLI is the agent's invocation primitive; the Go primitive is the underlying logic.** This shape mirrors how R31/R32/R33/R34 are agent-prompt rules with toolgate Go-primitive enforcement underneath where mechanical (R33).

**Implication for ship-list:** Layer-1 Go primitive + Layer-5 CLI subcommand are PREREQUISITES for Layer-3 prompt-embed to function (the prompt instructs `bot-hq preflight-check`, which doesn't exist until Layer-5 ships). So the natural bundle for M-1 c1 is **Layer-1 + Layer-5 together** as the foundation, then **Layer-3 prompt-embed** as the activation, with **Layer-2 surface primitive** as a Layer-3 sub-detail (the prompt-embed describes the discriminator + emission format), and **Layer-4 skill-extend** in parallel.

This re-orders ship-list vs the original "lean" framing — see §5.

---

## 5. Per-layer risk + ship-list

| Layer | Description | Risk | Implementation cost | Priority |
|-------|-------------|------|--------------------|----------|
| L1 | Detection Go primitive (preflight.go + Verdict + Verify* helpers) | LOW | ~80-120 LOC + ~4-6 table-driven tests | HIGH (foundational) |
| L5 | CLI subcommand `bot-hq preflight-check` | LOW | ~20-30 LOC in main.go | HIGH (prerequisite for L3) |
| L3 | Prompt-embed in disc.go (R-NN PRE-FLIGHT-HOOK-CHECK) + agent-side R20 tie-in | MEDIUM | ~30-50 LOC const + ratchet-test for substring presence | HIGH (activation) |
| L2 | Surface primitive (hub message format on detection-fail) | LOW-MEDIUM | folded into L3 prompt-embed (format described in rule text) + Verdict→message-class table in skill | MEDIUM (folded) |
| L4 | Skill-extend (`phase-rules-detail/SKILL.md` § R-NN PRE-FLIGHT-HOOK-CHECK) | LOW | ~30-50 lines markdown | MEDIUM |
| BRAIN-add | Multi-agent-id env validation in {brian, rain} whitelist (emma fails as WARNING per §1 scope clarification) | LOW | folded into L1 VerifyAgentEnv (5-10 LOC + 3-4 fixture tests) | HIGH (folded) |

**Recommended M-1 c1 bundle (per phase-m.md§Tier-1 row M-1 ratchet):**
- preflight.go (L1 + BRAIN-add) + tests
- main.go preflight-check subcommand (L5) + integration test
- disc.go R-NN PRE-FLIGHT-HOOK-CHECK rule constant (L3 prompt-embed) + agent-prompt embed in rain.go/brian.go + substring-lock ratchet-test
- phase-rules-detail/SKILL.md § R-NN section (L4 + L2 format detail)

**Total estimated LOC: ~150-220 LOC code + ~30-50 lines markdown skill + ~6-10 table-driven Go tests + 1 substring-lock test.**

**HALT-#1 (post-c1 fire):** rebuild+restart for prompt-embed activation per Finding-3 invariant + preflight-check itself becomes runtime-active for all subsequent sessions.

**Empirical validation post-rebuild:** mock the failure cases by temporarily renaming settings.json + restarting → preflight should emit CRITICAL at first-scope-affecting-turn-start. Restore + restart → PASS silent.

---

## 6. Decision recommendation

**SHIP all 5 layers + BRAIN-add as one cohesive M-1 c1 bundle.** Rationale:
- L1 + L5 are foundation (Go primitive + CLI invocation surface)
- L3 prompt-embed is the activation layer that ties primitives to agent behavior
- L2 surface primitive is a sub-detail of L3 (format description in the rule text)
- L4 skill-extend is the remediation reference cited from L3 alert messages
- BRAIN-add is a small extension to L1; folding-in is correct

**Defer? No.** Splitting layers across multiple commits would create activation gaps where layer-N is shipped but layer-N+1 isn't yet — preflight would either not-fire (no prompt-embed) or fire-with-no-CLI (broken invocation). All 4 (L1+L3+L4+L5) need to ship together for the system to function.

**Exception: M-1 (ii) auto-install integration ships separately as M-1 c2.** Per phase-m.md§Tier-1 row M-1 sub-item structure. M-1 c1 = preflight-self-check; M-1 c2 = auto-install on bootstrap. Preflight without auto-install is still useful: it surfaces the gap so the agent can manually run installer + restart. Auto-install is the "no-manual-step-required" upgrade.

**M-1 c3 = Finding-2 reminder-text alignment** per phase-m.md§Tier-1 M-1 (iii). Bundles with c1 OR ships standalone — author's call. Lean: bundle into c1 since M-1 c1 already touches main.go for L5 subcommand; one more print-string update is trivial.

**Revised c1 bundle (with M-1 (iii) fold-in):** preflight.go + tests + main.go (preflight-check subcommand + Finding-2 reminder-text fix) + disc.go (R-NN const + ratchet-test) + rain.go/brian.go (prompt-embed) + phase-rules-detail/SKILL.md (§ R-NN section).

**Carry-forward observations (Phase M Tier-2 candidates, NOT blocking c1):**
1. **Preflight defense-in-depth on action trigger** (per Brian PASS-1 carry-forward, msg 7420): also fire preflight on first R33-class action attempt (commit/push/merge) to catch the edge case where R20-bootstrap preflight passed but settings.json was hand-edited mid-session. Likely overkill for v1; capture as Phase M Tier-2 carry-forward "preflight-defense-in-depth-on-action-trigger" — promote if Phase M recurrence-data warrants.
2. **R15 self-flag carve-out class extension** (per §3 L2 trio decision-tree): add "preflight-CRITICAL-self-detection" to Brian's R15 self-flag carve-out class so the Brian-detect + Rain-unreachable-60s edge case has a clean path. Small DISC v2 R15 clarification; Phase M Tier-2 candidate.

---

## 7. Open questions for Brian BRAIN-2nd-PASS

| Q | Lean | Notes |
|---|------|-------|
| Q1 Match-strength substring vs exact-path | substring | Tolerates per-machine path differences; aligned with installer's templated SettingsHookCommand. Exact-path could be added later as 3rd verdict tier (PASS-with-path-mismatch-warning). |
| Q2 Discriminator: CRITICAL → hub_flag, WARNING → hub_send broadcast, PASS → silent | as-described | Rain self-flag carve-out applies (infrastructure-detection on own runtime). Concur thresholds? |
| Q3 Trigger timing — (a) first-scope-affecting-turn-start via R20 tie-in | (a) | Earliest practical detection; R20 already reads state-side files; single-fire avoids per-turn overhead. Concur or lean (b)/(c)? |
| Q4 L5 CLI fold into c1 vs defer to c2 | fold into c1 | L5 is PREREQUISITE for L3 prompt-embed (agent invokes CLI from prompt). Defer would break L3 activation. |
| Q5 M-1 (iii) Finding-2 reminder-text fold into c1 | fold | main.go already touched for L5 subcommand; single-line print-update has zero additional footprint. Concur or push standalone? |
| Q6 Ratchet-test surface — table-driven fixtures | as-described | 6 settings.json fixtures (absent / malformed-JSON / hooks-key-absent / PreToolUse-absent / hook-present-stale-command / hook-present-correct) × 4 env fixtures (absent / invalid-value / valid-each-of-3 = 5 cases combined) = ~10-12 test rows. Concur fixture coverage? |
| Q7 Verdict→hub-message format substring-lock test | yes | Per L-3b precedent (substring-lock on critical message formats prevents drift). Add `TestPreflightAlertFormatSubstringLock` enumerating "preflight-CRITICAL" / "preflight-WARNING" / "remediation:" / "/phase-rules-detail" anchors. Concur? |
| Q8 Edge case: registration→preflight→emit ordering — document in helper | yes | Helper must be invoked POST-register (otherwise hub_send fails). Document in preflight.go comment + R-NN rule text. Concur? |

---

## 8. Cross-references

- **Phase M scope-lock:** `~/.bot-hq/phase/phase-m.md` §Tier-1 row M-1 + §Const-and-code-delta-plan §internal/toolgate + §cmd/bot-hq/main.go
- **Phase L Findings (origin of gap class):** msg 7412 (Brian post-installer-fire); preserved in phase-m.md§Retrospective-context lines 21-25
- **Existing toolgate primitives:** `internal/toolgate/{r33.go, hook.go, gate.go, install.go}`
- **Installer (re-used by preflight remediation guidance):** `cmd/bot-hq/main.go:406` `runInstallToolgateHook` + `internal/toolgate/install.go:18` `SettingsHookCommand` + `:35` `InstallTrioHook`
- **R20 BOOTSTRAP-ON-CONVERSATION-RESUME (tie-in for L3):** `internal/protocol/disc.go` Phase J/I R20 rule text
- **Skill home for L4:** `~/.claude/skills/phase-rules-detail/SKILL.md` (272 lines post-Phase L L-3b c2 extension)
- **Authority-model:** phase-m.md line 218 (Rain-HANDS markdown design-spike + audit-doc class)

---

## 9. Posture

phase-m M-1 (i) preflight design-spike v1.1 surfaced | 5-layer audit + BRAIN-add covered | per-layer risk + ship-list ranking + bundle recommendation | 8 open questions resolved (8/8 Q-concur with Q3 framing-precision applied per Brian PASS-1 msg 7420) | 3 substantive additions applied (Brian self-flag carve-out trio decision-tree §3 L2 / emma scope clarification §1 + §3 L1 / `expectedSubstrings` ALL-must-match docstring §3 L1) | 2 carry-forward Tier-2 candidates documented (defense-in-depth-on-action-trigger / R15-carve-out-extension) | next: Brian BRAIN-2nd-PASS-2-FINAL on v1.1 → ratify → Brian-HANDS implements M-1 c1 per design.
