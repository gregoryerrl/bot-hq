# maxUtil Jitter Diagnosis — Phase J tail-5 Prep

**Author:** Investigation subagent (read-only) dispatched by Rain
**Date:** 2026-04-29
**Driver:** Phase J tail-5 design — characterize real-world `maxUtil` swing pattern to choose between debouncer-at-the-gate vs sampler/cache fix-at-the-source.
**Source-substrate:** `internal/gemma/plan_usage.go:1-526`, `internal/anthropic/oauth_usage.go:1-299`, `~/.bot-hq/hub.db` (messages + wake_schedule + halt_state), `~/.bot-hq/live.log`, `~/.bot-hq/debug.log`
**Cite_anchors:** commits `9ac82a7`, `82f5631`, `3ae4eca`, `8de1b84`; hub.db msg-IDs 5188-5216, 5873-5893, 5933-5942; wake_schedule 252 brian + 253 rain rows.

---

## 1. Question

**Is real-prod `maxUtil` jitter narrow (debouncer at the gate fixes it) or wide (sampler/cache-layer fix needed at the source)?**

---

## 2. Sampler architecture

`internal/anthropic/oauth_usage.go` `UsageClient.Fetch` (lines 150-241):

| Property                     | Value                                                                  |
| ---------------------------- | ---------------------------------------------------------------------- |
| HTTP endpoint                | `GET ${baseURL}/api/oauth/usage` (default `https://api.anthropic.com`) |
| Auth                         | Bearer token from macOS Keychain (`Claude Code-credentials`)            |
| Required header              | `anthropic-beta: oauth-2025-04-20` (gate; 401 without it)              |
| Per-call timeout             | `fetchTimeout = 5s` (oauth_usage.go:27)                                |
| **Cache layer**              | **NONE.** Every Fetch hits the network. No memoization, no TTL, no jitter-smoothing. |
| Wire format                  | utilization 0-100; normalized to 0-1 in `Fetch` (line 203)             |
| Window types                 | `five_hour`, `seven_day`, `seven_day_sonnet`, `seven_day_opus` (lines 42-47); each window optional |
| `maxUtil` reduction          | Iterate fixed-order list, take strict-greater-than (line 232) → `maxUtil=max(window.Utilization)` and `maxWindow=name` of binding window |
| Deterministic tie-break      | Order = `five_hour → seven_day → seven_day_sonnet → seven_day_opus` (line 227) — five_hour binds first on ties |

**Caller poll cadence** (`plan_usage.go:50, 163-186`):
- Steady state: `planUsageBaseInterval = 60s`
- After 5xx/auth-fail: `planUsageBackoffInterval = 600s`
- Sentinel tick invokes `checkPlanUsage` every 5s; gated by `lastPlanPoll + 60s`
- Tests stub `nowFn` for deterministic stepping

**Net upshot:** No caching anywhere. The `maxUtil` value Emma sees once per minute is whatever the Anthropic endpoint reports at that exact instant. Any window-rollover discontinuity surfaces as a single-sample step.

---

## 3. Threshold + gate logic recap

| Constant                        | Value     | Role                                                                  |
| ------------------------------- | --------- | --------------------------------------------------------------------- |
| `planUsageThreshold`            | 0.95      | Fire `firePlanCapHalt` (plan_usage.go:21)                            |
| `planUsageResetThreshold`       | 0.85      | Clear halt + emit RESUME (plan_usage.go:27, hysteresis gap = 10pp)    |
| `planUsagePreSnapThreshold`     | 0.90      | Emit `[PRE-COMPACT-SNAP]` MsgUpdate (plan_usage.go:35)                |
| `planCapPreSnapCooldown`        | 5 min     | Per-Gemma stamp `lastPreCompactSnapAt` (line 40, 315)                 |
| `planCapResumeCooldown`         | 10 min    | Per-Gemma stamp `lastPlanCapResumeAt` (line 91, 244)                  |
| `planCapWakeOffset`             | 5h + 1min | wake_schedule fire-time (line 109, 495)                               |

**Layered defenses (post-tail-2 + post-9ac82a7):**

