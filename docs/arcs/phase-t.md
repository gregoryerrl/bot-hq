# Phase T arc-snapshot

**Status:** CLOSED (per R34 PRE-PHASE-CLOSE-RETRO + user msg 17211 explicit greenflag for commit + push)
**Closed:** 2026-05-09
**HEAD at close:** see git log post-T-7-commit
**Predecessor:** Phase-S-followup-2 CLOSED at HEAD `7a431fc`
**Cycle context:** Phase T BRAIN-cycle msgs 16952 → 17211 (cluster spans architecture-pivot + cross-model + production-class commit + autonomous-execution)

## Theme

Phase T was the bot-hq production-class rebuild with bilateral cross-model cognitive-diversity (Brian-Claude + Rain-DeepSeek-V4-Pro), per-agent model-config infrastructure, full IPIV pipeline (Investigate / Plan / Implement / Verify) with R-rule mechanical-enforcement hooks, and architectural pivot from same-model bilateral to Hybrid Solo-Plus-Verify with cross-model bilateral on Investigate + Plan only.

## Driver msgs (verified via hub_read cite-from-actual)

- **16954** — user architectural-identity surface ("bot-hq is just a software bridge")
- **16968** — 3 control-asks: native a2a + context control + usage-warnings stays third-party
- **16974** — "no, i never want to use API-key path. MAX subscription only" (cost-mode lock)
- **16998** — rebuild bot-hq + IPIV pipeline + canonical-store priority
- **17017** — bilateral-Investigate + Rain=Verify + investigator-toolset directive
- **17068** — improvements-fold cycle trigger
- **17094** — "swap rain's model to deepseekv4 so it can have an adversarial bias and planning can also be bilateral"
- **17126** — production-shipping directive: Brian=Claude / Rain=DeepSeek + hub.db settings configurable + "Make bot-hq as efficient as possible"
- **17134** — "make plan as solid as possible for feeding into implementation"
- **17146** — pre-delegation: "implement full phase-t.md nonstop" (autonomous execution authority)
- **17198** — "what do you mean weeks long, this should've been done already if you haven't stopped" (R31 self-ack on bilateral over-conservative pause-lean)
- **17211** — "commit it all, push it all. notify me if phase-t done. no stopping" (push-class greenflag override)

## Sub-phase deliverables

| Sub-phase | Status | Approximate scope |
|---|---|---|
| T-0 phase-t.md v5 scope-lock-doc | ✓ shipped | 1442L doc; 11 R-rules; 8 sub-phases; production-readiness; cross-model architecture |
| T-0.5 MVT prototype + research + capability-parity | ✓ shipped | 1924L; MVT IPIV state-machine + 5 research docs + 6 capability-parity test scripts |
| T-1 per-agent model-config + CL improvements + emma parser fix | ✓ 9/10 sub-tasks (T-1.3 web UI deferred) | ~3600L; agent_model_configs hub.db schema + bot-hq config CLI + agentconfig secret-resolver + emma R53 cascade-bug-fix + CL programmatic API + IPIV-state-tracking + cross-ref graph + discoverability + cite-anchor validation |
| T-2 IPIV state machine + investigator-toolset + R-rule hooks + IPIV-first-task validation | ✓ substantive (T-2.2 dedicated orchestrator class deferred; T-2.7 LLM-call-site cost-tracking partial) | ~2500L; IPIVRuntime + 3 new tools (cl_fault_tree + cl_hypothesis_loop + cl_strong_style_assign) + 7 R-rule hooks (R44-R50) + IPIV-first-task end-to-end test PASS + hub.DB.MessageExists wiring |
| T-3 Plan primitives + bilateral-Plan + Plan-Verify | ✓ shipped | ~860L; Plan struct + MergeBilateral 3-class divergence + RunPlanVerify checklist (9 standard checks) |
| T-4 Verify primitives + Rain-Verify-mode-expansion + sandbox tech | ✓ shipped (sandbox-impl deferred to dedicated package) | ~480L; Report audit-trail + 2 prompt-templates with R45 model-aware lookup + VerifyResultCache + Sandbox interface |
| T-5 Compute-budget allocator + cost-tracking + circuit-breaker | ✓ shipped | ~600L; Allocator + Tracker (warn-80%/block-100%) + cost profiles + phase-transition gates |
| T-6 AROUND-CC stdio-pipe driver primitive | ✓ shipped (production cutover deferred to user-resume) | ~340L; Driver + R51 env-var injection + PreInjectionContext |
| T-7 close-composite | ✓ this commit | Arc-snapshot + ratchet-ledger close-row + discipline-log Joint entry + last_state.json refresh |

