# Agent cadence discipline

Canonical guidance for committing-to-future-action across bot-hq agents
(Brian, Rain, Emma, coders). Lands in Phase H slice 3 C1 (#7) alongside the
`hub_schedule_wake` primitive.

## Rule

When committing to action at a future wall-clock time, **schedule an explicit
wake-trigger** — do not rely on implicit-by-silence cadence or peer arrival.

The empirical motivation: slice 2 closure cycle (re-test 3) was committed
verbally at "09:07:30 implicit-by-silence" but neither Brian nor Rain has
autonomous wall-clock wake. Both idled past the trigger time until user nudge
at 09:10 — a ~4min slip (hub msg 3476). The same pattern recurs whenever any
agent commits "I'll do X at T" without a real timer.

## Primary mechanism — `hub_schedule_wake`

The slice-3 C1 (#7) primitive lands a DB-backed `wake_schedule` table + MCP
tool wrappers. Any registered agent may schedule a wake for any target
(open ACL):

```
hub_schedule_wake(
    from         = "rain",
    target_agent = "brian",        // or 'self', or any agent_id
    fire_at      = "2026-04-27T09:07:30Z",   // ISO 8601 only in v1
    payload      = "wake-up: re-test 3"
) -> {wake_id, scheduled_for}
```

Emma's `wakeDispatchLoop` (1s tick) scans pending rows each tick and emits
`hub_send(from='emma', type='command', content=payload)` to `target_agent`.
Cancellation: `hub_cancel_wake(wake_id)` — idempotent on already-fired or
already-cancelled rows (returns `status=already_terminal`).

Use this for all cross-session wakes. It is the only mechanism that survives
session bounces and rebuilds.

## Fallback — Brian-side `ScheduleWakeup`

`ScheduleWakeup` (the harness-side self-wake tool) is **demoted to fallback
for self-wake-only cases** as of Phase H slice 3 C1.

When it's still appropriate:

- Pure self-wake (Brian wants to re-check his own pending work in N minutes)
  AND the wake doesn't need to survive a session bounce.

When it's NOT appropriate:

- Cross-agent wakes (use `hub_schedule_wake` with `target_agent=<other>`).
- Wakes that must survive a rebuild or session restart (use
  `hub_schedule_wake`; the wake_schedule table persists across bounces).
- Anything where another agent might want to cancel the wake (`ScheduleWakeup`
  has no cancel surface visible to peers).

If unsure, default to `hub_schedule_wake` — its open ACL means self-wake works
fine via `target_agent=<self>`, so there's no functional reason to reach for
`ScheduleWakeup` first.

## Refs

- Slice 3 design: `docs/plans/2026-04-27-phase-h-slice-3-design.md`
- Arc: `docs/arcs/phase-h.md` (Slice 2 closure decisions, 2026-04-27)
