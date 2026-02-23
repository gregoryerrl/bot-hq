# Workspace Visualization — Design Document

## Overview

Add interactive flow diagrams to bot-hq workspaces so visual users can understand, navigate, and brainstorm on their codebase. Diagrams are generated and maintained by a new **Visualizer Bot** and rendered using **React Flow** on a dedicated page per workspace.

## Architecture: Four-Role Bot System

The Manager's responsibilities are split across four roles:

| Role | Type | Responsibility |
|------|------|----------------|
| **Manager** | Claude Code PTY instance | Receives commands, makes decisions, spawns bots |
| **Assistant Manager Bot** | Subagent | Health/status checks, `.bot-hq/` context management |
| **SW Engineer Bot** | Subagent | Implements tasks on feature branches |
| **Visualizer Bot** | Subagent | Generates and updates flow diagrams |

All bots are subagents spawned by the Manager. Manager remains the only PTY session.

## Diagram Data Model

### DB Table

```sql
diagrams: id, workspaceId, title, flowData (JSON), createdAt, updatedAt
```

### flowData JSON Structure (React Flow format)

**Nodes:**
- `id` — Unique node identifier
- `label` — Step description (e.g., "Validate registration input")
- `layer` — One of: `ux`, `frontend`, `backend`, `database`
- `description` — 1-2 sentence explanation of what happens at this step
- `files` — Array of `{ path, lineStart, lineEnd }` referencing involved source files
- `codeSnippets` — Key functions/lines at this step
- `position` — `{ x, y }` for React Flow layout
- `activeTask` — Optional `{ taskId, agentSessionId, state }` when a bot is working on files that touch this node

**Edges:**
- `id` — Unique edge identifier
- `source` — Source node ID
- `target` — Target node ID
- `label` — Context label (e.g., "POST /api/register")
- `condition` — Branch condition if applicable (e.g., "if valid", "if invalid")

### Layer Colors

| Layer | Color | Represents |
|-------|-------|------------|
| UX | Blue | User actions (clicks, fills form, sees error) |
| Frontend | Green | Client-side logic (validates, sends request, updates state) |
| Backend | Red | Server logic (receives, processes, queries) |
| Database | Purple | Data operations (INSERT, SELECT, UPDATE) |

## Diagram Content: User Flow Diagrams

Each diagram represents one **user-facing flow** traced end-to-end through the stack. Examples:

- "User Registration" — UX: fills form → FE: validates + POST → BE: hashes password + creates user → DB: INSERT → BE: returns session → FE: stores token + redirects
- "Task Creation" — UX: clicks create → FE: opens dialog + POST → BE: validates + inserts → DB: INSERT → BE: returns task → FE: updates list

One diagram per flow. Each workspace may have multiple flow diagrams.

## UI Design

### Flow List Page: `/workspaces/[id]/diagram`

- Header with workspace name
- Grid of **flow cards**, each showing:
  - Flow title
  - Layer breakdown (colored dots showing node count per layer)
  - Live status badge: idle (none), rotating wrench (bot working), clock/pending (awaiting review)
  - Last updated timestamp
- Clicking a card opens the full React Flow canvas

### React Flow Canvas

- Full-screen canvas with left-to-right layout
- **Node styling:** Rounded rectangles with layer color as left border + subtle background tint. Status icon in top-right (wrench for working, pending icon for review).
- **Edge styling:** Labels for context, condition labels on branches
- Users can drag nodes to reposition (positions persist to DB)
- Zoom and pan supported natively by React Flow

### Hover Tooltip (on node hover)

- Step description
- Layer label with color
- Files involved (e.g., `src/app/api/auth/register/route.ts:14-38`)
- If wrench/pending: which task and bot are active

### Click Dialog (on node click)

- Everything from hover, expanded
- Code snippets (key functions at this step)
- Connected nodes (what leads here, what follows)
- If wrench: "View Logs" button linking to `/logs?taskId=X`
- If pending: "Review Changes" button linking to `/pending`

### Live Status on Flow Cards

Flow cards reflect the aggregate status of their nodes:
- Any node has wrench → card shows wrench badge
- Any node has pending → card shows pending badge
- Both can appear simultaneously if multiple tasks touch the same flow

## Bot Spawning Flow

### On Manager Startup

1. Manager spawns **Assistant Manager Bot**
2. Assistant Manager does health checks — workspace status, git state, checks diagram staleness
3. Reports back: "Workspace X has no diagrams" or "Diagrams outdated"
4. Manager spawns **Visualizer Bot** for any workspace needing diagrams

### On Task Command (`TASK {id}`)

1. Manager spawns **Assistant Manager Bot** — prepares `.bot-hq/` context, gets task details
2. Manager spawns **SW Engineer Bot** and **Visualizer Bot** in parallel
3. SW Engineer implements the task
4. Visualizer marks affected nodes with wrench icon

### On SW Engineer Completion

1. Manager does health check (via Assistant Manager Bot)
2. Calls `task_update(done + branchName)`
3. Spawns **Visualizer Bot** — updates diagram content from git diff, swaps wrench to pending icon
4. Auto-clear fires

### On Review Accept

1. Commits on feature branch, clears branchName
2. Clears pending icon on affected diagram nodes

### On Review Delete/Retry

- Delete: clears pending icon, diagram unchanged
- Retry: swaps pending icon back to wrench when task restarts

## API Routes

| Method | Route | Purpose |
|--------|-------|---------|
| GET | `/api/diagrams?workspaceId={id}` | List all flow diagrams for a workspace |
| GET | `/api/diagrams/[id]` | Get single diagram with full nodes/edges JSON |
| POST | `/api/diagrams` | Create new diagram |
| PUT | `/api/diagrams/[id]` | Update diagram (bot or user repositioning) |
| DELETE | `/api/diagrams/[id]` | Remove a diagram |

## MCP Tools (for Visualizer Bot)

| Tool | Parameters | Purpose |
|------|-----------|---------|
| `diagram_list` | `{ workspaceId }` | List diagrams for a workspace |
| `diagram_get` | `{ diagramId }` | Get full diagram JSON |
| `diagram_create` | `{ workspaceId, title, flowData }` | Create a new flow diagram |
| `diagram_update` | `{ diagramId, flowData }` | Update an existing diagram |

## Implementation Notes

- React Flow is the rendering library (`@xyflow/react`)
- Diagram JSON stored as text blob in SQLite (via Drizzle)
- Visualizer Bot preserves user-adjusted node positions when updating content
- Node `activeTask` field is ephemeral — derived from cross-referencing task file changes with node `files` arrays
- The Visualizer Bot prompt template instructs it to trace user-facing flows end-to-end, categorize each step by layer, and output the React Flow JSON schema
