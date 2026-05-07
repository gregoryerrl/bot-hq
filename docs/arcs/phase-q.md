# Phase Q — per-project library of knowledge (single source of truth)

Cycle: 2026-05-07 single-session (continuation post Phase P drain close-composite + 3 hotfix commits 1f3a92c/7faf2ee/0d6b6a0). Phase Q close-composite originally landed Q-8a 7d3740c at user lightweight-pause checkpoint pre-BCC-pivot; amended at Q-8b close-composite to add Q-9 IPv6 hotfix + Phase R carry-forwards from BCC pivot session.

## §1 Scope-lock recap

Opened mid-session at `~/.bot-hq/phase/phase-q.md` v1 (Brian author msg 15214; Rain BRAIN-2nd PASS-1 msg 15215 + Rain PASS-2 with-residual msg 15218 + Rain amend-verify-clean msg 15223). 8 Tier-1 items per phase-q.md cluster table; user pre-delegated PROCEED-NOW absolute-greenlight covers LAYER-1 + LAYER-2 end-to-end.

Driver: user msg 15171 ("i don't see the tasks.md on global documents") + 15182 (bcc-ad-manager fetch error + "all our documents are on our own storage") + 15183 ("clive can't edit files, how can i plan with him then") + 15189 ((b) MIGRATION pick) + 15193 ("bot-hq will have its own library of knowledge on registered projects. SINGLE SOURCE OF TRUTH") + 15199 ("PROCEED NOW" + "agent-accessible AND webui-accessible").

## §2 Tier-1 commit ledger

| # | sha | title | numstat | Rain BRAIN-2nd |
|---|-----|-------|---------|----------------|
| pre-Q hotfix | `1f3a92c` | webui ambient focus context for Clive | shipped pre-halt | retro-PASS msg 15176 |
| pre-Q hotfix | `7faf2ee` | drop Clive's robotic-operator persona | shipped pre-halt | retro-PASS msg 15176 |
| pre-Q hotfix | `0d6b6a0` | webui re-inject Clive's focus context on mid-session change | shipped pre-halt | retro-PASS msg 15176 (peer-greenflag-msg-id self-cite drift logged) |
| Q-1 | `e34c54a` | webui surface tasks.md in Documents + URL-encode fetch paths | +23/-11 | PASS msg 15207 |
| Q-2 | `333ad39` | webui give Clive read_file + propose_file_edit tools | +258/0 | PASS msg 15211 |
| Q-3 | state-edit | Phase Q scope-lock doc author at `~/.bot-hq/phase/phase-q.md` | NON-REPO | PASS-2 msg 15218 (amend cycle on cite-trail) |
| Q-4 | state-edit | bcc-ad-manager 3-file migration to canonical-store + side-rename | NON-REPO (4 mv ops) | PASS msg 15222 |
| Q-5 | state-edit | per-project library scaffold (5 new subdirs × 3 projects + 6 .md files + 988 dir created) | NON-REPO | PASS msg 15227 |
| Q-6 | `38d6a83` | webui Phase Q library destinations + dual-root external-file surface | +482/-50 | PASS msg 15231 |
| Q-7 | state-edit | agent-rule update — README + 3 yaml library blocks + Phase Q section | NON-REPO | PASS msg 15235 |
| Q-8a | `7d3740c` | Phase Q arc-snapshot author at this file | initial 76 lines | at-stage PASS |
| Q-9 | `5d66f65` | webui dual loopback bind (127.0.0.1 + ::1) — IPv6 NetworkError hotfix on user-reported recurrence post-BCC-pivot | +35/-16 | at-stage PASS msg 15348 (b)-fire-now CONCUR re-affirmed at fire) |
| Q-8b | (this) | Phase Q close-composite (arc-snapshot amend + ratchet-ledger close-row + active-phase-q-closed snapshot + discipline-log Joint entry) | composite | at-stage-call |

Cumulative repo-side: 5 Q git commits (Q-1/Q-2/Q-6/Q-8a/Q-9) + 1 close-composite = 6 commits. ~+798/-77 net LOC. State-edits across Q-3/Q-4/Q-5/Q-7 are canonical-store-only (no bot-hq repo touch).

## §3 Phase Q state-edits (non-repo)

