# Bot-HQ Rebuild Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rebuild bot-hq as an AI-assisted task management tool with projects, hierarchical tasks, interactive visualizers, a context-aware command bar, and full MCP integration.

**Architecture:** Next.js App Router with SQLite/Drizzle. 8 DB tables (projects, tasks, task_notes, task_dependencies, diagrams, diagram_nodes, diagram_edges, diagram_groups). 29 MCP tools shared between web UI command bar (via Claude Code headless) and external Claude Code terminal. React Flow for visualizer diagrams.

**Tech Stack:** Next.js 16, Drizzle ORM, SQLite, React Flow, shadcn/ui, Tailwind CSS, MCP SDK, Claude Code headless

**Design doc:** `docs/plans/2026-03-23-bot-hq-rebuild-design.md`

---

## Phase 1: Database Schema & Migration

### Task 1: Projects table

**Files:**
- Modify: `src/lib/db/schema.ts`

**Step 1: Replace workspaces with projects table in schema**

Replace the existing `workspaces` table and add `projects`:

```typescript
// In src/lib/db/schema.ts - replace entire file contents

import { sqliteTable, text, integer, real, index } from "drizzle-orm/sqlite-core";

// Projects - top-level container for any initiative
export const projects = sqliteTable("projects", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull().unique(),
  description: text("description"),
  repoPath: text("repo_path"),
  status: text("status", { enum: ["active", "archived"] })
    .notNull()
    .default("active"),
  notes: text("notes"),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});

export type Project = typeof projects.$inferSelect;
export type NewProject = typeof projects.$inferInsert;
```

**Step 2: Verify TypeScript compiles**

Run: `npx tsc --noEmit 2>&1 | head -20`
Expected: Errors about missing references to old schema exports (diagrams, workspaces) — that's fine, we'll fix in next tasks.

**Step 3: Commit**

```bash
git add src/lib/db/schema.ts
git commit -m "feat: replace workspaces with projects table"
```

---

### Task 2: Tasks, task_notes, task_dependencies tables

**Files:**
- Modify: `src/lib/db/schema.ts`

**Step 1: Add tasks, task_notes, task_dependencies tables**

Append to `src/lib/db/schema.ts` after the projects table:

```typescript
// Tasks - belong to a project, support subtasks via parentTaskId
export const tasks = sqliteTable("tasks", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  projectId: integer("project_id")
    .notNull()
    .references(() => projects.id, { onDelete: "cascade" }),
  parentTaskId: integer("parent_task_id").references((): any => tasks.id, { onDelete: "cascade" }),
  title: text("title").notNull(),
  description: text("description"),
  state: text("state", {
    enum: ["todo", "in_progress", "done", "blocked"],
  })
    .notNull()
    .default("todo"),
  priority: integer("priority").default(0),
  tags: text("tags"), // JSON array
  dueDate: integer("due_date", { mode: "timestamp" }),
  order: integer("order").default(0),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("tasks_project_idx").on(table.projectId),
  index("tasks_state_idx").on(table.state),
  index("tasks_parent_idx").on(table.parentTaskId),
  index("tasks_due_idx").on(table.dueDate),
]);

export type Task = typeof tasks.$inferSelect;
export type NewTask = typeof tasks.$inferInsert;

// Task Notes - append-only log per task
export const taskNotes = sqliteTable("task_notes", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  taskId: integer("task_id")
    .notNull()
    .references(() => tasks.id, { onDelete: "cascade" }),
  content: text("content").notNull(),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("task_notes_task_idx").on(table.taskId),
]);

export type TaskNote = typeof taskNotes.$inferSelect;
export type NewTaskNote = typeof taskNotes.$inferInsert;

// Task Dependencies - join table for blocked-by relationships
export const taskDependencies = sqliteTable("task_dependencies", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  taskId: integer("task_id")
    .notNull()
    .references(() => tasks.id, { onDelete: "cascade" }),
  dependsOnTaskId: integer("depends_on_task_id")
    .notNull()
    .references(() => tasks.id, { onDelete: "cascade" }),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("task_deps_task_idx").on(table.taskId),
  index("task_deps_depends_idx").on(table.dependsOnTaskId),
]);

export type TaskDependency = typeof taskDependencies.$inferSelect;
export type NewTaskDependency = typeof taskDependencies.$inferInsert;
```

**Step 2: Commit**

```bash
git add src/lib/db/schema.ts
git commit -m "feat: add tasks, task_notes, task_dependencies tables"
```

---

### Task 3: Diagram tables (relational)

**Files:**
- Modify: `src/lib/db/schema.ts`

**Step 1: Replace old diagrams table with relational diagram tables**

Replace the existing `diagrams` export and add node/edge/group tables. Append to schema:

```typescript
// Diagrams - container for a visualizer
export const diagrams = sqliteTable("diagrams", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  projectId: integer("project_id")
    .notNull()
    .references(() => projects.id, { onDelete: "cascade" }),
  title: text("title").notNull(),
  template: text("template"), // "codebase", "roadmap", "process", etc.
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("diagrams_project_idx").on(table.projectId),
]);

export type Diagram = typeof diagrams.$inferSelect;
export type NewDiagram = typeof diagrams.$inferInsert;

// Diagram Groups - clusters of related nodes
export const diagramGroups = sqliteTable("diagram_groups", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  diagramId: integer("diagram_id")
    .notNull()
    .references(() => diagrams.id, { onDelete: "cascade" }),
  label: text("label").notNull(),
  color: text("color").notNull().default("#6b7280"),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("diagram_groups_diagram_idx").on(table.diagramId),
]);

export type DiagramGroup = typeof diagramGroups.$inferSelect;
export type NewDiagramGroup = typeof diagramGroups.$inferInsert;

// Diagram Nodes - individual nodes on the canvas
export const diagramNodes = sqliteTable("diagram_nodes", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  diagramId: integer("diagram_id")
    .notNull()
    .references(() => diagrams.id, { onDelete: "cascade" }),
  groupId: integer("group_id").references(() => diagramGroups.id, { onDelete: "set null" }),
  nodeType: text("node_type").notNull().default("default"),
  label: text("label").notNull(),
  description: text("description"),
  metadata: text("metadata"), // JSON: files, codeSnippets, custom fields
  positionX: real("position_x").notNull().default(0),
  positionY: real("position_y").notNull().default(0),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("diagram_nodes_diagram_idx").on(table.diagramId),
  index("diagram_nodes_group_idx").on(table.groupId),
]);

export type DiagramNode = typeof diagramNodes.$inferSelect;
export type NewDiagramNode = typeof diagramNodes.$inferInsert;

// Diagram Edges - connections between nodes
export const diagramEdges = sqliteTable("diagram_edges", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  diagramId: integer("diagram_id")
    .notNull()
    .references(() => diagrams.id, { onDelete: "cascade" }),
  sourceNodeId: integer("source_node_id")
    .notNull()
    .references(() => diagramNodes.id, { onDelete: "cascade" }),
  targetNodeId: integer("target_node_id")
    .notNull()
    .references(() => diagramNodes.id, { onDelete: "cascade" }),
  label: text("label"),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("diagram_edges_diagram_idx").on(table.diagramId),
]);

export type DiagramEdge = typeof diagramEdges.$inferSelect;
export type NewDiagramEdge = typeof diagramEdges.$inferInsert;
```

**Step 2: Update db/index.ts to remove old exports**

The `db/index.ts` does `export * from "./schema"` which re-exports everything. No change needed — the new schema replaces the old exports.

**Step 3: Delete old data and regenerate migration**

```bash
rm -rf data/bot-hq.db drizzle/
npx drizzle-kit generate
npx drizzle-kit push
```

**Step 4: Fix existing diagram API routes to use new schema**

Delete the old diagram API routes (they reference old schema with `flowData` column):

```bash
rm -rf src/app/api/diagrams
```

These will be rebuilt in Phase 3.

**Step 5: Verify TypeScript compiles**

Run: `npx tsc --noEmit`
Expected: Clean (no errors)

**Step 6: Commit**

```bash
git add -A
git commit -m "feat: add relational diagram tables (nodes, edges, groups)"
```

---

## Phase 2: API Routes

### Task 4: Project API routes

**Files:**
- Create: `src/app/api/projects/route.ts`
- Create: `src/app/api/projects/[id]/route.ts`

**Step 1: Create project list + create route**

```typescript
// src/app/api/projects/route.ts
import { NextResponse } from "next/server";
import { db, projects } from "@/lib/db";
import { eq, like, desc } from "drizzle-orm";

// GET /api/projects?status=active
export async function GET(request: Request) {
  try {
    const { searchParams } = new URL(request.url);
    const status = searchParams.get("status");

    let query = db.select().from(projects).orderBy(desc(projects.updatedAt));
    if (status) {
      query = query.where(eq(projects.status, status as "active" | "archived"));
    }

    const result = await query;
    return NextResponse.json(result);
  } catch (error) {
    console.error("Failed to list projects:", error);
    return NextResponse.json({ error: "Failed to list projects" }, { status: 500 });
  }
}

// POST /api/projects
export async function POST(request: Request) {
  try {
    const { name, description, repoPath, notes } = await request.json();

    if (!name) {
      return NextResponse.json({ error: "name is required" }, { status: 400 });
    }

    const [project] = await db
      .insert(projects)
      .values({ name, description, repoPath, notes })
      .returning();

    return NextResponse.json(project, { status: 201 });
  } catch (error) {
    console.error("Failed to create project:", error);
    return NextResponse.json({ error: "Failed to create project" }, { status: 500 });
  }
}
```

