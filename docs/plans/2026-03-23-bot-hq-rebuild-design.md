# Bot-HQ Rebuild — AI-Assisted Task Management Tool

**Date:** 2026-03-23
**Status:** Approved

## Overview

Bot-hq is a general-purpose, AI-assisted task management tool. It helps humans plan, organize, and understand their work across any domain — software engineering, learning, general tasks. The human does the actual work outside bot-hq.

**Core loop:**
1. Human creates projects and tasks in bot-hq
2. AI (Claude Code headless) helps research, organize, and generate visualizers via a command bar
3. Human works on tasks externally (e.g., Claude Code in terminal, manual work)
4. External Claude Code can interact with bot-hq via MCP tools

## Architecture

- **Next.js** web app (existing)
- **SQLite + Drizzle ORM** for persistence (existing)
- **Claude Code headless** (`claude --print`) for AI capabilities
- **MCP server** exposes all tools — used by both the command bar (via headless) and external Claude Code terminal
- **React Flow** for interactive visualizer diagrams (existing components)

## Pages & Layout

### Sidebar Navigation
- Dashboard (/)
- Projects (/projects)

### Pages

**`/` — Dashboard**
- Command bar at top (Cmd+K shortcut)
- Recent projects cards
- Recent tasks across all projects
- Quick stats

**`/projects` — Project List**
- Grid/list of project cards (name, task counts by state, last updated)
- "+ New Project" button
- Filter by status (active/archived)

**`/projects/[id]` — Project Detail**
Three tabs:
- **Tasks** — Nested list with subtask indentation. State toggles, priority badges, due dates, tags. "+ Add Task" button. Filter/sort by state, priority, tags, due date.
- **Visualizers** — Grid of diagram cards (FlowCard component). "+ New Visualizer" button.
- **Overview** — Project description, notes, repo path, stats summary.

**`/projects/[id]/visualizer/[diagramId]` — Visualizer Canvas**
- Full-screen React Flow canvas (FlowCanvas component)
- Back button to project
- Node click opens detail dialog

### Command Bar
- Fixed at top of every page, activated by click or Cmd+K
- Natural language input sent to Claude Code headless
- Claude Code headless has bot-hq MCP tools available
- One-shot: response appears inline, no conversation thread
- Context-aware: when on a project page, defaults to that project
- Implementation: Next.js API route (`/api/command`) spawns `claude --print` with MCP config

## Data Model

### Projects
```sql
projects (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT NOT NULL UNIQUE,
  description TEXT,
  repo_path   TEXT,              -- nullable, for codebase projects
  status      TEXT NOT NULL DEFAULT 'active',  -- active | archived
  notes       TEXT,              -- general context about the project
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
)
```

### Tasks
```sql
tasks (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  parent_task_id  INTEGER REFERENCES tasks(id) ON DELETE CASCADE,  -- nullable, for subtasks
  title           TEXT NOT NULL,
  description     TEXT,
  state           TEXT NOT NULL DEFAULT 'todo',  -- todo | in_progress | done | blocked
  priority        INTEGER DEFAULT 0,             -- 0-3
  tags            TEXT,                          -- JSON array
  due_date        INTEGER,                       -- nullable timestamp
  "order"         INTEGER DEFAULT 0,             -- for manual sorting
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL
)
-- Indexes: project_id, state, parent_task_id, due_date
```

### Task Notes
```sql
task_notes (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id    INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  content    TEXT NOT NULL,
  created_at INTEGER NOT NULL
)
-- Index: task_id
```

### Task Dependencies
```sql
task_dependencies (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id            INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  depends_on_task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  created_at         INTEGER NOT NULL
)
-- Index: task_id, depends_on_task_id
```

### Diagrams
```sql
diagrams (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  title      TEXT NOT NULL,
  template   TEXT,              -- nullable: "codebase", "roadmap", "process", etc.
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)
-- Index: project_id
```

### Diagram Nodes
```sql
diagram_nodes (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  diagram_id  INTEGER NOT NULL REFERENCES diagrams(id) ON DELETE CASCADE,
  group_id    INTEGER REFERENCES diagram_groups(id) ON DELETE SET NULL,
  node_type   TEXT NOT NULL DEFAULT 'default',  -- user-defined: "component", "api", "step", etc.
  label       TEXT NOT NULL,
  description TEXT,
  metadata    TEXT,              -- JSON: files, codeSnippets, custom fields
  position_x  REAL NOT NULL DEFAULT 0,
  position_y  REAL NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL
)
-- Index: diagram_id, group_id
```

### Diagram Edges
```sql
diagram_edges (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  diagram_id     INTEGER NOT NULL REFERENCES diagrams(id) ON DELETE CASCADE,
  source_node_id INTEGER NOT NULL REFERENCES diagram_nodes(id) ON DELETE CASCADE,
  target_node_id INTEGER NOT NULL REFERENCES diagram_nodes(id) ON DELETE CASCADE,
  label          TEXT,
  created_at     INTEGER NOT NULL
)
-- Index: diagram_id
```

### Diagram Groups
```sql
diagram_groups (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  diagram_id INTEGER NOT NULL REFERENCES diagrams(id) ON DELETE CASCADE,
  label      TEXT NOT NULL,
  color      TEXT NOT NULL DEFAULT '#6b7280',  -- hex color
  created_at INTEGER NOT NULL
)
-- Index: diagram_id
```