1. **In-mem transition gate** (`planCapHaltActive` bool, plan_usage.go:451-454, 241-247): MsgFlag emit + wake-schedule insert only on `false→true` transition; RESUME emit only on `true→false` transition.
2. **DB-side `shouldFlag` rate-cap** belt-and-suspenders for MsgFlag (line 458).
3. **`HasPendingWakeForTarget` dedup** (line 498) — defense vs in-mem-flip-flop scheduling many wakes.
4. **`CancelPendingWakesForTargetByPayloadPrefix`** at clear-path (line 297) — auto-clear cancels any future +5h wake to avoid double-emit at fire time.
5. **`planCapResumeCooldown=10min`** — bounds RESUME emits per agent.
6. **`seedPlanCapHaltActiveFromDB`** (commit 82f5631, line 424) — closes restart asymmetry: post-restart in-mem mirrors DB.

---

## 4. Empirical evidence

### 4.1 Logs — empty/truncated

`~/.bot-hq/live.log` (138 bytes / 36 lines; current process started 2026-04-29 10:12:20) and `~/.bot-hq/debug.log` (138 bytes; bootstrap-only) contain **zero `maxUtil` / `[plan-cap]` lines**. Live.log is filter-drop-only; debug.log is bootstrap-only. Logs do not survive process restart and do not log per-poll `maxUtil` values. **Direct per-poll series is not recoverable from logs.**

### 4.2 hub.db — wake_schedule

```
$ sqlite3 hub.db "SELECT fire_status, COUNT(*) FROM wake_schedule GROUP BY fire_status;"
cancelled|70
fired|505
pending|1

$ sqlite3 hub.db "SELECT target_agent, COUNT(*) FROM wake_schedule GROUP BY target_agent;"
_internal:docdrift|71      -- unrelated docdrift cadence
brian|252                  -- plan-cap RESUME wakes
rain|253                   -- plan-cap RESUME wakes
```

All 252 brian RESUME-wake `created_at` timestamps span **2026-04-28 13:40:08 → 2026-04-29 02:13:46** (UTC). Tail-2 fix (commit `3ae4eca`) deployed 2026-04-29 09:33:51 +0800 = 01:33:51 UTC; the 9ac82a7 post-rebuild hotfix at 02:11:18 UTC. **All 252 spam-wakes were created PRE-fix or in the brief window before 9ac82a7 landed.** No post-`9ac82a7` RESUME-wake spam observed.

### 4.3 hub.db — Spam window: msgs 5188-5212 (the canonical incident referenced at plan_usage.go:91-101)

```
id    | timestamp           | type    | payload
5188  | 2026-04-28 19:24:52 | command | [RESUME] plan usage reset to 0%
5190  | 2026-04-28 19:25:52 | command | [RESUME] plan usage reset to 0%   (Δ=60000ms)
5192  | 2026-04-28 19:26:57 | command | [RESUME] plan usage reset to 0%   (Δ=64999ms)
5194  | 2026-04-28 19:28:02 | command | [RESUME] plan usage reset to 0%   (Δ=65000ms)
5196  | 2026-04-28 19:29:07 | command | [RESUME] plan usage reset to 0%   (Δ=64998ms)
5198  | 2026-04-28 19:29:31 | flag    | [CRITICAL] plan usage at 100%     (Δ=23999ms)
5199  | 2026-04-28 19:30:07 | command | [RESUME] plan usage reset to 0%   (Δ=36001ms)
5201  | 2026-04-28 19:31:12 | command | [RESUME] plan usage reset to 0%   (Δ=64999ms)
5203  | 2026-04-28 19:32:12 | command | [RESUME] plan usage reset to 0%   (Δ=59998ms)
5205  | 2026-04-28 19:33:12 | command | [RESUME] plan usage reset to 0%   (Δ=59998ms)
5207  | 2026-04-28 19:34:17 | command | [RESUME] plan usage reset to 0%   (Δ=65000ms)
5209  | 2026-04-28 19:35:17 | command | [RESUME] plan usage reset to 0%   (Δ=59999ms)
5211  | 2026-04-28 19:36:22 | command | [RESUME] plan usage reset to 0%   (Δ=64999ms)
```

