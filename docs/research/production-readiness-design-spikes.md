# Production-Readiness Design-Spikes (Phase T-0.5)

**Captured:** 2026-05-09 (Phase T-0.5 production-readiness pre-flight)
**Purpose:** Design-spike for production-readiness items folded throughout Phase T sub-phases per phase-t.md v5; informs T-1 (foundational) + T-5 (cost-tracking) + T-6 (observability) implementation.

## Scope

Per phase-t.md v5 Production-readiness section, design-spikes for:
1. Cost-tracking + cost-budgeting
2. Rate-limiting + back-pressure
3. Error-recovery + fallback strategy
4. Observability (structured-logging + metrics + tracing)
5. Capability-parity validation pre/post-deployment

## 1. Cost-tracking + cost-budgeting (T-5 foundational; T-1 hooks)

### Problem

Brian-Claude (OAuth-MAX subscription) has flat-cost; Rain-DeepSeek-V4-Pro (API-key) has per-token-metered cost. Cross-model bilateral makes per-task cost visibility critical for:
- Budget-management (avoid runaway-spend on adversarial-bilateral-loops)
- Routing-decisions (provider-cost-comparison-aware per R53)
- Phase T retro metrics (T-7 cost-spend comparison vs v4 baseline)

### Design

**Per-task per-agent cost-tracking schema:**

```yaml
# ~/.bot-hq/projects/<project>/tasks/<task-id>/cost-tracking.yaml
task_id: <uuid>
opened_at: <iso8601>
closed_at: <iso8601>
total_cost_usd: <float>
cost_per_agent:
  brian:
    provider: anthropic
    model: claude-default
    cost_basis: oauth-max-subscription-flat-noted
    estimated_token_share:
      input: <int>
      output: <int>
    cost_usd_estimated: <float>  # estimated from token-share + claude-tier pricing
  rain:
    provider: deepseek
    model: deepseek-v4-pro
    cost_basis: api-key-metered
    actual_tokens:
      input: <int>
      output: <int>
    actual_cost_usd: <float>  # from DeepSeek API cost-reporting
cost_per_phase:
  investigate: <usd>
  plan: <usd>
  implement: <usd>
  verify: <usd>
budget_allocated_usd: <float>
budget_warn_threshold_pct: 80
budget_block_threshold_pct: 100
budget_breaches: [<list of trip-events>]
```

**Cost-tracking integration points:**
- `internal/agent/spawn.go` agent-spawn-routine emits cost-tracking-init event for new task
- Per-LLM-call (subprocess invocation): hook captures input/output token counts via subprocess-stdout parsing OR provider-API cost-endpoint poll
- Per-phase-transition: aggregate phase-cost from per-call deltas; persist to cost-tracking.yaml
- Per-task-close: aggregate task-cost; emit retro-event for Phase T metrics rollup

**Cost-budget circuit-breaker:**
- Default budget per-task: configurable via hub.db `budget_settings` table (per-task-class)
- Warn at 80%: emma alert + structured-log + dashboard-update
- Block at 100%: subprocess invocation FAILS; task-state=PAUSED-BUDGET; user-resume required to extend budget OR close-task

**Provider-API cost-reporting integration:**
- DeepSeek API cost endpoint (per DeepSeek docs; verify at T-1 implementation-time): poll per task or per N-calls
- Claude OAuth-MAX-subscription: NO per-call cost-reporting (flat-subscription); estimated-cost from token-counts + tier pricing
- Fallback: estimated-cost-from-token-counts when provider-API cost-reporting unavailable

**CLI + UI surfaces:**
- `bot-hq cost report --task=<id>` — per-task cost-spend report
- `bot-hq cost report --agent=<id> --since=<date>` — per-agent cost-spend report
- `bot-hq cost set-budget <task-id> <amount-usd>` — set/adjust per-task budget
- Web UI cost-dashboard at `internal/webui/cost/`

### Open questions for T-5 implementation

- Token-attribution accuracy for shared-context (e.g., system-prompt cached across calls)
- Multi-currency support (currently USD-only; deferred to Phase V)
- Long-term cost-history retention policy (currently 90 days; configurable)

## 2. Rate-limiting + back-pressure (T-1 sub-task; T-2 integration)

### Problem

DeepSeek API has per-key rate-limits (verify at T-1; typical providers ~60 req/min per-tier). Bot-hq Brian + Rain bilateral can produce request-bursts during Investigate-bilateral or Plan-bilateral phases. Need bot-hq-side throttling to respect rate-limits + propagate back-pressure.

### Design

**Per-key + per-agent rate-limit awareness:**

```yaml
# hub.db `rate_limit_configs` table (T-1 sub-task)
agent_id: <text>
provider: <text>
limit_type: requests_per_minute | tokens_per_minute | requests_per_day
limit_value: <int>
warn_threshold_pct: 80
block_threshold_pct: 95
backoff_strategy: exponential | linear | fixed
backoff_initial_ms: 1000
backoff_max_ms: 60000
```

