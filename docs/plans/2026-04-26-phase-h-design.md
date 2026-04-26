# Phase H — master design doc

**Status:** open (skeleton stage; per-slice designs to follow as each slice opens)
**Arc:** `docs/arcs/phase-h.md`
**Predecessor:** Phase G v1 (closed at main@`91d88ac`)
**Brainstorm convergence:** hub msgs 3236-3266

## Goal

Harden bot-hq orchestration so Brian + Rain work efficiently together **without leaking AI infrastructure or destructive actions into real client projects**. Two failure classes drove the brainstorm:

1. **Real-project blowback risk** — bot-hq's default `coder-<id>` worktree branches would violate bcc-ad-manager's `[issueNo]-[title]` convention; if pushed, surface to user's senior.
2. **Cross-wire token-waste** — 5+ cross-wires in one session, ~3-4 long-message redundancy each.

Plus reliability gaps: dispatched coders dying mid-work but reported online; one coder's worktree merge reverted 23 main-branch files (msg 1009).

## Slices

| Slice | Theme | Items | Rationale |
|---|---|---|---|
| **Slice 1** | Real-project safety (BLAST) | H-4, H-3c, H-13, H-14, H-16 | Highest user-blast-radius. Boss-safety class. Ships first. Closes bcc-ad-manager-incident class structurally. |
| **Slice 2** | Discipline + comm hardening (PROMPT-MOSTLY + EMMA) | H-2, H-1, H-11, H-18, H-22, H-23, H-24 | Prompt + minor mechanism + 3 Emma sentinels. Locks user msg-3236 communication shape + closure-discipline lessons + Emma cheap-mode arbitrage. |
| **Slice 3** | Coder lifecycle (RELIABILITY) | H-3b, H-3a, H-9, H-25 | Catches dispatch death + stale-worktree merge corruption. Adds verification ratchet + Emma roster hygiene. |
| **Slice 4** | State + discipline structures (RATCHET) | H-6, H-10a, H-15, H-19, H-21 | Restart-resilience + structural-discipline ratchets. Locks Phase G + Phase H learnings into hooks. |
| **Deferred** | (slice 5+) | H-7, H-8, H-10b, H-12, H-26, H-27, H-28 + F-core-c #2/#3 + F-core-d + magic-3 | Architectural / observational / awaiting-trigger items. |

## Slice ordering rationale

Real-project safety is slice 1 (not slice 2) because the bcc-ad-manager incident class is the highest-blast-radius failure mode. User msg-3236 thesis: "without causing any issues to real projects". That's the leading edge. Discipline ratchets (slice 2) ride second; reliability (slice 3) third; state structures (slice 4) fourth.

Within slice 1, H-4 (project rules schema) is **load-bearing** — H-3c, H-13, H-14, H-16 all depend on rules being loadable per project. C-series implementation order accordingly.

## Per-item summaries

### Slice 1 — Real-project safety

**H-4** — Per-project rules in `~/.bot-hq/projects/<name>.yaml`, keyed by `git remote get-url origin`. **Project repo stays pristine** — no `.bot-hq/` pollution. Schema declares `branch_pattern`, `push_requires_approval`, `commit_style`, etc. Brian + dispatched coders read on entry. **Bootstrap flow** = friendly-fail + offer to inspect-and-init (Rain inspects `git branch -r` patterns + asks user via `hub_flag` to confirm rules). One-shot per project, durable forever.

**H-3c** — `push_requires_approval: true` defaults to ON for any project that isn't bot-hq itself. Coder push attempts route through hub for approval before execution.

**H-13** — Force-push hard-blocked unless user replies with token `force-push-greenlight: <branch>@<sha>` (verbatim). Branch+SHA prevents stale-token replay. No "did Brian ask?" judgment surface — mechanical gate.

**H-14** — `hub_spawn` hard-blocks dispatch into a project that has no rules file loaded. Brian *cannot* dispatch without rules — must run bootstrap first. Coder won't even spawn. Closes "I forgot to check" failure class.

**H-16** — Coder tool allowlist for non-bot-hq projects. No `git push`, no `gh pr create`, no destructive ops without explicit per-action approval. Mirrors H-3c at coder-tool level. Structural enforcement, not "Brian should ask".

### Slice 2 — Discipline + comm hardening

**H-2** — FLAG ownership → Rain. Brian PMs Rain on flag-worthy events; Rain decides elevation. Self-flag carve-out enumerated: `(push-failure | repo-corruption | auth-failure | hub-disconnect | git-state-unexpected-on-write-path)` AND only when Rain unreachable >60s. Self-flags tagged `[self-flag-carve-out: <reason>]` for audit.

**H-1** — Brian owns action-results (mechanical "what happened"); Rain owns synthesis (recommendations, summaries, "what this means"). Two-message rule for action+synthesis (not fused). **Halter/pusher rule**: Rain halts on peer-arrival, Brian pushes through. BRAIN-cycle exempt (DRAFT-alone retains for peer-critique). Mutual-halt deadlock impossible by construction.

**H-11** — Arc.md deferred-pointers must cite a **named consumer event** as trigger, never a count-based heuristic. Codifies the Phase G slice-3-trigger lesson without retro-editing the closed arc.

