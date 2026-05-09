# Pre-Phase-T Performance Baseline

**Captured:** 2026-05-09 (Phase T-0.5 MVT pre-flight)
**Purpose:** Baseline current bot-hq performance for post-Phase-T comparison per R53 EFFICIENCY-DRIVEN-DESIGN.

## Methodology

Measurements taken on production bot-hq daemon with 7 active agents (brian/rain/emma/clive/discord/gemma/gemma-agent), 17,148 messages in hub.db, Phase T v5 BRAIN-cycle in flight. Statistical confidence: small sample (n=3-5) per metric; baseline establishes order-of-magnitude not statistical significance. Re-baseline post-T-1 with n>=30 for trend detection.

## Hub.db query latency (CLI-level via sqlite3 subprocess)

Baseline target: hub.db p99 latency for hot-path queries.

| Query class | Trials | Range | Notes |
|---|---|---|---|
| `SELECT COUNT(*) FROM messages WHERE id > 17000` | 5 | 17.4-20.1ms | includes sqlite3 process spawn (~15ms baseline) |
| `SELECT id, from_agent, to_agent, type, substr(content,1,50) FROM messages WHERE id > 17100 ORDER BY id LIMIT 50` (hub_read since-N pattern) | 3 | 17.4-18.1ms | similar profile |

**Native Go-level latency** (estimated; not directly measured): expected <5ms p99 for indexed queries (idx_messages_to + idx_messages_session + idx_messages_created); CLI-overhead dominates the 17-20ms measurement.

**Phase T-1 target:** <5ms p99 for hub.db hot-path queries (R52 + cite-anchor-validation cache + index-coverage tuning).

## Hub.db artifact size

| Metric | Value |
|---|---|
| hub.db file size | 18MB |
| hub.db-wal size | 4.1MB |
| hub.db-shm size | 32KB |
| Total messages | 17,148 |
| Active agents | 7 |
| Avg msg size (estimated) | ~1KB (18MB / 17K msgs) |

## Process baseline (current production)

| Process | Memory | CPU | Notes |
|---|---|---|---|
| `bot-hq` daemon (PID 58817) | 41MB | 1.5% | message routing + state |
| `claude` brian (PID 59153) | 526MB | 47.5% | active session |
| `claude` rain (PID 59717) | 98MB | 0.3% | idle session |
| `bot-hq mcp` (×2) | 4-14MB | 0% | MCP server processes |

**Brian's high CPU + memory** reflects active-session work (Phase T v5 REWRITE in flight); typical idle-session is <100MB / <1% CPU per Rain's profile.

## Agent spawn-time (proxy: claude --version subprocess spawn)

| Trial | Spawn time |
|---|---|
| 1 | 18ms |
| 2 | 16ms |
| 3 | 17ms |

Mean: ~17ms (subprocess fork + claude binary load + version print).

**Full agent-spawn-time** (claude subprocess + MCP-config-load + tool-discovery + initial-context-load): not directly measured; estimated ~3-5s based on observation. T-0.5 expanded MVT can capture concrete measurement.

**Phase T-1 target:** <3.5s p99 for full agent-spawn-time including R51 model-config-load + R52 secret-resolution overhead (target ≤500ms additional overhead vs current).

## TMux-keystroke driver baseline (Phase T-6 prerequisite)

Current driver: tmux send-keys to bot-hq-brian-* / bot-hq-rain-* sessions for inter-agent message delivery.

**Latency (estimated; not directly measured):** ~50-200ms typical per send-keys round-trip including pane-text capture + processing. Highly variable based on tmux pane state + claude readiness.

**Phase T-6 target:** ≥20% latency reduction vs tmux-keystroke baseline via stdio-pipe driver (R53 efficiency dimension).

## Hook-firing baseline

Current hooks (per ~/.bot-hq/.claude/ hook configs):
- R33 PreToolUse (gate-file-read enforcement)
- R36 Stop (outbound-discipline-mechanical)
- R40 (voice-mirror-discipline)
- outbound-miss-hook
- tool-permission-hook
- voice-mirror-log writes
- SessionStart hook

**Hook-firing reliability:** anecdotal high; no concrete metrics in current production. Phase T-6 prerequisite includes per-hook fire-rate metrics.

**Phase T-6 target:** ≥99% reliability across all hooks (with new hooks R44/R45/R47/R49/R50/R51/R52/R53 added).

## Token-spend baseline (Phase T BRAIN-cycle msgs 16998-17148)

**This BRAIN-cycle scope:**
- ~150 hub messages exchanged Brian + Rain + Emma + user (msg 16998 → 17148)
- Multiple subagent dispatches (msg 16873 LLM-call architecture / msg 16906 TOS posture / msg 17075 prior-session-history empirical-grounding)
- Multiple Read/Edit operations on phase-t.md v1 → v2 → v3 → v4 → v5 (1442L/119KB final)
- Multiple TESTS (1-7 DeepSeek capability-parity)

