# bot-hq — Project Instructions (for Claude Code)

You are working on the **bot-hq rebuild** — a from-scratch Rust + Slint reimagining of an existing Go/tmux/MCP daemon. The user is running this autonomously across multiple Claude Code sessions and needs you to work continuously and pick up where prior sessions left off.

## Read these files FIRST, in order:

1. **`ARCHITECTURE.md`** — single source of truth for architectural decisions. Don't re-litigate decisions documented here.
2. **`PLAN.md`** — phased implementation roadmap with verifications per phase.
3. **`PROGRESS.md`** — current status, what's done, what's blocked, where to resume.

These three are the canonical docs. Anything else is supplementary.

---

## Operating mode

You are the **orchestrator** for this build. Primary mode: dispatch sub-agents (via the `Agent` tool) to do parallel, well-scoped work; integrate results yourself.

- **Sub-agent dispatch is the primary work mechanism.** Tasks marked `[P]` in PLAN.md within the same phase can be dispatched in parallel (single message, multiple `Agent` tool calls). Brief each sub-agent per PLAN.md "Sub-agent dispatch guidance": goal, files, interface, tests, definition of done.
- **Integration is your job.** Sub-agents return work; you review, integrate, run tests, decide next steps. Log each dispatch + outcome in PROGRESS.md "Sub-agent dispatch log".
- **Take work in small testable chunks.** Compile + test after each integration. Catch issues early.
- **Don't wait for user approvals on routine choices that fall within decided architecture** — the user is offline. Default + log under "Decisions made autonomously" in PROGRESS.md.
- **When blocked,** mark in PROGRESS.md with: specific blocker, file/task affected, what info would unblock you. Then move to non-blocked work.
- **Don't commit / don't push.** Terminal state is **READY FOR HUMAN REVIEW** — leave the working tree dirty, document state, stop.

**Fallback if context fills:** if orchestrator context exhausts before completion, use PROGRESS.md as a handoff baton. The next fresh `claude-code` session reads CLAUDE.md → ARCHITECTURE.md → PLAN.md → PROGRESS.md and continues. This is contingency only — design for single-session completion.

---

## Critical rules

- **NEVER add `Co-Authored-By:` or any AI watermark** to commit messages or generated content. No attribution to Claude / AI / Anthropic in commits.
- **Working tree state:** existing Go-code files appear as unstaged deletions. Do NOT stage or revert them. They commit alongside the new Rust code at Phase 9.5 (human-driven).
- **Data paths during dev:** set `BOT_HQ_DATA_DIR=~/.bot-hq-dev/` in `.env`. The default `~/.bot-hq/` would collide with the still-running current bot-hq.
- **No `--no-verify` or hook-skipping.** Fix root causes.
- **Sub-agents:** tasks marked `[P]` in PLAN.md can be dispatched to sub-agents in parallel. See PLAN.md "Sub-agent dispatch guidance" for the briefing template.

---

## Handling ambiguity

If something is genuinely ambiguous and NOT decided in ARCHITECTURE.md or PLAN.md:

1. Default to the simplest reasonable choice consistent with the documented direction.
2. Note your choice + rationale in PROGRESS.md under **Decisions made autonomously**.
3. Continue. The user will review on return; choices are revisable later.

If the ambiguity is genuinely blocking and you can't proceed without an answer, document it as a blocker in PROGRESS.md and move to non-blocked work.

---

## Prerequisites

The project assumes:
- Rust stable toolchain installed (`rustup`, latest stable)
- `claude-code` CLI installed and authed (used as subprocess + needed for live tests)
- macOS (initial target; cross-platform later)

If a prerequisite is missing, document it in PROGRESS.md as a blocker (don't try to install system-wide tools yourself).

---

## How a typical session looks

1. Read ARCHITECTURE.md, PLAN.md, PROGRESS.md (in that order).
2. Look at PROGRESS.md "Currently working on" and "Session handoff log" to know where to resume.
3. Pick up the next pending task (or continue an in-flight one).
4. Work in compile-test-commit-to-progress-md cycles.
5. When context is filling, do a final PROGRESS.md update with a clear resume hint for the next session.
6. Stop.

Trust the plan. Trust prior sessions' notes in PROGRESS.md. Build forward.
