# Arc: Phase H

Status: open  | Branch: brian/phase-h-arc-skeleton (skeleton in flight; slice branches forthcoming)  | Opened: 2026-04-26  | Closed: —

## Context

Phase G v1 closed at main@`91d88ac` (slices 1, 1.5, 2 + arc-fix). Phase H opens to harden bot-hq orchestration so Brian + Rain work efficiently together **without leaking AI infrastructure or destructive actions into real client projects** (bcc-ad-manager, 988.utah.gov, future). User msg 3236 framed the goal; bilateral brainstorm Brian + Rain converged across hub msgs 3237-3262 over multiple challenge cycles.

Two real-incident classes drove scope:
1. **bcc-ad-manager dispatch failures** (hub msgs 999, 1009, 1736 era) — coders died mid-work but reported online; one coder's worktree merge reverted 23 main-branch files.
2. **Cross-wire / token-waste** (hub msgs 3107, 3133, 3192, 3250, 3253, 3258 — 6+ in one session) — both agents drafting in parallel before either saw the other, ~3-4 wasted long messages per incident.

Plus an ambient risk: bot-hq's default `coder-<id>` worktree branch naming would violate bcc-ad-manager's `[issueNo]-[title-with-dashes]` convention; if pushed, would surface to user's senior as a flagged anomaly. Per-project rules need to live somewhere — and not in the client repo.

Design doc (master): `docs/plans/2026-04-26-phase-h-design.md`.

## Decisions

