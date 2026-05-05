# Phase N N-1 — ID-based sessions design-spike

**Type:** Phase N v1 N-1 (a) design-spike doc (no impl this cycle)
**Date:** 2026-05-05
**Author:** Rain
**Status:** v1 design-spike pending Brian PASS-1 BRAIN-2nd → close-composite-stages-this-doc → user RATIFY at Phase N v2 scope-lock open

---

## 1. Theme + driver

User msg 7990 (2026-05-05 bcc-ad-manager session) proposed ID-based sessions feature: each session's data clusters under a unique ID for retrospective analysis (project + issues observed + messages + timestamps) + context-retention forward-loop. Bilateral concur msgs 7991+7993 + scope-lock per phase-n.md.

**Empirical anchor:** today's bcc-ad-manager pivot generated ~150+ hub messages, 2 PRs (#368 + #369 closing 5 issues), 2 durable-feedback files added, 1 investigation doc, 1 EOD clip — all with `session_id=null` despite existing `hub_session_create` + `session_id`-column infra. Reconstructing "what happened on May 5" two weeks from now requires hub backlog scrubbing + scattered file cross-reference. Session-cluster anchor would let us say "session-id `2026-05-05-bcc`" and pull the full thread.

## 2. Existing infra (substrate to leverage, not rebuild)

