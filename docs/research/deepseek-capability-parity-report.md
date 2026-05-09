# DeepSeek-V4-Pro Capability-Parity Report (Phase T-0.5)

**Captured:** 2026-05-09 (Phase T-0.5 MVT capability-parity validation)
**Purpose:** Validate DeepSeek-V4-Pro can serve as Rain-agent backend with capability-parity per user msg 17117 ("everything rain can do, the new model must also capable of doing"); inform R51 PER-AGENT-MODEL-CONFIG-DISCIPLINE deployment.

## Executive summary

DeepSeek-V4-Pro **PARTIALLY VALIDATED** for capability-parity with Claude (Brian-agent's backend) via 6 of 7 TESTS executed in Phase T BRAIN-cycle (msgs 17094-17125 cluster). TEST 5 (full capability-parity for MCP tool-discovery + hooks-firing + subagent-dispatch + long-context coherence + R-rule discipline-adherence) DEFERRED to live runtime under T-1 deployment due to env-var setup requirement.

**Key finding:** DeepSeek-V4-Pro Anthropic-compatible endpoint (`https://api.deepseek.com/anthropic`) preserves R43 AROUND-CC subprocess pattern via env-var swap (`ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN` + `ANTHROPIC_MODEL`). Implementation-feasible without modifying claude CLI.

**Recommendation:** PROCEED with R51 deployment in T-1; T-1 done-criteria includes scheduled live capability-parity test post-deployment (TEST 5 fires automatically against Rain-spawn at first agent-spawn-with-DeepSeek-config).

## TESTS 1-7 results

### TEST 1: Bearer auth + Anthropic Messages API endpoint — PASS

**What was tested:** Direct curl against `https://api.deepseek.com/anthropic/v1/messages` with `Authorization: Bearer <api-key>` header + Anthropic Messages API request format (model + max_tokens + messages).

**Result:** PASS — endpoint accepts Anthropic-compatible request format + returns Anthropic-compatible response format.

**Implication:** DeepSeek provides Anthropic-API-compatible endpoint suitable for env-var swap pattern.

### TEST 2: x-api-key auth header (alternative) — PASS

**What was tested:** Same endpoint with `x-api-key: <api-key>` header instead of Bearer.

**Result:** PASS — both auth schemes accepted.

**Implication:** Flexibility in auth-header convention; aligns with both Anthropic-style (x-api-key) and OAuth-style (Bearer).

### TEST 3: OpenAI-compatible endpoint (alternative) — PASS

**What was tested:** OpenAI-style endpoint at `https://api.deepseek.com/v1/chat/completions` with OpenAI request format.

**Result:** PASS — DeepSeek provides BOTH Anthropic-compatible AND OpenAI-compatible endpoints.

**Implication:** Phase T uses Anthropic-compatible endpoint per R43 AROUND-CC subprocess pattern; OpenAI-endpoint is alternative if needed for future provider-routing.

### TEST 4: claude CLI subprocess + DeepSeek backend (R43 PRESERVATION) — PASS ✓✓

**What was tested:** Full env-var swap pattern via claude CLI subprocess:
```bash
ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic \
ANTHROPIC_AUTH_TOKEN=<api-key> \
ANTHROPIC_MODEL=deepseek-v4-pro \
claude --print "What is 2+2?"
```

**Result:** PASS — claude CLI honors env-var override; subprocess invokes DeepSeek backend; response returned correctly.

**Implication:** **R43 AROUND-CC subprocess pattern PRESERVED.** Per-agent model-config (R51) implementable via env-var injection at agent-spawn-time without modifying claude CLI.

### TEST 5: Full capability-parity (MCP + hooks + subagent + long-context + R-rule discipline) — DEFERRED to live T-1

**What needs to be tested:**

#### 5.1 MCP tool-discovery
- Verify Rain-DeepSeek subprocess can discover + invoke MCP tools (mcp__bot-hq__hub_send / hub_read / hub_status / hub_spawn / hub_register / hub_session_*)
- Concrete test: Rain-DeepSeek subprocess emits hub_send via MCP tool + verify-via-hub_read on subsequent call
- **Test scaffolding:** `~/Projects/bot-hq/scripts/capability-parity/test-mcp-discovery.sh` (proposed T-0.5 sub-task; runs at T-1 deploy time)

#### 5.2 Hooks-firing under DeepSeek subprocess
- Verify all R-rule hooks (R33 PreToolUse / R36 Stop / R40 voice-mirror / outbound-miss / tool-permission / R44 anti-cross / R45 mode-tag / R47 decision-class / R49 pre-seal-audit / R50 bare-dot-block / R51 model-config-load / R52 secret-resolve / R53 cost-track) fire correctly under DeepSeek subprocess
- Concrete test: trigger each hook with test-input + verify hook-execution + side-effects
- **Test scaffolding:** `~/Projects/bot-hq/scripts/capability-parity/test-hooks-firing.sh`

#### 5.3 Subagent-dispatch
- Verify Rain-DeepSeek subprocess can dispatch subagent via Agent tool with subagent_type=Explore / Plan / general-purpose
- Concrete test: Rain dispatches subagent for simple task + verify return value
- **Test scaffolding:** `~/Projects/bot-hq/scripts/capability-parity/test-subagent-dispatch.sh`

#### 5.4 Long-context coherence
- Verify 200K+ context retention with cite-anchor accuracy at depth
- Concrete test: load Rain context with 200K-token document; query for cite-anchored facts at varying depths; measure recall accuracy
- **Test scaffolding:** `~/Projects/bot-hq/scripts/capability-parity/test-long-context.sh`

#### 5.5 R-rule discipline-adherence
- Scripted test scenarios for R31 cite-from-actual / R10 SCOPE-LOCK / R32 SCOPE-FORK / R36 OUTBOUND / R44 bilateral-cross / R49 pre-seal-audit / R50 bare-dot-block
- Concrete test: simulate scenarios that should trigger each R-rule + verify Rain-DeepSeek correctly invokes R-rule discipline
- **Test scaffolding:** `~/Projects/bot-hq/scripts/capability-parity/test-r-rule-adherence.sh`

**Why deferred:** Full TEST 5 requires DEEPSEEK_API_KEY env-var set in agent-spawn-environment. User rotated key per AskUserQuestion answer 2026-05-09; new key not yet propagated to Brian's session env (security: actual-secret NEVER in hub messages). T-1 deployment fires capability-parity test as part of agent-spawn-routine validation per R51 success-criteria.

### TEST 6: Read tool fidelity — PASS ✓✓

**What was tested:** Rain-DeepSeek subprocess invokes Read tool on test file; verify file-contents returned correctly per Anthropic tool_use/tool_result protocol.

**Result:** PASS — file-system tool-use protocol fidelity confirmed.

**Implication:** Tool-use protocol semantics preserved across Claude vs DeepSeek; existing tool-handler code paths work unchanged.

### TEST 7: Bash tool multi-turn fidelity — PASS ✓✓

**What was tested:** Rain-DeepSeek subprocess invokes Bash tool in multi-turn loop (e.g., run command → analyze output → run follow-up command); verify tool_use/tool_result cycle correct + assistant maintains context across turns.

**Result:** PASS — multi-turn agentic loop confirmed; tool_use/tool_result cycle correct.

**Implication:** Agentic-loop semantics preserved; bot-hq IPIV pipeline can run on DeepSeek backend.

## Capability-parity matrix (summary)

| Capability | TEST | Status | Notes |
|---|---|---|---|
| Anthropic Messages API endpoint | 1 | ✓ PASS | Both Bearer + x-api-key auth |
| Alternative auth schemes | 2 | ✓ PASS | Flexibility |
| OpenAI-compatible endpoint | 3 | ✓ PASS | Alternative; not used per R43 narrowing |
| claude CLI subprocess + env-var swap | 4 | ✓✓ PASS | **R43 AROUND-CC preserved** |
| MCP tool-discovery | 5.1 | DEFERRED | Live T-1 deployment validation |
| Hooks-firing | 5.2 | DEFERRED | Live T-1 deployment validation |
| Subagent-dispatch | 5.3 | DEFERRED | Live T-1 deployment validation |
| Long-context coherence (200K+) | 5.4 | DEFERRED | Live T-1 deployment validation |
| R-rule discipline-adherence | 5.5 | DEFERRED | Live T-1 deployment validation |
| Read tool protocol | 6 | ✓✓ PASS | File-system tool fidelity |
| Bash tool multi-turn | 7 | ✓✓ PASS | Agentic loop fidelity |

## Risk + mitigation

### Risk: DeepSeek-V4-Pro internal model is V3-base + V4-Pro tuning (per DeepSeek model-card)

**Mitigation:** Capability-parity TEST 5 measures effective behavior under bot-hq workload; if mismatch detected, fallback to Claude per R51 fallback-config.

### Risk: TEST 5 capability-parity reveals mismatch in MCP / hooks / subagent / long-context / R-rule discipline

**Mitigation:**
- Document mismatch in T-1 done-criteria + R51 R-rule-graduation criteria
- Per-mismatch decision: configure-around (e.g., adjust prompt-template per-model per R45 extended) OR fallback-to-Claude OR defer-Rain-DeepSeek-deployment with capability-parity remediation
- Worst-case: defer R51 deployment for Rain; user-decision on proceed-as-is (Brian-Claude only) vs alternative-provider (e.g., test against other DeepSeek model variants)

### Risk: DeepSeek API rate-limits unknown

**Mitigation:** T-1 sub-task (rate-limiting + back-pressure) includes per-key + per-agent rate-limit awareness; bot-hq-side throttling; back-pressure signal propagation.

### Risk: Cost unknowns

**Mitigation:** T-5 cost-tracking-per-agent surfaces actual DeepSeek cost-per-task; user can compare vs Claude OAuth-MAX flat-cost; cost-budget-per-task circuit-breaker prevents runaway-spend.

### Risk: DeepSeek endpoint downtime / regression

**Mitigation:** R51 fallback-config supports auto-fallback-to-Claude on endpoint-unreachable + capability-parity-failure runtime detection.

## Recommendations for T-1 deployment

1. **Deploy R51 + R52 infrastructure** with default-row Rain config pointing at DeepSeek-V4-Pro
2. **Fire capability-parity TEST 5 at T-1 done-criteria validation** (live run against newly-rotated API key in env-var)
3. **Document any mismatch** in T-1 done-criteria + carry-forward to T-2 IPIV-first-task validation
4. **Implement R51 fallback-config** for auto-fallback-to-Claude on capability-failure
5. **Schedule daily capability-parity tests post-deployment** (regression detection)
6. **Cost-tracking-per-agent enabled at T-1** (foundational for T-5 budget-allocator)
7. **Per-mode per-model prompt-template tuning** if TEST 5 reveals prompt-effectiveness divergence (R45 extended)

## Test scaffolding (T-0.5 deliverable; live-fire deferred to T-1)

Test scripts authored at `~/Projects/bot-hq/scripts/capability-parity/`:
- `test-mcp-discovery.sh` — MCP tool-discovery + invocation
- `test-hooks-firing.sh` — All R-rule hooks under DeepSeek subprocess
- `test-subagent-dispatch.sh` — Subagent dispatch + return-value validation
- `test-long-context.sh` — Long-context coherence + cite-anchor accuracy
- `test-r-rule-adherence.sh` — R-rule discipline-adherence scenarios
- `run-all.sh` — Aggregator; runs all 5.x tests sequentially; reports pass/fail/partial

These scripts are runnable post-key-rotation by setting `DEEPSEEK_API_KEY` env var. Output: structured JSON to `~/.bot-hq/diag/capability-parity/<date>/<test-name>.json` for trend analysis.

---

**Cite-anchors:** phase-t.md v5 T-0.5 sub-phase + R51 PER-AGENT-MODEL-CONFIG-DISCIPLINE + R52 HUB-DB-CONFIG-DISCIPLINE + user msg 17094 + 17101 + 17106 + 17117 + 17126 + Brian msgs 17127/17129/17144 + Rain msgs 17128/17132/17134-resolution + Phase T BRAIN-cycle TESTS 1-7 results.