- `~/.bot-hq/phase/phase-q.md` scope-lock v1 author + 4 amend passes for cite-trail accuracy (R31-sub recursive-amend pattern).
- `~/.bot-hq/projects/bcc-ad-manager/{eod,clips}/*.md` — 3 migrated files + 1 side-rename for date consistency (Q-4).
- `~/.bot-hq/projects/{988,bcc-ad-manager,bot-hq}/{architecture,decisions,conventions,glossary,audit-notes}/` — 15 new dirs + 6 README/INDEX templates (Q-5).
- `~/.bot-hq/projects/988/` dir created from scratch (was missing despite registered yaml).
- `~/.bot-hq/README.md` — directory map updated to Phase Q schema; canonical-store-class section extended; NEW Phase Q section explaining dual-window model + agent path conventions (Q-7).
- `~/.bot-hq/projects/{bcc-ad-manager,bot-hq,988}.yaml` — library: blocks added (root + schema + external_docs_root) (Q-7).

## §4 New endpoints / packages / surfaces

**HTTP endpoints (1):**
- `GET /api/external-file/{project}/{relpath}` — read-only mirror of `~/Projects/<project>/docs/`; registered-project allowlist + filepath.Rel containment guard against traversal (Q-6).

**Webui per-project destinations: 4 → 11.** Old `Etc` catch-all retired. New: Architecture / Decisions / Conventions / Glossary / Audit notes / EOD / Clips (all canonical-store) + Project docs (dual-root external).

**Clive tool surface: 4 → 6.** Added `read_file` (any canonical-rooted path; defaults to focus) + `propose_file_edit` (full-content replacement; user-approval-gated through existing `/api/files/{path}/clive` infra).

## §5 Discipline empirical headlines