**Bot-hq-side throttling:**
- Per-agent token-bucket rate-limiter (Go: `golang.org/x/time/rate`)
- Per-key shared rate-limiter (multiple agents sharing same provider-key)
- Throttle decisions: queue-and-defer (default) vs fail-fast (configurable per-agent)

**Back-pressure signal propagation:**
- IPIV state machine recognizes RATE-LIMITED phase-state
- Task auto-PAUSED on rate-limit-hit; resume on quota-recovery
- emma alerts on rate-limit-hit with per-agent breakdown
- Phase T metrics include rate-limit-hit-frequency

**Provider rate-limit auto-discovery:**
- T-1 implementation queries provider-API rate-limit-info endpoint at agent-spawn-time (where supported)
- Fallback to manually-configured limits via `bot-hq config set <agent-id> rate_limit ...`

### Open questions for T-1 implementation

- DeepSeek rate-limit specifics (per-key vs per-IP; per-minute vs per-day; verify at implementation)
- Anthropic Claude rate-limits for OAuth-MAX-subscription path (different from API-key tiers)
- Cross-agent rate-limit sharing semantics (multiple Rain instances sharing one DeepSeek key)

## 3. Error-recovery + fallback strategy (T-1 sub-task; R51 fallback-config)

### Problem

Subprocess invocation can fail (network / endpoint-down / model-rejected / capability-parity-regression). Bot-hq must recover gracefully without losing task-state.

### Design

**Failure-mode classification:**

| Failure | Detection | Recovery | Escalation |
|---|---|---|---|
| Subprocess-spawn-failure | exec.Cmd error | Retry 3× exponential-backoff | After 3 retries: fallback OR alert |
| Endpoint-unreachable | timeout / DNS-fail / connection-refused | Retry 3× exponential-backoff | After 3 retries: fallback OR alert |
| Model-name-rejected | provider-error response | No retry | Mark config-row=invalid; alert |
| Auth-failure | 401/403 response | No retry | Mark secret-rotation-required; alert |
| Capability-parity-regression | scheduled-test detection | Auto-fallback if R51-fallback-config | Alert + user-decision |
| Rate-limit-hit | 429 response | Backoff per rate-limit-config | Pause task; resume on quota-recovery |
| Cost-budget-exceeded | per-task tracking | Block subprocess invocation | Pause task; user-resume to extend budget |

**R51 fallback-config schema:**

```yaml
# hub.db `agent_fallback_configs` table (T-1 sub-task)
agent_id: rain
primary_config: <ref to agent_model_configs row>
fallback_config:
  - condition: endpoint-unreachable
    fallback_to: brian-claude-config-default  # use Brian's Claude config as Rain's fallback
    max_fallback_duration_minutes: 60
  - condition: capability-parity-regression
    fallback_to: brian-claude-config-default
    max_fallback_duration_minutes: 60
  - condition: auth-failure
    fallback_to: NONE  # auth-failure requires user-resolution; no auto-fallback
    alert_immediately: true
```

**Recovery procedure:**
- Subprocess-spawn-routine detects failure → classifies → retries OR fallbacks per R51-fallback-config
- All recovery-events structured-logged
- emma surfaces alerts at threshold (e.g., 3+ recovery-events in 10 minutes)
- IPIV state machine preserves task-state through recovery (no data-loss)

**Graceful-degradation:**
- DeepSeek-down → Rain-mode falls back to Claude (with capability-parity-noted-but-acceptable)
- All-providers-down → bot-hq enters degraded-mode with read-only operations + alert
- Hub.db corruption → restore from `~/.bot-hq/hub.db-wal` checkpoint

### Open questions for T-1 implementation

- Fallback-frequency tracking + ratchet (avoid runaway-fallback-loops)
- Fallback-to-different-model preserves R45 mode + prompt-template?
- Fallback-cost-implications (e.g., DeepSeek-cheaper than Claude → fallback-to-Claude raises cost; track in T-5)

## 4. Observability (T-1 + T-2 instrumentation; T-6 driver-integration)

### Problem

Multi-agent multi-model bilateral cycles need rich observability for debugging + monitoring + retro analysis.

### Design

**Structured-logging:**

All bot-hq daemon + agent-subprocess events emit structured JSON logs to:
- `~/.bot-hq/live.log` — current-day rotating
- `~/.bot-hq/debug.log` — verbose debug-level
- `~/.bot-hq/diag/<date>/<event-class>.jsonl` — per-class structured event-stream

Event schema (Go struct):

```go
type Event struct {
    Timestamp   time.Time              `json:"ts"`
    EventClass  string                 `json:"class"`     // agent-spawn / phase-transition / hook-fire / cost-track / etc.
    AgentID     string                 `json:"agent_id"`
    SessionID   string                 `json:"session_id"`
    TaskID      string                 `json:"task_id,omitempty"`
    Phase       string                 `json:"phase,omitempty"`
    Mode        string                 `json:"mode,omitempty"`
    Model       string                 `json:"model,omitempty"`
    CorrelationID string               `json:"correlation_id,omitempty"`  // for cross-agent bilateral tracing
    Severity    string                 `json:"severity"`  // debug / info / warn / error
    Message     string                 `json:"message"`
    Attrs       map[string]interface{} `json:"attrs,omitempty"`
}
```

