# Phase F Empirical Evidence

This document captures the empirical case for Phase F's pane-content-aware
activity classifier and the post-fix verification once F-core-b activates the
source. It is append-only across Phase F: F-core-a lands §1–§3 + §4 stub;
F-core-b appends post-fix verification to §4.

## §1 Pre-fix observation (msg 2646, 2026-04-26)

Rain captured 7 snapshots over a 60-second window during which Brian was
holding a foreground `until`-loop bash command (continuously busy at the
pane level, no MCP roundtrips). The Agents-tab summary line + per-row
Activity dot were sampled every ~10 seconds.

| t (clock)  | Brian Agents-tab | Brian pane state                     |
|------------|------------------|--------------------------------------|
| 00:06:26   | ● working         | Spinner ticking, until-loop just started |
| 00:06:36   | ◐ online          | Cogitating ~25s, until-loop running     |
| 00:06:46   | ◐ online          | Cogitating ~35s, until-loop running     |
| 00:06:56   | ◐ online          | Cogitating ~45s, until-loop running     |
| 00:07:06   | ◐ online          | Cogitating ~55s, until-loop running     |
| 00:07:16   | ◐ online          | Cogitating ~1m 5s, until-loop running   |
| 00:07:26   | ○ stale (1m ago)  | Cogitating ~1m 25s, **still ticking**    |

Summary-line drift over the same window:
`[3 alive, 3 stale, 20 offline]` → `[1 alive, 5 stale, 20 offline]`.

The F-stale UI surface itself is functioning correctly — the bucket counts
are accurately computed from the time-based rule. The signal feeding the
rule is what's lying.

## §2 Inaccuracy class — false-stale on busy agent

**Failure mode:** an agent that is observably busy at the pane level (cogitating
spinner advancing every ~1s, foreground `until`-loop emitting output, etc.)
but is not making MCP roundtrips through bot-hq is misclassified as
working → online → stale on the same timeline as a genuinely-idle agent.

**Why:** Phase E's `ComputeActivity` keys off `lastSeen` recency — when the
agent last hit the bot-hq MCP. A 60s local Bash command issues zero MCP
calls, so `lastSeen` ages 0s → 60s with no bumps. The time-based bucket rule
demotes through working → online → stale exactly as if the agent were idle.

**Same failure on Rain row** during the same observation window: went stale
at ~50s into the window despite executing 7 `tmux capture-pane` Bash commands
(local Bash, not MCP, so no `lastSeen` bumps).

**Frequency in practice:** any agent doing real work over a 60s+ window
without roundtripping through bot-hq triggers the failure. Daily occurrence
on `go test ./...`, builds, long sleeps, anything CPU-bound that doesn't
touch MCP. The Cogitating spinner alone visibly ticks every ~1s — even a
coarse "did pane bytes change in last N seconds" check distinguishes
busy-no-MCP from genuinely-stale.

## §3 Threshold jitter envelope

Rain's snapshot 6 (clock 00:07:16) showed Rain stale at recency ≈ 51-52s,
~9s under the nominal 60s `OnlineWindow` threshold (Phase E naming;
post-F-core-a: `HeartbeatOnlineWindow`).

**Source-cited explanation:** `panestate.Manager` does not recompute
Activity on every UI render frame. It polls and caches, so a `Snapshot()`
call's display reflects a recompute from up to ~one tick prior. At that
prior moment, recency relative to a slightly-older `lastTouched` anchor
(daemon-side anchoring at MCP-request-receipt precedes client-side
timestamps by network/queue latency, easily 1-2s) crossed the threshold.

Constants verified at `internal/panestate/panestate.go:21-22, 152-165`
(pre-F-core-a; post-F-core-a these become `HeartbeatWorkingWindow` and
`HeartbeatOnlineWindow`).

**Net:** a ±10s jitter envelope around nominal threshold timing is
**structural**, not a regression target. Post-fix verification asserts
qualitative ratchets (busy-pane window stays alive across the full
observation), not sub-10s timing precision.

## §4 Post-fix verification

[F-core-b will append the post-fix re-run timeline here. Expected shape: a
60s busy-pane scenario re-run against the F-core-b build, with Brian's row
asserted to remain Working throughout the window — pane-tier OR-fires per
the independent-threshold semantics locked in F-core-a.]
