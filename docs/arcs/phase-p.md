# Phase P — consumption-stability + carry-forward drain

Cycle: 2026-05-07 single-session (continuation post Phase O drain close-composite 4ae5666 PUBLIC).

## §1 Scope-lock recap

Opened mid-session at `~/.bot-hq/phase/phase-p.md` v1 (per Brian msg 14893 author + Rain msg 14894 BRAIN-2nd PASS-1 + bilateral converge). 10 Tier-1 items + close-composite per phase-p.md scope-lock; user fork-picks on P-9 + P-10 design-fork-class items via `proceed` / `PROCEED` confirmations.

Driver: user msg 14888 "proceed until every pending task is implemented. everything. absolute greenlight granted." + Phase O drain close-composite §Tier-2-deferred-to-Phase-P 7-item enumeration (ratchet-ledger §72) + Phase N v3 §Q6 OQ-5/-6/-7 productionize-class carry-forwards.

## §2 Tier-1 commit ledger

| # | sha | title | numstat | Rain BRAIN-2nd |
|---|-----|-------|---------|----------------|
| P-1 | `b95e9f9` | SSE live web feed of Clive tool calls | +454/-3 | HOLD→RE-PASS-2 (race fix) |
| P-2 | `4a14fb0` | CodeMirror lightweight YAML editor | +107/-18 | HOLD→RE-PASS-2 (count + SRI) |
| P-3 | `363d231` | Revert UI button (one-click revert) | +474/-0 | HOLD→RE-PASS-2 (double-click guard) |
| P-4a | `0ee7f7b` | Clive prompt-build integration | +194/-0 | PASS-forward |
| P-4b | `f08c2d5` | Clive write-guard hook | +297/-0 | PASS-forward |
| P-5 | `83a3923` | OQ-5 retention policy + age-based pruning | +361/-1 | HOLD→RE-PASS-2 (control-flow `return`) |
| P-6 | `532f650` | OQ-6 secrets-scan-on-manifest-author | +437/-0 | recursive-irony GitHub Push Protection blocked + reset+single-commit recovery |
| P-7 | `f3baa63` | OQ-7 cross-session-search indexed lookup | +414/-1 | PASS-forward + dead-import folded pre-commit |
| P-8 | state-edit | feedback_*.md migration → rules.yaml | NON-REPO | PASS-forward + seo_headings duplicate fix |
| P-9 | `d090b14` | Pending-actions queue (SQLite + sidebar widget) | +803/-1 | HOLD→RE-PASS-2 (`/ack` suffix check) |
| P-10 | `1a279a8` | Voice surface merged INTO workspace webui (internal/live deleted) | net ≈ -300 LOC | mid-cycle drift+correction; user-side caught |
| P-11 | (this) | Phase P drain close-composite | composite | at-stage-call |

