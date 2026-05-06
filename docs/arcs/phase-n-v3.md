# Phase N v3 — Clive workspace

**Cycle:** 2026-05-06
**Tier-1 commits:** 4 + close-composite (this) = 5 total
**Status:** PUBLIC-COMPLETE pending close-composite final-push
**Authors:** Brian (HANDS), Rain (BRAIN-2nd PASS at every staged diff)
**Driver:** user msgs 8666 + 8699 + 8711 — single source of truth canonical-store + Clive workspace web UI + structured rules-system + id-sessions writer-flow runtime wiring (the bilateral-LOCKED (α) carry from Phase N v2 close)

---

## 1. Scope-lock recap

`~/.bot-hq/phase/phase-n.md§v3` (Stage 1 LANDED 2026-05-06; bilateral PASS-2-FINAL msg 8717; user RATIFY implicit via absolute-greenlight msg 8711 per Phase N v2 msg 8075 precedent).

Substrate-in-v3 / consumption-in-Phase-O split (per scope-lock §realistic-scope-decision):

- **v3 ships SUBSTRATE:** file format + canonical-store layout + git-backed audit + web UI read + web UI write + Clive draft-author + daemon-single-writer + raw-YAML rules editor + 3-layer visibility. User can configure rules + edit docs immediately post-rebuild.
- **Phase O ships CONSUMPTION:** rules-store query at task-start + agent prompt-build integration + `feedback_*.md` migration + Clive autonomous-broader scope + OQ-5/-6/-7 (retention, privacy/secrets-scan, cross-session indexed lookup) + N-T2-a/b/i + #41-#55 + recursion-terminator self-id-verify.

## 2. Phase N v3 commits (5 commits)

- `54c9db8` v3a — id-sessions writer-flow wiring + canonical-store substrate + arc-snapshot phase-n-v2.md §4+§6 pair-amendments. `hub_session_create` + `hub_register` call `sessions.WriteManifest` + `sessions.WriteIndex` idempotently when project supplied; `mergeAgentList` helper preserves agent roster across same-day re-registrations. Trio-local: `~/.bot-hq/rules/{general,projects,agents}/` skeleton + `README.md` documenting layout. 3 new integration tests. Closes the writer-flow gap surfaced in v2 close-composite arc-snapshot framing per discipline-log §2026-05-06T(post-v2-close-pre-v3-open) R31 OVER-CLAIM phase-close-arc-snapshot-class anchor. (+250/-5)
- `cb039b0` v3a.5 — combined rules-schema + HTTP API spec design-spike doc at `docs/plans/2026-05-06-phase-n-v3-rules-and-api-design-spike.md` (+312L). 6 sections: theme + rules-schema (general/projects/agents split + per-project>general resolution + Go-struct validation + parallel migration) + HTTP API surface (read + write + Clive flow + revert) + auth (loopback only) + conflict-handling (mtime-check + 409 + UX) + 3-layer visibility wiring + impl surface + 7 OQ Rain concurred.
- `72757c4` v3b — webui read MVP (10 files +1358/-1). `bot-hq webui` CLI subcommand on 127.0.0.1:3849 (env BOT_HQ_WEBUI_PORT override). Read endpoints: GET /api/files (tree with skip-list) + GET /api/files/{path} (content + mtime) + GET /api/rules (deep-merge per project + agent layered) + GET /api/sessions + GET /api/clive/activity. Frontend single-page app: project picker + doc tree + read-only viewer + rules tree + Clive activity panel. Path-escape 3-layer defense (parent-traversal segment rejection + Clean + Rel verification). 13 tests. Two scope-shifts to v3c: SQLite read-index (filesystem direct serves MVP) + htmx → vanilla JS (vanilla simpler for read-only).
- `26cb4d4` v3c — webui write surface + Clive integration + rules editor + 3-layer visibility (9 files +1297/-15). Write endpoints: POST /api/files/{path} (mtime-check + 409 conflict + atomic temp+Rename + per-canonical-dir git audit commit + hub_send notification) + POST /api/files/{path}/clive[/approve|/cancel] (3-action draft-author flow, 10-min TTL proposalStore) + POST /api/files/{path}/revert (git checkout + new revert commit). Per-canonical-dir `.git` lazy init at "bot-hq daemon" identity. Rules schema Go-struct validation (Errors-block + Warnings-allow + unknown-keys-warned-not-blocked). `internal/protocol/disc.go` gains `PhaseNv3CliveExpansion` const for DISC v2 Clive role. Frontend: textarea editor + dirty-indicator + Save button + 409-conflict modal (overwrite/discard/keep). 21 webui tests total.
- `<this>` v3 close-composite — discipline-log sweep + Tier-2 re-eval + baseline-vs-final event-count + ratchet-ledger Phase N v3 close section + this arc-snapshot + AgentState refresh + R34 reflexive-bootstrap 5th self-application (after L-3a + Phase M close-bundle + Phase N v1 close + Phase N v2 close).