**Estimated token-spend:** order-of-magnitude 5-10M tokens this BRAIN-cycle (input + output combined; cross-agent). Concrete measurement requires per-agent cost-tracking infrastructure (T-5).

**Phase T-5 target:** per-task per-agent cost-tracking operational; per-BRAIN-cycle cost-attribution available; ≥30% token-spend reduction per BRAIN-cycle vs this baseline (R53 dimension).

## Bilateral sync-point overhead baseline

**Pre-bilateral-converge round-trips:** typical 3-5 hub_send ↔ hub_read cycles per substantive scope-decision. Each cycle ~2-30 minutes (LLM round-trip + agent processing time).

**Heartbeat-loop antipattern frequency (this session):** multiple instances pre-R50 deployment (e.g., msg 17141 stale-coder ping after Brian compact-cycle ~30m idle).

**Phase T-2 target:** R50 mechanical-block-on-bare-dot deployed; heartbeat-loop frequency <5% of turns.

**Phase T-2 target (R47 revised):** bilateral-cycle frequency 30-50% of cycles (raised from v4 <30% per cross-model genuine-diversity); cross-model-divergence-as-signal-not-noise rate ≥60%.

## emma cascade-mitigation overhead baseline

**Current pattern:** every hub_send emit >500 chars to bypass emma's auto-extract cap-rejection. Per this BRAIN-cycle observation: agents add cascade-mitigation note + intentional padding to avoid auto-promotion.

**Estimated overhead:** ~30-50% additional tokens per hub_send vs natural-length message (varies by message-class).

**Phase T-1.5 target:** R53 emma-parser-fix deployed; cascade-mitigation overhead structurally eliminated; natural message-length restored; estimated ~30-50% per-hub_send token-savings.

## Cite-drift baseline

**R31 STAT-CLAIM-CITE post-graduation instances:** 34+ per subagent prior-session-history report. Rain catch-rate ~40%. PhaseRv5MechanicalCiteFromHubRead graduated commit `26496e6` but instances continue increasing.

**Phase T-1 target:** R49 + cite-anchor-validation auto-fires at Write/Edit; R31 instance frequency reduces by ≥50% post-T-1 deployment.

## Verify-fail-loop baseline

**Current pattern:** ad-hoc; no formal Verify-mode. Loop-back triggered by user-feedback or peer-cross-check catch.

**Phase T-4 target:** Rain Verify-mode catches representative bug-class on test suite; max-3-retry circuit-breaker; structured loop-back via IPIV state machine.

## Cost-spend baseline

**Brian-Claude OAuth-MAX:** flat-cost subscription (no per-token metering at user-side; subscription billing).

**Rain-DeepSeek-V4-Pro:** API-key metered (per provider pricing); not currently in production (Phase T-1 deployment).

**Phase T-5 target:** per-task per-agent cost-spend tracking operational; queryable via `bot-hq cost report` CLI + web UI dashboard; cost-budget-per-task circuit-breaker with default 80% warn / 100% block.

## Production-readiness baseline gaps (Phase T-5 + T-1 targets)

| Capability | Current | Phase T target |
|---|---|---|
| Cost-tracking-per-agent | None | T-5 deployment |
| Rate-limiting + back-pressure | None | T-1 sub-task (per-key + per-agent quota-management) |
| Error-recovery + fallback | Ad-hoc | T-1 sub-task (R51 fallback-config) |
| Observability (structured-logging + metrics + tracing) | Partial (logs at ~/.bot-hq/live.log + debug.log) | T-1 + T-2 instrumentation |
| Capability-parity validation | TESTS 1-7 (partial) | T-0.5 full TEST 5 + scheduled post-deployment |
| Documentation + runbooks + ADRs | Partial (docs/arcs/ + docs/conventions/ + docs/decisions/ established) | Per-sub-phase deliverables |

## Recommended baseline-tracking continuation

**Per Phase T sub-phase done-criteria:** capture pre-vs-post measurement for each metric above. Compose into Phase T retrospective metrics rollup at T-7 close-composite (per phase-t.md v5 T-7 expanded scope).

**Tooling:** `bot-hq baseline measure` CLI command (proposed T-1 sub-task) for repeatable measurement across re-baseline events.

---

**Cite-anchors:** phase-t.md v5 T-0.5 sub-phase + R53 EFFICIENCY-DRIVEN-DESIGN + Phase T BRAIN-cycle msgs 16998-17148 + subagent prior-session-history report empirical-grounding.