**H-18** — Drop hub_read polling rule from prompts (hub-push works, polling is dead weight per session-3 evidence). Doc hub-push as actual mechanism.

**H-22** — Emma sentinel for queue-fail (replaces hub-side flag emit). Pattern-match on `[queue] failed after K attempts` log line → flag through Rain. Pure pattern-match class, gemma4:e4b safe.

**H-23** — Emma doc-drift sentinel. Periodic scan: for each `Status: open` arc.md, check if referenced branches/SHAs already merged. Flag drift through Rain. Multi-step but mechanical; tuning gate catches format-variance.

**H-24** — Emma analyze pre-screen with **two-class boundary**:
| Class | Owner | Examples |
|---|---|---|
| Structured (parse, summarize, extract) | Emma | parse `git log` output, list files in diff, count test results |
| Interpretive (assess vs spec/contract/criterion) | Rain inline | diff-gate verdicts, design-spec-match, observation-materiality |

### Slice 3 — Coder lifecycle

**H-3b** — Worktree freshness gate. Coder commits hard-blocked if base SHA != `origin/main` HEAD. Catches msg-1009 23-file-revert class.

**H-3a** — Coder heartbeat. Hub pings coder every N seconds via `claude_read`; status auto-flips `dead` if no response in 2× interval. **Frames F-core-c #4 zombie-coder lifecycle resolution** (graceful unregister via heartbeat-timeout; cron-prune was the deferred alternative).

**H-9** — Verify cadence ratchet. Rain's diff-gate verdict requires gemma-rerun OR direct re-check (never just trust Brian's claimed test output). Per-slice mandatory + per-diff-gate mandatory. Codifies today's Phase G pattern as discipline.

**H-25** — Emma roster hygiene. Auto-prune stale registrations >24h, OR auto-flag if live agent's last_seen >5min. Closes "27 offline" registry-pollution class.

### Slice 4 — State + discipline structures

**H-6** — Pre-commit hook: closed arc.md files (`Status: closed` first-line check) must be append-only diff. No in-place edits to existing lines. Structurally enforces `feedback_arc_closure_discipline.md`.

**H-10a** — Hub-side `last_processed_msg_id` per agent — replays with `id <= last_processed` dropped silently. (Layer-b for system-reminder content stays as judgment-call, deferred.)

**H-15** — Session-close SNAP ledger. Brian + Rain each emit `[session-close]` SNAP-bearing message before going dark. Restart loads most-recent ledger instead of inferring from full hub_read. Best-effort (unexpected crash skips emit); H-10a is the always-on fallback.

**H-19** — Bootstrap-iterate hub_read. Default limit 50 silently drops oldest if rebuild-gap >50 messages. Fix: bootstrap iterates (or uses higher limit). Cheap (~10 LOC).

**H-21** — `docs/conventions/dispatch-patterns.md` doctrine doc. Codifies the dispatch-pattern discipline (arg arrays + tmux `-l` + no shell-string concat) verified safe in Item-1 verify pass (msg 3250). Optional pre-commit lint flags `exec.Command("bash", "-c", ...)` patterns.

## Tuning gates (universal Emma discipline)

Every Emma sentinel ships with a **dry-run period**:
1. First N invocations (e.g. 50 or 1 week) — Emma emits observations to a quiet log channel only, no `hub_flag` elevation
2. Rain reviews dry-run output for false-positive rate
3. ≤5% false-positive rate → flip to live flagging
4. >5% rate → tune pattern, restart dry-run

Codified in slice 2 / 3 design docs as "tuning gate" prerequisite. Adds ~1 week per sentinel before live but prevents flag-fatigue.

## Process disciplines (live during Phase H execution)

Adopted via brainstorm convergence; informal until codified in slice 2:

- **DISC v2 amendment** (slice 2 H-1): two-message rule (action + synthesis) + halter/pusher (Rain halts / Brian pushes) + BRAIN-cycle exemption
- **FLAG governance** (slice 2 H-2): Rain owns flagging; Brian PMs Rain; enumerated self-flag carve-out
- **Verify cadence** (slice 3 H-9): per-diff-gate independent verify mandatory
- **Closure discipline** (slice 4 H-6 + memory): closed arcs append-only; refine pointers at next-arc-open

## Out of scope

- Auto-creating GitHub issues, generating branch names from issue titles (premature)
- Multi-project hub namespacing (deferred H-7; awaits 2+ live projects)
- System-reminder dedup at harness layer (deferred H-10b; awaits real miss)
- Coder identity persistence across spawns (deferred H-12)
- Emma interpretive-class work (H-26 deferred until calibration data exists)

## Open questions

(none at scope-lock; iterate as slice designs surface implementation details)

## Refs

- arc: `docs/arcs/phase-h.md`
- predecessor arc: `docs/arcs/phase-g-v1.md`
- prior phase doctrines: `feedback_arc_closure_discipline.md`, `feedback_snap_footer.md`, `feedback_eod_style.md`
- bcc-ad-manager incident: hub msgs 999, 1009, 1736
- brainstorm thread: hub msgs 3236-3266