## 3. Tier-2 re-eval (per Phase L/M precedent)

| ID | Origin | Disposition | Notes |
|----|--------|-------------|-------|
| OQ-5 retention policy + age-based pruning | v2 carry-forward | **defer Phase O** | productionize-class for sessions; not load-bearing for v3 substrate |
| OQ-6 privacy + secrets-scan-on-manifest-author | v2 carry-forward | **defer Phase O** | productionize-class; pairs with retention work |
| OQ-7 cross-session-search indexed lookup | v2 carry-forward | **defer Phase O** | extends N-1(b)-C index.md prep; pairs with v3c-deferred SQLite read-index |
| N-T2-a R15 self-flag carve-out extension | v2 carry-forward | **defer Phase O** | preflight-CRITICAL-self-detection class; not surfaced this cycle |
| N-T2-b Toolgate-class estimate-band refinement | v2 carry-forward | **defer Phase O — strengthened** | R37 sub-class candidate reinforced this cycle: web-frontend-LOC density distinct from Go-LOC (v3b 1358 actual vs 600-1000 band, v3c 1297 actual vs 700-1200 band; both within band on Go-only LOC). Phase O fixture-density modeling extension. |
| N-T2-i Audit-doc-presence-check | v2 carry-forward | **defer Phase O** | Phase L Tier-2 hold extension; not surfaced this cycle |
| #41-#55 v2 carry-forward queue | v2 carry-forward | **defer Phase O** | per v2 close discipline-log Joint entry; non-blocking for v3 substrate |
| Recursion-terminator self-id-verify | v3 cycle-open candidate | **defer Phase O** | tmux session-name vs BOT_HQ_AGENT_ID env match; mechanically terminates self-confabulation per discipline-log §2026-05-06T(post-v2-close-pre-v3-open) cycle-open root-cause |
| 4 v3c sub-deferrals (SSE / revert UI / CodeMirror / agent-side Clive consumption) | v3c judgment-calls | **defer Phase O** | substrate-vs-consumption split aligned; SSE + revert UI + CodeMirror are UX upgrades not load-bearing for MVP; agent-side Clive consumption was always Phase O per scope-lock split |

Net: zero Tier-2 graduations this cycle (all v2 + v3 candidates defer to Phase O productionize-class).

## 4. Baseline-vs-final event-count comparison

Per scope-lock §baseline-event-count-plan:

| Class | Baseline (v3 open) | Phase N v3 close | Delta | Notes |
|---|---|---|---|---|
| Git commits | 5 expected (v3a + v3a.5 + v3b + v3c + close-composite) | 5 actual (54c9db8 + cb039b0 + 72757c4 + 26cb4d4 + this) | 0 | Exact baseline match |
| State-edits pre-Stage-1 | 1 expected (2a discipline-log anchor) | 1 actual (LIVE) | 0 | |
| Stage-1 file-edit (scope-lock doc) | 1 expected | 1 actual (LANDED trio-local) | 0 | |
| Arc-snapshot amendments | 4 expected (§4 + §6 v2 + phase-n-v3.md authored at close) | 3 actual (§4 + §6 in v3a commit; phase-n-v3.md this commit) | -1 | v2 arc amendments folded into v3a; cleaner than separate commit |
| Rules.yaml skeletons | N expected | 5 actual (general + 4 agents + .gitkeep) | matched | Per-project files spawn lazily on first project-context use |
| Ratchet-ledger updates | ~5-10 live during cycle | ~6 actual (header pre-create + 4 commit row updates + close-row this) | matched | Drift-mitigation #4 cadence working |
| Discipline-log live-entries | TBD count | 1 (2a R31 anchor) + this close-sweep entry | matched | State-divergence-class observation: zero violations this cycle |
| Bilateral BRAIN-2nd-PASS handshakes | 10 expected (5×2) | 5 actual (one PASS per commit; cite-fix iteration on Stage 1 only) | -5 | Lower-than-expected because Rain leaned PASS-clean on most diffs |
| Final push-batch fire | 1 expected (5 commits) | TBD post-close-fire | n/a | Pending push-greenflag surface to user |
| Trio rebuild+restart | 1 expected | TBD post-push | n/a | Per user "1 rebuild+restart" directive |

