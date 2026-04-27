# Bootstrap-iterate hub_read (H-19)

**Status:** convention (Phase H slice 4 H-19)
**Origin:** Slice 2 verify pass (msgs 3250-3251) surfaced that `hub_read` caps at 50 messages per call; large backlogs (post-rebuild catch-up, returning-from-idle, multi-hour-gap) silently truncate.
**Codified in:** slice 4 design `docs/plans/2026-04-27-phase-h-slice-4-design.md` §H-19 (Shape B locked at slice-open BRAIN-cycle msg 3786)

## Rule

> **H-19 (process):** Any caller of `hub_read` that needs a complete view (bootstrap catch-up, post-idle resume, audit pass) must iterate with `since_id = last_msg.id` until an empty batch returns. A single `hub_read` call is **not** authoritative for backlog completeness — its 50-msg cap is a paging size, not a "you are caught up" signal.

## Why

Treating a single `hub_read` as complete works in steady-state (hubs typically have <50 unread per agent in normal operation), but fails at exactly the moments where catch-up matters most — post-rebuild, after a long idle, or when joining mid-incident with hours of accumulated traffic. The truncation is silent: caller sees 50 messages, has no signal that 200 more exist, builds a partial mental model, and acts on it.

The Phase H slice 3 #2 atomic register-return watermark gives every agent a `current_max_msg_id` at registration time. Iteration is the receiver-side discipline that pairs with that primitive — without iteration, the watermark only tells you what existed at register time, not what you've actually seen.

## Pattern

Caller-side iteration. Pseudocode:

```
batch = hub_read(agent_id=me, since_id=replay_cutoff_watermark)
while batch is non-empty:
    process(batch)
    last_id = batch[-1].id
    batch = hub_read(agent_id=me, since_id=last_id)
# empty batch = caught up
```

**Termination:** the empty batch is the "caught up" signal. There is no other signal; do not rely on batch-length-< 50 as a proxy (the hub may return exactly 50 by coincidence with more pending).

**Replay-cutoff composition:** during iteration, apply the per-session replay-cutoff filter (`msg.ID <= current_max_msg_id` returned at register time = silent-discard, per Phase H slice 3 #4). The iteration is mechanical; the replay-cutoff is semantic. Both run.

**Hard-cap policy:** caller-side. If a caller wants to bound work (e.g. "give up after 1000 messages"), the caller imposes the cap. The hub does not.

## Why caller-side (Shape B), not server-side (Shape A)

Considered at slice 4 BRAIN-cycle (msgs 3783/3786):

- Server-side `iterate: true` flag would conflict with the per-agent `current_max_msg_id` watermark semantics — it has no defined interaction with watermark advancement
- Caller-side iteration keeps watermark explicit and composable with replay-cutoff
- Caller-paced iteration is naturally policy-flexible (each agent picks its own hard-cap, processing model, batching)
- 0 LOC server-side vs 40-80 LOC for Shape A — meaningful at multi-cluster slice cap

## Refs

- `internal/brian/brian.go::initialPrompt` — STARTUP step 1 cites this convention
- Slice 3 #2 atomic register-return watermark (commit `06b75e9`) — the primitive this pattern composes with
- Slice 3 #4 replay silent-mode (commit `47eb514`) — the receiver-side filter this pattern runs alongside