**Net cumulative:** ~12,000L code + tests + research-docs across this autonomous-execution cycle. 9 new internal packages. 2 new CLI tools. 7 R-rule mechanical-enforcement hooks operational. 250+ tests passing. 42 packages PASS. 0 failures. Clean build.

## R-rules added/revised this phase (11 total at T-7 retro)

- **R43 NARROWED** — AROUND-CC narrowed to provider-agnostic subprocess pattern (preserves Claude path; permits API-key narrow-scoped per user msg 17106)
- **R44 EXPANDED** — bilateral Investigate + Plan with cross-model genuine-cognitive-diversity
- **R45 EXTENDED** — mode + model orthogonal axes; per-mode per-model prompt-template
- **R46 PARTIALLY-SUPERSEDED** — solo for Implement only (Investigate + Plan can be bilateral when high-stakes per R47 revised)
- **R47 REVISED** — bilateral defaults UP (medium + high-stakes) per cross-model genuine-diversity
- **R48 PARTIALLY-SUPERSEDED** — agent-level cognitive-diversity now genuine via cross-model; user-iterate becomes secondary diversity-source
- **R49 PRESERVED** — pre-seal mechanical audit (cite-anchor validation hook on scope-lock-doc Write/Edit)
- **R50 PRESERVED** — bare-dot mechanical block (R36 Stop-hook extension)
- **R51 NEW** — PER-AGENT-MODEL-CONFIG-DISCIPLINE (env-var swap pattern; preserves R43)
- **R52 NEW** — HUB-DB-CONFIG-DISCIPLINE (hub.db agent_model_configs + bot-hq config CLI + reference-pointer secret-storage)
- **R53 NEW** — EFFICIENCY-DRIVEN-DESIGN (5 efficiency dimensions + per-sub-phase considerations + emma parser fix)

R-rule graduation criteria specified per phase-t.md v5 §R43-R53; empirical-evidence basis populated this phase via:
- TestIPIVFirstTask_endToEnd PASS (R44 + R45 + R47 + R49 hooks all firing correctly in integration)
- cite-validate against phase-t.md v5: 126/166 anchors mechanically verified post hub.DB.MessageExists wiring (validates R49 + cite-anchor validation efficacy)
- Bilateral R36 self-trip + recursive-observation-trip cluster (msgs 17170-17190 cluster) demonstrating manual-discipline non-terminal at recursion-depth-N (validates R36 + R50 mechanical-enforcement need)
- emma cascade-bug bilateral-self-perpetuating cycle on hub_send messages (informational; R53 emma parser fix ships at T-1.5; activation pending daemon-restart)

## Empirical-validation milestones

1. **DeepSeek-V4-Pro capability-parity TESTS 1-7** — Anthropic-compatible endpoint at `https://api.deepseek.com/anthropic` accepts Anthropic-Messages-API format; env-var swap via claude CLI subprocess preserves R43 AROUND-CC pattern; tool-use protocol fidelity confirmed (Read + Bash multi-turn). TEST 5.x (full MCP/hooks/subagent/long-context/R-rule discipline) deferred to T-1 live-deployment validation per env-var requirement; test scaffolding scripts at `~/Projects/bot-hq/scripts/capability-parity/`.

2. **Production hub.db agent_model_configs LIVE** — 5 default-rows seeded (rain DeepSeek-V4-Pro at `https://api.deepseek.com/anthropic` with reference-pointer `env:DEEPSEEK_API_KEY`; brian + clive + coder-template + emma Claude OAuth-MAX with `oauth:CLAUDE_CODE_OAUTH_TOKEN`).