Cumulative LOC: v3a 250 + v3a.5 312 + v3b 1358 + v3c 1297 = **3217 LOC** vs scope-estimate band 1820-3120 LOC (midpoint 2470 ±618 = 1852-3088). Actual 3217 exceeds upper-bound by ~4% — reinforces R37 sub-class web-frontend-LOC observation (v3b + v3c both web-UI commits inflate LOC vs Go-only band). Go-only LOC: 3217 - frontend (v3b 386 + v3c 320 = 706) = 2511 — within band on Go-only.

## 5. Retrospective

### What worked

- **Bilateral architecture lock pre-impl** — substantial up-front BRAIN-cycle on architecture (Q-rules-1-6 + Q1/Q3-Q7 + v3a/b/c sequencing + drift-mitigation 5-point) avoided mid-impl re-litigation. 8 unified questions surfaced + answered + locked → 5 commits drove without scope-fork churn.
- **Drift-mitigation #4 phase-doc + ratchet-ledger live-update working** — pre-create v3 ratchet section header + per-commit row update saved end-of-cycle compaction work. State-anchor cadence held end-to-end across 4 staged diffs + 5 commits.
- **Substrate-vs-consumption split** — keeping agent-side rules consumption + `feedback_*.md` migration in Phase O let v3 ship a focused substrate. User can edit rules + docs immediately post-rebuild without prompt-build coupling.
- **Single-write-path daemon-committer (msg 8693 LOCK)** — cleaner than dual-path option (a) Brian-proxy or (b) Clive direct-filesystem. Sidesteps Clive-as-bare-HANDS expansion + unifies audit/visibility/conflict-handling.
- **Path-escape 3-layer defense** — caught the filepath.Clean parent-traversal swallow class (TestResolveCanonicalPath_RejectsEscape FAILED first run); fix shipped same-cycle. Strong empirical for security-class testing-discipline.
- **R37 stage-2 cite-from-actual at every commit** — within band on Go-only LOC across all 5 commits; web-frontend-LOC drift identified as new R37 sub-class candidate (Phase O fold-in).

### What surfaced (carry-forward)

- **Web-frontend-LOC density distinct from Go-LOC** — v3b actual 1358 / 600-1000 band (+36%) and v3c actual 1297 / 700-1200 band (+8%) both exceed upper-bound. Go-only LOC stayed within band in both cases. Phase O candidate: refine R37 estimate-band per commit-class (Go-only / web-frontend / hook-class / impl-class / test-fixture-density) per N-T2-b strengthening.
- **filepath.Clean parent-traversal swallow class** — empirical security-class catch in v3b. Defense pattern: reject ".." segments BEFORE Clean, not after. Documented inline in resolveCanonicalPath. Generalizes: any path-resolution helper that uses filepath.Clean must validate segments before normalization.
- **DISC v2 Clive expansion as cohabitating const** — v3c added `PhaseNv3CliveExpansion` without modifying `DiscV2RoleAndPolicyShared`. Pattern: phase-cycle role expansions get their own const + agent-side prompt-build composes when ready (Phase O). Avoids prompt-rendering breakage from pre-impl rule-text shifts.

### Discipline-log live-entries (this cycle)