Cumulative repo-side: 10 commits + 1 close-composite = 11 commits. ~+3540/-25 net LOC across drain (excluding P-10's deletion-heavy refactor which nets ~-300 LOC).

## §3 Phase P state-edits (non-repo)

- `~/.bot-hq/phase/phase-p.md` scope-lock v1 author
- `~/.bot-hq/ratchets/active.md` — 7 datapoints added (P-2/P-3/P-5/P-6/P-7/P-9 + close-row) + Phase P close-row + Phase Q deferrals section
- `~/.bot-hq/rules/general.yaml` NEW (P-8 cluster A bot-hq trio discipline 13 entries + tone/greenlight/hubDiscipline structured fields + user_preferences)
- `~/.bot-hq/projects/bcc-ad-manager.yaml` extended (P-8 cluster B 3 entries + disguise compliance section)
- `~/.bot-hq/projects/988.yaml` NEW (P-8 cluster C 1 entry + stack metadata + cornerstone reference)

## §4 New endpoints / packages / subcommands

**HTTP endpoints (5):**
- `GET /api/clive/activity` (SSE branch via Accept: text/event-stream — P-1)
- `GET /api/files/{path}/history` (revert UI history fetcher — P-3)
- `GET /api/pending-actions` (list / count / all — P-9)
- `POST /api/pending-actions/{id}/ack` (idempotent ack — P-9)
- `GET /api/voice/ws` (WebSocket; voice integrated into workspace — P-10)

**Go packages (2 NEW):**
- `internal/clive` (P-4a; AgentID/Name/Type consts + InitialPrompt() embedding PhaseNv3CliveExpansion)
- `internal/clivewriteguard` (P-4b; PreToolUse hook BLOCKING Edit/Write/MultiEdit/NotebookEdit/Bash for Clive — dormant until install)

**CLI subcommands (2 NEW):**
- `bot-hq session-prune [--days N] [--dry-run]` (P-5)
- `bot-hq session-search [--limit N] <query>` (P-7)

**Retired:**
- `internal/live` package (~850 LOC; gemini.go + server.go + web/* migrated INTO internal/webui as voice.go + voice_gemini.go + static/voice.js + UI elements directly in static/index.html topbar)
- `bot-hq voice :3847` separate process (live.NewServer call removed from cmd/bot-hq/main.go startup)

## §5 Discipline empirical headlines

**1. R31-sub-FILE-LINE-CITE peer-cross-check (N-T2-c) load-bearing post-graduation**: 4+ Phase P recurrences caught at BRAIN-2nd-time, all corrected pre-commit. P-2 datapoint count drift (#6 → #5/#8) + P-5 control-flow fall-through (missing `return`) + P-6 SHA-cite drift via wrong-hash-algo (the sub-class graduation extension) + P-9 `/ack` suffix check.

**2. R37 above-threshold-flag carry-forward continues to be defensible-class**: P-3 (2.05:1; 4 surface units) + P-5 sessions.go (2.14:1; 4 surface units) + P-9 mixed-class (1.05:1 aggregate). All carry-forward to Phase Q close-eval per ratchet enforcement.

**3. Recursive-irony empirical at meta-level (P-6 GitHub Push Protection)**: shipping a secrets-scan blocked by GitHub's secrets-scan = independent empirical evidence that secrets-scanning works as a category. Recovery via reset+single-commit-recommit (per CLAUDE.md `reset --soft` non-destructive distinction).

**4. Mid-cycle drift+correction (P-10)**: scope-lock fork picks (a-i single-port-mux + c-i tab-in-workspace) initially impl'd as iframe-embed shortcut → user msg 15054 caught as scope-divergence → re-proposed as reverse-proxy (B) → user msg 15061 redirected as deletion-class scope question → user msg 15068 "why route? why not directly. I hate this session" final-clarified as direct-merge → executed as deletion+migration (internal/live → internal/webui). 4 hop-chain scope-correction cycle; user-side correction was the load-bearing terminator.

**5. 9 post-graduation HOLD→RE-PASS cycles**: 4 Phase O + 5 Phase P. Pattern continues to catch quality-gate-class issues pre-commit.

**6. Phase O scope-misread carry-forward identified at P-10 user-confrontation**: phase-n.md:467 Phase O scope ("agent-side CONSUMPTION + Clive autonomous-broader + OQ-5/-6/-7 + N-T2-a/b/i + #41-#55 + recursion-terminator self-id-verify") was NOT what we shipped in "Phase O drain" — that was webui UX/feature carry-forward backlog from phase-n.md:818-826. The ACTUAL :467 Phase O scope was effectively done across Phase P (P-4a/b consumption + P-5/-6/-7 OQ + P-8 feedback migration), with N-T2-a/b/i + #41-#55 + recursion-terminator self-id-verify deferred to Phase Q.

## §6 Phase Q carry-forwards

- **N-T2-b-extension R37 threshold-fitting analysis** (≥6 Phase P datapoints; analysis ready Phase Q)
- **N-T2-c R31-sub-FILE-LINE-CITE** graduation re-eval (≥4 Phase P recurrences caught defensively; graduate-or-deprecate at Phase Q)
- **N-T2-a R15 self-flag-extension** (preflight-CRITICAL self-detection; original Phase O scope, never addressed)
- **N-T2-i Audit-doc-presence-check** (original Phase O scope, never addressed)
- **#41-#55 v2 carry-forward queue** (deprecate-candidates if no recurrence; original Phase O scope, never addressed)
- **recursion-terminator self-id-verify** (tmux session-name vs BOT_HQ_AGENT_ID env match; original Phase O scope, never addressed)
- **P-3 SHA-validation in handleFileRevert** (security-review OQ)
- **P-4b clivewriteguard install-side wiring** (settings.json injection; aligns with future Clive spawn pipeline)
- **Memory file retirement decision** (P-8 user-decision-class; per-file delete vs keep-as-cite-source)
- **USER-EXERCISE walkthrough** (8-point baseline + 12-point Phase O+P combined gate; deferred non-blocking per user "absolute greenlight + everything implemented" — user runs at convenience post-close)
- **R32 SCOPE-FORK-CONFIRMATION extension**: implementation-time-verification (not just stage-call-time) to prevent shortcut-class divergences like P-10 iframe (Phase P discipline-log carry-forward to consider rule-text update)
- **Absolute-greenlight discipline carve-out**: clarifying-question ≠ new-decision-input (saved at Rain feedback memory; consider formalizing as R-rule sub-clause)

## §7 Cross-references

- **Phase O drain arc-snapshot:** `docs/arcs/phase-n-v3.x.md` (4ae5666 close-composite)
- **Phase P scope-lock:** `~/.bot-hq/phase/phase-p.md` v1
- **Ratchet-ledger Phase P close-row:** `~/.bot-hq/ratchets/active.md` § "Phase P drain — Tier-1 close"
- **Phase N origin:** `~/.bot-hq/phase/phase-n.md` (v1 / v2 / v3 / v3.x cluster)
- **R34 7th self-application:** this close-composite (after L-3a + Phase M close + Phase N v1/v2/v3 close + Phase O drain close + Phase P drain close)
