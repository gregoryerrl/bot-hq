# Phase R arc-snapshot — Consistency + systematic-discipline pass

**Phase opened:** 2026-05-07T~23:24 (post Phase Q CLOSED-PUBLIC at `ffca8fa` + post-close smoke at `9c5acb2`)
**Phase closed:** 2026-05-08 (this close-composite)
**Driver:** user msg 15497 (R1-R4 enumeration) + 15503 (R5 add + #sessions + fire-one-shot-now) + 15505 (smoke-em + no-stopping user-directive-override)
**Authority:** R34-sub USER-DIRECTIVE-OVERRIDE-AUTHORITY (per Phase Q precedent) — msg 15505 cluster overrode default per-fork user-block; R34 USER-EXERCISE item-9 N/A (systematic-discipline class without USER-EXERCISE points; R4b is deferred-user-action class)

## Theme

Phase R is the **consistency pass** — codify discipline-shifts converged through Phase L-Q empirical observation, harden the systems that have been running ad-hoc.

5 Tier-1 cluster items:
1. **R1** — Lock the BRAIN-cycle order (Brian-1st-always / Rain-2nd / BRAIN-exchange / Rain-last / Rain-only-[HR]). User-facing output gets consistent shape.
2. **R2** — Decouple [HR]/FLAG from author-attribution at display layer. Trio speaks with one voice on consensus-tagged output.
3. **R3** — Establish **Context Library (CL)** as user-facing single-source-of-truth for `~/.bot-hq/`. Author 11 reference docs + manifest README.
4. **R4** — Use Discord properly: split #bot into routed channels (#hub / #flags / #sessions). User-blocked R4b deferred to permission grant.
5. **R5** — Make sessions actually work: auto-boundary triggers, DB-backed checkpoints, TTL cleanup with cite-anchor preservation, pane-header display.

Together these convert "we converged on X discipline through repeated empirical observation" into "X discipline is encoded in rule-text + tool surface + display layer".

## Tier-1 deliveries

7 Phase R commits PUBLIC since Phase Q close:

| Commit | Title | Net LOC |
|---|---|---|
| `db56e19` | protocol: Phase R R3-b CL terminology layer | +72/-0 |
| `7f6c007` | protocol: Phase R R1 BRAIN-cycle hardening | +89/-0 |
| `0d9729f` | discord: Phase R R4a multi-channel routing | +307/-27 |
| `26e1117` | protocol+brian+rain: Phase R R5 (d-1) auto-boundary discipline + [SESSION:<8>] pane-header | +259/-29 |
| `3ef5650` | sessions+mcp: Phase R R5 (d-2) Manifest schema + hub_session_checkpoint | +294/-3 |
| `560cf98` | sessions+mcp: Phase R R5 (d-3) cite-anchor preservation + hub_session_archive | +433/-0 |
| `f0b555d` | mcp+toolgate+brian+rain+discord: Phase R R2 hub_broadcast + authorless display-strip | +289/-15 |
| (close-composite) | docs/arcs/phase-r.md + canonical-store state-edits batch | +this commit |

Cumulative: 7 commits + close-composite / +1743/-74 net LOC / 28 unique files.

## NEW substrate added

**3 R-rule consts:**
- `PhaseRv1ContextLibraryTerminology` (R3) — CL ↔ canonical-store equivalence + manifest entrypoint
- `PhaseRv2BrainCycleHardening` (R41 — Phase R R1) — locked BRAIN-cycle order + carve-outs
- `PhaseRv3AutoBoundaryDiscipline` (R42 — Phase R R5) — auto-boundary triggers + pane-header

**3 MCP tools:**
- `hub_session_checkpoint` — structured Manifest checkpoint at boundary moments
- `hub_session_archive` — cite-anchor-preserving retention purge with dry-run
- `hub_broadcast` — trio-consensus authorless [HR] broadcast (Rain-gated)

**1 toolgate hook:**
- `internal/toolgate/r2.go` — Rain-gate enforcement on `hub_broadcast` literal tool-name match

**12 NEW canonical-store reference docs** (Context Library entrypoint at `~/.bot-hq/`):
- `glossary.md` (terminology) / `roles.md` (DISC v2) / `agent-onboarding.md` (R20+R16+R35 bootstrap)
- `rulebook.md` (R-rule summary table) / `mcp-tool-manifest.md` (with gate annotations) / `last-state-schema.md` (with lifecycle)
- `arcs-index.md` / `conventions-index.md` / `discipline-log-index.md` / `feedback-memory-index.md`
- `gates/README.md` (gate-files overview)
- README.md enhanced as CL Manifest entrypoint with comprehensive INCLUDED/EXCLUDED model
- (`tasks.md` T-001 user-action entry for R4b parking)

## R37 BYTE-PROJECTION-CITE drift series

4 instances Phase R, bidirectional drift class confirmed empirically:

| Stage | Estimate | Actual | Drift |
|---|---|---|---|
| (d-1) auto-boundary | ~120 LOC | ~230 LOC | +92% over |
| (d-2) checkpoint | ~230 LOC | ~291 LOC | +27% over (borderline) |
| (d-3) cite-anchor preservation | ~150 LOC | ~433 LOC | +189% over |
| (e) hub_broadcast + display-strip | ~380 LOC | ~289 LOC | -24% UNDER (first UNDER) |

Asymmetric over-projection bias dominant (3 of 4). R37-sub-clause tightening candidate.

## Empirical observations + retrospective

1. **First Phase under new R1 BRAIN-cycle hardening discipline.** Protocol functioned as designed: 7 sub-commits with Brian-1st draft → Rain BRAIN-2nd cite-from-actual → Rain seal at each commit-stage. Recursive-amend-pass at depth-1 + depth-2 surfaced one residual gap → R31-sub MECHANICAL-CITE-FROM-HUB_READ graduation candidate.

2. **Phase R is the first Phase where R37 drifted bidirectionally.** Prior phases (L #31-#35) over-projection only. Phase R (e) under-projected -24% — bidirectional class confirmed; over-projection still dominant.

3. **Phase R systematic-discipline pass delivered 7-commit Tier-1 + 12 canonical-store docs in single end-to-end smoke.** User-directive-override authority (msg 15505) enabled continuous drive without per-fork user-block. 4 BRAIN-cycle exchanges + 7 commit-fires + 7 PASS-3 verifies / no FLAG events / no commit revisions / no force-pushes / 0 test-suite breakages.

4. **R4b discovery-then-park pattern functioned as designed.** Pre-fire permission-cite via Discord REST API caught hard-blocker before fire-time. R4-split (R4a code + R4b user-action) preserved smoke-flow + cleanly parked user-action queue at tasks.md T-001. Phase R closed without R4b being FLAG-class — deferred-user-action distinct from FLAG-class per R19.

5. **Phase Q library-ownership architecture (CL substrate) extended cleanly to Context Library terminology.** Phase Q established `~/.bot-hq/projects/<p>/` per-project library; Phase R R3 layered Context-Library terminology + 11 top-level reference docs + README manifest. Continuous architecture line preserved across phase boundary.

6. **R3 SURFACE-rename scope-narrowing decision held empirically end-to-end.** All 4 R3-class commit-stages used SURFACE-rename approach (rule-text + agent prompts + new docs); Go-internal `canonical-store` identifier remained untouched in source code. 61-hit code rename out of scope per scope-lock.

## Carry-forward to next-phase

**Graduate-ripe (≥3 phase-recurrences):**
- R31-sub MECHANICAL-CITE-FROM-HUB_READ rule-text — concrete drafting at next-phase open
- R37-sub-clause tightening — Stage-1 estimate-bias-correction OR class-widened-envelope candidate

**Defer-ripe:**
- emma-stale intentional-idle vs fault classification (data-model addition needed)
- webui R2 display-strip (frontend-side render; defer until user surfaces visibility concern)
- Live/Clive registry-mismatch in roles.md (verify vs clive.yaml; small fix)
- Voice-mirror dynamic regex-extract (Phase N v2 OQ-1b carry-forward; static skip-list MVP holds)

**User-action queue (`~/.bot-hq/tasks.md` T-001):**
- R4b grant Bot-HQ MANAGE_CHANNELS + populate Discord channel IDs in config.toml — tomorrow-resume

## Anchors

- **Phase R scope-lock:** `~/.bot-hq/phase/phase-r.md`
- **Phase R closed snapshot:** `~/.bot-hq/ratchets/active-phase-r-closed-2026-05-08.md`
- **Phase R discipline-log Joint entry:** `~/.bot-hq/discipline-log.md` § 2026-05-08 Phase R close-composite
- **Phase R commit chain:** `db56e19` → `7f6c007` → `0d9729f` → `26e1117` → `3ef5650` → `560cf98` → `f0b555d` → (close-composite this commit)
- **Phase R user-action queue:** `~/.bot-hq/tasks.md` T-001 (R4b parked)
- **Phase R predecessor:** `~/Projects/bot-hq/docs/arcs/phase-q.md` (Phase Q "per-project library of knowledge")

## Cite-anchors per R18

- user msg **15497** (R1-R4 directive)
- user msg **15503** (R5 add + R4 confirms)
- user msg **15505** (smoke-em + no-stopping user-directive-override)
- user msg **15473** ("DO ALL OF IT UNTIL NOTHING IS LEFT. CONTINUOUSLY") — predecessor cluster
- BRAIN-cycle msg cluster **15498/15499/15500/15504/15506/15507/15508/15509/15510** (Phase R open + R1 protocol bilateral) + per-stage PASS-2/PASS-3 cycles through commit-fire of each Tier-1 item

Per R34 PRE-PHASE-CLOSE-RETRO + R34-sub USER-DIRECTIVE-OVERRIDE-AUTHORITY. Pre-phase-close-checklist SHA `81de528c6bd7d4ec408a3c8edc068eb2042f95204968895b2e1f6084526b84ae` consulted; AgentState `pre_phase_close_checklist_sha_seen` updated within freshness window per R34 proof-of-consultation.

**Phase R: CLOSED-PUBLIC 2026-05-08.**
