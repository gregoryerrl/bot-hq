# Phase H — Slice 3 design (Coder lifecycle / RELIABILITY)

**Status:** design — pending Rain BRAIN-second + diff-gate
**Arc:** `docs/arcs/phase-h.md`
**Master design:** `docs/plans/2026-04-26-phase-h-design.md`
**Prior slice designs:** `docs/plans/2026-04-26-phase-h-slice-1-design.md`, `docs/plans/2026-04-26-phase-h-slice-2-design.md` (slice 2 implied via commit `23979e0`; doc path may differ)
**Branch (planned):** `brian/phase-h-slice-3` (cut from main `1e7da8d` after this design merges)

## Goal

Harden the coder/agent lifecycle so bot-hq can detect dead coders, prevent stale-base commits, prune zombie registrations, and provide reliable agentic time-triggers. Closes the in-slice-2-cycle observations on post-rebuild context loss + sentinel-replay hysteresis interactions, plus the originally-planned RELIABILITY items deferred from slice 2.

## Backlog (8 items, verified against `docs/arcs/phase-h.md`)

### Originally-planned (4 items, RELIABILITY theme)

| ID | Item | Status |
|---|---|---|
| **H-3a** | Coder heartbeat (frames F-core-c #4 zombie-coder lifecycle resolution) | shape pending design |
| **H-3b** | Worktree freshness gate (block coder commit if base SHA != origin/main HEAD) | shape pending design |
| **H-9** | Verify cadence ratchet (per-slice + per-diff-gate independent verify mandatory) | shape pending design |
| **H-25** | Emma roster hygiene (auto-prune stale registrations >24h or auto-flag live agent's `last_seen >5min`) | shape pending design |

### Backlog enrichments from slice-2 cycle (4 items, all with sealed primary shapes)

| # | Item | Sealed shape |
|---|---|---|
| **#2** | Post-rebuild context-bootstrap replay | Per-agent `relaunchWatermark = MAX(msg_id at register-time)`; `hub_register` returns `{agent_record, current_max_msg_id}` atomically — no race window between register-success and watermark-read |
| **#4** | Replay silent-mode | `replayThroughSentinel` runs in silent-mode — does NOT call `shouldFlag`, only checks ledger dedup. Decouples replay-path from dispatch-path so hydration side-effects (ledger write) are isolated from notification side-effects (hysteresis arm). Production correctness, not just test ergonomics. Fallback shape (a) — narrower test-mode-only env-var bypass — kept available if (b) scope-creeps. Per-message-id hysteresis (c) rejected. |
| **#6** | H-23 periodic invoker | Re-arms gold-class verification gate for H-23 docdrift surface when this lands; couples to #7's scheduling primitive |
| **#7** | Agentic time-trigger primitive (NEW) | Emma hosts a DB-backed `wake_schedule` table + new MCP tool `hub_schedule_wake(target_agent, fire_at_time, payload)`; Emma's existing tick loop checks pending entries each cycle and emits `hub_send` to `target_agent` with payload at `fire_at_time`, marking entry as fired. Reuses Emma's running clock (~80-150 LOC: schedule table migration + MCP tool wrapper + Emma loop). Cross-session shared-infra primitive — any agent can schedule a wake for any other agent (or self), unlike per-session `ScheduleWakeup`. |

## Triage proposal (open for joint BRAIN reconcile)

8 items is large for one slice (slice 1 = 5, slice 2 = 9). Two triage shapes worth weighing:

### Triage Option A — All 8 in Slice 3

**Pro:** backlog enrichments #4 + #6 are tightly coupled to slice-2's H-22/H-23 sentinel infrastructure landed in `4f5d75e`/`fc4913a` — landing them in slice 3 closes the sentinel-correctness loop cleanly. #2 (relaunchWatermark) + #7 (wake_schedule) are infrastructure-shared with the originally-planned items (state continuity + Emma extensions). Theme coherence: "agent reliability + scheduling primitives."

**Con:** large scope; ~2-3 weeks of implementation cycles at slice 1/2 cadence. Extended slice = harder to keep BRAIN-cycle context fresh through closure.

### Triage Option B — Split into Slice 3 (originally-planned) + Slice 3.5 or Slice 4 (backlog enrichments)

- Slice 3: H-3a, H-3b, H-9, H-25 (4 items, RELIABILITY theme strict)
- Slice 3.5 or absorbed into Slice 4: #2, #4, #6, #7 (sentinel-infra + scheduling primitives)

**Pro:** tighter slice scope; cleaner theme boundary (coder lifecycle hardening only); easier closure.

**Con:** delays #4 (replay silent-mode is production-correctness — every rebuild risks the hysteresis-replay pathology); delays #7 (scheduling primitive unblocks #6 + benefits all agent-side discipline). #4 already has a slipped-fire incident pattern from slice-2 cycle.

### Triage Option A LOCKED with budget-cap discipline (R1)

Reasoning:
1. Backlog enrichments are sealed-shape (small implementation cost per item)
2. #4 production-correctness urgency favors sooner-not-later
3. #7 + #6 coupling → land together for atomic infrastructure value
4. #2 + H-3a (coder state) share infrastructure surface (hub-state-determinism)
5. Theme coherence: all 8 items touch agent/coder lifecycle in some way

**R1 — Budget-cap discipline:** Lock Option A as *intent* but reserve right to defer C8/C9 (or even C5-C9 if cadence strains) to slice 3.5 if mid-cycle BRAIN-checkpoint shows strain. Don't pre-commit all 8 as must-land. **Concrete trigger:** at C4 completion (~half-way), joint BRAIN-checkpoint on cadence. If on slice-2 pace → continue all 8. If slower → cap remaining items + open slice 3.5 design.

## Suggested implementation order (assuming Triage Option A)

Load-bearing dependencies + complexity-first ordering:

| C# | Item | Why this order |
|---|---|---|
| **C1** | #7 (agentic time-trigger primitive) | Load-bearing for #6 (H-23 periodic invoker uses wake_schedule); benefits H-25 + H-3a + #2 too. ~80-150 LOC bounded scope. |
| **C2** | #4 (replay silent-mode) | Production-correctness; small Go change in `replayThroughSentinel`; independent of other items |
| **C3** | #2 (relaunchWatermark + atomic hub_register return) | Foundational for any coder/agent state-continuity work; H-3a depends on this for accurate "fresh vs replayed" event distinction |
| **C4** | H-3a (coder heartbeat) | Depends on #2 (state continuity) for correctness; defines zombie-detection signal |
| **C5** | H-25 (Emma roster hygiene) | Convenient via #7 wake_schedule for periodic prune (soft dep, NOT hard per O7 — H-25 could implement via Emma's existing tick loop directly if C5 lands before #7); uses H-3a's heartbeat signal for stale-detection |
| **C6** | H-3b (worktree freshness gate) | Depends on coder commit-flow visibility; per-coder per-action approval gate (H-16 territory but stricter for staleness) |
| **C7** | #6 (H-23 periodic invoker) | Depends on #7 wake_schedule; re-arms gold-gate for H-23 docdrift |
| **C8** | H-9 (verify cadence ratchet) | Process item; mostly doc + small enforcement; can land last as closure-class deliverable |
| **C9** | Slice closure (test plan + arc decision-log + slices table update + this design fold) | Per slice-1 / slice-2 precedent |

**Brian-lean estimate:** ~10-14 commits across ~6-8 BRAIN/diff-gate cycles, similar cadence to slice 2.

## First-item scope-lock proposal — C1 (#7 agentic time-trigger primitive)

Per Rain's framing, "first item scope-lock" is the natural next step after triage agreement. Sketching #7's scope:

### Architecture

**Schema migration** (new file `internal/hub/migrations/NNN_create_wake_schedule.sql` or per existing migration convention):

```sql
CREATE TABLE wake_schedule (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_agent TEXT NOT NULL,
    fire_at TIMESTAMP NOT NULL,
    payload TEXT,                  -- JSON-encoded message body
    created_by TEXT NOT NULL,      -- agent_id of scheduler
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    fired_at TIMESTAMP,            -- NULL until dispatched; set on Emma fire
    fire_status TEXT               -- 'pending' / 'fired' / 'failed'
);
CREATE INDEX idx_wake_schedule_pending_fire_at ON wake_schedule(fire_at) WHERE fire_status = 'pending';
```

**MCP tool** (new tool in MCP server):

```
hub_schedule_wake(
    target_agent: string,    // agent_id to wake (or 'self')
    fire_at: timestamp,      // ISO 8601 only in v1 (per O5 — relative-time syntactic sugar deferred to v2)
    payload: string          // raw string; structured wrap deferred to v2 (per architecture-question 3)
) -> {wake_id: int, scheduled_for: timestamp}
```

V1 also includes `hub_cancel_wake(wake_id)` per architecture-question 2 (~5 LOC; material safety for plan-changes).

**Emma tick-loop extension** (~20-50 LOC in Emma's existing loop):

- Each tick (existing cadence): query `SELECT * FROM wake_schedule WHERE fire_status='pending' AND fire_at <= NOW()`
- For each match: call `hub_send(from='emma', to=target_agent, type='command', content=payload)` then UPDATE row to fire_status='fired', fired_at=NOW()
- On hub_send failure: UPDATE to fire_status='failed' (retry policy out-of-scope for v1; left for later iteration)

### Acceptance criteria (proposed)

- Schema migration runs cleanly on fresh DB + idempotent on existing DB
- **Verify SQLite version supports partial indexes (≥3.8.0)** — if not, fall back to full index on `(fire_status, fire_at)` (per O4)
- `hub_schedule_wake` tool callable by any agent (brian/rain/emma/coder); returns wake_id + computed fire_at timestamp; rejects non-ISO-8601 fire_at input with clear error (per O5)
- `hub_cancel_wake(wake_id)` removes pending entry; idempotent on already-fired/already-cancelled
- Emma fires within `≤(tick_interval + 1s)` of `fire_at` for pending entries
- **`fire_status` state-machine invariants documented (per O6):** valid transitions are `pending → fired` (success), `pending → failed` (hub_send error), `pending → cancelled` (via `hub_cancel_wake`); no other transitions in v1. Future state additions ("retrying", etc.) require state-machine consistency review
- Fired entries do not re-fire (state machine correct)
- Failed entries surface via Emma's existing log path
- Integration test: `hub_schedule_wake` from Brian, fire_at = +5s, asserts target receives the message within 10s
- Brian-side `ScheduleWakeup` demoted to fallback for self-wake-only — **doc note added to `docs/agent-protocols.md` (or equivalent canonical Brian-side cadence guidance location) PLUS one-line addition to memory `feedback_skill_output_visibility.md`** (the `ScheduleWakeup` demotion is adjacent to skill-output-visibility discipline; per O8)

### Open questions for joint BRAIN

1. **Granularity of fire_at:** seconds-precision sufficient OR need ms? (Suggest seconds — Emma tick is presumably ~1-5s, sub-second precision is illusory)
2. **Cancellation:** v1 supports schedule + fire only, OR also `hub_cancel_wake(wake_id)`? (Suggest: cancellation in v1; ~5 LOC adds material safety for agents that change plans)
3. **Payload schema:** raw string OR structured (e.g., `{type: 'response', content: '...'}`)? (Suggest: raw string for v1; structured wraps in payload-shape complexity prematurely)
4. **Retry policy on fired-failed:** drop OR retry next tick OR exponential? (Suggest: drop in v1; failed-fire surfaces via Emma log → investigation → manual reschedule)
5. **Permissions:** any agent can schedule for any other (current sealed shape) OR per-target ACL? (Suggest: open in v1; bot-hq trust model is uniform across registered agents)

### Estimated implementation cost

~80-150 LOC total:
- ~15 LOC schema migration
- ~30 LOC MCP tool wrapper (input validation + DB insert + return shape)
- ~20-50 LOC Emma tick-loop extension
- ~30-50 LOC integration tests

Single PR; per-commit-per-component shape (commit 1: migration, commit 2: MCP tool, commit 3: Emma loop, commit 4: tests). 24h soak before rebuild + first cross-session wake test.

## Open questions for joint BRAIN (slice-level, beyond first-item)

- **Triage Option A vs B** — see triage section above
- **C1 (#7) vs alternative first-item** — Brian-lean is #7 for load-bearing reasons; Rain may have alternative (#4 production-correctness urgency could justify earlier landing)
- **Slice budget cap** — slice 1 was 5 items / ~1 week elapsed; slice 2 was 9 items / ~1 day elapsed (intensive); slice 3 with 8 items at what cadence? Lean cap discipline (Rain msg 3625 framing) applies if cycle stretches.
- **Test plan philosophy (R2 LOCKED)** — slice 2 surfaced the gold-vs-silver acceptance class taxonomy (P-1 doc). Slice 3 explicitly marks each surface's class:
  - **Gold (5):** #4 (replay silent-mode — correctness-class), #7 (cross-session primitive — load-bearing infrastructure), #2 (relaunchWatermark — state-correctness across rebuilds), H-3a (zombie detection — RELIABILITY core), H-3b (worktree freshness gate — blocks bad-base commits, correctness-class)
  - **Silver (3):** H-9 (process item; mostly doc + small enforcement), H-25 (uses Emma loop infrastructure), #6 (silver-post-#7-gold; inherits via "uses tested primitive")

## Cap-discipline note

Per Rain msg 3625 + Brian acked (lean-cap fallback if cycle stretches past ~45min): tonight's cycle = arc-open + backlog triage + first-item scope-lock (this doc). **Detailed per-item architecture for C2-C9 deferred to tomorrow morning's session.** Don't push slice-3 implementation tonight.

## Next-step proposal

1. Rain BRAIN-second on this design (triage Option A vs B + C1 #7 first-item lean + 5 open questions on #7 architecture)
2. Folds + revisions per Rain feedback
3. Cap tonight's cycle at design-draft-with-Rain-second (do NOT start C1 implementation tonight)
4. Tomorrow morning: arc decision-log entry + slice-3 design diff-gate + C1 implementation start

If Rain BRAIN-second surfaces blocking-factual-corrections (slice-2 design had 3), fold + iterate before the cap.

## Refs

- arc: `docs/arcs/phase-h.md`
- master design: `docs/plans/2026-04-26-phase-h-design.md`
- slice 1 design: `docs/plans/2026-04-26-phase-h-slice-1-design.md`
- slice 2 design: per arc, opened `23979e0`; doc path may need verification
- slice-2 closure decisions: arc lines 53-57 (gold/silver acceptance class taxonomy + implicit-by-silence cadence discipline)
- backlog enrichments source: arc lines 47-51