## MCP Tools (29 total)

### Project Tools (6)
| Tool | Description |
|------|-------------|
| `project_list` | List projects, filterable by status |
| `project_get` | Get project with stats (task counts by state, diagram count) |
| `project_create` | Create project (name, description, repoPath?) |
| `project_update` | Update project fields |
| `project_delete` | Delete project (cascade) |
| `project_search` | Search projects by keyword |

### Task Tools (9)
| Tool | Description |
|------|-------------|
| `task_list` | List tasks for project (filter by state, priority, parent, due date) |
| `task_get` | Get task with subtasks and notes |
| `task_create` | Create task (title, description, projectId, parentTaskId?, priority?, tags?, dueDate?) |
| `task_update` | Update task fields (state, priority, tags, dueDate, description) |
| `task_delete` | Delete task (cascade subtasks) |
| `task_move` | Reparent or reorder a task |
| `task_add_note` | Append timestamped note to a task |
| `task_add_dependency` | Mark task as blocked by another task |
| `task_search` | Search tasks by keyword across all projects |

### Diagram Tools (13)
| Tool | Description |
|------|-------------|
| `diagram_list` | List diagrams for a project |
| `diagram_get` | Get full diagram (nodes, edges, groups) |
| `diagram_create` | Create diagram (projectId, title, template?) |
| `diagram_delete` | Delete diagram |
| `diagram_add_node` | Add node (diagramId, label, nodeType, description, metadata?, groupId?, position?) |
| `diagram_update_node` | Update node data (label, description, metadata, position, groupId) |
| `diagram_remove_node` | Remove node + connected edges |
| `diagram_add_edge` | Connect two nodes (sourceId, targetId, label?) |
| `diagram_remove_edge` | Remove a connection |
| `diagram_add_group` | Create node group/cluster (diagramId, label, color) |
| `diagram_bulk_add` | Batch add nodes + edges + groups (for initial generation) |
| `diagram_query` | Search nodes by keyword, type, group, or file reference |
| `diagram_auto_layout` | Recalculate node positions server-side |

### Utility (1)
| Tool | Description |
|------|-------------|
| `summary` | Status summary of a project or all projects |

## API Routes

### Command
- `POST /api/command` — Send natural language to Claude Code headless, returns response

### Projects
- `GET /api/projects` — List projects
- `POST /api/projects` — Create project
- `GET /api/projects/[id]` — Get project
- `PATCH /api/projects/[id]` — Update project
- `DELETE /api/projects/[id]` — Delete project

### Tasks
- `GET /api/projects/[id]/tasks` — List tasks for project
- `POST /api/projects/[id]/tasks` — Create task
- `GET /api/tasks/[id]` — Get task
- `PATCH /api/tasks/[id]` — Update task
- `DELETE /api/tasks/[id]` — Delete task
- `POST /api/tasks/[id]/notes` — Add note
- `POST /api/tasks/[id]/dependencies` — Add dependency
- `PATCH /api/tasks/[id]/move` — Reparent/reorder

### Diagrams
- `GET /api/projects/[id]/diagrams` — List diagrams
- `POST /api/projects/[id]/diagrams` — Create diagram
- `GET /api/diagrams/[id]` — Get diagram (assembled React Flow format)
- `DELETE /api/diagrams/[id]` — Delete diagram
- `POST /api/diagrams/[id]/nodes` — Add node
- `PATCH /api/diagrams/[id]/nodes/[nodeId]` — Update node
- `DELETE /api/diagrams/[id]/nodes/[nodeId]` — Remove node
- `POST /api/diagrams/[id]/edges` — Add edge
- `DELETE /api/diagrams/[id]/edges/[edgeId]` — Remove edge
- `POST /api/diagrams/[id]/groups` — Add group
- `POST /api/diagrams/[id]/bulk` — Bulk add
- `GET /api/diagrams/[id]/query` — Query nodes
- `POST /api/diagrams/[id]/layout` — Auto layout

### Search
- `GET /api/search` — Global search across projects, tasks, diagram nodes

## Component Changes

### Existing (adapt)
- **FlowCanvas** — Same React Flow canvas. Receives assembled `{ nodes, edges }` from API. No changes to drag/interaction logic.
- **FlowNode** — Make `nodeType` and color dynamic instead of hardcoded `LAYER_STYLES`. Color comes from the node's group color. Type label comes from `nodeType` field.
- **FlowCard** — Preview card for diagram list. Gets node/edge/group counts from API instead of parsing JSON blob.
- **NodeDetailDialog** — Dynamic metadata display instead of fixed `files[]` structure. Shows arbitrary key-value pairs from metadata JSON.

### New
- **CommandBar** — Text input with Cmd+K activation, inline response display.
- **ProjectCard** — Card for project list (name, description, task counts, last updated).
- **TaskList** — Nested task list with subtask indentation, state toggles, priority/tag badges, due dates.
- **TaskItem** — Single task row with inline actions (state toggle, edit, delete).
- **AddTaskDialog** — Form for creating/editing tasks.
- **AddProjectDialog** — Form for creating/editing projects.
- **ProjectTabs** — Tab container for Tasks/Visualizers/Overview within project detail.

## Tech Stack (unchanged)
- Next.js 16 (App Router)
- SQLite + Drizzle ORM
- React Flow (@xyflow/react)
- shadcn/ui components
- Tailwind CSS
- Claude Code headless for AI
- MCP SDK for tool server