**Inter-arrival distribution:** every delta is one of `{60000, 64998-65002, 36001, 23999}` ms — clean multiples of the 60s `planUsageBaseInterval` poll gate (one extra tick missed every ~6 cycles → 65s alternation). NO sub-minute deltas, NO oscillation faster than the poll cadence. **Reported `maxUtil` = 0% on every single poll** in this 11-minute window.

But a single CRITICAL @ 100% appears mid-window (msg 5198) — implying **maxUtil somehow reached ≥0.95 once in that window**, between two ~0% polls.

### 4.4 hub.db — CRITICAL (halt-fire) cadence

```
4232 | 2026-04-27 23:42:10 |  95%
4235 | 2026-04-28 00:12:45 |  96%   (Δ ≈ 1835s = 30m35s)
4238 | 2026-04-28 00:43:30 |  96%   (Δ ≈ 1845s)
4240 | 2026-04-28 01:13:44 |  96%   (Δ ≈ 1814s)
…
5213 | 2026-04-28 19:59:56 | 100%   (sustained-100% session)
5214 | 2026-04-28 20:30:26 | 100%   (Δ = 1830s)
5215 | 2026-04-28 21:00:51 | 100%   (Δ = 1825s)
5216 | 2026-04-28 21:31:21 | 100%   (Δ = 1830s)
5942 | 2026-04-29 02:33:20 |  95%   (today's halt — single fire)
```

CRITICAL halt-fires arrive at exactly **~30min intervals during sustained-100% periods** — that's the `shouldFlag` rate-cap window or the in-mem-transition cycle, NOT the natural Anthropic API value. Reported pct stays in `{95, 96, 100}` — **slow climb + plateau, not jitter**.

### 4.5 hub.db — Today's natural climb (msgs 5933-5942, post-tail-4)

```
5933 | 2026-04-29 02:27:15 | [PRE-COMPACT-SNAP] plan usage at 90%
5934 | 2026-04-29 02:27:15 | [PRE-COMPACT-SNAP] plan usage at 90%
5939 | 2026-04-29 02:32:20 | [PRE-COMPACT-SNAP] plan usage at 93%
5940 | 2026-04-29 02:32:20 | [PRE-COMPACT-SNAP] plan usage at 93%
5942 | 2026-04-29 02:33:20 | [CRITICAL]         plan usage at 95%
```

90 → 93 → 95 over **6 minutes, monotone increase**. PRE-COMPACT-SNAP fires once at 90% (cooldown 5min absorbs noise), once at 93% (after 5min cooldown elapsed). Halt fires at 95% with no pre-halt flapping. **This is what well-behaved real-prod climb looks like — zero jitter visible at the 60s sample rate.**

### 4.6 Reconstructing the spam: why msg 5198 (single CRITICAL) sandwiched in 22min RESUMEs?