- **2026-05-06T(post-v2-close-pre-v3-open) joint** — R31 OVER-CLAIM phase-close-arc-snapshot-class sub-class anchor (LIVE pre-v3a; cite for v3a §4+§6 amendments). Distinct from prior R31 sub-classes (STAT-CLAIM-CITE / FILESYSTEM-SIGNAL-CITE / OVER-CLAIM-DISCIPLINE) — targets framing-class language in retroactive cycle-close documents.
- **Web-frontend-LOC R37 sub-class strengthening** — v3b + v3c empirical reinforces N-T2-b candidacy. Phase O graduate-or-defer.
- **Drift-mitigation #4 phase-doc + ratchet-ledger live-update** — empirical: per-commit state-anchor cadence held cleanly across 5 commits + 4 BRAIN-cycles. Generalizable pattern for multi-commit phase cycles.

## 6. Phase O scope (carry-forward from v3 close)

- **Agent-side rules consumption** — rules-query at task-start + prompt-build integration with general/projects/agents resolution. Pairs with `feedback_*.md` migration.
- **`feedback_*.md` migration to rules-store** — incremental per-feedback-file migration; agents read both during transition (parallel approach per Q-rules-2 LOCKED) with YAML preferring on conflict.
- **OQ-5 retention policy + age-based pruning** — productionize-class for ID-sessions cleanup.
- **OQ-6 privacy + secrets-scan-on-manifest-author** — productionize-class.
- **OQ-7 cross-session-search indexed lookup** — pairs with v3c-deferred SQLite read-index.
- **N-T2-a R15 self-flag carve-out extension** — preflight-CRITICAL-self-detection class.
- **N-T2-b Toolgate-class estimate-band refinement** — R37 sub-class extension; web-frontend-LOC strengthened candidate this cycle.
- **N-T2-i Audit-doc-presence-check** — Phase L Tier-2 hold extension.
- **#41-#55 v2 carry-forward queue** — non-blocking for v3 substrate; Phase O graduate-or-defer per maturity-criterion.
- **Recursion-terminator self-id-verify** — tmux session-name vs `BOT_HQ_AGENT_ID` env match; mechanical terminator for self-confabulation pattern (cycle-open root-cause anchor).
- **4 v3c sub-deferrals** — SSE live web feed of Clive activity (UX upgrade); one-click revert UI button (frontend wiring); CodeMirror raw-YAML editor (defer over textarea+monospace); agent-side Clive consumption of `PhaseNv3CliveExpansion` (always Phase O per substrate-vs-consumption split).

## 7. Cross-references

- **Phase N v3 scope-lock:** `~/.bot-hq/phase/phase-n.md§v3` (Stage 1 LANDED 2026-05-06)
- **Phase N v2 arc-snapshot:** `docs/arcs/phase-n-v2.md` (with v3a §4+§6 amendments)
- **Phase N v1 arc-snapshot:** `docs/arcs/phase-n-v1.md`
- **Phase M arc-snapshot:** `docs/arcs/phase-m.md` (R34 reflexive-bootstrap precedent)
- **v3a.5 design-spike:** `docs/plans/2026-05-06-phase-n-v3-rules-and-api-design-spike.md`
- **2a R31 OVER-CLAIM phase-close-arc-snapshot-class anchor:** `~/.bot-hq/discipline-log.md` §2026-05-06T(post-v2-close-pre-v3-open)
- **Phase N v3 R-rule consts:** `internal/protocol/disc.go` `PhaseNv3CliveExpansion`
- **Phase N v3 new package:** `internal/webui/`
- **CLI subcommand:** `bot-hq webui [--port N]` (cmd/bot-hq/main.go)
- **Trio-local artifacts:** `~/.bot-hq/rules/{general,projects,agents}/` skeleton + `~/.bot-hq/README.md` documenting canonical-store layout
- **Phase N v3 5-commit cluster:** 54c9db8 → cb039b0 → 72757c4 → 26cb4d4 → (close-composite this commit)
- **Bilateral architecture decisions:** msgs 8669 + 8675 + 8693 + 8703 + 8708 + 8709 + 8714 + 8717 + 8723 + 8731 + 8738 + 8741
- **User direction:** msgs 8666 + 8670 + 8682 + 8683 + 8689 + 8699 + 8702 + 8711
