# Bot-HQ Manager + Subagent Architecture

**Date:** 2025-01-20
**Status:** Design complete, pending implementation

## Overview

Redesign bot-hq to use a single persistent Claude Code session as an orchestration manager, with subagents handling individual tasks. This replaces the current model of spawning separate headless Claude processes per task.

**Inspiration:** Combines Ralph Wiggum's criteria-driven iteration with GSD's context management to solve context rot.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  bot-hq server                                              │
│      │                                                      │
│      └── Spawns ONE persistent Claude Code (Manager)        │
│          claude --dangerously-approve-all --chrome -p       │
│               │                                             │
│               ├── Runs startup checks                       │
│               ├── Receives UI commands                      │
│               └── Spawns subagents per task                 │
│                    (Task tool, fresh 200k each)             │
└─────────────────────────────────────────────────────────────┘
```

### Manager Responsibilities

- Startup health checks & cleanup
- Receive UI commands ("start task 5")
- Spawn subagents with full context
- Monitor progress (read PROGRESS.md)
- Decide: complete / retry / escalate
- Update task states in bot-hq
- Stay lean (delegate, don't accumulate)

### Context Rot Prevention

| Technique | Source | Implementation |
|-----------|--------|----------------|
| Fresh 200k per executor | GSD | Task tool spawns isolated subagent |
| Criteria-driven completion | Ralph | Build must pass + task criteria |
| Iterate until done | Ralph | Fresh subagent if not complete |
| Files as memory | Both | WORKSPACE.md, STATE.md, PROGRESS.md |
| Manager stays lean | GSD | Orchestrates, doesn't implement |

**Core insight:** Claude is stateless, filesystem is the brain.

## Context File Structure

**Location:** `~/Projects/.bot-hq/` (central, outside repos)

```
~/Projects/
├── .bot-hq/
│   ├── MANAGER_PROMPT.md          # Startup instructions (customizable)
│   ├── QUEUE.md                   # Current operational state
│   │
│   └── workspaces/
│       └── {workspace-name}/
│           ├── WORKSPACE.md       # Auto-generated + user edits
│           │                      # (architecture, conventions, gotchas)
│           ├── STATE.md           # Session state, recent decisions
│           │
│           └── tasks/
│               └── {task-id}/
│                   └── PROGRESS.md
│
├── project-a/                     # Actual repos (untouched)
├── project-b/
```

### PROGRESS.md Format

```yaml
---
iteration: 3
max_iterations: 10
status: in_progress | blocked | complete
blocker_hash: "type-error-auth-module"
last_error: "Type mismatch in auth.ts line 42"
criteria_met: false
build_passes: true
---

## Completed
- Fixed import paths
- Updated user model

## Current blocker
Type mismatch between UserDTO and User interface...

## Next steps
- Align interface definitions
```

## Task Lifecycle

### States

```typescript
type TaskState = 'new' | 'queued' | 'in_progress' | 'needs_help' | 'done';
```

### Flow

```
User clicks "Start Task"
         │
         ▼
Manager spawns subagent (Task tool)
Task: queued → in_progress
         │
         ▼
Subagent works (reads WORKSPACE.md, STATE.md, PROGRESS.md)
         │
         ▼
Subagent writes PROGRESS.md, exits
         │
    ┌────┴────┐
    ▼         ▼
Complete   Not complete
    │         │
    │    ┌────┴────┐
    │    ▼         ▼
    │  Same      New progress
    │  blocker   made
    │  3x? OR    │
    │  max iter? │
    │    │       │
    │    ▼       ▼
    │  needs_   Fresh
    │  help     subagent
    │           (iterate)
    ▼
Show diff in UI
    │
    ├── [Accept] → Push branch → done
    ├── [Reject and Remove Task] → Delete branch, delete task
    └── [Retry] → Add feedback → queued
