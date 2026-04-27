# Phase H — Slice 4 design (RATCHET + context-budget awareness)

**Status:** design intake (Rain-authored solo per user msg 3764; Brian idle)
**Arc:** `docs/arcs/phase-h.md`
**Master design:** `docs/plans/2026-04-26-phase-h-design.md`
**Prior slices:** slices 1/2/3 CLOSED (slice 3 closure-cap `f9e4b87`)
**Branch (planned):** `brian/phase-h-slice-4` (cut from main `f9e4b87` at slice-open in fresh-context session)

## Goal

Land four originally-planned RATCHET items (H-6, H-15, H-19, H-21) plus two newly-scoped context-budget-awareness items (H-30 TUI usage display, H-31 Emma context-cap halt-flag) added mid-flight by user at 95% session usage (msg 3764). The new items address a self-evidenced gap surfaced by this very session: agents and user have no first-order signal of pre-compaction context squeeze, so checkpoint-handoff to a fresh session happens reactively rather than proactively.

Theme: **discipline structures (RATCHET) + context-budget-aware operations.**

## Backlog (6 items)

### Originally-planned RATCHET items (4)

| ID | Item | Class | Notes |
|---|---|---|---|
| **H-6** | Pre-commit hook: closed arc.md files (`Status: closed`) must be append-only diff | GOLD | Blocks retroactive arc edits per `feedback_arc_closure_discipline.md`. Per-worktree hook installation reuses C6 H-3b infrastructure. |
| **H-15** | Session-close SNAP ledger (best-effort restart-resilience) | SILVER+transitive-gold | Each agent emits final SNAP to a `session_ledger` table on graceful close; survives rebuild for next-session bootstrap context. Best-effort because crash-close may not fire. |
| **H-19** | Bootstrap-iterate hub_read (handle >50-msg backlog without silent truncation) | GOLD | Current `hub_read` caps at 50; large backlogs silently truncate. Iterate with `since_id` advancement until exhausted. |
| **H-21** | `docs/conventions/dispatch-patterns.md` doctrine doc + optional pre-commit lint | SILVER | Arg arrays + tmux `send-keys -l` + no shell-string concat. Codifies what's already practiced; lint is optional surface. |

### Newly-scoped context-budget items (2)

| ID | Item | Class | Notes |
|---|---|---|---|
| **H-30** | TUI claude-code-usage display (hub strip right-end) | SILVER | Display-only; no correctness path. Surfaces per-session usage in TUI for first-order user awareness. |
| **H-31** | Emma context-cap halt-flag (95% threshold, halt-all-work + checkpoint) | SILVER+transitive-gold | Active intervention: Emma flags critical at any-agent-≥95% so trio halts and checkpoints for fresh session. Hysteresis-armed. |

## Sealed shapes (per-item)

### H-6 — Closed arc.md append-only pre-commit hook