**Step 2: Create project get/update/delete route**

```typescript
// src/app/api/projects/[id]/route.ts
import { NextResponse } from "next/server";
import { db, projects, tasks, diagrams } from "@/lib/db";
import { eq, sql, and } from "drizzle-orm";

// GET /api/projects/[id]
export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const project = await db.query.projects.findFirst({
      where: eq(projects.id, parseInt(id)),
    });

    if (!project) {
      return NextResponse.json({ error: "Project not found" }, { status: 404 });
    }

    // Get task counts by state
    const taskCounts = await db
      .select({
        state: tasks.state,
        count: sql<number>`count(*)`,
      })
      .from(tasks)
      .where(eq(tasks.projectId, parseInt(id)))
      .groupBy(tasks.state);

    const diagramCount = await db
      .select({ count: sql<number>`count(*)` })
      .from(diagrams)
      .where(eq(diagrams.projectId, parseInt(id)));

    return NextResponse.json({
      ...project,
      taskCounts: Object.fromEntries(taskCounts.map((r) => [r.state, r.count])),
      diagramCount: diagramCount[0]?.count || 0,
    });
  } catch (error) {
    console.error("Failed to get project:", error);
    return NextResponse.json({ error: "Failed to get project" }, { status: 500 });
  }
}

// PATCH /api/projects/[id]
export async function PATCH(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const body = await request.json();

    const updates: Record<string, unknown> = { updatedAt: new Date() };
    if (body.name !== undefined) updates.name = body.name;
    if (body.description !== undefined) updates.description = body.description;
    if (body.repoPath !== undefined) updates.repoPath = body.repoPath;
    if (body.status !== undefined) updates.status = body.status;
    if (body.notes !== undefined) updates.notes = body.notes;

    const [updated] = await db
      .update(projects)
      .set(updates)
      .where(eq(projects.id, parseInt(id)))
      .returning();

    if (!updated) {
      return NextResponse.json({ error: "Project not found" }, { status: 404 });
    }

    return NextResponse.json(updated);
  } catch (error) {
    console.error("Failed to update project:", error);
    return NextResponse.json({ error: "Failed to update project" }, { status: 500 });
  }
}

// DELETE /api/projects/[id]
export async function DELETE(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    await db.delete(projects).where(eq(projects.id, parseInt(id)));
    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to delete project:", error);
    return NextResponse.json({ error: "Failed to delete project" }, { status: 500 });
  }
}
```

**Step 3: Verify build**

Run: `npx tsc --noEmit`
Expected: Clean

**Step 4: Commit**

```bash
git add src/app/api/projects/
git commit -m "feat: add project API routes (CRUD)"
```

---

### Task 5: Task API routes

**Files:**
- Create: `src/app/api/projects/[id]/tasks/route.ts`
- Create: `src/app/api/tasks/[id]/route.ts`
- Create: `src/app/api/tasks/[id]/notes/route.ts`
- Create: `src/app/api/tasks/[id]/dependencies/route.ts`
- Create: `src/app/api/tasks/[id]/move/route.ts`

**Step 1: Create task list + create route (scoped to project)**

```typescript
// src/app/api/projects/[id]/tasks/route.ts
import { NextResponse } from "next/server";
import { db, tasks } from "@/lib/db";
import { eq, and, isNull, asc, desc } from "drizzle-orm";

// GET /api/projects/[id]/tasks?state=todo&parent=null
export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { searchParams } = new URL(request.url);
    const state = searchParams.get("state");
    const parent = searchParams.get("parent");

    const conditions = [eq(tasks.projectId, parseInt(id))];
    if (state) conditions.push(eq(tasks.state, state as any));
    if (parent === "null") conditions.push(isNull(tasks.parentTaskId));
    else if (parent) conditions.push(eq(tasks.parentTaskId, parseInt(parent)));

    const result = await db
      .select()
      .from(tasks)
      .where(and(...conditions))
      .orderBy(asc(tasks.order), desc(tasks.createdAt));

    return NextResponse.json(result);
  } catch (error) {
    console.error("Failed to list tasks:", error);
    return NextResponse.json({ error: "Failed to list tasks" }, { status: 500 });
  }
}

// POST /api/projects/[id]/tasks
export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const body = await request.json();

    if (!body.title) {
      return NextResponse.json({ error: "title is required" }, { status: 400 });
    }

    const [task] = await db
      .insert(tasks)
      .values({
        projectId: parseInt(id),
        parentTaskId: body.parentTaskId || null,
        title: body.title,
        description: body.description,
        state: body.state || "todo",
        priority: body.priority || 0,
        tags: body.tags ? JSON.stringify(body.tags) : null,
        dueDate: body.dueDate ? new Date(body.dueDate) : null,
        order: body.order || 0,
      })
      .returning();

    return NextResponse.json(task, { status: 201 });
  } catch (error) {
    console.error("Failed to create task:", error);
    return NextResponse.json({ error: "Failed to create task" }, { status: 500 });
  }
}
```

**Step 2: Create single task get/update/delete**

```typescript
// src/app/api/tasks/[id]/route.ts
import { NextResponse } from "next/server";
import { db, tasks, taskNotes, taskDependencies } from "@/lib/db";
import { eq, asc } from "drizzle-orm";

// GET /api/tasks/[id]
export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const task = await db.query.tasks.findFirst({
      where: eq(tasks.id, parseInt(id)),
    });

    if (!task) {
      return NextResponse.json({ error: "Task not found" }, { status: 404 });
    }

    const subtasks = await db
      .select()
      .from(tasks)
      .where(eq(tasks.parentTaskId, parseInt(id)))
      .orderBy(asc(tasks.order));

    const notes = await db
      .select()
      .from(taskNotes)
      .where(eq(taskNotes.taskId, parseInt(id)))
      .orderBy(asc(taskNotes.createdAt));

    const dependencies = await db
      .select()
      .from(taskDependencies)
      .where(eq(taskDependencies.taskId, parseInt(id)));

    return NextResponse.json({ ...task, subtasks, notes, dependencies });
  } catch (error) {
    console.error("Failed to get task:", error);
    return NextResponse.json({ error: "Failed to get task" }, { status: 500 });
  }
}

// PATCH /api/tasks/[id]
export async function PATCH(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const body = await request.json();

    const updates: Record<string, unknown> = { updatedAt: new Date() };
    if (body.title !== undefined) updates.title = body.title;
    if (body.description !== undefined) updates.description = body.description;
    if (body.state !== undefined) updates.state = body.state;
    if (body.priority !== undefined) updates.priority = body.priority;
    if (body.tags !== undefined) updates.tags = JSON.stringify(body.tags);
    if (body.dueDate !== undefined) updates.dueDate = body.dueDate ? new Date(body.dueDate) : null;
    if (body.order !== undefined) updates.order = body.order;

    const [updated] = await db
      .update(tasks)
      .set(updates)
      .where(eq(tasks.id, parseInt(id)))
      .returning();

    if (!updated) {
      return NextResponse.json({ error: "Task not found" }, { status: 404 });
    }

    return NextResponse.json(updated);
  } catch (error) {
    console.error("Failed to update task:", error);
    return NextResponse.json({ error: "Failed to update task" }, { status: 500 });
  }
}

// DELETE /api/tasks/[id]
export async function DELETE(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    await db.delete(tasks).where(eq(tasks.id, parseInt(id)));
    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to delete task:", error);
    return NextResponse.json({ error: "Failed to delete task" }, { status: 500 });
  }
}
```

**Step 3: Create task notes route**

```typescript
// src/app/api/tasks/[id]/notes/route.ts
import { NextResponse } from "next/server";
import { db, taskNotes } from "@/lib/db";
import { eq, asc } from "drizzle-orm";

// GET /api/tasks/[id]/notes
export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const notes = await db
      .select()
      .from(taskNotes)
      .where(eq(taskNotes.taskId, parseInt(id)))
      .orderBy(asc(taskNotes.createdAt));
    return NextResponse.json(notes);
  } catch (error) {
    return NextResponse.json({ error: "Failed to get notes" }, { status: 500 });
  }
}

// POST /api/tasks/[id]/notes
export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { content } = await request.json();

    if (!content) {
      return NextResponse.json({ error: "content is required" }, { status: 400 });
    }

    const [note] = await db
      .insert(taskNotes)
      .values({ taskId: parseInt(id), content })
      .returning();

    return NextResponse.json(note, { status: 201 });
  } catch (error) {
    return NextResponse.json({ error: "Failed to add note" }, { status: 500 });
  }
}
```

**Step 4: Create task dependencies route**

```typescript
// src/app/api/tasks/[id]/dependencies/route.ts
import { NextResponse } from "next/server";
import { db, taskDependencies } from "@/lib/db";
import { eq } from "drizzle-orm";

// POST /api/tasks/[id]/dependencies
export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { dependsOnTaskId } = await request.json();

    if (!dependsOnTaskId) {
      return NextResponse.json({ error: "dependsOnTaskId is required" }, { status: 400 });
    }

    const [dep] = await db
      .insert(taskDependencies)
      .values({ taskId: parseInt(id), dependsOnTaskId })
      .returning();

    return NextResponse.json(dep, { status: 201 });
  } catch (error) {
    return NextResponse.json({ error: "Failed to add dependency" }, { status: 500 });
  }
}

// DELETE /api/tasks/[id]/dependencies
export async function DELETE(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { dependsOnTaskId } = await request.json();
    await db
      .delete(taskDependencies)
      .where(
        eq(taskDependencies.taskId, parseInt(id))
      );
    return NextResponse.json({ success: true });
  } catch (error) {
    return NextResponse.json({ error: "Failed to remove dependency" }, { status: 500 });
  }
}
```