```

### Stuck Detection

- Track `blocker_hash` in PROGRESS.md frontmatter
- Same hash 3 consecutive times → escalate to `needs_help`
- Max iterations (default 10) → escalate to `needs_help`
- Human provides guidance, task requeued

## Startup Sequence

```
bot-hq server starts
         │
         ▼
Spawn Claude Code manager:
claude --dangerously-approve-all --chrome -p
         │
         ▼
Manager reads .bot-hq/MANAGER_PROMPT.md
         │
         ▼
Manager runs startup tasks:
├── Verify workspace paths exist
├── Verify repos are valid git repos
├── Clean stale task files
├── Reset stuck in_progress tasks → queued
├── Generate WORKSPACE.md for new workspaces
└── Report summary to terminal
         │
         ▼
Manager idle, awaiting UI commands
```

**First-run:** `.bot-hq/` doesn't exist → Manager creates structure

## Subagent Spawning

Manager constructs prompt with full context:

```
You are a task executor for workspace: {workspace.name}
Working directory: {workspace.repoPath}

## Context
{WORKSPACE.md content}

## Current State
{STATE.md content}

## Task
{task.title}
{task.description}

## Success Criteria
- Build must pass: `{workspace.buildCommand}`
- {task.completion_criteria if defined}

## Previous Progress
{PROGRESS.md content or "No previous attempts"}

## Human Feedback (if retry)
{task.feedback}

## Instructions
1. Work on the task in {workspace.repoPath}
2. Create/use branch: feature/task-{task.id}
3. Commit your changes
4. Run build command to verify
5. Update PROGRESS.md with your results
6. Exit when done or blocked
```

## User-Customizable Settings (via bot-hq UI)

### Global Settings

| Setting | Default | Description |
|---------|---------|-------------|
| Manager Prompt | (template) | Startup instructions |
| Max Iterations | 10 | Fallback limit before escalation |
| Stuck Threshold | 3 | Same blocker N times → escalate |

### Per-Workspace Settings

| Setting | Description |
|---------|-------------|
| WORKSPACE.md | Project context, architecture, conventions |
| Build Command | Success criteria baseline |

All editable from bot-hq UI, stored in `.bot-hq/` files.

## Database Schema Changes

### Remove

- `approvals` table
- `agentSessions` table
- `pending_review` state

### Modify tasks table

```sql
-- Add
completion_criteria TEXT
iteration_count INTEGER
max_iterations INTEGER
feedback TEXT
```

### New settings table

```sql
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT,
  updated_at TIMESTAMP
);
```

## Implementation Phases

### Phase 1: Foundation
- Create .bot-hq/ structure and initializer
- Add settings table to database
- Build Settings UI (manager prompt, thresholds)
- Build WORKSPACE.md editor in workspace page

### Phase 2: Manager Session
- Remove session create/delete from terminal tab
- Implement manager spawner on server boot
- Manager reads MANAGER_PROMPT.md, runs startup checks
- Manager idles, awaiting commands

### Phase 3: Task Flow
- "Start Task" button sends command to manager
- Manager spawns subagent via Task tool
- Subagent writes PROGRESS.md
- Manager monitors and handles iteration loop
- Add needs_help state and UI

### Phase 4: Review Flow
- Remove approvals table and routes
- Build diff review UI (replaces approval UI)
- Implement Accept / Reject and Remove Task / Retry
- Accept pushes branch to remote
- Reject deletes branch and task

### Phase 5: Cleanup
- Remove ClaudeCodeAgent class
- Remove agentSessions table
- Remove pending_review state
- Update all task state references
- Test full flow end-to-end

## Data Migration

**Clean slate.** No migration of existing data.

- Drop old tables
- Create new schema
- User reconfigures workspaces fresh
- Manager auto-generates WORKSPACE.md for each new workspace

## Review Actions

| Button | Action |
|--------|--------|
| **Accept** | Push branch to remote, task → done |
| **Reject and Remove Task** | Delete branch, delete task entirely |
| **Retry** | Human adds feedback, task → queued |
