# Phase H — Slice 3 completion design (C2-C9)

**Status:** design — joint Brian + Rain BRAIN-locked (msgs 3692-3696)
**Arc:** `docs/arcs/phase-h.md`
**Predecessor design:** `docs/plans/2026-04-27-phase-h-slice-3-design.md` (covers triage A + C1 #7; C1 merged at `4582772`)
**Branch:** `brian/phase-h-slice-3-completion` cut from main `4582772`

## Goal

Complete slice 3 (RELIABILITY): close C2-C9 covering 7 remaining items + slice closure. Per joint BRAIN-lock, key decisions are locked upfront — no per-coder-inline-design under the "never time-pressured" greenflag user established at msg 3686.

## Pre-locks (from joint BRAIN cycle)

### #2 SUBSUMES H-10a (single mechanism, not two)

#2 (slice 3) and H-10a (slice 4) both target post-rebuild replay drop. Two implementations are redundant — one mechanism wins.

**Locked: implement #2's agent-side atomic register-return watermark.** H-10a's hub-side per-agent dedup cursor is NOT separately implemented. Rationale:
- Agent-side atomic snapshot at register-time is race-free by construction — watermark is "everything ≤ this ID happened before I came online; ignore"
- Hub-side dedup needs an "ack cursor" mechanism with risk of lost messages (hub thinks agent processed N, agent crashed mid-processing)
- Smaller surface — zero hub-protocol-change beyond register-return shape

Slice 4 drops H-10a from its item list. Slice 4 = 4 items (H-6, H-15, H-19, H-21), theme tightens to RATCHET-only.

### Cap-discipline DROPPED

Original slice 3 design had R1 budget-cap with mid-cycle BRAIN-checkpoint at C4 to defer remaining items if cadence strained. With user's "never time-pressured" + Phase H greenflag (msg 3686, msg 3679), cap-discipline is no longer load-bearing. Implement all 7 items + C9 closure properly. Per-commit Rain diff-gates as we go.

### C4 (H-3a) heartbeat = Shape γ hybrid gated on `tmux_target` meta

Three shapes were weighed: α (explicit `hub_heartbeat` MCP tool only), β (tmux-pane-activity only), γ (hybrid — implicit `last_seen` from MCP + tmux-pane-activity backup for long-bash windows).

**Locked Shape γ.** Reasoning:
- Coder mid-`Bash` (timeout 600s) makes zero MCP calls → `last_seen` stales without pane activity backup
- bot-hq is fundamentally a tmux orchestrator (coders run in panes by design); tmux-pane-activity coupling is architecturally consistent
- Tmux-pane-activity check gated on `agent.Meta.tmux_target` being set (per `hubRegister` in `mcp/tools.go:196-204`); agents without `tmux_target` (future webhook/voice agents) fall back to Shape α explicit heartbeat

Stale = both signals stale (last_seen > 5min AND no pane activity since last tick).

## C2 — #4 replay silent-mode (GOLD class, HANDS-direct)

**Architecture:** Modify `gemma.go` to add `OnHubMessageReplay` (silent variant) + change `replayThroughSentinel` to call it instead of `OnHubMessage`.

`OnHubMessageReplay` runs SentinelMatch + writes to dry-run ledger if pattern matches (correctness — dedup cross-bounce overlap), but skips `shouldFlag` calls (no hysteresis arming) and skips Discord/Rain emit paths (no spam from replay).

Behaviorally:
- Replay matches dry-run pattern → AppendToDryRunLedger (no shouldFlag, no Rain emit)
- Replay matches always-flag pattern → no-op (no Discord spam from replay)
- Replay matches non-dry-run observation → no-op (no Rain emit from replay)

`pollSentinel` (cross-process catch-up path) is NOT changed — it processes live post-watermark messages and should fire `shouldFlag` normally.

**Acceptance:**
- Boot-replay of N matching msgs does NOT arm `flagHistory["sentinel-obs:queueFailPattern"]`
- Boot-replay still writes ledger entries for dry-run patterns (dedup invariant preserved)
- Live trigger post-boot fires `shouldFlag` correctly (no false-suppress from replay)

**Tests:**
- `TestReplaySilentDoesNotArmHysteresis` — populate DB with matching msgs; call replay; assert flagHistory empty
- `TestReplaySilentWritesDryRunLedger` — populate DB with dry-run-pattern msgs; call replay; assert ledger entries present
- `TestReplayThenLiveDispatch` — replay first, then live OnHubMessage call on a new matching message; assert live path fires shouldFlag (not blocked by replay)

**Gold runtime test:** Force a rebuild with hub log containing the queue-fail pattern; verify subsequent live trigger fires within tick window (vs the 30min hysteresis suppress observed at slice-2 closure cycle msg 3463).

**Est: ~50-80 LOC.**

## C3 — #2 atomic register-return watermark (GOLD class, Coder)

**Architecture:**

Schema:
```sql
ALTER TABLE agents ADD COLUMN last_seen_msg_id INTEGER NOT NULL DEFAULT 0;
```
(Column name `last_seen_msg_id` distinct from `last_seen` timestamp; semantically "highest msg.ID this agent had visibility into at register-time".)

`hub_register` MCP tool (`internal/mcp/tools.go:162-227`) — atomic transaction:
1. Insert/replace agent row
2. Read `MAX(id)` from messages table
3. Update agent's `last_seen_msg_id = MAX(id)`
4. Return `{status: "registered", agent_id, current_max_msg_id: <int>}`

Agent-side filter: STARTUP prompt updated to instruct agents to silently discard incoming hub messages with `msg.ID <= current_max_msg_id` returned by their register call.

**Acceptance:**
- Fresh agent register returns `current_max_msg_id` matching `MAX(messages.id)` at register-time (race-free via SQLite transaction)
- Re-register on rebuild advances the watermark cleanly
- Agent receives watermark in tool response; STARTUP prompt instructs filter behavior

**Tests:**
- `TestHubRegisterReturnsCurrentMaxMsgID` — populate DB with N messages; register agent; assert returned watermark = N
- `TestHubRegisterAtomicWithMaxMsgID` — concurrent insert + register; assert watermark is consistent with what's visible at register-commit
- `TestHubRegisterRerunAdvancesWatermark` — register; insert msgs; register again; assert second watermark > first

**Gold runtime test:** Tonight's queue-replay live evidence (msgs 3660/3661 post-rebuild #17) is the test fixture. Post-implementation: agents see no replay-of-pre-rebuild messages.

**Est: ~60-100 LOC.**

## C4 — H-3a coder heartbeat (GOLD class, Coder, Shape γ)

**Architecture:**

Two surfaces:

**Surface 1 — implicit heartbeat via existing `UpdateAgentLastSeen`:** every MCP call already updates `last_seen` for the calling agent. Reuses existing infra; no new tool needed for the common case.

**Surface 2 — tmux-pane-activity backup signal (Emma-side):** Emma tick checks `tmux capture-pane -p -t <pane>` and compares against last-tick capture. Pane producing new output = alive. Used when `last_seen > 5min` but agent has `tmux_target` set; if pane is also silent, agent is genuinely stale.

**Stale detection logic (Emma-side):**
```
For each online agent:
  if (now - last_seen) > 5min:
    if agent.Meta.tmux_target is set:
      if pane has activity since last tick: continue (alive but quiet)
      else: flag stale (PM Rain once, with hysteresis)
    else:
      flag stale (Shape α fallback — no tmux backup signal)
```

**Acceptance:**
- Emma stale-detection threshold = 5min (configurable per future H-25 work)
- Stale agent triggers single PM to Rain (with hysteresis to prevent re-flag every tick)
- `tmux_target`-set agents protected from false-positive during long-bash windows

**Tests:**
- `TestStaleDetectionFiresOnSilentMcpAndPane` — agent's last_seen > 5min ago; pane silent since last tick; verify stale-flag fires
- `TestStaleDetectionSuppressedByPaneActivity` — agent's last_seen > 5min ago; pane has new output since last tick; verify NO stale-flag
- `TestStaleDetectionFallbackForNoTmuxTarget` — agent without tmux_target; last_seen > 5min; verify stale-flag fires (Shape α path)
- `TestStaleDetectionHysteresis` — same stale agent across multiple ticks; verify only one PM-to-Rain emitted

**Gold runtime test:** Spawn a real coder; SIGSTOP its tmux pane; verify Emma fires stale-alert within 5-6min window.

**Est: ~120-180 LOC.**

## C5 — H-25 Emma roster hygiene (SILVER+transitive-gold, HANDS-direct)

**Architecture:** Emma tick adds periodic prune query — `DELETE FROM agents WHERE last_seen < NOW() - 24h AND status = 'offline'`. Live agents protected by status check. Prune fires every Nth tick (e.g., once per hour, configurable) — not every tick.

Audit log: pruned IDs written to Emma's existing log path.

**Acceptance:**
- Stale-offline rows pruned on schedule (24h+ since last_seen)
- Live agents NOT pruned (status check)
- Audit log of pruned IDs

**Tests:**
- `TestPruneRemovesStaleOffline` — insert old offline rows; fire prune; verify removed
- `TestPruneSparesOnline` — insert recent online row; fire prune; verify retained
- `TestPruneSparesRecentOffline` — insert offline row with last_seen < 24h; fire prune; verify retained

**Silver justification:** Mechanical query + cron-like trigger; correctness verified via unit test + transitive-gold via shared tested `last_seen` path (covered by C4 gold).

**Est: ~60-100 LOC.**

## C6 — H-3b worktree freshness gate (GOLD class, Coder)

**Architecture:** Per-worktree `.git/hooks/pre-commit` installed at worktree creation time. Hook:
1. Runs `git fetch origin --quiet`
2. Checks `git merge-base --is-ancestor HEAD origin/main`
3. If ancestor check fails → hard-fail with clear error message guiding rebase

Worktree-creation wiring: `internal/projects/`-side worktree spawn extends to install the pre-commit hook script. Coder STARTUP prompt updated with awareness so coder doesn't get surprised.

**Acceptance:**
- Hook installed automatically at coder worktree creation
- Stale-base commit attempts hard-fail with clear rebase guidance
- Fresh-base commits proceed normally

**Tests:**
- `TestWorktreeHookInstallsOnSpawn` — spawn worktree; verify pre-commit hook present + executable
- `TestPreCommitHookFailsOnStaleBase` — simulate origin/main advance; attempt commit from stale worktree; verify hard-fail
- `TestPreCommitHookPassesOnFreshBase` — fresh worktree on origin/main HEAD; verify commit succeeds

**Gold runtime test:** Manually push to main from one process; attempt commit from coder worktree based on pre-push SHA; verify hard-fail with rebase guidance.

**Est: ~80-120 LOC.**

## C7 — #6 H-23 periodic invoker (SILVER+transitive-gold, HANDS-direct)

**Architecture:** Wire H-23 doc-drift sentinel scan into wake_schedule via internal-handler dispatch. Bot-hq startup schedules recurring wake (configurable interval, default 30min) targeting an internal handler that fires `runDocDriftSentinel()`. Re-arms wake on each fire (loop).

Internal-handler dispatch shape: `wake_schedule` rows with `target_agent='_internal:docdrift'`; Emma's `wakeDispatchLoop` checks for `_internal:` prefix and routes to internal handler instead of `hub_send`.

**Acceptance:**
- Doc-drift sentinel fires on schedule
- Each fire produces ledger entry when drift detected (per H-23)
- No drift detected → no ledger noise
- Re-arm robust against missed fires (handler always re-schedules next)

**Tests:**
- `TestPeriodicInvokerFiresOnSchedule` — schedule wake; advance clock; verify handler fired
- `TestPeriodicInvokerReArmsAfterFire` — verify next-fire scheduled after handler runs
- `TestInternalDispatchRoutingPrefix` — wake row with `_internal:` target_agent routes to handler not hub_send

**Silver justification:** Wires existing tested H-23 logic (slice 2) into existing tested wake_schedule (slice 3 C1); both primitives gold-verified independently. Transitive-gold confidence.

**Est: ~30-50 LOC.**

## C8 — H-9 verify cadence ratchet (SILVER, HANDS-direct)

**Architecture:** Doc-only with optional pre-commit hook. New convention doc `docs/conventions/verify-cadence.md`:
- Per-slice runtime test mandatory before closure (codifies slice-2 P-1)
- Per-diff-gate independent verify mandatory (codifies slice 2 R2 acceptance taxonomy)
- **Brian self-check before Rain diff-gate** for obvious-mechanical commits (saves Rain queue depth + token churn)
- Cross-references `docs/conventions/agent-cadence-discipline.md` (slice 3 C1) + slice 2 P-1 doc

**Acceptance:**
- Doc exists at `docs/conventions/verify-cadence.md`
- Cross-references all relevant precedents

**Silver justification:** Process/doc item; no runtime behavior to gold-verify.

**Est: ~40-60 LOC.**

## C9 — slice 3 closure

Per slice-1/2 precedent:
- Test plan execution (joint Brian+Rain runtime tests for all 4 GOLD surfaces + transitive-gold confirmation for SILVER)
- Arc decision-log entry covering C1 + C2-C8 + slice 3 closure
- Slices table update (Slice 3 → CLOSED)
- Memory updates if applicable
- Hand-off to slice 4 design intake

**Est: ~30 LOC (mostly arc edits).**

## Implementation order + dispatch shape

| C# | Item | Class | Est LOC | Dispatch |
|---|---|---|---|---|
| C2 | #4 replay silent-mode | GOLD | 50-80 | HANDS-direct |
| C3 | #2 atomic register-return watermark | GOLD | 60-100 | Coder |
| C4 | H-3a heartbeat (Shape γ) | GOLD | 120-180 | Coder |
| C5 | H-25 roster hygiene | SILVER+transitive | 60-100 | HANDS-direct |
| C6 | H-3b worktree freshness gate | GOLD | 80-120 | Coder |
| C7 | #6 periodic invoker | SILVER+transitive | 30-50 | HANDS-direct |
| C8 | H-9 verify cadence ratchet | SILVER | 40-60 | HANDS-direct |
| C9 | slice 3 closure | — | ~30 | HANDS-direct |

**Total est: ~470-720 LOC.** Sequential per-commit ordering for clean Rain diff-gate stream. Coder dispatches (C3, C4, C6) fire single-coder per item (no parallel; "never time-pressured" + token-efficient).

## Slice 4 implications

Slice 4 drops H-10a (subsumed by #2). Final slice 4 scope: **H-6** (closed-arc append-only pre-commit hook), **H-15** (session-close SNAP ledger), **H-19** (bootstrap-iterate hub_read >50-msg backlog), **H-21** (dispatch-patterns doctrine doc). Theme: "discipline structures (RATCHET)" — pure, no cross-cutting state.

## Refs

- Predecessor: `docs/plans/2026-04-27-phase-h-slice-3-design.md`
- Arc: `docs/arcs/phase-h.md`
- Slice 3 C1 merge: main `4582772`
- Joint BRAIN cycle: hub msgs 3692 (Rain draft), 3695 (Brian critique), 3696 (Rain fold)
- User greenflag: msg 3686 ("never time-pressured"), msg 3679 ("greenflag on all things for Phase H")