**Step 5: Create task move route**

```typescript
// src/app/api/tasks/[id]/move/route.ts
import { NextResponse } from "next/server";
import { db, tasks } from "@/lib/db";
import { eq } from "drizzle-orm";

// PATCH /api/tasks/[id]/move
export async function PATCH(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { parentTaskId, order } = await request.json();

    const updates: Record<string, unknown> = { updatedAt: new Date() };
    if (parentTaskId !== undefined) updates.parentTaskId = parentTaskId;
    if (order !== undefined) updates.order = order;

    const [updated] = await db
      .update(tasks)
      .set(updates)
      .where(eq(tasks.id, parseInt(id)))
      .returning();

    if (!updated) {
      return NextResponse.json({ error: "Task not found" }, { status: 404 });
    }

    return NextResponse.json(updated);
  } catch (error) {
    return NextResponse.json({ error: "Failed to move task" }, { status: 500 });
  }
}
```

**Step 6: Verify build**

Run: `npx tsc --noEmit`
Expected: Clean

**Step 7: Commit**

```bash
git add src/app/api/tasks/ src/app/api/projects/
git commit -m "feat: add task API routes (CRUD, notes, dependencies, move)"
```

---

### Task 6: Diagram API routes

**Files:**
- Create: `src/app/api/projects/[id]/diagrams/route.ts`
- Create: `src/app/api/diagrams/[id]/route.ts`
- Create: `src/app/api/diagrams/[id]/nodes/route.ts`
- Create: `src/app/api/diagrams/[id]/nodes/[nodeId]/route.ts`
- Create: `src/app/api/diagrams/[id]/edges/route.ts`
- Create: `src/app/api/diagrams/[id]/edges/[edgeId]/route.ts`
- Create: `src/app/api/diagrams/[id]/groups/route.ts`
- Create: `src/app/api/diagrams/[id]/bulk/route.ts`
- Create: `src/app/api/diagrams/[id]/query/route.ts`
- Create: `src/app/api/diagrams/[id]/layout/route.ts`

**Step 1: Create diagram list + create (scoped to project)**

```typescript
// src/app/api/projects/[id]/diagrams/route.ts
import { NextResponse } from "next/server";
import { db, diagrams } from "@/lib/db";
import { eq, desc } from "drizzle-orm";

// GET /api/projects/[id]/diagrams
export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const result = await db
      .select()
      .from(diagrams)
      .where(eq(diagrams.projectId, parseInt(id)))
      .orderBy(desc(diagrams.updatedAt));
    return NextResponse.json(result);
  } catch (error) {
    return NextResponse.json({ error: "Failed to list diagrams" }, { status: 500 });
  }
}

// POST /api/projects/[id]/diagrams
export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { title, template } = await request.json();

    if (!title) {
      return NextResponse.json({ error: "title is required" }, { status: 400 });
    }

    const [diagram] = await db
      .insert(diagrams)
      .values({ projectId: parseInt(id), title, template })
      .returning();

    return NextResponse.json(diagram, { status: 201 });
  } catch (error) {
    return NextResponse.json({ error: "Failed to create diagram" }, { status: 500 });
  }
}
```

**Step 2: Create diagram get/delete with assembled React Flow format**

```typescript
// src/app/api/diagrams/[id]/route.ts
import { NextResponse } from "next/server";
import { db, diagrams, diagramNodes, diagramEdges, diagramGroups } from "@/lib/db";
import { eq } from "drizzle-orm";

// GET /api/diagrams/[id] — returns assembled React Flow format
export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const diagram = await db.query.diagrams.findFirst({
      where: eq(diagrams.id, parseInt(id)),
    });

    if (!diagram) {
      return NextResponse.json({ error: "Diagram not found" }, { status: 404 });
    }

    const nodes = await db
      .select()
      .from(diagramNodes)
      .where(eq(diagramNodes.diagramId, parseInt(id)));

    const edges = await db
      .select()
      .from(diagramEdges)
      .where(eq(diagramEdges.diagramId, parseInt(id)));

    const groups = await db
      .select()
      .from(diagramGroups)
      .where(eq(diagramGroups.diagramId, parseInt(id)));

    // Assemble React Flow format
    const flowNodes = nodes.map((n) => ({
      id: String(n.id),
      type: "flowNode",
      position: { x: n.positionX, y: n.positionY },
      data: {
        label: n.label,
        nodeType: n.nodeType,
        description: n.description || "",
        metadata: n.metadata ? JSON.parse(n.metadata) : {},
        groupId: n.groupId,
        groupColor: n.groupId
          ? groups.find((g) => g.id === n.groupId)?.color || "#6b7280"
          : undefined,
      },
    }));

    const flowEdges = edges.map((e) => ({
      id: String(e.id),
      source: String(e.sourceNodeId),
      target: String(e.targetNodeId),
      label: e.label || undefined,
    }));

    return NextResponse.json({
      ...diagram,
      nodes: flowNodes,
      edges: flowEdges,
      groups,
    });
  } catch (error) {
    return NextResponse.json({ error: "Failed to get diagram" }, { status: 500 });
  }
}

// DELETE /api/diagrams/[id]
export async function DELETE(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    await db.delete(diagrams).where(eq(diagrams.id, parseInt(id)));
    return NextResponse.json({ success: true });
  } catch (error) {
    return NextResponse.json({ error: "Failed to delete diagram" }, { status: 500 });
  }
}
```

**Step 3: Create node routes**

```typescript
// src/app/api/diagrams/[id]/nodes/route.ts
import { NextResponse } from "next/server";
import { db, diagramNodes } from "@/lib/db";
import { eq } from "drizzle-orm";

// POST /api/diagrams/[id]/nodes
export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const body = await request.json();

    if (!body.label) {
      return NextResponse.json({ error: "label is required" }, { status: 400 });
    }

    const [node] = await db
      .insert(diagramNodes)
      .values({
        diagramId: parseInt(id),
        groupId: body.groupId || null,
        nodeType: body.nodeType || "default",
        label: body.label,
        description: body.description,
        metadata: body.metadata ? JSON.stringify(body.metadata) : null,
        positionX: body.positionX ?? body.position?.x ?? 0,
        positionY: body.positionY ?? body.position?.y ?? 0,
      })
      .returning();

    return NextResponse.json(node, { status: 201 });
  } catch (error) {
    return NextResponse.json({ error: "Failed to add node" }, { status: 500 });
  }
}
```

```typescript
// src/app/api/diagrams/[id]/nodes/[nodeId]/route.ts
import { NextResponse } from "next/server";
import { db, diagramNodes, diagramEdges } from "@/lib/db";
import { eq, or } from "drizzle-orm";

// PATCH /api/diagrams/[id]/nodes/[nodeId]
export async function PATCH(
  request: Request,
  { params }: { params: Promise<{ id: string; nodeId: string }> }
) {
  try {
    const { nodeId } = await params;
    const body = await request.json();

    const updates: Record<string, unknown> = {};
    if (body.label !== undefined) updates.label = body.label;
    if (body.description !== undefined) updates.description = body.description;
    if (body.nodeType !== undefined) updates.nodeType = body.nodeType;
    if (body.groupId !== undefined) updates.groupId = body.groupId;
    if (body.metadata !== undefined) updates.metadata = JSON.stringify(body.metadata);
    if (body.positionX !== undefined) updates.positionX = body.positionX;
    if (body.positionY !== undefined) updates.positionY = body.positionY;

    const [updated] = await db
      .update(diagramNodes)
      .set(updates)
      .where(eq(diagramNodes.id, parseInt(nodeId)))
      .returning();

    if (!updated) {
      return NextResponse.json({ error: "Node not found" }, { status: 404 });
    }

    return NextResponse.json(updated);
  } catch (error) {
    return NextResponse.json({ error: "Failed to update node" }, { status: 500 });
  }
}

// DELETE /api/diagrams/[id]/nodes/[nodeId]
export async function DELETE(
  request: Request,
  { params }: { params: Promise<{ id: string; nodeId: string }> }
) {
  try {
    const { nodeId } = await params;
    const nid = parseInt(nodeId);

    // Delete connected edges first
    await db.delete(diagramEdges).where(
      or(eq(diagramEdges.sourceNodeId, nid), eq(diagramEdges.targetNodeId, nid))
    );

    await db.delete(diagramNodes).where(eq(diagramNodes.id, nid));
    return NextResponse.json({ success: true });
  } catch (error) {
    return NextResponse.json({ error: "Failed to remove node" }, { status: 500 });
  }
}
```

**Step 4: Create edge routes**

