# Arc: Phase G v1

Status: closed  | Branch: — (all slices merged to main@`209f6f0`)  | Opened: 2026-04-26  | Closed: 2026-04-26

## Context

Phase F closed silently with the F-core-c instrumentation cleanup (main@`521ac31`). Phase G v1 is the first slice of bot-hq UX + persistence improvements: visible UX wins (jump-to-present + agent-pane modal) plus a foundational persistence layer (SNAP-typed schema + arc/SNAP DB tables + arc.md narrative). Process conventions adopted alongside (greenflag scope stamping, restate-before-execute, user-surface check gate).

Brainstormed bilaterally Brian + Rain at hub msgs 2911–2922; saltegge greenflagged scope **β** (joint recommendation: A + B + #20) at msg 2923 and granted Rain greenflag-final until rebuild flag.

Design doc: `docs/plans/2026-04-26-phase-g-v1-design.md`.

## Decisions

- 2026-04-26 11:13 — F-core arc closed (Phase F end-state lock); main@`521ac31`, origin synced.
- 2026-04-26 ~11:50 — Phase G brainstorm opened; ideation across UX, DB, MD, emma, token efficiency, agent-collab.
- 2026-04-26 ~12:15 — Joint slate locked bilaterally (Brian + Rain); 19 candidates triaged into v1 / v2 / v3 / shelf / skip.
- 2026-04-26 ~12:30 — Convention v1 adopted effective hub msg 2922: greenflag scope stamping, restate-before-execute, user-surface check gate. Formal lock in DISC/CLAUDE.md deferred to v2.
- 2026-04-26 12:35 — Saltegge greenflag scope = β (Stages 1+2 + #20 + convention v1). Rain greenflag-final until rebuild flag.
- 2026-04-26 ~12:45 — Slice ordering locked: Slice 1 = A + #20 (UX wins, ships first), Slice 2 = B (persistence, ships post-rebuild). 2 rebuilds total.
- 2026-04-26 ~13:00 — Rain EYES gate on spec returned conditional greenflag with C1-C4 critical fixes (citation, SetSize gate, drop `-e`, add `-S -500`), P1-P3 soft pushbacks (commit reorder, arc.md skeleton at v1 start, stale-gen UX explicit), A1-A4 test gaps. All accepted; spec respun.
- 2026-04-26 ~13:05 — Slice 1 commit shape locked: A1 (jump-to-present + arc.md skeleton) → #20 (rebuild_gen) → A2 (agent-pane modal).
- 2026-04-26 — Slice 1 merged to main; rebuilds #1–#9 closed gate after iterative UX surface fixes.
- 2026-04-26 — Slice 1.5 (focus model rewrite + scrolled-up indicator + modal cleanup) merged at main@`9b17042`; rebuild #10 + #11 PASS, 5/5 eyeball-gate cleared incl. SQL belt.
- 2026-04-26 — Slice 2 design refined post-Rain pre-dispatch pushback (msg 3133): no `snap_entries` table, JSON column on messages instead, paren-depth as v1's sole escape mechanism, slice-2-only design doc. User greenflag at msg 3145.
- 2026-04-26 — Slice 2 shipping: C1 `cf2c4a2` (`internal/snap` package), C2 `0625d5c` (`messages.snap_json` migration), C3 `e29cae9` (send-path hook + 4 Rain obs fold), C4 (this update + design doc).

## Deferred

- **Phase G v1 slice 3** — normalized `snap_entries` rows / query layer over `messages.snap_json`; held pending workload trigger (Brian probe SQL pattern from msg 3174 recurs ≥3× as a recurring status-check). Until then, design doc §6 escape hatch holds.
- **F-core-c #2** — DI architecture cleanup (design call, awaits engagement).
- **F-core-c #3** — concurrent-spawn collision (Rain scan: no evidence, kept on shelf for completeness).
- **F-core-c #4** — zombie-coder lifecycle (design call: graceful unregister vs cron-prune).
- **F-core-d** — `internal/hub/db.go:53` open-mode finding (filed adjacent during F-core-c).
- **Test cleanup** — magic-3 → `len(visibleSet)` in `internal/ui/strip_test.go`.
- **Phase G v2 candidates:** Sessions→Arc tab UI consumer, ack flag (`ack: required|optional|none`), pre-spec adversarial dispatch codification, hub_flag rebuild-procedure variant, Discord pinned open-flags message, hub_read per-type budget, formal DISC/CLAUDE.md updates for Convention v1, magic-3 test cleanup.
- **Phase G v3 (gate carefully):** emma boot-summarization, message pre-classify, log anomaly tail.

## Refs

- design doc (master): `docs/plans/2026-04-26-phase-g-v1-design.md`
- design doc (slice 1.5): `docs/plans/2026-04-26-phase-g-v1-slice-1.5-design.md`
- design doc (slice 2): `docs/plans/2026-04-26-phase-g-v1-slice-2-design.md`
- prior phase: `docs/plans/phase-e.md`, `docs/plans/2026-04-23-bot-hq-hub-design.md`
- branch (slice 1): `brian/phase-g-v1-slice-1` (merged)
- branch (slice 1.5): `brian/phase-g-v1-slice-1.5` + `brian/phase-g-v1-slice-1.5-followup` (merged at `9b17042`)
- branch (slice 2): `brian/phase-g-v1-slice-2` (merged at `209f6f0`)
- commits (slice 1.5): C1 `ba4db0b`, C2 `ce01a0f`, C3 `ef63baf`, followup `9b17042`
- commits (slice 2): C1 `cf2c4a2`, C2 `0625d5c`, C3 `e29cae9`, C4 `209f6f0`