**Metrics:**

Prometheus-style metrics exposed at `localhost:<bot-hq-port>/metrics`:
- `bot_hq_agent_spawn_seconds{agent_id}` — agent-spawn-time histogram
- `bot_hq_subprocess_success_rate{agent_id, model}` — success-rate gauge
- `bot_hq_per_rule_fire_rate{rule_id}` — R-rule fire-rate counter
- `bot_hq_cost_spend_per_agent_usd{agent_id, provider}` — cost-spend gauge
- `bot_hq_cross_model_divergence_rate{task_class}` — divergence-rate gauge
- `bot_hq_verify_pass_rate{verify_mode}` — Verify-pass-rate gauge
- `bot_hq_hub_db_query_duration_seconds{query_class}` — query-duration histogram
- `bot_hq_message_throughput_total{from_agent, to_agent}` — message counter

**Tracing:**

OpenTelemetry-style distributed tracing for cross-model bilateral cycles:
- Span: Brian-Claude Investigate-pre-hypothesis → Rain-DeepSeek Investigate-pre-hypothesis → bilateral-merge
- Correlation-ID: `task-<uuid>-cycle-<N>` linked across both agents
- Per-span attributes: agent_id, model, phase, sub_phase, sync-point

**Per-task observability dashboard:**

Web UI extension at `internal/webui/observability/`:
- Per-task IPIV cycle visualization (waterfall: I → P → Implement → V with sub-phase + mode breakdown)
- Per-task cost-spend timeline (per-agent stacked-area)
- Per-task R-rule-fire timeline (event-spike per-rule)
- Per-task bilateral-cross-model divergence-events (signal-vs-noise classification)
- Per-task Verify-pass/fail history with loop-back annotations

**Alerting thresholds:**

Configured via `~/.bot-hq/alerting.yaml`:
- Per-rule-fire-rate-anomaly (e.g., R31 fire-rate > 3 per hour)
- Cost-budget-circuit-breaker-trip (immediate)
- Capability-parity-regression (immediate)
- Secret-resolution-failure (immediate)
- Subprocess-spawn-failure-rate > 5% (warn)
- Endpoint-unreachable-rate > 10% (warn)

### Open questions for T-1/T-6 implementation

- OpenTelemetry collector deployment (local-only OR external-exporter)
- Metrics retention (current: in-process; consider Prometheus-exporter for long-term)
- Log-rotation strategy (current: daily; consider size-based rotation)

## 5. Capability-parity validation pre/post-deployment

### Pre-deployment (T-0.5 + T-1 done-criteria)

Per `docs/research/deepseek-capability-parity-report.md` TESTS 5.1-5.5 (DEFERRED to live T-1 deployment). Test scaffolding at `~/Projects/bot-hq/scripts/capability-parity/`.

T-1 done-criteria includes: all TESTS 5.x PASS or documented-mismatch-with-mitigation-plan.

### Post-deployment (scheduled)

- Daily capability-parity test run (cron-class scheduled)
- Per-config-change capability-parity test trigger
- Regression-detection-and-alert pipeline:
  - Run TESTS 5.x against current Rain-DeepSeek config
  - Compare results against baseline
  - Detect regression (e.g., MCP discovery now fails after working before)
  - Alert via emma + structured-log + dashboard
  - Auto-fallback to Claude if R51-fallback-config defined

**Implementation:**
- T-1 sub-task: scheduled capability-parity test runner at `~/Projects/bot-hq/cmd/bot-hq-capability-test/`
- Cron-config at `~/.bot-hq/cron/capability-parity-daily.yaml`
- Result storage at `~/.bot-hq/diag/capability-parity/<date>/<test-name>.json`

## Summary of design-spike outputs (T-0.5 deliverables)

| Item | Output | T-1 dep | T-5 dep | T-6 dep |
|---|---|---|---|---|
| Cost-tracking schema + CLI | This doc + ADR 0046 | Hooks at agent-spawn | T-5 implementation | — |
| Rate-limiting schema + behavior | This doc | T-1 implementation | — | — |
| Fallback-config schema | This doc + ADR 0049 (TBD) | T-1 implementation | — | — |
| Observability event-schema + metrics + tracing | This doc | T-1 instrumentation | — | T-6 driver-integration |
| Capability-parity validation pipeline | docs/research/deepseek-capability-parity-report.md + scripts/capability-parity/ | T-1 deployment | — | — |

---

**Cite-anchors:** phase-t.md v5 T-0.5 sub-phase + Production-readiness section + R51/R52/R53 NEW + R45 EXTENDED + R47 REVISED + Brian msgs 17127/17129/17144 + Rain msgs 17128/17132/17134-resolution.