```typescript
// src/app/api/diagrams/[id]/edges/route.ts
import { NextResponse } from "next/server";
import { db, diagramEdges } from "@/lib/db";

// POST /api/diagrams/[id]/edges
export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { sourceNodeId, targetNodeId, label } = await request.json();

    if (!sourceNodeId || !targetNodeId) {
      return NextResponse.json(
        { error: "sourceNodeId and targetNodeId are required" },
        { status: 400 }
      );
    }

    const [edge] = await db
      .insert(diagramEdges)
      .values({
        diagramId: parseInt(id),
        sourceNodeId,
        targetNodeId,
        label,
      })
      .returning();

    return NextResponse.json(edge, { status: 201 });
  } catch (error) {
    return NextResponse.json({ error: "Failed to add edge" }, { status: 500 });
  }
}
```

```typescript
// src/app/api/diagrams/[id]/edges/[edgeId]/route.ts
import { NextResponse } from "next/server";
import { db, diagramEdges } from "@/lib/db";
import { eq } from "drizzle-orm";

// DELETE /api/diagrams/[id]/edges/[edgeId]
export async function DELETE(
  request: Request,
  { params }: { params: Promise<{ id: string; edgeId: string }> }
) {
  try {
    const { edgeId } = await params;
    await db.delete(diagramEdges).where(eq(diagramEdges.id, parseInt(edgeId)));
    return NextResponse.json({ success: true });
  } catch (error) {
    return NextResponse.json({ error: "Failed to remove edge" }, { status: 500 });
  }
}
```

**Step 5: Create group routes**

```typescript
// src/app/api/diagrams/[id]/groups/route.ts
import { NextResponse } from "next/server";
import { db, diagramGroups } from "@/lib/db";

// POST /api/diagrams/[id]/groups
export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { label, color } = await request.json();

    if (!label) {
      return NextResponse.json({ error: "label is required" }, { status: 400 });
    }

    const [group] = await db
      .insert(diagramGroups)
      .values({
        diagramId: parseInt(id),
        label,
        color: color || "#6b7280",
      })
      .returning();

    return NextResponse.json(group, { status: 201 });
  } catch (error) {
    return NextResponse.json({ error: "Failed to add group" }, { status: 500 });
  }
}
```

**Step 6: Create bulk add route**

```typescript
// src/app/api/diagrams/[id]/bulk/route.ts
import { NextResponse } from "next/server";
import { db, diagramNodes, diagramEdges, diagramGroups, diagrams } from "@/lib/db";
import { eq } from "drizzle-orm";

// POST /api/diagrams/[id]/bulk
export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const diagramId = parseInt(id);
    const { nodes, edges, groups } = await request.json();

    // Insert groups first (nodes may reference them)
    const groupIdMap = new Map<string, number>(); // tempId -> realId
    if (groups && groups.length > 0) {
      for (const g of groups) {
        const [inserted] = await db
          .insert(diagramGroups)
          .values({ diagramId, label: g.label, color: g.color || "#6b7280" })
          .returning();
        if (g.tempId) groupIdMap.set(g.tempId, inserted.id);
      }
    }

    // Insert nodes
    const nodeIdMap = new Map<string, number>(); // tempId -> realId
    if (nodes && nodes.length > 0) {
      for (const n of nodes) {
        const groupId = n.groupTempId ? groupIdMap.get(n.groupTempId) : n.groupId;
        const [inserted] = await db
          .insert(diagramNodes)
          .values({
            diagramId,
            groupId: groupId || null,
            nodeType: n.nodeType || "default",
            label: n.label,
            description: n.description,
            metadata: n.metadata ? JSON.stringify(n.metadata) : null,
            positionX: n.positionX ?? n.position?.x ?? 0,
            positionY: n.positionY ?? n.position?.y ?? 0,
          })
          .returning();
        if (n.tempId) nodeIdMap.set(n.tempId, inserted.id);
      }
    }

    // Insert edges (resolve tempIds)
    let edgesInserted = 0;
    if (edges && edges.length > 0) {
      for (const e of edges) {
        const sourceId = e.sourceTempId ? nodeIdMap.get(e.sourceTempId) : e.sourceNodeId;
        const targetId = e.targetTempId ? nodeIdMap.get(e.targetTempId) : e.targetNodeId;
        if (sourceId && targetId) {
          await db.insert(diagramEdges).values({
            diagramId,
            sourceNodeId: sourceId,
            targetNodeId: targetId,
            label: e.label,
          });
          edgesInserted++;
        }
      }
    }

    // Update diagram timestamp
    await db.update(diagrams).set({ updatedAt: new Date() }).where(eq(diagrams.id, diagramId));

    return NextResponse.json({
      groups: groupIdMap.size,
      nodes: nodeIdMap.size,
      edges: edgesInserted,
    }, { status: 201 });
  } catch (error) {
    console.error("Failed to bulk add:", error);
    return NextResponse.json({ error: "Failed to bulk add" }, { status: 500 });
  }
}
```

**Step 7: Create query route**

```typescript
// src/app/api/diagrams/[id]/query/route.ts
import { NextResponse } from "next/server";
import { db, diagramNodes, diagramGroups } from "@/lib/db";
import { eq, and, like, or } from "drizzle-orm";

// GET /api/diagrams/[id]/query?q=auth&type=backend&groupId=1
export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { searchParams } = new URL(request.url);
    const q = searchParams.get("q");
    const nodeType = searchParams.get("type");
    const groupId = searchParams.get("groupId");

    const conditions = [eq(diagramNodes.diagramId, parseInt(id))];

    if (q) {
      conditions.push(
        or(
          like(diagramNodes.label, `%${q}%`),
          like(diagramNodes.description, `%${q}%`),
          like(diagramNodes.metadata, `%${q}%`)
        )!
      );
    }
    if (nodeType) conditions.push(eq(diagramNodes.nodeType, nodeType));
    if (groupId) conditions.push(eq(diagramNodes.groupId, parseInt(groupId)));

    const nodes = await db
      .select()
      .from(diagramNodes)
      .where(and(...conditions));

    return NextResponse.json(nodes);
  } catch (error) {
    return NextResponse.json({ error: "Failed to query nodes" }, { status: 500 });
  }
}
```

**Step 8: Create auto-layout route**

```typescript
// src/app/api/diagrams/[id]/layout/route.ts
import { NextResponse } from "next/server";
import { db, diagramNodes, diagramEdges } from "@/lib/db";
import { eq } from "drizzle-orm";

// POST /api/diagrams/[id]/layout — simple top-down layout
export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const diagramId = parseInt(id);

    const nodes = await db.select().from(diagramNodes).where(eq(diagramNodes.diagramId, diagramId));
    const edges = await db.select().from(diagramEdges).where(eq(diagramEdges.diagramId, diagramId));

    // Simple layered layout: topological sort + column placement
    const adjacency = new Map<number, number[]>();
    const inDegree = new Map<number, number>();
    for (const n of nodes) {
      adjacency.set(n.id, []);
      inDegree.set(n.id, 0);
    }
    for (const e of edges) {
      adjacency.get(e.sourceNodeId)?.push(e.targetNodeId);
      inDegree.set(e.targetNodeId, (inDegree.get(e.targetNodeId) || 0) + 1);
    }

    // BFS topological sort
    const queue = nodes.filter((n) => (inDegree.get(n.id) || 0) === 0).map((n) => n.id);
    const layers: number[][] = [];
    const visited = new Set<number>();

    while (queue.length > 0) {
      const layer = [...queue];
      layers.push(layer);
      queue.length = 0;
      for (const nodeId of layer) {
        visited.add(nodeId);
        for (const child of adjacency.get(nodeId) || []) {
          inDegree.set(child, (inDegree.get(child) || 0) - 1);
          if (inDegree.get(child) === 0 && !visited.has(child)) {
            queue.push(child);
          }
        }
      }
    }

    // Place unvisited nodes (cycles or disconnected) in last layer
    const remaining = nodes.filter((n) => !visited.has(n.id)).map((n) => n.id);
    if (remaining.length > 0) layers.push(remaining);

    // Assign positions
    const NODE_WIDTH = 250;
    const NODE_HEIGHT = 100;
    const LAYER_GAP = 150;
    const NODE_GAP = 50;

    for (let layerIdx = 0; layerIdx < layers.length; layerIdx++) {
      const layer = layers[layerIdx];
      const totalWidth = layer.length * NODE_WIDTH + (layer.length - 1) * NODE_GAP;
      const startX = -totalWidth / 2;

      for (let nodeIdx = 0; nodeIdx < layer.length; nodeIdx++) {
        const nodeId = layer[nodeIdx];
        const x = startX + nodeIdx * (NODE_WIDTH + NODE_GAP);
        const y = layerIdx * (NODE_HEIGHT + LAYER_GAP);

        await db
          .update(diagramNodes)
          .set({ positionX: x, positionY: y })
          .where(eq(diagramNodes.id, nodeId));
      }
    }

    return NextResponse.json({ success: true, layers: layers.length, nodes: nodes.length });
  } catch (error) {
    return NextResponse.json({ error: "Failed to auto-layout" }, { status: 500 });
  }
}
```

**Step 9: Verify build**

Run: `npx tsc --noEmit`
Expected: Clean

**Step 10: Commit**

```bash
git add src/app/api/diagrams/ src/app/api/projects/
git commit -m "feat: add diagram API routes (nodes, edges, groups, bulk, query, layout)"
```

---

### Task 7: Search API route

**Files:**
- Create: `src/app/api/search/route.ts`

**Step 1: Create global search route**