- 2026-04-26 ~17:30 — Phase H brainstorm opened (msg 3236); productivity hardening focus, no real-project blowback.
- 2026-04-26 — Brian opener (msg 3237) — 4 user themes mapped to H-1 through H-8 (8 candidate items), 4-slice ordering proposal.
- 2026-04-26 — Rain critique (msg 3238) — H-4 location flipped from `.bot-hq/project.yaml` (in client repo, AI-leak risk) to `~/.bot-hq/projects/<name>.yaml` (bot-hq-side, repo pristine). Slice ordering reshuffled — real-project safety = slice 1, not slice 2. H-2 enumeration tightened with audit tag. +H-9 to H-13 from EYES vantage.
- 2026-04-26 — Brian counter (msg 3239) — 6 concedes, 2 refinements, +H-14 (pre-dispatch rules gate, hard-block) + H-15 (session-close ledger).
- 2026-04-26 — Rain synthesis (msg 3242) — 13-item shape, +H-16 (coder allowlist) + H-17 (worktree audit, later dropped). User decisions named.
- 2026-04-26 — User picked all defaults (msg 3245) + observed hub_read polling staleness + invited Emma into scope.
- 2026-04-26 — Verify pass (Brian item 1 / Rain items 2-4, msgs 3250-3251). Item 1 SAFE (already-hardened in prior phase via `exec.Command` arg arrays + tmux `send-keys -l` literal mode + gemma allowlist gate); dropped from scope. Items 2-4 produced refinements: +H-18 (drop polling rule), +H-19 (bootstrap-iterate hub_read), +H-20→H-22 (queue-fail flag, now Emma sentinel), +H-21 (dispatch-pattern doctrine), H-3a relabeled as F-core-c #4 resolution. H-17 dropped (worktrees physically isolated by design); F-core-d dropped (unrelated finding).
- 2026-04-26 — Emma menu negotiation (msgs 3257-3260) — Brian's 3 + Rain's 4 collided on H-22/23/24 numbers; Brian's msg-3261 surfaced silent-drop, Rain re-synthesized as 7-item explicit menu (H-22 through H-28) renumbered to kill collision permanently.
- 2026-04-26 — User capability challenge on Emma reasoning (msg "did you consider Emma is inferior model"). Rain audited per-item (msg 3263); H-26 (restart-context summarizer) flagged as real interpretive miss-risk → deferred until H-22/23/25 tuning periods produce calibration data. Net Emma scope: 4 active + 3 deferred.
- 2026-04-26 — Halter/pusher rule converged after 3 iterations: **Rain halts on peer-arrival, Brian pushes through.** Reasoning: H-1 two-message ordering (action-first, synthesis-second) preserved; action-results easier to fold into delayed synthesis than vice versa; mutual-halt deadlock impossible by construction. BRAIN cycles exempt (DRAFT-alone retains for peer-critique).
- 2026-04-26 — User locked final scope. Emma pick: revised (4 active, H-26 deferred). Halter/pusher: confirmed Rain halts / Brian pushes. Implicit-accept on H-19, H-21, H-3a-relabel, H-22-replaces-H-20-mechanism, tuning-gate discipline, H-24 two-class boundary.
- 2026-04-26 — Slice 1 implementation complete (in-flight): C1 `edbcc09` (`internal/projects/` package, H-4 load-bearing), C2 `fc6064a` (hub_spawn pre-flight gate, H-14), C3 `2846146` (coder preamble extension, H-3c + H-16), C4 `fcd8cb6` (force-push token mechanism, H-13). All 4 Rain diff-gates PASS. C5 closure folds 7 micro-observations from C1/C2/C3 reviews + bcc-ad-manager.yaml + 988-utah-gov.yaml exemplars + slice 1 design framing fold + branch-advance-race v2 risk-matrix add. **Branch:** `brian/phase-h-slice-1`.
- 2026-04-26 — Slice 1 ff-merged to main as `b06961c` (C5 closure `b06961c`). All 5 Rain diff-gates PASS. Branch `brian/phase-h-slice-1` retained on origin for rollback safety.
- 2026-04-26 — Rebuild #13 fired post-merge; trio (brian/rain/emma) re-registered under slice 1 gates active. Test-first methodology adopted (per Rain pre-rebuild rec) — runtime test on first dispatch instead of slice 2 design.
- 2026-04-26 — Slice 1 runtime test surfaced exemplar form-coupling bug (rules SSH vs actual HTTPS for `bot-hq.yaml`). H-14 gate fired correctly with no side-effects (verified empirically via `claude_list` across both block paths — neither tmux session nor worktree created on block). Hotfix `4440425` (Path C) unifies all 4 exemplars to empty-placeholder + MUST-set convention; A1 fix to installed `~/.bot-hq/projects/bot-hq.yaml` (HTTPS form) lives outside repo. Path B (gate canonicalization SSH↔HTTPS) deferred to slice 2 candidate. Rain hotfix diff-gate PASS. Path 1 retry preamble verified GREEN (lenient bot-hq dispatch: BRANCH NAMING present, PUSH/FORCE-PUSH/BLOCKED suppressed — load-bearing conditional-emitter check). **Slice 1 H-14 gate 5/5 paths verified live.** Slice 1 CLOSED.

## Slices

| Slice | Theme | Items | Status |
|---|---|---|---|
| **Slice 1** | Real-project safety (BLAST) | H-4 + H-3c + H-13 + H-14 + H-16 | CLOSED — merged to main (`b06961c`) + hotfix `4440425` (Path C exemplar form-coupling fix); 5/5 H-14 gate paths runtime-verified live |
| **Slice 2** | Discipline + comm hardening (PROMPT-MOSTLY) | H-2 + H-1 + H-11 + H-18 + H-22 + H-23 + H-24 | design-pending |
| **Slice 3** | Coder lifecycle (RELIABILITY) | H-3b + H-3a + H-9 + H-25 | design-pending |
| **Slice 4** | State + discipline structures (RATCHET) | H-6 + H-10a + H-15 + H-19 + H-21 | design-pending |

## Item index