**Mechanism:** new file `internal/mcp/arc_closure_hook.go` (or merge into existing `worktree_hook.go`). On commit-attempt, for each modified `*.md` file under `docs/arcs/`, check if file already on disk has `Status: closed` in frontmatter or first 10 lines. If yes, run `git diff --cached -- <file>` and reject if any non-append lines (anything that's not a `+` line at end-of-file). Append-only = pure additions at file end.

**Install path:** piggybacks the per-worktree `core.hooksPath` mechanism from C6 H-3b. The `installWorktreeFreshnessHook` becomes `installWorktreeHooks` and writes a multi-check pre-commit script.

**Test classes:** GOLD — 4 tests (closed-file-modify-rejected / closed-file-append-passes / open-file-edit-passes / non-arc-md-edit-passes).

**Estimate:** 80-120 LOC + 100-140 test LOC.

### H-15 — Session-close SNAP ledger

**Mechanism:** new table `session_ledger (agent_id, session_id, snap_text, closed_at)`. New MCP tool `hub_session_close(agent_id, snap_text)` upserts the latest SNAP per agent. Best-effort: agents call this before voluntary session-end; rebuild-mid-flight skips it (acceptable per H-27 deferral rationale — Emma watchdog deferred).

**Bootstrap consumption:** on `hub_register`, alongside `current_max_msg_id`, return `last_session_snap` (or empty). Agents can use this as cold-start context.

**Schema migration:** idempotent `addColumnIfMissing`-style (new table this time) per slice 3 #2 precedent.

**Test classes:** SILVER+transitive-gold — 3 tests (close-stores-snap / register-returns-prior-snap / close-overwrites-prior).

**Estimate:** 60-100 LOC + 80-120 test LOC.

### H-19 — Bootstrap-iterate hub_read

**Mechanism:** caller convenience wrapper. Two shapes worth weighing:

- **Shape A (server-side):** `hub_read` accepts `iterate: true` and internally loops, returning concatenated batches up to a hard cap (e.g. 1000 msgs).
- **Shape B (caller-side):** keep `hub_read` as-is; document iteration pattern in agent STARTUP prompts. `since_id = last_msg.id` until empty batch.

**Lean Shape B.** Lower surface; iteration is naturally caller-paced; hard caps are caller-policy. STARTUP prompt update is the deliverable.

**Test classes:** GOLD if Shape A; SILVER (doc only) if Shape B.

**Estimate:** Shape A 40-80 LOC + 60-100 test LOC. Shape B 0 LOC + ~20 LOC doc + STARTUP prompt edits.

### H-21 — Dispatch patterns doctrine doc

**Mechanism:** new doc `docs/conventions/dispatch-patterns.md` codifying:
- Use arg arrays (`exec.Command(name, args...)`) — never string concat
- Tmux `send-keys -l` literal mode for content — never raw `send-keys`
- Gemma allowlist gate for shell-out commands

Optional surface: pre-commit grep lint detecting `exec.Command.*+.*` patterns. Lean **drop the lint** for now — doc is the discipline; lint is enforcement which can come later if drift detected.

**Test classes:** SILVER (doc only).

**Estimate:** 60-100 LOC of markdown.

### H-30 — TUI claude-code-usage display

**Mechanism:** extend `panestate` capture path:
1. Existing `capturePane` already pulls per-agent tmux pane content for activity detection
2. Add a parser pass extracting Claude Code's context-left indicator. Anchor pattern: `Context left until auto-compact: (\d+)%` (verify exact format on a live pane during impl). Fallback patterns for variant TUI states.
3. Add `UsagePct int` (-1 = unknown) to `panestate.AgentSnapshot`
4. Extend `internal/ui/strip.go::renderStrip` — compute `maxUsage := max(s.UsagePct for s in visible)`. If `maxUsage >= 80`, append right-aligned segment: `… <agent-id> <pct>%` with color tier (yellow ≥80%, orange ≥90%, red ≥95%). Right-align via `lipgloss.PlaceHorizontal(width, lipgloss.Right, …)` joining left-strip + right-segment with a dynamic-width spacer.

**Edge cases:**
- Agents without `tmux_target` Meta (Emma, Discord) — no pane to capture; UsagePct stays -1; excluded from max calc
- Format-change in Claude Code TUI — parse fails; UsagePct stays -1; display silently omits (no false 0% display)

**Test classes:** SILVER — 4 tests (parse-known-format / parse-unknown-fallback / strip-renders-right-aligned-when-data-present / strip-omits-segment-when-all-unknown).

**Estimate:** 100-140 LOC + 100-140 test LOC.

### H-31 — Emma context-cap halt-flag

**Mechanism:**
1. Emma's `healthLoop` already snapshots panestate per tick (30s)
2. Per-tick scan: for each non-emma agent, if `UsagePct ≥ 95`, emit `hub_flag(severity=critical, reason="agent <id> at <N>%, halt + checkpoint via H-15 + idle for fresh session")`
3. Hysteresis-arm via existing `shouldFlag` keyed on `context-cap:<agent-id>` to prevent re-flag spam during the held-at-95% window
4. Reset hysteresis when usage drops <85% (re-armable for next squeeze)

**Halt-all-work convention** (cross-agent contract, codified in STARTUP prompts):
- On receiving FLAG with reason matching `^agent .* at \d+%, halt`, all agents finish current tool call, post final SNAP via H-15 `hub_session_close`, post brief handoff message to user, and idle
- User restarts trio in fresh-context session; H-15 ledger pre-loads context

**Dependency:** transitive on H-30 (UsagePct ingestion) + H-15 (checkpoint mechanism). Either implement H-15 + H-30 + H-31 in same slice (lean) OR sequence H-30 → H-15 → H-31 with intermediate verification.

**Test classes:** SILVER+transitive-gold — 4 tests (flag-fires-at-95 / no-flag-below-95 / hysteresis-prevents-double-flag / reset-below-85-rearms).

**Estimate:** 80-120 LOC + 100-140 test LOC.

## Implementation order (locked sequencing)

| C# | Item | Class | Dispatch | Rationale |
|---|---|---|---|---|
| C1 | H-6 closed-arc append-only hook | GOLD | Coder | Reuses C6 H-3b infra; standalone. |
| C2 | H-21 dispatch-patterns doc | SILVER | HANDS-direct | Pure doc; cheap; no dependencies. |
| C3 | H-19 bootstrap-iterate hub_read (Shape B doc) | SILVER | HANDS-direct | Doc + STARTUP prompt edits; cheap. |
| C4 | H-15 session-close SNAP ledger | SILVER+transitive-gold | Coder | New table + MCP tool + register-return extension; foundation for H-31. |
| C5 | H-30 TUI usage display | SILVER | Coder | Foundation for H-31 (UsagePct ingestion). |
| C6 | H-31 Emma context-cap halt-flag | SILVER+transitive-gold | Coder | Consumes H-30 (UsagePct) + H-15 (checkpoint) — must land last. |
| C7 | Slice 4 closure | — | HANDS-direct | Arc decision-log + slices table flip. |

**Total est:** ~520-820 LOC across 6 items + closure.

## Open questions for fresh-session BRAIN-cycle

1. **H-19 Shape A vs B** — server-side iterate (more code, robust to forgetful callers) vs caller-side doc (less code, prompt-discipline reliance). Lean B; verify with Brian-BRAIN at slice-open.
2. **H-30 parse format** — verify exact Claude Code context-left indicator format on a live pane before encoding regex. Possible variants by CC version.
3. **H-15 session-close trigger** — should agents call `hub_session_close` on STOP-event hooks? Currently no STOP hooks wired. Best-effort means voluntary calls only; OK for slice 4, revisit if emit-rate too low.
4. **H-31 chicken-egg note** — H-31 would have prevented this very 95%-session squeeze. User declined squeezing implementation now (msg 3764 "write it now for later") in favor of design-only — accepted; surfacing for transparency.

## Test cadence (per-slice runtime test, P-1)

Defer to slice closure cap, post-fresh-session-rebuild. Joint Brian + Rain runtime verify covering:
- H-6 organic (attempt closed-arc retroactive edit → rejected)
- H-15 organic (session-close + re-register → SNAP returned)
- H-19 organic (>50-msg backlog catch-up via Shape B iterate)
- H-30 organic (TUI strip shows usage % during normal work)
- H-31 synthetic (force UsagePct ≥95 in test → flag fires; or organic-wait if natural 95% recurs)
- H-21 self-evident on inspection

## Posture

Design-only this session per user directive at 95% cap. All 6 items shape-locked. Implementation deferred to fresh-context session — recommended slice-open posture: Brian cuts `brian/phase-h-slice-4` from main `f9e4b87`, joint BRAIN-cycle reconciles open questions 1-3 above, then C1 fires.