```typescript
// src/app/api/search/route.ts
import { NextResponse } from "next/server";
import { db, projects, tasks, diagramNodes } from "@/lib/db";
import { like, or } from "drizzle-orm";

// GET /api/search?q=auth
export async function GET(request: Request) {
  try {
    const { searchParams } = new URL(request.url);
    const q = searchParams.get("q");

    if (!q) {
      return NextResponse.json({ error: "q parameter required" }, { status: 400 });
    }

    const pattern = `%${q}%`;

    const matchedProjects = await db
      .select()
      .from(projects)
      .where(or(like(projects.name, pattern), like(projects.description, pattern)));

    const matchedTasks = await db
      .select()
      .from(tasks)
      .where(or(like(tasks.title, pattern), like(tasks.description, pattern)));

    const matchedNodes = await db
      .select()
      .from(diagramNodes)
      .where(
        or(
          like(diagramNodes.label, pattern),
          like(diagramNodes.description, pattern),
          like(diagramNodes.metadata, pattern)
        )
      );

    return NextResponse.json({
      projects: matchedProjects,
      tasks: matchedTasks,
      diagramNodes: matchedNodes,
    });
  } catch (error) {
    return NextResponse.json({ error: "Search failed" }, { status: 500 });
  }
}
```

**Step 2: Commit**

```bash
git add src/app/api/search/
git commit -m "feat: add global search API route"
```

---

### Task 8: Command API route

**Files:**
- Create: `src/app/api/command/route.ts`

**Step 1: Create command route that invokes Claude Code headless**

```typescript
// src/app/api/command/route.ts
import { NextResponse } from "next/server";
import { spawn } from "child_process";
import path from "path";
import { db, projects } from "@/lib/db";
import { eq } from "drizzle-orm";

// POST /api/command
export async function POST(request: Request) {
  try {
    const { input, context } = await request.json();

    if (!input) {
      return NextResponse.json({ error: "input is required" }, { status: 400 });
    }

    // Build context prefix
    let contextPrefix = "";
    if (context?.projectId) {
      const project = await db.query.projects.findFirst({
        where: eq(projects.id, context.projectId),
      });
      if (project) {
        contextPrefix += `The user is currently viewing project "${project.name}" (ID: ${project.id}).`;
        if (project.repoPath) contextPrefix += ` Repo path: ${project.repoPath}.`;
      }
    }
    if (context?.diagramId) {
      contextPrefix += ` They are looking at diagram ID ${context.diagramId}.`;
    }
    if (context?.taskId) {
      contextPrefix += ` They are looking at task ID ${context.taskId}.`;
    }
    if (context?.label) {
      contextPrefix += ` Current view: ${context.label}.`;
    }

    const systemPrompt = `You are the bot-hq assistant. You help the user manage projects, tasks, and visualizer diagrams using the available MCP tools. ${contextPrefix}

Respond concisely. If the user asks you to create, update, or delete something, use the appropriate MCP tool and confirm what you did. If they ask a question, answer it directly.`;

    // Determine working directory
    let cwd = process.cwd();
    if (context?.projectId) {
      const project = await db.query.projects.findFirst({
        where: eq(projects.id, context.projectId),
      });
      if (project?.repoPath) cwd = project.repoPath;
    }

    const mcpConfigPath = path.join(process.cwd(), ".mcp.json");

    // Spawn claude --print
    const result = await new Promise<string>((resolve, reject) => {
      const args = ["--print", "-s", systemPrompt, "-p", input];

      const child = spawn("claude", args, {
        cwd,
        timeout: 120000,
        env: {
          ...process.env,
          // Strip nested session detection vars
          CLAUDE_CODE_SESSION: undefined,
          CLAUDE_CODE_ENTRY_POINT: undefined,
        },
      });

      let stdout = "";
      let stderr = "";

      child.stdout.on("data", (data: Buffer) => {
        stdout += data.toString();
      });

      child.stderr.on("data", (data: Buffer) => {
        stderr += data.toString();
      });

      child.on("close", (code) => {
        if (code === 0) {
          resolve(stdout.trim());
        } else {
          reject(new Error(stderr || `claude exited with code ${code}`));
        }
      });

      child.on("error", reject);
    });

    return NextResponse.json({ response: result });
  } catch (error) {
    console.error("Command failed:", error);
    const message = error instanceof Error ? error.message : "Command failed";
    return NextResponse.json({ error: message }, { status: 500 });
  }
}
```

**Step 2: Commit**

```bash
git add src/app/api/command/
git commit -m "feat: add command API route (Claude Code headless)"
```

---

## Phase 3: MCP Tools

### Task 9: Project MCP tools

**Files:**
- Create: `src/mcp/tools/projects.ts`
- Modify: `src/mcp/server.ts`

**Step 1: Create project MCP tools**

```typescript
// src/mcp/tools/projects.ts
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, projects } from "../lib/db/index.js";
import { eq, like, or, desc, sql } from "drizzle-orm";
import { tasks, diagrams } from "../lib/db/schema.js";

export function registerProjectTools(server: McpServer) {
  server.tool("project_list", "List all projects", {
    status: z.enum(["active", "archived"]).optional().describe("Filter by status"),
  }, async ({ status }) => {
    let query = db.select().from(projects).orderBy(desc(projects.updatedAt));
    if (status) query = query.where(eq(projects.status, status));
    const result = await query;
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  });

  server.tool("project_get", "Get project details with stats", {
    projectId: z.number().describe("Project ID"),
  }, async ({ projectId }) => {
    const project = await db.query.projects.findFirst({
      where: eq(projects.id, projectId),
    });
    if (!project) return { content: [{ type: "text", text: "Project not found" }], isError: true };

    const taskCounts = await db
      .select({ state: tasks.state, count: sql<number>`count(*)` })
      .from(tasks)
      .where(eq(tasks.projectId, projectId))
      .groupBy(tasks.state);

    const diagramCount = await db
      .select({ count: sql<number>`count(*)` })
      .from(diagrams)
      .where(eq(diagrams.projectId, projectId));

    return {
      content: [{
        type: "text",
        text: JSON.stringify({
          ...project,
          taskCounts: Object.fromEntries(taskCounts.map((r) => [r.state, r.count])),
          diagramCount: diagramCount[0]?.count || 0,
        }, null, 2),
      }],
    };
  });

  server.tool("project_create", "Create a new project", {
    name: z.string().describe("Project name"),
    description: z.string().optional().describe("Project description"),
    repoPath: z.string().optional().describe("Path to repository on disk"),
    notes: z.string().optional().describe("General context notes"),
  }, async ({ name, description, repoPath, notes }) => {
    const [project] = await db.insert(projects).values({ name, description, repoPath, notes }).returning();
    return { content: [{ type: "text", text: JSON.stringify(project, null, 2) }] };
  });

  server.tool("project_update", "Update a project", {
    projectId: z.number().describe("Project ID"),
    name: z.string().optional(),
    description: z.string().optional(),
    repoPath: z.string().optional(),
    status: z.enum(["active", "archived"]).optional(),
    notes: z.string().optional(),
  }, async ({ projectId, ...updates }) => {
    const setValues: Record<string, unknown> = { updatedAt: new Date() };
    for (const [k, v] of Object.entries(updates)) {
      if (v !== undefined) setValues[k] = v;
    }
    const [updated] = await db.update(projects).set(setValues).where(eq(projects.id, projectId)).returning();
    if (!updated) return { content: [{ type: "text", text: "Project not found" }], isError: true };
    return { content: [{ type: "text", text: JSON.stringify(updated, null, 2) }] };
  });

  server.tool("project_delete", "Delete a project and all its data", {
    projectId: z.number().describe("Project ID"),
  }, async ({ projectId }) => {
    await db.delete(projects).where(eq(projects.id, projectId));
    return { content: [{ type: "text", text: `Project ${projectId} deleted` }] };
  });

  server.tool("project_search", "Search projects by keyword", {
    query: z.string().describe("Search keyword"),
  }, async ({ query }) => {
    const pattern = `%${query}%`;
    const result = await db.select().from(projects).where(
      or(like(projects.name, pattern), like(projects.description, pattern))
    );
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  });
}
```

**Step 2: Commit**

```bash
git add src/mcp/tools/projects.ts
git commit -m "feat: add project MCP tools"
```

---

### Task 10: Task MCP tools

**Files:**
- Create: `src/mcp/tools/tasks.ts`

**Step 1: Create task MCP tools**