| ID | Item | Slice |
|---|---|---|
| H-1 | Brian-action / Rain-synthesis split + halter/pusher rule (Rain halts, Brian pushes) + halt-on-peer-arrival mechanic | 2 |
| H-2 | FLAG ownership → Rain (enumerated self-flag carve-out + audit tag) | 2 |
| H-3a | Coder heartbeat (frames F-core-c #4 zombie-coder lifecycle resolution) | 3 |
| H-3b | Worktree freshness gate (block coder commit if base SHA != origin/main HEAD) | 3 |
| H-3c | `push_requires_approval: true` default-on for non-bot-hq projects | 1 |
| H-4 | Per-project rules in `~/.bot-hq/projects/<name>.yaml`, keyed by `git remote get-url origin` | 1 |
| H-6 | Pre-commit hook: closed arc.md files (`Status: closed`) must be append-only diff | 4 |
| H-9 | Verify cadence ratchet (per-slice + per-diff-gate independent verify mandatory) | 3 |
| H-10a | Hub-side `last_processed_msg_id` per agent — silent replay drop | 4 |
| H-11 | Arc.md deferred-pointers must cite named consumer event, never count-based heuristic | 2 |
| H-13 | Force-push hard-blocked unless user replies with token `force-push-greenlight: <branch>@<sha>` (verbatim) | 1 |
| H-14 | `hub_spawn` hard-blocks dispatch into project with no rules file loaded (bootstrap or fail) | 1 |
| H-15 | Session-close SNAP ledger (best-effort restart-resilience) | 4 |
| H-16 | Coder tool allowlist for non-bot-hq projects (no git push / gh pr create / destructive ops without per-action approval) | 1 |
| H-18 | Drop hub_read polling rule from prompts; doc hub-push as actual mechanism | 2 |
| H-19 | Bootstrap-iterate hub_read (handle >50-msg backlog without silent truncation) | 4 |
| H-21 | `docs/conventions/dispatch-patterns.md` doctrine doc (arg arrays + tmux -l + no shell-string concat) + optional pre-commit lint | 4 |
| H-22 | Emma sentinel for queue-fail (replaces hub-side flag emit) | 2 |
| H-23 | Emma doc-drift sentinel (arc.md `Status: open` + branch/SHA reachable from main = drift) | 2 |
| H-24 | Emma analyze pre-screen with two-class boundary (structured → Emma; interpretive → Rain inline) | 2 |
| H-25 | Emma roster hygiene (auto-prune stale registrations >24h or auto-flag live agent's last_seen >5min) | 3 |

## Deferred

- **H-7** — multi-project hub-message namespacing (revisit when 2+ live projects with active dispatches actually coexist)
- **H-8** — full task ledger w/ `(task_id, owner, state, blocking_on)` (revisit when restart-mid-multi-step-contract becomes recurring)
- **H-10b** — system-reminder dedup at harness layer (revisit when judgment-call replay handling causes a real miss)
- **H-12** — coder identity persistence (stable ID per task, not per spawn) — revisit when same logical coder needs resume-after-death
- **H-26** — Emma restart-context summarizer (interpretive synthesis class, miss-risk on gemma4:e4b — defer until H-22/23/25 tuning periods produce Emma calibration data; revisit with named consumer trigger when calibration ≥95% on simpler sentinels)
- **H-27** — Emma session-close watchdog (best-effort² — Emma may also be down during unexpected rebuild; revisit if H-15 emit-rate drops below threshold)
- **H-28** — Emma cross-wire detector (observational only; revisit if halter/pusher rule needs tuning evidence)
- **F-core-c #2** — DI architecture cleanup (design call, awaits engagement; no Phase H overlap).
- **F-core-c #3** — concurrent-spawn collision (Rain scan: no evidence; loose adjacency to H-7).
- **F-core-d** — `internal/hub/db.go:53` open-mode finding (filed adjacent during F-core-c; no Phase H overlap).
- **Test cleanup** — magic-3 → `len(visibleSet)` in `internal/ui/strip_test.go`.

## Refs

- design doc (master): `docs/plans/2026-04-26-phase-h-design.md`
- design doc (slice 1): `docs/plans/2026-04-26-phase-h-slice-1-design.md`
- prior arc: `docs/arcs/phase-g-v1.md`
- brainstorm thread: hub msgs 3236-3266
- bcc-ad-manager incident evidence: hub msgs 999, 1009, 1736 (dispatch death + 23-file revert + branch convention)