**1. R31-sub recursive amend-pass cite-drift (Phase L #16/#17/#19/#20/#23 pattern materialized in Phase Q):** phase-q.md amend cycle missed Driver-section bullets despite Driver-msgs header being corrected; Rain caught the partial-update at PASS-2 review. Preventative adopted mid-flight: post-amend grep for old msg-IDs across whole doc — caught one additional residual at line 75 (Push-class section) that the per-section amend missed. Captured as Phase Q discipline-log entry with preventative cite. Recursion-terminator: post-amend global-grep is the load-bearing pattern; per-section amend is the recurrence-driver.

**2. R37 envelope-compliant Q-2 +73% over upper bound:** estimate 40-60 LOC, actual prod-only +104 driven by read_file companion + notifyProposal helper + comprehensive test coverage class. Self-flagged at staged-time pre-PASS (exemplary R37 application per Phase L #31 precedent class). Tests +154 separate.

**3. Filesystem-class operations (Q-4 + Q-5 + Q-7) NOT in bot-hq repo audit trail:** 4 of 8 Tier-1 items are canonical-store-only state-edits. Phase Q baseline event-count reflects this distinction (7 git commits + 1 canonical-store doc author event for Q-3, vs straight Phase P pattern of 11 git commits). Status framing in phase-q.md cluster table called this out per Rain msg 15215 obs-2.

**4. R32 destructive-class pre-fire visibility despite PROCEED-NOW:** Q-4 3-file migration (`mv` deletes source) qualified as destructive-class. Rain msg 15201 surfaced the discriminator: user PROCEED-NOW absolute-greenlight covers scope+commit+push class, NOT destructive cross-storage moves where source files are deleted. Pre-fire enumeration to user via Brian msg 15196 + 15217 stage-call satisfied the visibility requirement. Adopted obs-B content-date semantic over operation-date for filename consistency with prior side-rename pattern.

**5. doc-after-impl mitigation pattern (Q-1 + Q-2 shipped pre-Q-3-author):** R10 SCOPE-LOCK-BEFORE-IMPL is technically violated when implementation lands before scope-lock-doc author. Rain msg 15205 mitigation: cite phase-q.md as architectural anchor in post-doc commits + log doc-after-impl as Phase Q discipline-log carry-forward. Brian's order chose ship-velocity (small-bug-fix-first); switch to doc-first per Rain msg 15205 alternative re-order suggestion at Q-3 boundary.

## §6 Phase R carry-forwards (Phase Q discipline-log queue)

- **R37 borderline drifts from prior phases continue to defensible-class** — Phase O 6b82921 + Phase P P-3 363d231 + Phase Q Q-2 333ad39 — pattern: small surface-units (3-5 funcs/structs) + comprehensive test coverage = above-threshold-defensible. Future R37 sub-rule candidate: "small-surface-unit class N/A by class".
- **External-file endpoint symlink-resolution gap** (Q-6 obs-1): handleExternalFile uses filepath.Clean + filepath.Rel for traversal-guard but doesn't EvalSymlinks. Threat-model low (user-owned ~/Projects/) but defensive-depth principle applies. Follow-up commit scope.
- **Q-7 yaml.external_docs_root field consumed only documentationally** (Rain msg 15235 obs-1): Q-6 resolveProjectExternalDocs constructs path programmatically from project-stem; doesn't read the new yaml field. Alignment-gap when project-stem ≠ on-disk-dir-name (theoretical for 988; would manifest if user creates `~/Projects/988.utah.gov`). Two resolutions: (a) read yaml field in resolver, or (b) ensure on-disk dir matches stem.
- **Q-5 orphan-dir** (Rain msg 15227 obs-2): `~/.bot-hq/projects/boncom-labs-live-on-playbooks/` exists with no `.yaml`. User-decision class; defer to user surface or Phase R review.
- **R31-sub amend-pass cite-drift** (Phase Q discipline-log new entry): per Rain msg 15218; preventative = post-amend global-grep for old IDs. Recurrence-validated within this very arc-snapshot (Rain msg 15237 caught additional cite-drift in §1+§2; ironic empirical of §5 item 1's exact pattern).
- **doc-after-impl tolerance pattern** (Phase Q discipline-log new entry): user PROCEED-NOW absolute-greenlight substantively satisfies R10 even when doc lands after first commits, provided post-doc commits cite the doc.
- **USER-EXERCISE-PRE-PHASE-CLOSE deferred** (R34 item 9): user offline pending rebuild+restart; phase-close formal declaration awaits user-side validation that the library + dual-root + Clive edit + tasks.md all work end-to-end. Q-8 ships prep artifacts; phase-close-fire happens on user return.
- **R32 SCOPE-FORK miss bilateral on issue 355 BCC pivot** (Q-8b cycle empirical): both Brian + Rain branched off main without checking `git branch -a | grep 355` first. Existing remote branch had 3 commits already covering all 3 design-doc paths. User caught the miss. Recovery clean (orphan branch discarded, work moved to existing branch + rebased + polished + force-pushed). Phase R rule-text refinement candidate: pre-branch-off discovery step belongs in any new-branch flow.
- **R36 vs HEARTBEAT-LOOP-ANTIPATTERN bilateral OUTBOUND-MISS** (Q-8b cycle empirical): both agents emitted "Idle." pane text without hub_send during halt-handoff symmetric silent-commitment cycle, tripping R36 hook. Resolution candidate: zero-pane-output (no text at all) OR canonical hub_send-wrap exit pattern at handshake-close.
- **Bilateral R31 stat-drift recursive on doc-migration audit** (msg 15343/15344/15347/15348): Brian +2 plans / Rain -1 plan / actual 34 — both agents drifted, both self-corrected within 2 round-trips. R31-sub recursion-terminator continues to validate at meta-level.
- **emma-stale-coder detection doesn't distinguish intentional-idle vs fault**: hub-side classification refinement candidate.
- **R34 user-directive override of USER-EXERCISE item-9** (msg 15454): user explicit "finish all and then i will rebuild+restart" overrides item-9-blocking. Authority-class disambiguation candidate (user-directive vs agent-bypass).
- **Q-9 IPv6 hotfix bundle-into-close-composite pattern**: hotfix discovered mid-pivot-return, fired as final pre-close commit, bundled into Phase Q close rather than spawning Phase R cycle. Pattern empirically appropriate for single-line/single-file class.

## §7 Cross-references

- **Phase P arc-snapshot:** `docs/arcs/phase-p.md`
- **Phase Q scope-lock:** `~/.bot-hq/phase/phase-q.md` v1 + 4 amends
- **Ratchet-ledger Phase Q close-row:** `~/.bot-hq/ratchets/active.md` § "Phase Q library — Tier-1 close"
- **Phase Q library schema reference:** `~/.bot-hq/README.md` § "Phase Q — bot-hq is the owner of per-project knowledge"
- **R34 9th self-application:** this close-composite (after L-3a + Phase M close + Phase N v1/v2/v3 close + Phase O drain close + Phase P drain close + Phase Q library Q-8a + Phase Q close-composite Q-8b)
- **Phase Q closed snapshot:** `~/.bot-hq/ratchets/active-phase-q-closed-2026-05-07.md`
- **Phase Q discipline-log Joint entry:** `~/.bot-hq/discipline-log.md§2026-05-07T(Phase-Q-close-composite)`