```typescript
// src/mcp/tools/tasks.ts
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, tasks, taskNotes, taskDependencies } from "../lib/db/index.js";
import { eq, and, like, or, asc, desc, isNull } from "drizzle-orm";

export function registerTaskTools(server: McpServer) {
  server.tool("task_list", "List tasks for a project", {
    projectId: z.number().describe("Project ID"),
    state: z.enum(["todo", "in_progress", "done", "blocked"]).optional(),
    parentTaskId: z.number().nullable().optional().describe("Parent task ID, null for top-level only"),
    priority: z.number().optional(),
  }, async ({ projectId, state, parentTaskId, priority }) => {
    const conditions = [eq(tasks.projectId, projectId)];
    if (state) conditions.push(eq(tasks.state, state));
    if (parentTaskId === null) conditions.push(isNull(tasks.parentTaskId));
    else if (parentTaskId !== undefined) conditions.push(eq(tasks.parentTaskId, parentTaskId));
    if (priority !== undefined) conditions.push(eq(tasks.priority, priority));

    const result = await db.select().from(tasks).where(and(...conditions)).orderBy(asc(tasks.order), desc(tasks.createdAt));
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  });

  server.tool("task_get", "Get task details with subtasks and notes", {
    taskId: z.number().describe("Task ID"),
  }, async ({ taskId }) => {
    const task = await db.query.tasks.findFirst({ where: eq(tasks.id, taskId) });
    if (!task) return { content: [{ type: "text", text: "Task not found" }], isError: true };

    const subtasks = await db.select().from(tasks).where(eq(tasks.parentTaskId, taskId)).orderBy(asc(tasks.order));
    const notes = await db.select().from(taskNotes).where(eq(taskNotes.taskId, taskId)).orderBy(asc(taskNotes.createdAt));
    const deps = await db.select().from(taskDependencies).where(eq(taskDependencies.taskId, taskId));

    return { content: [{ type: "text", text: JSON.stringify({ ...task, subtasks, notes, dependencies: deps }, null, 2) }] };
  });

  server.tool("task_create", "Create a new task", {
    projectId: z.number().describe("Project ID"),
    title: z.string().describe("Task title"),
    description: z.string().optional(),
    parentTaskId: z.number().optional().describe("Parent task ID for subtasks"),
    priority: z.number().min(0).max(3).optional().describe("0=low, 3=critical"),
    tags: z.array(z.string()).optional(),
    dueDate: z.string().optional().describe("ISO date string"),
  }, async ({ projectId, title, description, parentTaskId, priority, tags, dueDate }) => {
    const [task] = await db.insert(tasks).values({
      projectId, title, description,
      parentTaskId: parentTaskId || null,
      priority: priority || 0,
      tags: tags ? JSON.stringify(tags) : null,
      dueDate: dueDate ? new Date(dueDate) : null,
    }).returning();
    return { content: [{ type: "text", text: JSON.stringify(task, null, 2) }] };
  });

  server.tool("task_update", "Update a task", {
    taskId: z.number().describe("Task ID"),
    title: z.string().optional(),
    description: z.string().optional(),
    state: z.enum(["todo", "in_progress", "done", "blocked"]).optional(),
    priority: z.number().min(0).max(3).optional(),
    tags: z.array(z.string()).optional(),
    dueDate: z.string().nullable().optional(),
  }, async ({ taskId, ...updates }) => {
    const setValues: Record<string, unknown> = { updatedAt: new Date() };
    if (updates.title !== undefined) setValues.title = updates.title;
    if (updates.description !== undefined) setValues.description = updates.description;
    if (updates.state !== undefined) setValues.state = updates.state;
    if (updates.priority !== undefined) setValues.priority = updates.priority;
    if (updates.tags !== undefined) setValues.tags = JSON.stringify(updates.tags);
    if (updates.dueDate !== undefined) setValues.dueDate = updates.dueDate ? new Date(updates.dueDate) : null;

    const [updated] = await db.update(tasks).set(setValues).where(eq(tasks.id, taskId)).returning();
    if (!updated) return { content: [{ type: "text", text: "Task not found" }], isError: true };
    return { content: [{ type: "text", text: JSON.stringify(updated, null, 2) }] };
  });

  server.tool("task_delete", "Delete a task and its subtasks", {
    taskId: z.number().describe("Task ID"),
  }, async ({ taskId }) => {
    await db.delete(tasks).where(eq(tasks.id, taskId));
    return { content: [{ type: "text", text: `Task ${taskId} deleted` }] };
  });

  server.tool("task_move", "Reparent or reorder a task", {
    taskId: z.number().describe("Task ID"),
    parentTaskId: z.number().nullable().optional().describe("New parent (null = top-level)"),
    order: z.number().optional().describe("New sort order"),
  }, async ({ taskId, parentTaskId, order }) => {
    const setValues: Record<string, unknown> = { updatedAt: new Date() };
    if (parentTaskId !== undefined) setValues.parentTaskId = parentTaskId;
    if (order !== undefined) setValues.order = order;

    const [updated] = await db.update(tasks).set(setValues).where(eq(tasks.id, taskId)).returning();
    if (!updated) return { content: [{ type: "text", text: "Task not found" }], isError: true };
    return { content: [{ type: "text", text: JSON.stringify(updated, null, 2) }] };
  });

  server.tool("task_add_note", "Add a timestamped note to a task", {
    taskId: z.number().describe("Task ID"),
    content: z.string().describe("Note content"),
  }, async ({ taskId, content }) => {
    const [note] = await db.insert(taskNotes).values({ taskId, content }).returning();
    return { content: [{ type: "text", text: JSON.stringify(note, null, 2) }] };
  });

  server.tool("task_add_dependency", "Mark a task as blocked by another task", {
    taskId: z.number().describe("The blocked task ID"),
    dependsOnTaskId: z.number().describe("The blocker task ID"),
  }, async ({ taskId, dependsOnTaskId }) => {
    const [dep] = await db.insert(taskDependencies).values({ taskId, dependsOnTaskId }).returning();
    return { content: [{ type: "text", text: JSON.stringify(dep, null, 2) }] };
  });

  server.tool("task_search", "Search tasks by keyword across all projects", {
    query: z.string().describe("Search keyword"),
  }, async ({ query }) => {
    const pattern = `%${query}%`;
    const result = await db.select().from(tasks).where(
      or(like(tasks.title, pattern), like(tasks.description, pattern))
    );
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  });
}
```

**Step 2: Commit**

```bash
git add src/mcp/tools/tasks.ts
git commit -m "feat: add task MCP tools"
```

---

### Task 11: Diagram MCP tools

**Files:**
- Create: `src/mcp/tools/diagrams.ts`

**Step 1: Create diagram MCP tools**

