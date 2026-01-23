# AGENTS.md - Bot-HQ

> **For AI Agents**: This is your context file. Read this first to understand the project.
>
> Automatically discovered by: Claude Code, Cursor, Windsurf, Cline, Aider, and other AI coding assistants.

---

## 1. Project Overview

**Bot-HQ** is a task orchestration and agent management system that coordinates Claude Code agents to work on software development tasks autonomously.

### Tech Stack
- **Frontend**: Next.js 16.1.1 (React 19), TypeScript, Tailwind CSS, shadcn/ui
- **Backend**: Next.js API routes (Node.js)
- **Database**: SQLite with Drizzle ORM (WAL mode)
- **Terminal**: PTY-based (node-pty) for spawning Claude Code agents
- **AI Integration**: MCP (Model Context Protocol) server + Anthropic SDK
- **Port**: 7890

### Project Location
```
/Users/gregoryerrl/Projects/bot-hq
```

---

## 2. Main Workloop Flow

### Inspirations & Credits

Bot-HQ's architecture draws from two key inspirations:

| Feature | Inspired By | Description |
|---------|-------------|-------------|
| **Criteria-Driven Iteration** | [Ralph Wiggum](https://github.com/human-software-language/ralph-wiggum) | The iteration loop where tasks are retried with feedback until completion criteria are met. Manager evaluates work against defined criteria, provides feedback on failures, and triggers autonomous retries. |
| **Smart Context Management** | [GSD (Get Shit Done)](https://github.com/get-shit-done/gsd) | The `.bot-hq/` directory structure with markdown files for context persistence. WORKSPACE.md, STATE.md, and PROGRESS.md files maintain context across sessions and enable effective handoff between manager and subagents. |

#### Criteria-Driven Iteration (Ralph Wiggum)

The iteration model works as follows:
```
┌─────────────────────────────────────────────────────────────┐
│                    ITERATION LOOP                           │
│                                                             │
│   Task has: completionCriteria, iterationCount, maxIterations│
│                                                             │
│   ┌──────────┐     ┌──────────┐     ┌──────────┐           │
│   │  Execute │────►│ Evaluate │────►│ Criteria │           │
│   │   Task   │     │  Result  │     │   Met?   │           │
│   └──────────┘     └──────────┘     └────┬─────┘           │
│                                          │                  │
│                         ┌────────────────┼────────────────┐ │
│                         │ YES            │ NO             │ │
│                         ▼                ▼                │ │
│                    ┌─────────┐    ┌─────────────┐         │ │
│                    │  DONE   │    │ iteration++ │         │ │
│                    │         │    │ + feedback  │         │ │
│                    └─────────┘    └──────┬──────┘         │ │
│                                          │                │ │
│                                          ▼                │ │
│                                   ┌─────────────┐         │ │
│                                   │ iteration < │         │ │
│                                   │    max?     │         │ │
│                                   └──────┬──────┘         │ │
│                                          │                │ │
│                              ┌───────────┼───────────┐    │ │
│                              │ YES       │ NO        │    │ │
│                              ▼           ▼           │    │ │
│                         ┌────────┐  ┌────────────┐   │    │ │
│                         │ RETRY  │  │ NEEDS_HELP │   │    │ │
│                         │(queued)│  │   (HITL)   │   │    │ │
│                         └────────┘  └────────────┘   │    │ │
│                                                      │    │ │
└──────────────────────────────────────────────────────┘────┘ │
```

#### Smart Context Management (GSD)

The markdown-based context system:
```
.bot-hq/
├── MANAGER_PROMPT.md          # Global manager instructions (customizable)
├── QUEUE.md                   # Current task queue status
├── .manager-status            # Manager running state
│
└── workspaces/
    └── {workspace-name}/
        ├── WORKSPACE.md       # Project overview, architecture, conventions
        │                      # - Tech stack, dependencies
        │                      # - Code patterns and standards
        │                      # - Important file locations
        │
        ├── STATE.md           # Current working state
        │                      # - What was last worked on
        │                      # - Any pending items
        │                      # - Session continuity info
        │
        └── tasks/
            └── {task-id}/
                └── PROGRESS.md    # Per-task work log
                                   # - Status (in_progress/done)
                                   # - Iteration count
                                   # - Work log entries
                                   # - Completed items
                                   # - Blockers encountered
```

**Why Markdown Files?**
- Human-readable and editable
- Version-controllable (git-friendly)
- Survives process restarts
- Easy context injection into prompts
- No complex serialization needed

---

### Design Philosophy: Autonomous + Human-in-the-Loop (HITL)

Bot-HQ is designed with a **hybrid autonomy model**:

| Mode | Description | When It Happens |
|------|-------------|-----------------|
| **AUTONOMOUS** | System works independently, no human needed | Task execution, code generation, commits, verification |
| **HITL** | Human decision required before proceeding | Task creation, assignment, brainstorming questions, final review |

**Key Principle**: The system maximizes autonomous work while ensuring humans retain control at critical decision points.

---

### The Core Loop (with Autonomy Markers)

```
═══════════════════════════════════════════════════════════════════
                         AUTONOMOUS ZONE
    Once started, the system runs without human intervention
═══════════════════════════════════════════════════════════════════

┌─────────────────────────────────────────────────────────────────┐
│ [AUTONOMOUS] STARTUP OPERATION                                  │
│──────────────────────────────────────────────────────────────── │
│  Manager initializes automatically:                             │
│  • Checks for orphaned tasks (stuck in_progress)                │
│  • Resets stuck tasks to queued                                 │
│  • Loads workspace contexts                                     │
│  • Ready to receive work                                        │
└─────────────────────────────────────────────────────────────────┘


═══════════════════════════════════════════════════════════════════
                           HITL ZONE
         Human decision required - system waits for input
═══════════════════════════════════════════════════════════════════

┌─────────────────────────────────────────────────────────────────┐
│ [HITL] TASK ACQUISITION                                         │
│──────────────────────────────────────────────────────────────── │
│  Human decides WHAT work to do:                                 │
│                                                                 │
│  ┌─────────────────────┐    ┌─────────────────────┐            │
│  │   GitHub Issue      │    │    Normal Task      │            │
│  │   ─────────────     │    │    ───────────      │            │
│  │   Human selects     │    │   Human creates     │            │
│  │   which issue to    │    │   task with title,  │            │
│  │   work on           │    │   description,      │            │
│  │                     │    │   workspace         │            │
│  └──────────┬──────────┘    └──────────┬──────────┘            │
│             │                          │                        │
│             └──────────┬───────────────┘                        │
│                        ▼                                        │
│              Task created (state: "new")                        │
│                                                                 │
│  WHY HITL: Human controls what enters the system                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ [HITL] TASK ASSIGNMENT                                          │
│──────────────────────────────────────────────────────────────── │
│   Human clicks "Assign" → Task moves to queue                   │
│   Task state: "new" → "queued"                                  │
│                                                                 │
│   WHY HITL: Human confirms task is ready for work               │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ [HITL] START TASK                                               │
│──────────────────────────────────────────────────────────────── │
│   Human clicks "Start" → Manager receives task                  │
│   Task state: "queued" → "in_progress"                          │
│                                                                 │
│   WHY HITL: Human triggers autonomous execution                 │
│                                                                 │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │  AFTER THIS POINT: SYSTEM RUNS AUTONOMOUSLY             │   │
│   │  Human is NOT needed until review or brainstorming      │   │
│   └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘


═══════════════════════════════════════════════════════════════════
                         AUTONOMOUS ZONE
              System works independently from here
═══════════════════════════════════════════════════════════════════

                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ [AUTONOMOUS] MANAGER EVALUATION                                 │
│──────────────────────────────────────────────────────────────── │
│   Manager analyzes task complexity automatically:               │
│   • Is the task vague or ambiguous?                             │
│   • Are there multiple valid approaches?                        │
│   • Missing acceptance criteria?                                │
│   • Architectural decisions needed?                             │
│                                                                 │
│   ┌─────────────────┐           ┌─────────────────┐            │
│   │ Needs Clarity   │           │ Clear Enough    │            │
│   │ ─────────────   │           │ ────────────    │            │
│   │ → HITL: Ask     │           │ → AUTONOMOUS:   │            │
│   │   Questions     │           │   Execute       │            │
│   └────────┬────────┘           └────────┬────────┘            │
│            │                             │                      │
└────────────┼─────────────────────────────┼──────────────────────┘
             │                             │
             ▼                             │

═══════════════════════════════════════════════════════════════════
                    CONDITIONAL HITL ZONE
          Only triggered if manager needs clarification
═══════════════════════════════════════════════════════════════════

┌─────────────────────────────┐            │
│ [HITL] BRAINSTORMING        │            │
│─────────────────────────────│            │
│  Manager outputs:           │            │
│  [AWAITING_INPUT:{taskId}]  │            │
│  Question: ...?             │            │
│  Options:                   │            │
│  1. Option A                │            │
│  2. Option B                │            │
│  [/AWAITING_INPUT]          │            │
│                             │            │
│  Task state: "awaiting_input"            │
│  *** SYSTEM PAUSES ***      │            │
│  Human answers in /pending  │            │
│  Task state: "queued"       │            │
│  *** SYSTEM RESUMES ***     │            │
│                             │            │
│  WHY HITL: Human provides   │            │
│  direction on ambiguous     │            │
│  requirements               │            │
└──────────────┬──────────────┘            │
               │                           │
               └─────────────┬─────────────┘


═══════════════════════════════════════════════════════════════════
                         AUTONOMOUS ZONE
         Full autonomous execution - no human needed
═══════════════════════════════════════════════════════════════════

                             ▼
┌─────────────────────────────────────────────────────────────────┐
│ [AUTONOMOUS] CONTEXT HANDOFF TO SUBAGENT                        │
│──────────────────────────────────────────────────────────────── │
│   Manager prepares detailed prompt for subagent:                │
│   • Task title and description                                  │
│   • Workspace path and context                                  │
│   • Required steps (branch creation, implementation, etc.)      │
│   • PROGRESS.md tracking file location                          │
│   • Commit message format                                       │
│                                                                 │
│   Manager spawns subagent via PTY with:                         │
│   `claude --dangerously-skip-permissions`                       │
│                                                                 │
│   NO HUMAN NEEDED: Context transfer is automatic                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ [AUTONOMOUS] SUBAGENT EXECUTION                                 │
│──────────────────────────────────────────────────────────────── │
│   Subagent works completely independently:                      │
│   1. git checkout -b task/{id}-{slug}                           │
│   2. Create/update .bot-hq/.../PROGRESS.md                      │
│   3. Implement the changes (write code, tests, etc.)            │
│   4. git commit -m "feat(task-{id}): {description}"             │
│   5. Report completion to manager                               │
│                                                                 │
│   NO HUMAN NEEDED: Full autonomous coding                       │
│                                                                 │
│   Exception: If blocked repeatedly (3+ times same issue):       │
│   Task state → "needs_help" (triggers HITL)                     │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ [AUTONOMOUS] VERIFICATION & COMPLETION                          │
│──────────────────────────────────────────────────────────────── │
│   Manager verifies automatically:                               │
│   • Reads PROGRESS.md to confirm work done                      │
│   • Validates completion criteria                               │
│   • Calls task_update(taskId, state: "done")                    │
│   • Sets branchName for UI review                               │
│                                                                 │
│   Task state: "in_progress" → "done"                            │
│                                                                 │
│   NO HUMAN NEEDED: Automated verification                       │
└─────────────────────────────────────────────────────────────────┘


═══════════════════════════════════════════════════════════════════
                           HITL ZONE
            Human reviews and decides next action
═══════════════════════════════════════════════════════════════════

                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ [HITL] USER REVIEW                                              │
│──────────────────────────────────────────────────────────────── │
│   Human reviews git diff of feature branch                      │
│                                                                 │
│   ┌─────────────────┐           ┌─────────────────┐            │
│   │    Accept       │           │  Request Changes│            │
│   │    ──────       │           │  ───────────────│            │
│   │  Task complete  │           │  Provide feedback│           │
│   │  Remove from DB │           │  Task → "queued" │           │
│   │  Human decides  │           │  *** TRIGGERS    │           │
│   │  next action    │           │  AUTONOMOUS      │           │
│   │                 │           │  ITERATION ***   │           │
│   └─────────────────┘           └─────────────────┘            │
│                                                                 │
│   WHY HITL: Human has final say on code quality                 │
│   Human can accept OR trigger another autonomous cycle          │
└─────────────────────────────────────────────────────────────────┘
```

---

### Autonomy Summary

| Phase | Mode | Human Action | System Action |
|-------|------|--------------|---------------|
| Startup | AUTONOMOUS | None | Initialize, reset orphaned tasks |
| Task Creation | HITL | Create/select task | Wait for input |
| Task Assignment | HITL | Click "Assign" | Move to queue |
| Task Start | HITL | Click "Start" | Begin autonomous work |
| Manager Evaluation | AUTONOMOUS | None | Analyze complexity |
| Brainstorming | HITL (conditional) | Answer questions | Wait for response |
| Context Handoff | AUTONOMOUS | None | Prepare & spawn subagent |
| Subagent Execution | AUTONOMOUS | None | Write code, commit |
| Verification | AUTONOMOUS | None | Validate completion |
| Review | HITL | Accept or reject | Wait for decision |
| Iteration | AUTONOMOUS | None | Retry with feedback |

### Key Design Decisions

1. **Human Controls Entry**: All work enters through human decision (task creation/assignment)
2. **Autonomous Execution**: Once started, coding happens without interruption
3. **Smart Interrupts**: System only asks questions when genuinely ambiguous
4. **Human Controls Exit**: Final review ensures quality gate
5. **Feedback Loop**: Rejection triggers autonomous retry, not manual fixing

### Task State Machine (with Autonomy Markers)

```
    HITL ZONE                          AUTONOMOUS ZONE
    ─────────                          ───────────────
        │                                    │
        ▼                                    ▼
┌───────────────┐                   ┌─────────────────┐
│               │                   │                 │
│    [new]      │───── assign ─────►│   [queued]      │
│               │     (HITL)        │                 │
│  Human creates│                   │  Ready for      │
│  task         │                   │  manager        │
│               │                   │                 │
└───────────────┘                   └────────┬────────┘
                                             │
                                        start (HITL)
                                             │
                                             ▼
                                    ┌─────────────────┐
                                    │                 │
                                    │ [in_progress]   │◄──────────┐
                                    │                 │           │
                                    │  AUTONOMOUS     │      (retry with
                                    │  execution      │       feedback)
                                    │                 │           │
                                    └───────┬─────────┘           │
                                            │                     │
                          ┌─────────────────┼─────────────────┐   │
                          │                 │                 │   │
                          ▼                 ▼                 ▼   │
                 ┌─────────────┐   ┌─────────────┐   ┌───────────────┐
                 │             │   │             │   │               │
                 │[awaiting_   │   │[needs_help] │   │   [done]      │
                 │  input]     │   │             │   │               │
                 │             │   │  HITL:      │   │  HITL:        │
                 │  HITL:      │   │  Human must │   │  Human        │
                 │  Answer     │   │  intervene  │   │  reviews      │
                 │  questions  │   │             │   │               │
                 │             │   │             │   │               │
                 └──────┬──────┘   └─────────────┘   └───────┬───────┘
                        │                                    │
                   (answer)                            (accept/reject)
                        │                                    │
                        └───────────► [queued] ◄─────────────┘
                                     (if rejected)
```

**States with Autonomy Mode**:

| State | Mode | Description | Human Action Required |
|-------|------|-------------|----------------------|
| `new` | HITL | Task just created | Assign to queue |
| `queued` | HITL→AUTO | Ready for work | Click "Start" to begin |
| `in_progress` | AUTONOMOUS | Manager/subagent working | None - system runs independently |
| `awaiting_input` | HITL | Manager needs clarification | Answer brainstorming questions |
| `needs_help` | HITL | Stuck after multiple failures | Provide guidance or manual fix |
| `done` | HITL | Work complete | Review and accept/reject |

**Transition Triggers**:

| Transition | Trigger | Mode |
|------------|---------|------|
| new → queued | Human clicks "Assign" | HITL |
| queued → in_progress | Human clicks "Start" | HITL |
| in_progress → awaiting_input | Manager detects ambiguity | AUTO→HITL |
| awaiting_input → queued | Human answers question | HITL→AUTO |
| in_progress → needs_help | 3+ repeated blockers | AUTO→HITL |
| in_progress → done | Subagent completes work | AUTONOMOUS |
| done → queued | Human rejects with feedback | HITL→AUTO |
| done → (removed) | Human accepts | HITL (final) |

---

## 3. Database Schema

### Tasks Table
```sql
CREATE TABLE tasks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  workspaceId INTEGER NOT NULL REFERENCES workspaces(id),
  sourceRemoteId INTEGER REFERENCES gitRemotes(id),  -- If from GitHub issue
  sourceRef TEXT,                                     -- Issue #, MR number
  title TEXT NOT NULL,
  description TEXT,
  state TEXT DEFAULT 'new',  -- new|queued|in_progress|awaiting_input|needs_help|done
  priority INTEGER DEFAULT 0,
  agentPlan TEXT,
  branchName TEXT,           -- Feature branch created
  completionCriteria TEXT,
  iterationCount INTEGER DEFAULT 0,
  maxIterations INTEGER,     -- Override global default
  feedback TEXT,             -- Human feedback on retry

  -- Brainstorming fields
  waitingQuestion TEXT,      -- Question manager is asking
  waitingContext TEXT,       -- Conversation context
  waitingSince TEXT,         -- When started waiting

  assignedAt TEXT,
  updatedAt TEXT
);
```

### Workspaces Table
```sql
CREATE TABLE workspaces (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT UNIQUE NOT NULL,
  repoPath TEXT NOT NULL,        -- Absolute path to repo
  linkedDirs TEXT,               -- JSON array of linked directories
  buildCommand TEXT,
  agentConfig TEXT,              -- JSON AgentConfig
  createdAt TEXT
);
```

### GitRemotes Table
```sql
CREATE TABLE gitRemotes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  workspaceId INTEGER REFERENCES workspaces(id),  -- NULL = global
  provider TEXT NOT NULL,        -- github|gitlab|bitbucket|gitea|custom
  name TEXT NOT NULL,
  url TEXT NOT NULL,
  owner TEXT,
  repo TEXT,
  credentials TEXT,              -- Encrypted JSON {token: string}
  isDefault INTEGER DEFAULT 0,
  createdAt TEXT,
  updatedAt TEXT
);
```

### Logs Table
```sql
CREATE TABLE logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  workspaceId INTEGER,
  taskId INTEGER,
  type TEXT NOT NULL,            -- agent|test|approval|error|health
  message TEXT NOT NULL,
  details TEXT,                  -- JSON
  createdAt TEXT
);
```

### Settings Table
```sql
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT,
  updatedAt TEXT
);
```

---

## 4. MCP Tools Reference

### Task Tools (`/src/mcp/tools/tasks.ts`)

| Tool | Purpose | Parameters |
|------|---------|------------|
| `task_list` | List tasks with filters | `workspaceId?`, `state?` |
| `task_get` | Get full task details | `taskId` (required) |
| `task_create` | Create new task | `workspaceId`, `title`, `description`, `priority?` |
| `task_update` | Update task properties | `taskId`, `priority?`, `state?`, `notes?` |
| `task_assign` | Move task new→queued | `taskId` |

### Agent Tools (`/src/mcp/tools/agents.ts`)

| Tool | Purpose | Parameters |
|------|---------|------------|
| `agent_list` | Get manager status | (none) |
| `agent_start` | Send task to manager | `taskId` |
| `agent_stop` | Stop task in progress | `taskId` |

### Monitoring Tools (`/src/mcp/tools/monitoring.ts`)

| Tool | Purpose | Parameters |
|------|---------|------------|
| `logs_get` | Get recent logs | `taskId?`, `type?`, `limit?` |
| `status_overview` | Dashboard overview | (none) |
| `workspace_list` | List all workspaces | (none) |
| `workspace_create` | Create workspace | `name`, `repoPath`, `linkedDirs?`, `buildCommand?` |
| `workspace_delete` | Delete workspace | `workspaceId` |
| `workspace_update` | Update workspace | `workspaceId`, `linkedDirs?`, `buildCommand?` |
| `github_list_all_issues` | List GitHub issues | (none) |
| `github_create_task_from_issue` | Create task from issue | `workspaceId`, `issueNumber`, `priority?` |

---

## 5. Manager Orchestration

### Persistent Manager Architecture

**Key File**: `/src/lib/manager/persistent-manager.ts`

The manager is a single persistent Claude Code PTY session that:
1. Runs as background process via `claude --dangerously-skip-permissions`
2. Has session ID `"manager"` (constant)
3. Tracks status via file `.bot-hq/.manager-status`
4. Orchestrates all subagent work

### Startup Flow
```
1. PersistentManager.start() called
2. Initialize .bot-hq directory structure
3. Spawn PTY session with Claude Code
4. Wait for idle state (4s no output)
5. Send startup initialization command
6. Manager executes:
   - status_overview MCP tool
   - task_list to find orphaned tasks
   - task_update to reset stuck in_progress tasks
```

### Command Sending Pattern
```typescript
// Text sent first, then wait, then Enter
pty.write(text);
await sleep(300);
pty.write('\u000d');  // Carriage return
```

### PTY Configuration
- Buffer: 100KB per session
- Terminal: 120 cols × 30 rows
- Shell: zsh (macOS default)
- Environment: TERM=xterm-256color

---

## 6. Context Handoff

### Manager → Subagent Prompt Structure

```markdown
# Task: {title}

## Workspace
- Path: {repoPath}
- Name: {workspaceName}

## Your Mission
{description}

## REQUIRED STEPS

### 1. Create Feature Branch
git checkout -b task/{id}-{slug}

### 2. Create PROGRESS.md
File: .bot-hq/workspaces/{name}/tasks/{id}/PROGRESS.md

### 3. Do the Work
[Implementation details from task description]

### 4. Update PROGRESS.md
Mark completed items, note any blockers

### 5. Commit Changes
git commit -m "feat(task-{id}): {description}"

### 6. Report Completion
Work complete. Manager handles verification.
```

### Brainstorming Format
```
[AWAITING_INPUT:{taskId}]
Question: What authentication method should we use?
Options:
1. JWT tokens with refresh
2. Session-based with cookies
3. OAuth2 with external provider
[/AWAITING_INPUT]
```

Parser detects `[AWAITING_INPUT:{taskId}]` markers and sets task state to `awaiting_input`.

---

## 7. File Structure

### Project Structure
```
bot-hq/
├── src/
│   ├── app/                    # Next.js pages and API routes
│   │   ├── api/                # API endpoints
│   │   │   ├── tasks/          # Task CRUD
│   │   │   ├── workspaces/     # Workspace management
│   │   │   ├── terminal/       # Terminal/PTY management
│   │   │   ├── git-remote/     # Git integration
│   │   │   └── manager/        # Manager control
│   │   ├── claude/             # Manager terminal page
│   │   ├── pending/            # Brainstorming tasks page
│   │   ├── workspaces/         # Workspace management pages
│   │   ├── git-remote/         # Git remote config page
│   │   ├── logs/               # Log viewer pages
│   │   └── settings/           # Settings pages
│   ├── components/             # React components
│   │   ├── taskboard/          # Task list, cards, dialogs
│   │   ├── settings/           # Settings UI components
│   │   └── ui/                 # shadcn/ui base components
│   ├── lib/                    # Core logic
│   │   ├── db/                 # Database schema and connection
│   │   ├── manager/            # Persistent manager logic
│   │   ├── bot-hq/             # .bot-hq file management
│   │   └── agents/             # Agent configuration
│   └── mcp/                    # MCP server
│       ├── server.ts           # MCP entry point
│       └── tools/              # Tool definitions
├── data/                       # SQLite database
├── drizzle/                    # Database migrations
└── docs/                       # Documentation
```

### .bot-hq Directory Structure
```
~/.bot-hq/
├── MANAGER_PROMPT.md           # Customizable manager instructions
├── QUEUE.md                    # Task queue status
├── .manager-status             # File-based status flag
└── workspaces/
    └── {workspaceName}/
        ├── WORKSPACE.md        # Project context
        ├── STATE.md            # Current state tracking
        └── tasks/
            └── {taskId}/
                └── PROGRESS.md # Task work log
```

---

## 8. Key Files Reference

### Core Logic
| File | Purpose |
|------|---------|
| `src/lib/manager/persistent-manager.ts` | Manager lifecycle and control |
| `src/lib/pty-manager.ts` | PTY/terminal session management |
| `src/lib/bot-hq/index.ts` | .bot-hq file structure management |
| `src/lib/bot-hq/templates.ts` | Default prompts and templates |
| `src/lib/settings.ts` | Settings management |

### Database
| File | Purpose |
|------|---------|
| `src/lib/db/schema.ts` | All table definitions |
| `src/lib/db/index.ts` | Drizzle connection (lazy-loaded) |

### MCP Server
| File | Purpose |
|------|---------|
| `src/mcp/server.ts` | MCP server setup |
| `src/mcp/tools/tasks.ts` | Task management tools |
| `src/mcp/tools/agents.ts` | Agent control tools |
| `src/mcp/tools/monitoring.ts` | Workspace and monitoring |

### API Routes
| File | Purpose |
|------|---------|
| `src/app/api/terminal/manager/route.ts` | Manager session control |
| `src/app/api/tasks/route.ts` | Task CRUD |
| `src/app/api/tasks/[id]/assign/route.ts` | Task assignment |
| `src/app/api/workspaces/route.ts` | Workspace CRUD |
| `src/app/api/git-remote/route.ts` | Git remote config |

### UI Pages
| File | Purpose |
|------|---------|
| `src/app/page.tsx` | Taskboard (main dashboard) |
| `src/app/claude/page.tsx` | Manager terminal |
| `src/app/pending/page.tsx` | Brainstorming tasks |
| `src/app/workspaces/page.tsx` | Workspace management |
| `src/app/git-remote/page.tsx` | Git remote setup |

---

## 9. Configuration

### Agent Config Type
```typescript
interface AgentConfig {
  approvalRules: string[];      // Commands requiring approval
  blockedCommands: string[];    // Explicitly blocked
  customInstructions: string;   // Workspace-specific instructions
  allowedPaths: string[];       // Filesystem restrictions
}
```

### Default Config
```typescript
{
  approvalRules: ["git push", "git force-push", "npm publish"],
  blockedCommands: ["rm -rf /", "sudo rm"],
  customInstructions: "",
  allowedPaths: []
}
```

### Environment Variables
| Variable | Purpose | Default |
|----------|---------|---------|
| `BOT_HQ_SCOPE` | Working directory | `/Users/gregoryerrl/Projects` |
| `BOT_HQ_URL` | MCP server URL | `http://localhost:7890` |
| `SHELL` | Shell to use | `zsh` |

---

## 10. UI Routes

| Route | Page | Purpose |
|-------|------|---------|
| `/` | Taskboard | Task management dashboard |
| `/pending` | Pending | Brainstorming/awaiting input tasks |
| `/workspaces` | Workspaces | Workspace list & management |
| `/git-remote` | Git Remote | Configure GitHub/GitLab remotes |
| `/claude` | Claude | Manager terminal & chat |
| `/docs` | Docs | Documentation viewer |
| `/logs` | Logs | System logs |
| `/settings` | Settings | Global settings |

---

## 11. Current System State

**Last Updated**: 2026-01-24 (Setup improvements)

```
Manager Status: Running (Input needed)
Session ID: manager

Tasks:
  - New: 0
  - Queued: 0
  - In Progress: 0
  - Needs Help: 0
  - Done: 7

Workspaces: 9 configured
  - bcc-bi
  - bot-hq
  - isekai.dev
  - nokona-configurator
  - postgres-toolset-poc
  - nokona.com
  - helena-mt
  - polymekanix
  - test-workspace

All UI Issues: RESOLVED (2026-01-23)
  - Taskboard displays tasks correctly
  - Workspaces page displays workspaces correctly
  - Chat view renders clean output
```

---

## Appendix A: Browser Test Report (2026-01-23)

### Test Results Summary

**MCP API Layer**: All Passed
- Task creation, assignment, retrieval, update all working
- Workspace listing working
- Status overview working

**Browser UI Issues Found**:

| Issue | Severity | Status |
|-------|----------|--------|
| Sidebar navigation blocked | High | Open |
| Page stuck in "Rendering" | High | Open |
| Terminal input unresponsive | Medium | Open |
| Manager not starting via UI | High | Open |

### Recommendations

1. **Investigate "Rendering" Loop** - Check `src/app/claude/page.tsx` for infinite useEffect loops
2. **Fix Manager Session Management** - `src/lib/manager/persistent-manager.ts`
3. **Review Navigation Component** - `src/components/sidebar.tsx`

### Observed Successful Workflow (Task 6)

The terminal history shows Task 6 completed successfully:
1. Manager received task
2. Subagent spawned (10 tool uses, 20.2k tokens, 47s)
3. Created branch `task/6-test-auto-submit`
4. Added verification files
5. Committed changes
6. Task marked done via MCP

---

## Appendix B: Common Commands

### Setup Commands
```bash
npm run setup              # Interactive setup wizard
npm run setup:quick        # Non-interactive setup with defaults
npm run setup:verify       # Verify installation health
npm run setup:reset        # Reset database and state

# Setup with options
npm run setup -- --help          # Show all options
npm run setup -- --port 8080     # Custom port
npm run setup -- --scope ~/code  # Custom projects directory
npm run setup -- -y --skip-mcp   # Quick setup, skip MCP
```

### Troubleshooting Commands
```bash
npm run doctor             # Diagnose common issues
npm run doctor:fix         # Diagnose and auto-fix issues
```

### Development Commands
```bash
npm run local        # Start dev server (uses BOT_HQ_PORT or 7890)
npm run dev          # Alias for local
npm run build        # Build for production
npm run start        # Start production server
npm run mcp          # Run MCP server standalone
npm run lint         # Run ESLint
```

### Database Commands
```bash
npm run db:push      # Push schema changes to database
npm run db:studio    # Open Drizzle Studio (database browser)
npm run db:generate  # Generate migration files
npm run db:migrate   # Run migrations
```

### MCP Tools (via Claude)
```
# List tasks
task_list

# Create task
task_create workspaceId=15 title="..." description="..."

# Assign task
task_assign taskId=7

# Start agent on task
agent_start taskId=7

# Check status
status_overview
```

### Environment Variables
| Variable | Default | Description |
|----------|---------|-------------|
| `ANTHROPIC_API_KEY` | (required) | API key for Claude agents |
| `BOT_HQ_PORT` | 7890 | Server port |
| `BOT_HQ_URL` | http://localhost:7890 | Server URL (for MCP) |
| `BOT_HQ_SCOPE` | ~/Projects | Working directory for agents |
| `BOT_HQ_SHELL` | $SHELL | Shell for terminal sessions |
| `BOT_HQ_MAX_ITERATIONS` | 3 | Max task retry iterations |