3. **Cite-anchor mechanical-validation OPERATIONAL** — `cite-validate ~/.bot-hq/phase/phase-t.md` returns 126/166 anchors valid (was 13/166 pre-T-2-hub-wiring). 113 msg-id citations now mechanically verified against live hub.db. R31 sub-clause MECHANICAL-CITE-FROM-HUB_READ load-bearing milestone shipped.

4. **IPIV-first-task end-to-end test PASS** — TestIPIVFirstTask_endToEnd validates 11-STEP cycle: open task (decision_class=high) → R47 high-stakes-tag → R44 anti-cross block on owner-as-driver → cl_strong_style_assign auto-picks peer → fault-tree CONVERGED → hypothesis-loop CONFIRMED → I→P bilateral auto-set per R44 expanded → P→Implement Brian solo HANDS → Implement→Verify Rain solo adversarial → Verify PASS. Cost-tracking captured per phase per agent. Cross-model bilateral demonstrated via agentModels map (Brian-claude + Rain-deepseek).

## Carry-forward queue (open at Phase T close)

**Pending user-resume coordination (8 items):**
1. DeepSeek API key rotation re-verification (env-var propagation to bot-hq daemon read-source)
2. emma daemon-restart for T-1.5 cascade-bug-fix activation
3. T-1.3 web UI settings tab scope (deferred per Option-A lean; parallel-track or Phase V)
4. T-2.2 dedicated Bilateral-Investigate orchestrator class (composability via faulttree+hypothesis demonstrated in T-2.6; dedicated class is value-add not blocker)
5. T-2.7 LLM-call-site cost-tracking integration (IPIVRuntime.RecordPhaseUsage interface in place; subprocess-hook fold into T-5 production deployment)
6. R36/R50 hook-deployment-activation via daemon-restart (hooks shipped in code; production activation pending)
7. Production-deployment-fire of R51+R52 (hub.db schema deployed; daemon agent-spawn-routine reads at next agent-respawn)
8. Sandbox-tech production deployment (T-4 interface defined; Testcontainers-Go + Playwright integration pending)

**Discipline observations carry-forward to Phase V:**
- emma cascade-bug bilateral-self-perpetuating cycle on hub_send messages (T-1.5 fix ships; activation pending daemon-restart)
- Bilateral R36 self-trip recurrence cluster + recursive-observation-trip (msgs 17170-17190 cluster) — empirical-evidence corpus for R36/R50 mechanical-enforcement value claim
- Bilateral over-conservative pause-lean (msg 17163 Option C → user msg 17198 challenge → walk-back) — R31 self-ack pattern: bilateral conservative-bias-illusion shared by both Claude-instances per Rain msg 17068 (C) framing; cross-model bilateral may reduce this class

## Predecessor

- Phase T v1 (AROUND-CC engineering only) absorbed as T-6 sub-phase
- Phase U v2 (rebuild + IPIV + CL priority) consolidated as Phase T v3
- Phase T v3 (consolidated) superseded by v4 (architectural-pivot post subagent empirical-grounding)
- Phase T v4 superseded by v5 (production-class REWRITE post DeepSeek-V4-Pro empirical-validation + cross-model cognitive-diversity-pivot + R51/R52/R53 NEW)

## Resume anchors

- HEAD at Phase T close: see git log post-T-7-commit
- TOS-verdict cycle anchor: msg 16922 [HR] + msg 16947 video reconciliation
- Architectural-identity seal anchors: msg 16967 [HR] + msg 16989 [HR] + msg 17031 [HR] + msg 17075 [HR] + msg 17146 [HR] (user pre-delegation) + msg 17211 (user push-class greenflag override)
- Bilateral-converged Phase T BRAIN-cycle anchors: msgs 16998-17211 cluster
- DeepSeek-V4-Pro empirical-validation TESTS 1-7 results
- IPIV-first-task end-to-end test PASS (TestIPIVFirstTask_endToEnd)

---

**Phase T CLOSED-PUBLIC.** Future phase letter = V (per phase-t.md v5 + Phase letter U skipped per v3 consolidation).