Bot-hq already has:
- **`session_id` column** on hub messages (`internal/hub/db.go` schema; nullable; per existing `hub_send` tool param)
- **Session lifecycle tools:** `hub_session_create` / `hub_session_join` / `hub_session_close` (per ToolSearch surface; underused — today's session never fired any of them)
- **Per-agent AgentState saves** at `~/.bot-hq/<agent>/last_state.json` (per R20; captures snapshot per-agent, not per-session)
- **Project-scoped artifacts** under `~/.bot-hq/projects/<project>/{plans/, eod/, clips/}` (today's bcc-ad-manager pivot used this)
- **Bot-hq scope artifacts** at `docs/plans/` + `docs/arcs/` (in-repo, per Phase M precedent)

What's missing: aggregator that links these by session-id + analysis-view + context-retention forward-loop.

## 3. Design questions (Q-I through Q-V)

### Q-I — Session boundaries

What defines a session start/end? Options:
- **(α) Auto-detect on pivot** — user msgs containing "pivot to <project>" / "session done" / "checkpoint" / similar pattern → trio fires `hub_session_create`/`_close` automatically
- **(β) User-tag explicit** — user explicitly directs "open session for <project>" / "close session" → no auto-detect
- **(γ) Calendar-day** — auto-rolls at midnight; one session per project per day
- **(δ) Hybrid (α+β)** — auto-detect default; user can override-tag

Lean: **(δ)** hybrid. Auto-detect catches the common case (today's "full pivot" + "session done" framing); user-tag handles edge cases (mid-day project switch, multi-project work-burst). Boundary-doc convention: session-id format `YYYY-MM-DD-<project>` or `YYYY-MM-DD-<project>-<n>` for multi-session days.

### Q-II — Schema / format

Frontmatter strict, body free-form (matches auto-memory durable feedback pattern). Frontmatter fields:
- `id` — session-id (e.g., `2026-05-05-bcc-ad-manager`)
- `project` — target project key (e.g., `bcc-ad-manager`, `bot-hq`, `988-utah-gov`)
- `start_ts` / `end_ts` — ISO-8601 timestamps
- `start_msg_id` / `end_msg_id` — hub.db msg-id range
- `agents` — list of trio agent IDs active during session
- `pivot_in_msg_id` / `pivot_out_msg_id` — explicit pivot markers if applicable
- `parent_session_id` — for nested sessions (e.g., bot-hq → bcc-ad-manager pivot today)

Body (free-form markdown sections):
- Deliverables (PRs / commits / SHIPPED items)
- Issues addressed (per-issue one-line + status)
- Empirical observations (rule-class generalizable patterns surfaced)
- Follow-ups (open work for next session)
- Durable-feedback deltas (memory files added/edited)
- Cross-references (pointers to investigation docs / EOD clips / arc-snapshots)

### Q-III — Authoring trigger

When does the manifest get written? Options:
- **(a) At session-close** — single write at `hub_session_close` event; full manifest with final state
- **(b) Rolling** — manifest grows during session; updated on key events (commit-fire, PR-open, pivot, halt)
- **(c) Hybrid** — minimal manifest at session-create (id + start_ts + project); finalized at session-close

Lean: **(c)** hybrid. Minimal-create avoids drift-management overhead during session; close-finalization captures final state. Matches `hub_session_close` existing tool semantics.

### Q-IV — Consumer interface

How is the session-cluster consumed? Options:
- **(i) CLI surface:** `bot-hq session-analyze <id>` outputs the manifest + computed-fields (msg counts / commit counts / etc.)
- **(ii) File-only:** consumers read `~/.bot-hq/sessions/<id>/manifest.md` directly
- **(iii) Both** — CLI for computed analysis; file for direct access

Lean: **(iii)** both. CLI is the primary "give me a summary of session X" surface; file is the durable artifact. CLI implementation thin (read manifest + run hub-DB queries scoped to msg-id range + format output).

### Q-V — Context-retention forward-loop

How does session-cluster auto-load on next-session-create-against-same-project? Options:
- **(p) CLAUDE.md analogue per-session** — write `~/.bot-hq/sessions/<id>/agent-context.md` that gets merged into agent prompts on next session-create-against-same-project
- **(q) hub_register injects last-session-snap-extended** — extend existing `last_session_snap` mechanism to include cross-session anchor (last N session-ids per project)
- **(r) On-demand load** — agent calls a `hub_session_load <id>` tool to inject specific session context when relevant
- **(s) Combined (p+r)** — auto-load most-recent + on-demand for specific past sessions

Lean: **(s)** combined. Auto-load most-recent-per-project keeps continuity (today→tomorrow); on-demand handles "what happened in session X two weeks ago" use cases.

## 4. Proposed storage shape

```
~/.bot-hq/sessions/
├── 2026-05-05-bcc-ad-manager/
│   ├── manifest.md          # frontmatter + free-form body (per Q-II)
│   ├── agent-context.md     # auto-load context for next-session (per Q-V option (p))
│   └── (optional) thin-pointers.md  # file references to existing artifacts (eod-clip / investigation docs / etc.)
├── 2026-05-04-bot-hq-phase-m/
│   └── manifest.md
└── index.md                 # rolling list of session-ids (auto-maintained)
```

`manifest.md` is **thin** — references existing artifacts (e.g., `~/.bot-hq/projects/bcc-ad-manager/eod/eod-clip.md`) rather than duplicating their content. Reduces drift between manifest and source-of-truth artifacts.

## 5. Hub-DB integration

- `session_id` column already exists on hub messages — no schema change needed
- `hub_session_create` returns session-id; trio uses it on subsequent `hub_send` calls
- `hub_session_close` triggers manifest finalization (post-N-1 (b) impl)
- New tool (Phase N v2 candidate): `hub_session_load <id>` for on-demand context retrieval

## 6. Implementation sketch (Phase N v2 scope)

Phase N v1 ships this design-spike only. Phase N v2 implements per these design decisions:

1. **Boundary detector:** trio agent prompts include pivot-pattern recognition (user msgs matching `(?i)(full pivot|pivot to|session done|checkpoint|EOD|end of day)` → fire `hub_session_create`/`_close`)
2. **Manifest authoring:** `hub_session_close` extends existing tool to write `~/.bot-hq/sessions/<id>/manifest.md` with frontmatter + body computed from hub-DB queries (msg-id range + agent activity + commit references)
3. **CLI surface:** new `bot-hq session-analyze <id>` subcommand reads manifest + executes summary queries
4. **Context-retention auto-load:** `hub_register` extends `last_session_snap` return to include cross-session-pointer per-project; new agent-context.md auto-loads
5. **Index maintenance:** session-create/close updates `~/.bot-hq/sessions/index.md` rolling list
6. **Tests:** hub-DB integration + manifest schema + boundary-detector regex + CLI output format

Estimated Phase N v2 scope: ~400-600 LOC code + 200-300 LOC tests + ~30-50L skill markdown.

## 7. Open questions for Phase N v2 RATIFY

- **OQ-1:** auto-detect regex set — beyond `pivot/done/checkpoint`, what other pivot signals? user-research-class question
- **OQ-2:** session-id format conflict on multi-session-same-day — `-1` `-2` suffix vs timestamp suffix — minor decision
- **OQ-3:** manifest update on already-closed session (e.g., user retroactively adds context) — append-only? edit-allowed? versioning?
- **OQ-4:** parent_session_id semantics — today's `bot-hq` parent → `bcc-ad-manager` child pivot pattern. Or are pivots always sibling sessions?
- **OQ-5:** retention policy — session-clusters never deleted? rolling N-day archive? tied to git-history?
- **OQ-6:** privacy-class data — if a session pulls in user's git workspace state, secrets-scan before write?
- **OQ-7:** cross-session-search performance — if 100s of session-clusters accumulate, does CLI need indexed-search?

## 8. Empirical anchor cite

- 2026-05-05 user msg 7990 — feature proposal
- Today's bcc-ad-manager session: 150+ hub messages all `session_id=null` despite existing infra; 5 issues addressed across 2 PRs; rich state scattered across artifact-files; reconstruction-cost-in-2-weeks high without session-cluster anchor
- Bilateral converge msgs 7991-7995 (Brian + Rain independent surfaces both landed on same design dimensions)
- Phase N v1 scope-lock at `~/.bot-hq/phase/phase-n.md` includes N-1 work-units split across (a) doc-this-cycle / (b/c) impl-Phase-N-v2

---

(end of design-spike v1; pending Brian PASS-1 BRAIN-2nd → close-composite stages this doc + ratchet-ledger update + arc-snapshot phase-n-v1.md → push-batch elevation USER-ONLY-ABSOLUTE → user-fired rebuild+restart → Phase N v2 cycle for impl)