```typescript
// src/mcp/tools/diagrams.ts
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, diagrams, diagramNodes, diagramEdges, diagramGroups } from "../lib/db/index.js";
import { eq, and, like, or, sql } from "drizzle-orm";

export function registerDiagramTools(server: McpServer) {
  server.tool("diagram_list", "List diagrams for a project", {
    projectId: z.number().describe("Project ID"),
  }, async ({ projectId }) => {
    const result = await db.select().from(diagrams).where(eq(diagrams.projectId, projectId));
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  });

  server.tool("diagram_get", "Get a diagram with all nodes, edges, and groups", {
    diagramId: z.number().describe("Diagram ID"),
  }, async ({ diagramId }) => {
    const diagram = await db.query.diagrams.findFirst({ where: eq(diagrams.id, diagramId) });
    if (!diagram) return { content: [{ type: "text", text: "Diagram not found" }], isError: true };

    const nodes = await db.select().from(diagramNodes).where(eq(diagramNodes.diagramId, diagramId));
    const edges = await db.select().from(diagramEdges).where(eq(diagramEdges.diagramId, diagramId));
    const groups = await db.select().from(diagramGroups).where(eq(diagramGroups.diagramId, diagramId));

    return { content: [{ type: "text", text: JSON.stringify({ ...diagram, nodes, edges, groups }, null, 2) }] };
  });

  server.tool("diagram_create", "Create a new empty diagram", {
    projectId: z.number().describe("Project ID"),
    title: z.string().describe("Diagram title"),
    template: z.string().optional().describe("Template type: codebase, roadmap, process, etc."),
  }, async ({ projectId, title, template }) => {
    const [diagram] = await db.insert(diagrams).values({ projectId, title, template }).returning();
    return { content: [{ type: "text", text: JSON.stringify(diagram, null, 2) }] };
  });

  server.tool("diagram_delete", "Delete a diagram and all its contents", {
    diagramId: z.number().describe("Diagram ID"),
  }, async ({ diagramId }) => {
    await db.delete(diagrams).where(eq(diagrams.id, diagramId));
    return { content: [{ type: "text", text: `Diagram ${diagramId} deleted` }] };
  });

  server.tool("diagram_add_node", "Add a node to a diagram", {
    diagramId: z.number().describe("Diagram ID"),
    label: z.string().describe("Node label"),
    nodeType: z.string().optional().describe("Node type (e.g., component, api, step)"),
    description: z.string().optional(),
    metadata: z.record(z.unknown()).optional().describe("Arbitrary metadata (files, codeSnippets, etc.)"),
    groupId: z.number().optional().describe("Group ID to assign node to"),
    positionX: z.number().optional().describe("X position"),
    positionY: z.number().optional().describe("Y position"),
  }, async ({ diagramId, label, nodeType, description, metadata, groupId, positionX, positionY }) => {
    const [node] = await db.insert(diagramNodes).values({
      diagramId, label,
      nodeType: nodeType || "default",
      description, groupId: groupId || null,
      metadata: metadata ? JSON.stringify(metadata) : null,
      positionX: positionX ?? 0,
      positionY: positionY ?? 0,
    }).returning();
    await db.update(diagrams).set({ updatedAt: new Date() }).where(eq(diagrams.id, diagramId));
    return { content: [{ type: "text", text: JSON.stringify(node, null, 2) }] };
  });

  server.tool("diagram_update_node", "Update a diagram node", {
    nodeId: z.number().describe("Node ID"),
    label: z.string().optional(),
    nodeType: z.string().optional(),
    description: z.string().optional(),
    metadata: z.record(z.unknown()).optional(),
    groupId: z.number().nullable().optional(),
    positionX: z.number().optional(),
    positionY: z.number().optional(),
  }, async ({ nodeId, ...updates }) => {
    const setValues: Record<string, unknown> = {};
    if (updates.label !== undefined) setValues.label = updates.label;
    if (updates.nodeType !== undefined) setValues.nodeType = updates.nodeType;
    if (updates.description !== undefined) setValues.description = updates.description;
    if (updates.metadata !== undefined) setValues.metadata = JSON.stringify(updates.metadata);
    if (updates.groupId !== undefined) setValues.groupId = updates.groupId;
    if (updates.positionX !== undefined) setValues.positionX = updates.positionX;
    if (updates.positionY !== undefined) setValues.positionY = updates.positionY;

    const [updated] = await db.update(diagramNodes).set(setValues).where(eq(diagramNodes.id, nodeId)).returning();
    if (!updated) return { content: [{ type: "text", text: "Node not found" }], isError: true };

    await db.update(diagrams).set({ updatedAt: new Date() }).where(eq(diagrams.id, updated.diagramId));
    return { content: [{ type: "text", text: JSON.stringify(updated, null, 2) }] };
  });

  server.tool("diagram_remove_node", "Remove a node and its connected edges", {
    nodeId: z.number().describe("Node ID"),
  }, async ({ nodeId }) => {
    await db.delete(diagramEdges).where(or(eq(diagramEdges.sourceNodeId, nodeId), eq(diagramEdges.targetNodeId, nodeId)));
    await db.delete(diagramNodes).where(eq(diagramNodes.id, nodeId));
    return { content: [{ type: "text", text: `Node ${nodeId} and connected edges removed` }] };
  });

  server.tool("diagram_add_edge", "Connect two nodes with an edge", {
    diagramId: z.number().describe("Diagram ID"),
    sourceNodeId: z.number().describe("Source node ID"),
    targetNodeId: z.number().describe("Target node ID"),
    label: z.string().optional().describe("Edge label"),
  }, async ({ diagramId, sourceNodeId, targetNodeId, label }) => {
    const [edge] = await db.insert(diagramEdges).values({ diagramId, sourceNodeId, targetNodeId, label }).returning();
    await db.update(diagrams).set({ updatedAt: new Date() }).where(eq(diagrams.id, diagramId));
    return { content: [{ type: "text", text: JSON.stringify(edge, null, 2) }] };
  });

  server.tool("diagram_remove_edge", "Remove an edge connection", {
    edgeId: z.number().describe("Edge ID"),
  }, async ({ edgeId }) => {
    await db.delete(diagramEdges).where(eq(diagramEdges.id, edgeId));
    return { content: [{ type: "text", text: `Edge ${edgeId} removed` }] };
  });

  server.tool("diagram_add_group", "Create a node group/cluster", {
    diagramId: z.number().describe("Diagram ID"),
    label: z.string().describe("Group label"),
    color: z.string().optional().describe("Hex color (default: #6b7280)"),
  }, async ({ diagramId, label, color }) => {
    const [group] = await db.insert(diagramGroups).values({
      diagramId, label, color: color || "#6b7280",
    }).returning();
    return { content: [{ type: "text", text: JSON.stringify(group, null, 2) }] };
  });

  server.tool("diagram_bulk_add", "Batch add nodes, edges, and groups to a diagram", {
    diagramId: z.number().describe("Diagram ID"),
    groups: z.array(z.object({
      tempId: z.string(),
      label: z.string(),
      color: z.string().optional(),
    })).optional(),
    nodes: z.array(z.object({
      tempId: z.string(),
      label: z.string(),
      nodeType: z.string().optional(),
      description: z.string().optional(),
      metadata: z.record(z.unknown()).optional(),
      groupTempId: z.string().optional(),
      positionX: z.number().optional(),
      positionY: z.number().optional(),
    })).optional(),
    edges: z.array(z.object({
      sourceTempId: z.string(),
      targetTempId: z.string(),
      label: z.string().optional(),
    })).optional(),
  }, async ({ diagramId, groups, nodes, edges }) => {
    const groupIdMap = new Map<string, number>();
    const nodeIdMap = new Map<string, number>();

    if (groups) {
      for (const g of groups) {
        const [inserted] = await db.insert(diagramGroups).values({
          diagramId, label: g.label, color: g.color || "#6b7280",
        }).returning();
        groupIdMap.set(g.tempId, inserted.id);
      }
    }

    if (nodes) {
      for (const n of nodes) {
        const groupId = n.groupTempId ? groupIdMap.get(n.groupTempId) : undefined;
        const [inserted] = await db.insert(diagramNodes).values({
          diagramId,
          groupId: groupId || null,
          nodeType: n.nodeType || "default",
          label: n.label,
          description: n.description,
          metadata: n.metadata ? JSON.stringify(n.metadata) : null,
          positionX: n.positionX ?? 0,
          positionY: n.positionY ?? 0,
        }).returning();
        nodeIdMap.set(n.tempId, inserted.id);
      }
    }

    let edgesCreated = 0;
    if (edges) {
      for (const e of edges) {
        const sourceId = nodeIdMap.get(e.sourceTempId);
        const targetId = nodeIdMap.get(e.targetTempId);
        if (sourceId && targetId) {
          await db.insert(diagramEdges).values({
            diagramId, sourceNodeId: sourceId, targetNodeId: targetId, label: e.label,
          });
          edgesCreated++;
        }
      }
    }

    await db.update(diagrams).set({ updatedAt: new Date() }).where(eq(diagrams.id, diagramId));
    return {
      content: [{
        type: "text",
        text: JSON.stringify({
          groupsCreated: groupIdMap.size,
          nodesCreated: nodeIdMap.size,
          edgesCreated,
          groupIdMap: Object.fromEntries(groupIdMap),
          nodeIdMap: Object.fromEntries(nodeIdMap),
        }, null, 2),
      }],
    };
  });

  server.tool("diagram_query", "Search nodes in a diagram", {
    diagramId: z.number().describe("Diagram ID"),
    query: z.string().optional().describe("Search keyword"),
    nodeType: z.string().optional().describe("Filter by node type"),
    groupId: z.number().optional().describe("Filter by group ID"),
  }, async ({ diagramId, query, nodeType, groupId }) => {
    const conditions = [eq(diagramNodes.diagramId, diagramId)];
    if (query) {
      const pattern = `%${query}%`;
      conditions.push(or(
        like(diagramNodes.label, pattern),
        like(diagramNodes.description, pattern),
        like(diagramNodes.metadata, pattern),
      )!);
    }
    if (nodeType) conditions.push(eq(diagramNodes.nodeType, nodeType));
    if (groupId) conditions.push(eq(diagramNodes.groupId, groupId));

    const result = await db.select().from(diagramNodes).where(and(...conditions));
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  });

  server.tool("diagram_auto_layout", "Recalculate node positions using topological sort", {
    diagramId: z.number().describe("Diagram ID"),
  }, async ({ diagramId }) => {
    const nodes = await db.select().from(diagramNodes).where(eq(diagramNodes.diagramId, diagramId));
    const edges = await db.select().from(diagramEdges).where(eq(diagramEdges.diagramId, diagramId));

    const adjacency = new Map<number, number[]>();
    const inDegree = new Map<number, number>();
    for (const n of nodes) { adjacency.set(n.id, []); inDegree.set(n.id, 0); }
    for (const e of edges) {
      adjacency.get(e.sourceNodeId)?.push(e.targetNodeId);
      inDegree.set(e.targetNodeId, (inDegree.get(e.targetNodeId) || 0) + 1);
    }

    const queue = nodes.filter((n) => (inDegree.get(n.id) || 0) === 0).map((n) => n.id);
    const layers: number[][] = [];
    const visited = new Set<number>();

    while (queue.length > 0) {
      const layer = [...queue];
      layers.push(layer);
      queue.length = 0;
      for (const nodeId of layer) {
        visited.add(nodeId);
        for (const child of adjacency.get(nodeId) || []) {
          inDegree.set(child, (inDegree.get(child) || 0) - 1);
          if (inDegree.get(child) === 0 && !visited.has(child)) queue.push(child);
        }
      }
    }

    const remaining = nodes.filter((n) => !visited.has(n.id)).map((n) => n.id);
    if (remaining.length > 0) layers.push(remaining);

    for (let li = 0; li < layers.length; li++) {
      const layer = layers[li];
      const totalWidth = layer.length * 300;
      const startX = -totalWidth / 2;
      for (let ni = 0; ni < layer.length; ni++) {
        await db.update(diagramNodes).set({
          positionX: startX + ni * 300,
          positionY: li * 250,
        }).where(eq(diagramNodes.id, layer[ni]));
      }
    }

    await db.update(diagrams).set({ updatedAt: new Date() }).where(eq(diagrams.id, diagramId));
    return { content: [{ type: "text", text: `Layout complete: ${layers.length} layers, ${nodes.length} nodes` }] };
  });
}
```

**Step 2: Commit**

```bash
git add src/mcp/tools/diagrams.ts
git commit -m "feat: add diagram MCP tools"
```

---

### Task 12: Summary MCP tool + register all tools

**Files:**
- Create: `src/mcp/tools/summary.ts`
- Modify: `src/mcp/server.ts`

**Step 1: Create summary tool**