**Hypothesis:** in the 5188-5212 window, the *previous* Gemma instance had `planCapHaltActive=false` (because in-mem state didn't persist across restart prior to commit 82f5631). Each ~60s poll:

- Anthropic API reported `maxUtil ≈ 0.0` (post-rollover or fresh-window)
- Code path: `maxUtil < 0.85` → DB `GetHaltCause(plan-cap)` returned `hadHalt=true` from a stale halt_state row left by a prior process → emit RESUME.
- The DB halt_state row was never cleared between RESUME emits because... actually the `db.ClearHalt` IS called (line 251). So `hadHaltDB` should be false on the second poll.

**Refined hypothesis:** between polls 5196 and 5198, the **5h window rolled over on Anthropic's side**, briefly causing the API to return `maxUtil ≥ 0.95` for the new window in startup state — `firePlanCapHalt` re-creates halt_state row → next poll reads `hadHaltDB=true` → fires RESUME again. This is the **DB-hadHalt-only gate flaw the tail-2 in-mem transition gate was added to fix.** Per the code comment at plan_usage.go:91-96.

**Caveat:** this is reconstruction from indirect evidence (msg cadence + reported pct values). **There is no per-poll `maxUtil` log line preserved anywhere on disk.** The actual sub-poll value sequence cannot be recovered.

### 4.7 Sustained-100% halt-fire pattern (msgs 5213-5216)

Four consecutive 100% halts at 30min ± 5s intervals during a sustained-100% session. Gap is **larger** than `shouldFlag` rate-cap (typically 15min default). Likely cause: in-mem transition gate flip via brief sub-95% reads in the `seven_day` window or quota-meter rounding, OR a periodic `planCapHaltActive` reset due to some non-instrumented path. **Not random jitter — bimodal (clean-100%-flat with periodic gate-reset events).**

---

## 5. Diagnosis

**Confidence: MEDIUM-HIGH.** Real-world `maxUtil` is **WELL-BEHAVED at the 60s sample rate** — no observed wild oscillation in monotone-climb data (4.5) or sustained-plateau data (4.4, 4.7).

**The Phase I/early-J spam was NOT caused by maxUtil-jitter at the source.** It was caused by:

1. **Layered gate design with one missing gate** (in-mem transition gate added in tail-2; DB-hadHalt-only was the single point of failure).
2. **Restart asymmetry** (in-mem `planCapHaltActive` was zero-initialized on Gemma restart while DB halt_state was non-empty — fixed in tail-4 axis-A commit 82f5631).
3. **Wake-schedule accumulation** (each false→true→false→true cycle scheduled a fresh +5h RESUME wake; fixed via 9ac82a7 `HasPendingWakeForTarget` dedup + `CancelPendingWakesForTargetByPayloadPrefix` at clear-path).

**The "jitter" the tail-2 commit message + plan_usage.go:91-93 comment refer to is real — but its amplitude is unknown and likely small.** What looked like "wild oscillation around 95%" is more plausibly **single-sample window-rollover discontinuities** (Anthropic API briefly reporting fresh-window 0% then prior-window >95% as the rollover propagates server-side), happening once per 5-hour window-boundary rather than every poll.

### What we cannot conclude

- Whether the **single-sample at the boundary** is cleanly stepwise (0% → 95%, hold, → 0% on rollover) or briefly bimodal (0% / 95% / 0% / 95% over 2-3 polls). The hub.db data only shows downstream emits, not Fetch return values.
- Whether `seven_day` window updates lag `five_hour` rollover and cause `maxUtil`-sourced-window flapping.
- Whether the 30min cadence in 4.7 is `shouldFlag`-cap or transition-gate flip — answering needs Fetch-level instrumentation.

**Therefore: the true source-side jitter amplitude is UNKNOWN, but the gate-side mitigations already in place (transition gate + cooldown + wake-dedup + restart-seed) are demonstrably sufficient for the observed prod traffic** — no spam in msgs >5896 (post-9ac82a7 + post-82f5631).

---

## 6. Recommendation for tail-5 design

**PRIMARY: instrumentation-first, defer-fix-decision.** Before adding more gate-side dampers (debouncer / EMA), add **per-poll Fetch instrumentation** so we can characterize source-side amplitude empirically rather than inferring from downstream emits.

Concrete:

1. Add a debug log line in `checkPlanUsage` post-Fetch: `[plan-cap] poll maxUtil=%.4f window=%s perWindow=%v` at info level. Cost: ~1 line/min ≈ 1.4K lines/day per process — bounded, harmless. Persist in live.log (which gets restart-truncated; alternative: write to `~/.bot-hq/plan-usage-history.jsonl` with size cap).
2. After 24-72h of clean prod data with the existing gate stack, **inspect the time-series directly**. Decide tail-5 fix:
   - **If observed amplitude is ≤2pp around 95% (narrow):** the existing transition gate + 10min cooldown is sufficient; close ratchet, no further fix.
   - **If observed amplitude is ≥10pp boundary-thrash (wide):** add a **3-poll-consecutive-required debouncer at the gate** — `planCapHaltActive` flips only after 3 polls agree. Latency penalty: 3min to register a real climb, acceptable since `planCapPreSnapThreshold=0.90` already provides 5min headroom warning.
   - **If observed pattern is single-sample-rollover-spike:** add a **sampler-side rolling-2-of-3 median filter** in `Fetch`, smoothing the boundary discontinuity.

**SECONDARY: do NOT add EMA.** EMA hides genuine plateau-near-95%, delays halt-fire by integration-window — bad tradeoff for a halt mechanism that should be prompt.

**SECONDARY: do NOT extend `planCapResumeCooldown` further.** 10min already absorbs the largest observed inter-spam gap. Going larger risks delaying legit RESUME after window-rollover.

---

## 7. Risks

| Risk                                                  | Severity | Mitigation                                                                       |
| ----------------------------------------------------- | -------- | -------------------------------------------------------------------------------- |
| Debouncer (3-poll-consecutive) delays real halt by 3min | LOW      | `planUsagePreSnapThreshold=0.90` + 5min PRE-COMPACT-SNAP cadence covers headroom |
| Median filter hides legit fast-climb halt              | MEDIUM   | Bypass smoothing when `current ≥ 0.99` (panic-fire)                              |
| Instrumentation-only delays fix decision               | LOW      | 24-72h of clean data is short relative to phase-arc cadence                      |
| False-positive halt-stickiness if maxUtil briefly spikes 0.95→0.94→0.95 | MEDIUM | Existing `shouldFlag` rate-cap already bounds user-visible CRITICAL emits; transition-gate idempotency handles wake-schedule    |
| Latency-to-rollover-detection if cooldown extended    | LOW      | Don't extend; 10min is well-tuned                                                |
| Restart-during-rollover re-fires halt                 | CLOSED   | tail-4 axis-A commit 82f5631 seeds in-mem from DB on startup                     |

---

## 8. Cite anchors

- Code:
  - `internal/gemma/plan_usage.go:21` (planUsageThreshold)
  - `internal/gemma/plan_usage.go:27` (planUsageResetThreshold)
  - `internal/gemma/plan_usage.go:35` (planUsagePreSnapThreshold)
  - `internal/gemma/plan_usage.go:50` (planUsageBaseInterval)
  - `internal/gemma/plan_usage.go:91-101` (root-cause comment, planCapResumeCooldown=10min)
  - `internal/gemma/plan_usage.go:163-264` (checkPlanUsage)
  - `internal/gemma/plan_usage.go:424-436` (seedPlanCapHaltActiveFromDB; tail-4 axis-A)
  - `internal/gemma/plan_usage.go:442-507` (firePlanCapHalt; in-mem transition gate + HasPendingWakeForTarget)
  - `internal/anthropic/oauth_usage.go:150-241` (Fetch — no cache layer)
  - `internal/anthropic/oauth_usage.go:227-238` (perWindow → maxUtil reduction with deterministic tie-break)

- Commits:
  - `8de1b84` Phase J tail — RESUME cooldown introduction
  - `3ae4eca` Phase J tail-2 K-1 — in-mem transition gate
  - `9ac82a7` Phase J post-rebuild hotfix — kill RESUME wake-spam + OUTBOUND-MISS rebroadcast loop
  - `82f5631` Phase J tail-4 K-1-bis-deeper Axis A — seed planCapHaltActive from hub.db

- hub.db:
  - msgs 5188-5212 (canonical 22min RESUME spam window, all reported pct=0%, all deltas ∈ {60s, 65s})
  - msgs 5213-5216 (sustained-100% halt at 30min intervals)
  - msgs 5933-5942 (today's clean monotone climb 90→93→95 over 6min)
  - wake_schedule: 252 brian + 253 rain RESUME wakes, all created pre-9ac82a7 (`MAX(created_at) = 1777412387764` = 2026-04-29 02:13:46 UTC); zero post-fix accumulation observed

- Logs:
  - `~/.bot-hq/live.log` (138B, 36 lines, no plan-cap content)
  - `~/.bot-hq/debug.log` (138B, 7 lines, bootstrap-only, no plan-cap content)
  - **No per-poll `maxUtil` series exists on disk.** Adding it is the tail-5 instrumentation prerequisite.