```typescript
// src/mcp/tools/summary.ts
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, projects, tasks, diagrams } from "../lib/db/index.js";
import { eq, sql, desc } from "drizzle-orm";

export function registerSummaryTools(server: McpServer) {
  server.tool("summary", "Get status summary of a project or all projects", {
    projectId: z.number().optional().describe("Project ID (omit for all projects)"),
  }, async ({ projectId }) => {
    if (projectId) {
      const project = await db.query.projects.findFirst({ where: eq(projects.id, projectId) });
      if (!project) return { content: [{ type: "text", text: "Project not found" }], isError: true };

      const taskCounts = await db
        .select({ state: tasks.state, count: sql<number>`count(*)` })
        .from(tasks).where(eq(tasks.projectId, projectId)).groupBy(tasks.state);

      const diagramCount = await db
        .select({ count: sql<number>`count(*)` })
        .from(diagrams).where(eq(diagrams.projectId, projectId));

      const blockedTasks = await db.select()
        .from(tasks).where(eq(tasks.projectId, projectId));

      const blocked = blockedTasks.filter((t) => t.state === "blocked");

      return {
        content: [{
          type: "text",
          text: JSON.stringify({
            project: project.name,
            status: project.status,
            taskCounts: Object.fromEntries(taskCounts.map((r) => [r.state, r.count])),
            diagramCount: diagramCount[0]?.count || 0,
            blockers: blocked.map((t) => ({ id: t.id, title: t.title })),
          }, null, 2),
        }],
      };
    }

    // All projects summary
    const allProjects = await db.select().from(projects).where(eq(projects.status, "active")).orderBy(desc(projects.updatedAt));
    const summaries = [];

    for (const project of allProjects) {
      const taskCounts = await db
        .select({ state: tasks.state, count: sql<number>`count(*)` })
        .from(tasks).where(eq(tasks.projectId, project.id)).groupBy(tasks.state);

      summaries.push({
        id: project.id,
        name: project.name,
        taskCounts: Object.fromEntries(taskCounts.map((r) => [r.state, r.count])),
      });
    }

    return { content: [{ type: "text", text: JSON.stringify(summaries, null, 2) }] };
  });
}
```

**Step 2: Update MCP server to register all tools**

```typescript
// src/mcp/server.ts
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { registerProjectTools } from "./tools/projects.js";
import { registerTaskTools } from "./tools/tasks.js";
import { registerDiagramTools } from "./tools/diagrams.js";
import { registerSummaryTools } from "./tools/summary.js";

const server = new McpServer({
  name: "bot-hq",
  version: "2.0.0",
});

registerProjectTools(server);
registerTaskTools(server);
registerDiagramTools(server);
registerSummaryTools(server);

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  console.error("[bot-hq MCP] Server started");
}

main().catch((error) => {
  console.error("[bot-hq MCP] Fatal error:", error);
  process.exit(1);
});
```

**Step 3: Commit**

```bash
git add src/mcp/
git commit -m "feat: add summary MCP tool, register all 29 tools"
```

---

## Phase 4: Frontend — Pages & Components

### Task 13: Update visualizer components for dynamic node types

**Files:**
- Modify: `src/components/diagrams/flow-node.tsx`
- Modify: `src/components/diagrams/flow-card.tsx`
- Modify: `src/components/diagrams/flow-canvas.tsx`
- Modify: `src/components/diagrams/node-detail-dialog.tsx`

**Step 1: Update FlowNode for dynamic types**

Replace the hardcoded `LAYER_STYLES` in `flow-node.tsx` with dynamic styling based on `nodeType` and `groupColor` from node data. The node should display `nodeType` as label and use `groupColor` for the border/accent if available, otherwise hash the `nodeType` string to a color.

**Step 2: Update FlowCard**

Remove JSON blob parsing. The card now receives `nodeCount`, `edgeCount`, `groupCount` as props instead of `flowData`.

**Step 3: Update NodeDetailDialog**

Display arbitrary `metadata` as key-value pairs instead of fixed `files[]` structure.

**Step 4: Commit**

```bash
git add src/components/diagrams/
git commit -m "feat: update visualizer components for dynamic node types"
```

---

### Task 14: Project list page

**Files:**
- Create: `src/app/projects/page.tsx`
- Create: `src/components/projects/project-card.tsx`
- Create: `src/components/projects/add-project-dialog.tsx`

**Step 1: Create ProjectCard component**

Card showing project name, description, task count badges (todo/in_progress/done/blocked), diagram count, last updated. Clickable — navigates to `/projects/[id]`.

**Step 2: Create AddProjectDialog**

Dialog with form: name (required), description, repo path (optional), notes (optional). Uses shadcn Dialog + Input + Textarea.

**Step 3: Create projects list page**

Page at `/projects` with Header, "+ New Project" button, grid of ProjectCards. Fetches from `GET /api/projects`.

**Step 4: Update sidebar to include Projects link**

Add `{ href: "/projects", label: "Projects", icon: FolderOpen }` to navItems in sidebar and mobile-nav.

**Step 5: Commit**

```bash
git add src/app/projects/ src/components/projects/ src/components/layout/
git commit -m "feat: add projects list page"
```

---

### Task 15: Project detail page with tabs

**Files:**
- Create: `src/app/projects/[id]/page.tsx`
- Create: `src/components/projects/task-list.tsx`
- Create: `src/components/projects/task-item.tsx`
- Create: `src/components/projects/add-task-dialog.tsx`

**Step 1: Create TaskItem component**

Row showing task title, state badge, priority indicator, due date, tags. Indent for subtasks. Inline state toggle button. Click to expand/show subtasks.

**Step 2: Create TaskList component**

Fetches tasks from `GET /api/projects/[id]/tasks?parent=null`, renders TaskItems recursively (subtasks indented). "+ Add Task" button. Filter/sort controls.

**Step 3: Create AddTaskDialog**

Dialog with form: title (required), description, priority select (0-3), tags input, due date picker. Optional parent task ID for creating subtasks.

**Step 4: Create project detail page**

Page at `/projects/[id]` with three tabs (shadcn Tabs):
- **Tasks** tab — TaskList component
- **Visualizers** tab — grid of FlowCards + "+ New Visualizer" button, fetches from `GET /api/projects/[id]/diagrams`
- **Overview** tab — project description, notes, repo path, stats

**Step 5: Commit**

```bash
git add src/app/projects/ src/components/projects/
git commit -m "feat: add project detail page with tasks, visualizers, overview tabs"
```

---

### Task 16: Visualizer canvas page

**Files:**
- Create: `src/app/projects/[id]/visualizer/[diagramId]/page.tsx`

**Step 1: Create visualizer page**

Page at `/projects/[id]/visualizer/[diagramId]`. Fetches assembled diagram from `GET /api/diagrams/[diagramId]`. Renders FlowCanvas with back button. Saves position changes via `PATCH /api/diagrams/[id]/nodes/[nodeId]`.

**Step 2: Commit**

```bash
git add src/app/projects/
git commit -m "feat: add visualizer canvas page"
```

---

### Task 17: Command bar component

**Files:**
- Create: `src/components/command-bar/command-bar.tsx`
- Create: `src/components/command-bar/command-context.tsx`
- Modify: `src/app/layout.tsx`

**Step 1: Create CommandContext provider**

React context that tracks current page context: `{ projectId?, diagramId?, taskId?, label? }`. Updated by pages via a `useCommandContext()` hook. The label is what displays in the command bar (e.g., "Auth Flow Diagram", "bcc-ad-manager").

**Step 2: Create CommandBar component**

Fixed bar at top of main content area. Shows context label badge (e.g., `[Auth Flow Diagram]`) when on a specific view. Text input activated by click or Cmd+K. On submit, POSTs to `/api/command` with input + context. Shows response inline below the input. Loading state while waiting.

**Step 3: Add to layout**

Wrap app in CommandContext provider. Render CommandBar above `{children}` in main content area.

**Step 4: Commit**

```bash
git add src/components/command-bar/ src/app/layout.tsx
git commit -m "feat: add context-aware command bar"
```

---

### Task 18: Wire command context into pages

**Files:**
- Modify: `src/app/projects/[id]/page.tsx`
- Modify: `src/app/projects/[id]/visualizer/[diagramId]/page.tsx`

**Step 1: Set command context in project detail page**

Call `useCommandContext().setContext({ projectId, label: project.name })` when project data loads.

**Step 2: Set command context in visualizer page**

Call `useCommandContext().setContext({ projectId, diagramId, label: diagram.title })` when diagram data loads.

**Step 3: Commit**

```bash
git add src/app/projects/
git commit -m "feat: wire command context into project and visualizer pages"
```

---

### Task 19: Update dashboard

**Files:**
- Modify: `src/app/page.tsx`

**Step 1: Update dashboard page**

Show recent projects (fetch from `/api/projects?status=active`), recent tasks across all projects. Simple cards linking to project detail pages.

**Step 2: Commit**

```bash
git add src/app/page.tsx
git commit -m "feat: update dashboard with recent projects and tasks"
```

---

### Task 20: Final cleanup and verify

**Step 1: Delete old DB and regenerate**

```bash
rm -rf data/bot-hq.db drizzle/
npx drizzle-kit generate
npx drizzle-kit push
```

**Step 2: Type check**

Run: `npx tsc --noEmit`
Expected: Clean

**Step 3: Build**

Run: `npx next build`
Expected: All routes compile successfully

**Step 4: Commit**

```bash
git add -A
git commit -m "chore: final cleanup, regenerate DB migration"
```
