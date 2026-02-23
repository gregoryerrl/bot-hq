# Workspace Visualization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add interactive React Flow diagrams to bot-hq workspaces, generated and maintained by a Visualizer Bot, so visual users can see and interact with user flows in their codebase.

**Architecture:** New `diagrams` DB table stores React Flow JSON per user flow per workspace. New MCP tools (`diagram_list/get/create/update`) let the Visualizer Bot manage them. New API routes serve the data to a React Flow canvas at `/workspaces/[id]/diagram`. The Manager prompt is updated to know about the Visualizer Bot role.

**Tech Stack:** `@xyflow/react` (React Flow), Drizzle ORM (SQLite), Next.js API routes, existing MCP server pattern.

---

### Task 1: Install React Flow dependency

**Files:**
- Modify: `package.json`

**Step 1: Install @xyflow/react**

Run: `cd /Users/gregoryerrl/Projects/bot-hq && npm install @xyflow/react`

**Step 2: Verify installation**

Run: `node -e "require('@xyflow/react')"`
Expected: No error

**Step 3: Commit**

```bash
git add package.json package-lock.json
git commit -m "feat: add @xyflow/react for workspace visualization"
```

---

### Task 2: Add `diagrams` table to DB schema

**Files:**
- Modify: `src/lib/db/schema.ts` (add after `settings` table, ~line 117)

**Step 1: Add the diagrams table and type exports to schema.ts**

Add after the `settings` table definition:

```typescript
// Diagrams - React Flow user flow diagrams per workspace
export const diagrams = sqliteTable("diagrams", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id")
    .notNull()
    .references(() => workspaces.id, { onDelete: "cascade" }),
  title: text("title").notNull(),
  flowData: text("flow_data").notNull(), // JSON: { nodes: ReactFlowNode[], edges: ReactFlowEdge[] }
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("diagrams_workspace_idx").on(table.workspaceId),
]);
```

Add type exports at the bottom with the others:

```typescript
export type Diagram = typeof diagrams.$inferSelect;
export type NewDiagram = typeof diagrams.$inferInsert;
```

**Step 2: Generate and run migration**

Run: `cd /Users/gregoryerrl/Projects/bot-hq && npm run db:generate && npm run db:push`
Expected: Migration generated and applied successfully

**Step 3: Verify table exists**

Run: `cd /Users/gregoryerrl/Projects/bot-hq && node -e "const Database = require('better-sqlite3'); const db = new Database('data/bot-hq.db'); console.log(db.prepare('SELECT sql FROM sqlite_master WHERE name = \"diagrams\"').get())"`
Expected: Shows CREATE TABLE statement

**Step 4: Commit**

```bash
git add src/lib/db/schema.ts drizzle/
git commit -m "feat: add diagrams table for workspace flow visualization"
```

---

### Task 3: Create diagram API routes

**Files:**
- Create: `src/app/api/diagrams/route.ts` (list + create)
- Create: `src/app/api/diagrams/[id]/route.ts` (get + update + delete)

**Step 1: Create `src/app/api/diagrams/route.ts`**

```typescript
import { NextResponse } from "next/server";
import { db, diagrams, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

// GET /api/diagrams?workspaceId=1 — List diagrams for a workspace
export async function GET(request: Request) {
  try {
    const { searchParams } = new URL(request.url);
    const workspaceId = searchParams.get("workspaceId");

    if (!workspaceId) {
      return NextResponse.json({ error: "workspaceId required" }, { status: 400 });
    }

    const diagramList = await db
      .select({
        id: diagrams.id,
        workspaceId: diagrams.workspaceId,
        title: diagrams.title,
        flowData: diagrams.flowData,
        createdAt: diagrams.createdAt,
        updatedAt: diagrams.updatedAt,
      })
      .from(diagrams)
      .where(eq(diagrams.workspaceId, parseInt(workspaceId)));

    return NextResponse.json(diagramList);
  } catch (error) {
    console.error("Failed to list diagrams:", error);
    return NextResponse.json({ error: "Failed to list diagrams" }, { status: 500 });
  }
}

// POST /api/diagrams — Create a new diagram
export async function POST(request: Request) {
  try {
    const { workspaceId, title, flowData } = await request.json();

    if (!workspaceId || !title || !flowData) {
      return NextResponse.json(
        { error: "workspaceId, title, and flowData required" },
        { status: 400 }
      );
    }

    // Verify workspace exists
    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, workspaceId),
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    const [newDiagram] = await db
      .insert(diagrams)
      .values({
        workspaceId,
        title,
        flowData: typeof flowData === "string" ? flowData : JSON.stringify(flowData),
      })
      .returning();

    return NextResponse.json(newDiagram, { status: 201 });
  } catch (error) {
    console.error("Failed to create diagram:", error);
    return NextResponse.json({ error: "Failed to create diagram" }, { status: 500 });
  }
}
```

**Step 2: Create `src/app/api/diagrams/[id]/route.ts`**

```typescript
import { NextResponse } from "next/server";
import { db, diagrams } from "@/lib/db";
import { eq } from "drizzle-orm";

// GET /api/diagrams/[id] — Get a single diagram
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

    return NextResponse.json(diagram);
  } catch (error) {
    console.error("Failed to get diagram:", error);
    return NextResponse.json({ error: "Failed to get diagram" }, { status: 500 });
  }
}

// PUT /api/diagrams/[id] — Update a diagram
export async function PUT(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const body = await request.json();

    const existing = await db.query.diagrams.findFirst({
      where: eq(diagrams.id, parseInt(id)),
    });

    if (!existing) {
      return NextResponse.json({ error: "Diagram not found" }, { status: 404 });
    }

    const updates: Record<string, unknown> = { updatedAt: new Date() };
    if (body.title) updates.title = body.title;
    if (body.flowData) {
      updates.flowData = typeof body.flowData === "string"
        ? body.flowData
        : JSON.stringify(body.flowData);
    }

    await db.update(diagrams).set(updates).where(eq(diagrams.id, parseInt(id)));

    const updated = await db.query.diagrams.findFirst({
      where: eq(diagrams.id, parseInt(id)),
    });

    return NextResponse.json(updated);
  } catch (error) {
    console.error("Failed to update diagram:", error);
    return NextResponse.json({ error: "Failed to update diagram" }, { status: 500 });
  }
}

// DELETE /api/diagrams/[id] — Delete a diagram
export async function DELETE(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    await db.delete(diagrams).where(eq(diagrams.id, parseInt(id)));
    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to delete diagram:", error);
    return NextResponse.json({ error: "Failed to delete diagram" }, { status: 500 });
  }
}
```

**Step 3: Verify API compiles**

Run: `cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit --pretty 2>&1 | head -20`
Expected: No errors in diagram route files (existing errors in other files are OK)

**Step 4: Commit**

```bash
git add src/app/api/diagrams/
git commit -m "feat: add diagram CRUD API routes"
```

---

### Task 4: Add diagram MCP tools

**Files:**
- Create: `src/mcp/tools/diagrams.ts`
- Modify: `src/mcp/server.ts` (register the new tools)

**Step 1: Create `src/mcp/tools/diagrams.ts`**

```typescript
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, diagrams, workspaces } from "../../lib/db/index.js";
import { eq } from "drizzle-orm";

export function registerDiagramTools(server: McpServer) {
  server.tool(
    "diagram_list",
    "List flow diagrams for a workspace",
    {
      workspaceId: z.number().describe("The workspace ID"),
    },
    async ({ workspaceId }) => {
      const diagramList = await db
        .select({
          id: diagrams.id,
          title: diagrams.title,
          workspaceId: diagrams.workspaceId,
          updatedAt: diagrams.updatedAt,
        })
        .from(diagrams)
        .where(eq(diagrams.workspaceId, workspaceId));

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(diagramList, null, 2),
          },
        ],
      };
    }
  );

  server.tool(
    "diagram_get",
    "Get full details of a specific diagram including flow data",
    {
      diagramId: z.number().describe("The diagram ID"),
    },
    async ({ diagramId }) => {
      const diagram = await db.query.diagrams.findFirst({
        where: eq(diagrams.id, diagramId),
      });

      if (!diagram) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: `Diagram ${diagramId} not found` }),
            },
          ],
        };
      }

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(
              {
                id: diagram.id,
                title: diagram.title,
                workspaceId: diagram.workspaceId,
                flowData: JSON.parse(diagram.flowData),
                createdAt: diagram.createdAt,
                updatedAt: diagram.updatedAt,
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  server.tool(
    "diagram_create",
    "Create a new flow diagram for a workspace",
    {
      workspaceId: z.number().describe("The workspace ID"),
      title: z.string().describe("Diagram title (e.g., 'User Registration')"),
      flowData: z.string().describe("JSON string of { nodes: [...], edges: [...] } in React Flow format"),
    },
    async ({ workspaceId, title, flowData }) => {
      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, workspaceId),
      });

      if (!workspace) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: `Workspace ${workspaceId} not found` }),
            },
          ],
        };
      }

      // Validate JSON
      try {
        JSON.parse(flowData);
      } catch {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: "Invalid flowData JSON" }),
            },
          ],
        };
      }

      const [newDiagram] = await db
        .insert(diagrams)
        .values({ workspaceId, title, flowData })
        .returning();

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              diagramId: newDiagram.id,
              message: `Diagram "${title}" created`,
            }),
          },
        ],
      };
    }
  );

  server.tool(
    "diagram_update",
    "Update an existing diagram's flow data or title",
    {
      diagramId: z.number().describe("The diagram ID to update"),
      title: z.string().optional().describe("New title"),
      flowData: z.string().optional().describe("Updated JSON string of { nodes: [...], edges: [...] }"),
    },
    async ({ diagramId, title, flowData }) => {
      const existing = await db.query.diagrams.findFirst({
        where: eq(diagrams.id, diagramId),
      });

      if (!existing) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: `Diagram ${diagramId} not found` }),
            },
          ],
        };
      }

      const updates: Record<string, unknown> = { updatedAt: new Date() };
      if (title) updates.title = title;
      if (flowData) {
        try {
          JSON.parse(flowData);
        } catch {
          return {
            content: [
              {
                type: "text" as const,
                text: JSON.stringify({ error: "Invalid flowData JSON" }),
              },
            ],
          };
        }
        updates.flowData = flowData;
      }

      await db.update(diagrams).set(updates).where(eq(diagrams.id, diagramId));

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              message: `Diagram ${diagramId} updated`,
            }),
          },
        ],
      };
    }
  );
}
```

**Step 2: Register diagram tools in `src/mcp/server.ts`**

Add import at top:

```typescript
import { registerDiagramTools } from "./tools/diagrams.js";
```

Add registration after existing tools:

```typescript
registerDiagramTools(server);
```

**Step 3: Verify MCP server compiles**

Run: `cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit --pretty 2>&1 | grep -i diagram`
Expected: No errors

**Step 4: Commit**

```bash
git add src/mcp/tools/diagrams.ts src/mcp/server.ts
git commit -m "feat: add diagram MCP tools for Visualizer Bot"
```

---

### Task 5: Create the Flow List page (`/workspaces/[id]/diagram`)

**Files:**
- Create: `src/app/workspaces/[id]/diagram/page.tsx`
- Create: `src/components/diagrams/flow-card.tsx`

**Step 1: Create the flow card component `src/components/diagrams/flow-card.tsx`**

```typescript
"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Wrench, Clock } from "lucide-react";
import { formatDistanceToNow } from "date-fns";

interface FlowNode {
  id: string;
  data: {
    layer: "ux" | "frontend" | "backend" | "database";
    activeTask?: { taskId: number; state: string } | null;
  };
}

interface FlowCardProps {
  id: number;
  title: string;
  flowData: string;
  updatedAt: string | Date;
  onClick: (id: number) => void;
}

const LAYER_COLORS: Record<string, string> = {
  ux: "bg-blue-500",
  frontend: "bg-green-500",
  backend: "bg-red-500",
  database: "bg-purple-500",
};

export function FlowCard({ id, title, flowData, updatedAt, onClick }: FlowCardProps) {
  let nodes: FlowNode[] = [];
  try {
    const parsed = JSON.parse(flowData);
    nodes = parsed.nodes || [];
  } catch {
    // Invalid JSON, show empty
  }

  // Count nodes per layer
  const layerCounts: Record<string, number> = {};
  let hasWorking = false;
  let hasPending = false;

  for (const node of nodes) {
    const layer = node.data?.layer || "backend";
    layerCounts[layer] = (layerCounts[layer] || 0) + 1;

    if (node.data?.activeTask) {
      if (node.data.activeTask.state === "in_progress") hasWorking = true;
      if (node.data.activeTask.state === "pending") hasPending = true;
    }
  }

  return (
    <Card
      className="p-4 cursor-pointer hover:bg-muted/50 transition-colors"
      onClick={() => onClick(id)}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex-1 min-w-0">
          <h3 className="font-medium truncate">{title}</h3>
          <div className="flex items-center gap-1.5 mt-2">
            {["ux", "frontend", "backend", "database"].map((layer) =>
              layerCounts[layer] ? (
                <div key={layer} className="flex items-center gap-1">
                  <div className={`h-2.5 w-2.5 rounded-full ${LAYER_COLORS[layer]}`} />
                  <span className="text-xs text-muted-foreground">{layerCounts[layer]}</span>
                </div>
              ) : null
            )}
          </div>
          <p className="text-xs text-muted-foreground mt-2">
            Updated {formatDistanceToNow(new Date(updatedAt), { addSuffix: true })}
          </p>
        </div>
        <div className="flex items-center gap-1">
          {hasWorking && (
            <Badge variant="outline" className="text-orange-500 border-orange-500">
              <Wrench className="h-3 w-3 mr-1" />
              Working
            </Badge>
          )}
          {hasPending && (
            <Badge variant="outline" className="text-yellow-500 border-yellow-500">
              <Clock className="h-3 w-3 mr-1" />
              Pending
            </Badge>
          )}
        </div>
      </div>
    </Card>
  );
}
```

**Step 2: Create the diagram list page `src/app/workspaces/[id]/diagram/page.tsx`**

```typescript
"use client";

import { useState, useEffect, useCallback, use } from "react";
import { useRouter } from "next/navigation";
import { Header } from "@/components/layout/header";
import { FlowCard } from "@/components/diagrams/flow-card";

interface DiagramSummary {
  id: number;
  title: string;
  workspaceId: number;
  flowData: string;
  createdAt: string;
  updatedAt: string;
}

interface WorkspaceInfo {
  id: number;
  name: string;
}

export default function DiagramListPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const router = useRouter();
  const [diagrams, setDiagrams] = useState<DiagramSummary[]>([]);
  const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null);
  const [loading, setLoading] = useState(true);

  const fetchData = useCallback(async () => {
    try {
      const [diagramRes, workspaceRes] = await Promise.all([
        fetch(`/api/diagrams?workspaceId=${id}`),
        fetch(`/api/workspaces/${id}`),
      ]);

      if (diagramRes.ok) {
        setDiagrams(await diagramRes.json());
      }
      if (workspaceRes.ok) {
        setWorkspace(await workspaceRes.json());
      }
    } catch (error) {
      console.error("Failed to fetch diagram data:", error);
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 10000);
    return () => clearInterval(interval);
  }, [fetchData]);

  function handleFlowClick(diagramId: number) {
    window.location.assign(`/workspaces/${id}/diagram/${diagramId}`);
  }

  return (
    <div className="flex flex-col h-full">
      <Header
        title={workspace ? `${workspace.name} — Diagrams` : "Diagrams"}
        description="Interactive user flow diagrams for this workspace"
      />
      <div className="flex-1 p-4 md:p-6">
        {loading ? (
          <div className="text-muted-foreground">Loading diagrams...</div>
        ) : diagrams.length === 0 ? (
          <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
            No diagrams yet. The Visualizer Bot will generate flow diagrams when tasks are processed for this workspace.
          </div>
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {diagrams.map((d) => (
              <FlowCard
                key={d.id}
                id={d.id}
                title={d.title}
                flowData={d.flowData}
                updatedAt={d.updatedAt}
                onClick={handleFlowClick}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
```

**Step 3: Add workspace GET API if it doesn't exist**

Check if `src/app/api/workspaces/[id]/route.ts` exists. If not, create it:

```typescript
import { NextResponse } from "next/server";
import { db, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, parseInt(id)),
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    return NextResponse.json(workspace);
  } catch (error) {
    console.error("Failed to get workspace:", error);
    return NextResponse.json({ error: "Failed to get workspace" }, { status: 500 });
  }
}
```

**Step 4: Verify page compiles**

Run: `cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit --pretty 2>&1 | grep -i diagram`
Expected: No errors

**Step 5: Commit**

```bash
git add src/app/workspaces/ src/components/diagrams/ src/app/api/workspaces/
git commit -m "feat: add diagram list page and flow cards"
```

---

### Task 6: Create the React Flow canvas page

**Files:**
- Create: `src/app/workspaces/[id]/diagram/[diagramId]/page.tsx`
- Create: `src/components/diagrams/flow-canvas.tsx`
- Create: `src/components/diagrams/flow-node.tsx`
- Create: `src/components/diagrams/node-detail-dialog.tsx`

**Step 1: Create custom node component `src/components/diagrams/flow-node.tsx`**

```typescript
"use client";

import { memo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { Wrench, Clock } from "lucide-react";

export interface FlowNodeData {
  label: string;
  layer: "ux" | "frontend" | "backend" | "database";
  description: string;
  files: { path: string; lineStart?: number; lineEnd?: number }[];
  codeSnippets?: string[];
  activeTask?: { taskId: number; state: string } | null;
  [key: string]: unknown;
}

const LAYER_STYLES: Record<string, { border: string; bg: string; label: string; color: string }> = {
  ux: { border: "border-l-blue-500", bg: "bg-blue-500/5", label: "UX", color: "text-blue-500" },
  frontend: { border: "border-l-green-500", bg: "bg-green-500/5", label: "Frontend", color: "text-green-500" },
  backend: { border: "border-l-red-500", bg: "bg-red-500/5", label: "Backend", color: "text-red-500" },
  database: { border: "border-l-purple-500", bg: "bg-purple-500/5", label: "Database", color: "text-purple-500" },
};

function FlowNodeComponent({ data }: NodeProps) {
  const [showTooltip, setShowTooltip] = useState(false);
  const nodeData = data as unknown as FlowNodeData;
  const style = LAYER_STYLES[nodeData.layer] || LAYER_STYLES.backend;
  const activeTask = nodeData.activeTask;

  return (
    <div
      className={`relative rounded-lg border-l-4 border bg-card shadow-sm min-w-[180px] max-w-[240px] ${style.border} ${style.bg}`}
      onMouseEnter={() => setShowTooltip(true)}
      onMouseLeave={() => setShowTooltip(false)}
    >
      <Handle type="target" position={Position.Left} className="!bg-muted-foreground" />

      <div className="px-3 py-2">
        <div className="flex items-center justify-between gap-1">
          <span className={`text-[10px] font-medium uppercase ${style.color}`}>
            {style.label}
          </span>
          {activeTask?.state === "in_progress" && (
            <Wrench className="h-3 w-3 text-orange-500 animate-spin" />
          )}
          {activeTask?.state === "pending" && (
            <Clock className="h-3 w-3 text-yellow-500" />
          )}
        </div>
        <p className="text-sm font-medium mt-1 leading-tight">{nodeData.label}</p>
      </div>

      <Handle type="source" position={Position.Right} className="!bg-muted-foreground" />

      {/* Hover tooltip */}
      {showTooltip && (
        <div className="absolute z-50 top-full left-0 mt-2 w-72 p-3 rounded-lg border bg-popover text-popover-foreground shadow-lg">
          <p className="text-sm">{nodeData.description}</p>
          {nodeData.files?.length > 0 && (
            <div className="mt-2">
              <p className="text-xs font-medium text-muted-foreground mb-1">Files:</p>
              {nodeData.files.map((f, i) => (
                <p key={i} className="text-xs font-mono text-muted-foreground">
                  {f.path}
                  {f.lineStart ? `:${f.lineStart}` : ""}
                  {f.lineEnd ? `-${f.lineEnd}` : ""}
                </p>
              ))}
            </div>
          )}
          {activeTask && (
            <p className="text-xs mt-2 text-orange-500">
              Task #{activeTask.taskId} — {activeTask.state}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

export const FlowNode = memo(FlowNodeComponent);
```

**Step 2: Create node detail dialog `src/components/diagrams/node-detail-dialog.tsx`**

```typescript
"use client";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Wrench, Clock, ExternalLink } from "lucide-react";
import type { FlowNodeData } from "./flow-node";

interface NodeDetailDialogProps {
  open: boolean;
  onClose: () => void;
  node: { id: string; data: FlowNodeData } | null;
  connectedNodes: { incoming: string[]; outgoing: string[] };
}

const LAYER_LABELS: Record<string, { label: string; color: string }> = {
  ux: { label: "UX", color: "bg-blue-500" },
  frontend: { label: "Frontend", color: "bg-green-500" },
  backend: { label: "Backend", color: "bg-red-500" },
  database: { label: "Database", color: "bg-purple-500" },
};

export function NodeDetailDialog({ open, onClose, node, connectedNodes }: NodeDetailDialogProps) {
  if (!node) return null;

  const data = node.data;
  const layer = LAYER_LABELS[data.layer] || LAYER_LABELS.backend;

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-w-lg max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <div className={`h-3 w-3 rounded-full ${layer.color}`} />
            {data.label}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {/* Layer */}
          <div>
            <Badge variant="outline">{layer.label}</Badge>
          </div>

          {/* Description */}
          <div>
            <h4 className="text-sm font-medium mb-1">Description</h4>
            <p className="text-sm text-muted-foreground">{data.description}</p>
          </div>

          {/* Files */}
          {data.files?.length > 0 && (
            <div>
              <h4 className="text-sm font-medium mb-1">Files</h4>
              <div className="space-y-1">
                {data.files.map((f, i) => (
                  <p key={i} className="text-xs font-mono text-muted-foreground">
                    {f.path}
                    {f.lineStart ? `:${f.lineStart}` : ""}
                    {f.lineEnd ? `-${f.lineEnd}` : ""}
                  </p>
                ))}
              </div>
            </div>
          )}

          {/* Code snippets */}
          {data.codeSnippets && data.codeSnippets.length > 0 && (
            <div>
              <h4 className="text-sm font-medium mb-1">Code</h4>
              {data.codeSnippets.map((snippet, i) => (
                <pre
                  key={i}
                  className="text-xs bg-muted p-2 rounded overflow-x-auto mt-1"
                >
                  {snippet}
                </pre>
              ))}
            </div>
          )}

          {/* Connected nodes */}
          {(connectedNodes.incoming.length > 0 || connectedNodes.outgoing.length > 0) && (
            <div>
              <h4 className="text-sm font-medium mb-1">Connections</h4>
              {connectedNodes.incoming.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  From: {connectedNodes.incoming.join(", ")}
                </p>
              )}
              {connectedNodes.outgoing.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  To: {connectedNodes.outgoing.join(", ")}
                </p>
              )}
            </div>
          )}

          {/* Active task */}
          {data.activeTask && (
            <div className="flex items-center gap-2">
              {data.activeTask.state === "in_progress" ? (
                <Badge variant="outline" className="text-orange-500 border-orange-500">
                  <Wrench className="h-3 w-3 mr-1" />
                  Task #{data.activeTask.taskId} — Working
                </Badge>
              ) : (
                <Badge variant="outline" className="text-yellow-500 border-yellow-500">
                  <Clock className="h-3 w-3 mr-1" />
                  Task #{data.activeTask.taskId} — Pending Review
                </Badge>
              )}
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  if (data.activeTask!.state === "in_progress") {
                    window.location.assign(`/logs`);
                  } else {
                    window.location.assign(`/pending`);
                  }
                }}
              >
                <ExternalLink className="h-3 w-3 mr-1" />
                {data.activeTask.state === "in_progress" ? "View Logs" : "Review Changes"}
              </Button>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
```

**Step 3: Create the flow canvas component `src/components/diagrams/flow-canvas.tsx`**

```typescript
"use client";

import { useCallback, useMemo, useState } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  type Node,
  type Edge,
  type OnNodesChange,
  type OnEdgesChange,
  applyNodeChanges,
  applyEdgeChanges,
  MarkerType,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { FlowNode, type FlowNodeData } from "./flow-node";
import { NodeDetailDialog } from "./node-detail-dialog";

interface FlowCanvasProps {
  diagramId: number;
  initialNodes: Node[];
  initialEdges: Edge[];
  onPositionsChange?: (nodes: Node[]) => void;
}

const nodeTypes = { flowNode: FlowNode };

const defaultEdgeOptions = {
  animated: true,
  markerEnd: { type: MarkerType.ArrowClosed },
  style: { strokeWidth: 2 },
};

export function FlowCanvas({
  diagramId,
  initialNodes,
  initialEdges,
  onPositionsChange,
}: FlowCanvasProps) {
  const [nodes, setNodes] = useState<Node[]>(initialNodes);
  const [edges, setEdges] = useState<Edge[]>(initialEdges);
  const [selectedNode, setSelectedNode] = useState<{ id: string; data: FlowNodeData } | null>(null);

  const onNodesChange: OnNodesChange = useCallback(
    (changes) => {
      setNodes((nds) => {
        const updated = applyNodeChanges(changes, nds);
        // Debounce position saves
        const hasDragStop = changes.some((c) => c.type === "position" && c.dragging === false);
        if (hasDragStop && onPositionsChange) {
          onPositionsChange(updated);
        }
        return updated;
      });
    },
    [onPositionsChange]
  );

  const onEdgesChange: OnEdgesChange = useCallback(
    (changes) => setEdges((eds) => applyEdgeChanges(changes, eds)),
    []
  );

  const onNodeClick = useCallback(
    (_: React.MouseEvent, node: Node) => {
      setSelectedNode({ id: node.id, data: node.data as unknown as FlowNodeData });
    },
    []
  );

  const connectedNodes = useMemo(() => {
    if (!selectedNode) return { incoming: [], outgoing: [] };

    const incoming = edges
      .filter((e) => e.target === selectedNode.id)
      .map((e) => {
        const sourceNode = nodes.find((n) => n.id === e.source);
        return (sourceNode?.data as unknown as FlowNodeData)?.label || e.source;
      });

    const outgoing = edges
      .filter((e) => e.source === selectedNode.id)
      .map((e) => {
        const targetNode = nodes.find((n) => n.id === e.target);
        return (targetNode?.data as unknown as FlowNodeData)?.label || e.target;
      });

    return { incoming, outgoing };
  }, [selectedNode, edges, nodes]);

  return (
    <div className="w-full h-full">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={onNodeClick}
        nodeTypes={nodeTypes}
        defaultEdgeOptions={defaultEdgeOptions}
        fitView
        fitViewOptions={{ padding: 0.2 }}
      >
        <Background />
        <Controls />
      </ReactFlow>

      <NodeDetailDialog
        open={!!selectedNode}
        onClose={() => setSelectedNode(null)}
        node={selectedNode}
        connectedNodes={connectedNodes}
      />
    </div>
  );
}
```

**Step 4: Create the canvas page `src/app/workspaces/[id]/diagram/[diagramId]/page.tsx`**

```typescript
"use client";

import { useState, useEffect, useCallback, use } from "react";
import { Button } from "@/components/ui/button";
import { ArrowLeft } from "lucide-react";
import { FlowCanvas } from "@/components/diagrams/flow-canvas";
import type { Node, Edge } from "@xyflow/react";

interface DiagramData {
  id: number;
  title: string;
  workspaceId: number;
  flowData: string;
  createdAt: string;
  updatedAt: string;
}

export default function DiagramCanvasPage({
  params,
}: {
  params: Promise<{ id: string; diagramId: string }>;
}) {
  const { id: workspaceId, diagramId } = use(params);
  const [diagram, setDiagram] = useState<DiagramData | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    async function fetchDiagram() {
      try {
        const res = await fetch(`/api/diagrams/${diagramId}`);
        if (res.ok) {
          setDiagram(await res.json());
        }
      } catch (error) {
        console.error("Failed to fetch diagram:", error);
      } finally {
        setLoading(false);
      }
    }
    fetchDiagram();
  }, [diagramId]);

  const handlePositionsChange = useCallback(
    async (updatedNodes: Node[]) => {
      if (!diagram) return;

      try {
        const flowData = JSON.parse(diagram.flowData);
        flowData.nodes = updatedNodes.map((n) => ({
          ...flowData.nodes.find((fn: Node) => fn.id === n.id),
          position: n.position,
        }));

        await fetch(`/api/diagrams/${diagramId}`, {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ flowData }),
        });
      } catch (error) {
        console.error("Failed to save positions:", error);
      }
    },
    [diagram, diagramId]
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        Loading diagram...
      </div>
    );
  }

  if (!diagram) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        Diagram not found
      </div>
    );
  }

  let nodes: Node[] = [];
  let edges: Edge[] = [];
  try {
    const parsed = JSON.parse(diagram.flowData);
    nodes = (parsed.nodes || []).map((n: Node) => ({
      ...n,
      type: "flowNode",
    }));
    edges = parsed.edges || [];
  } catch {
    // Invalid JSON
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-3 px-4 py-3 border-b">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => window.location.assign(`/workspaces/${workspaceId}/diagram`)}
        >
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <h1 className="text-lg font-semibold">{diagram.title}</h1>
      </div>
      <div className="flex-1">
        <FlowCanvas
          diagramId={diagram.id}
          initialNodes={nodes}
          initialEdges={edges}
          onPositionsChange={handlePositionsChange}
        />
      </div>
    </div>
  );
}
```

**Step 5: Verify everything compiles**

Run: `cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit --pretty 2>&1 | grep -E "(diagram|flow)" -i`
Expected: No errors in diagram/flow files

**Step 6: Commit**

```bash
git add src/app/workspaces/ src/components/diagrams/
git commit -m "feat: add React Flow canvas and diagram detail pages"
```

---

### Task 7: Add diagram navigation to sidebar and workspace list

**Files:**
- Modify: `src/components/layout/sidebar.tsx` (no change needed — workspaces link exists)
- Modify: `src/components/settings/workspace-list.tsx` (add "Diagrams" link per workspace)

**Step 1: Read workspace-list.tsx to understand current structure**

Read `src/components/settings/workspace-list.tsx` to understand the layout.

**Step 2: Add a "Diagrams" button/link to each workspace card**

Add a small button or link next to each workspace that navigates to `/workspaces/{id}/diagram`. Use the `GitGraph` or `Workflow` lucide icon. The exact edit depends on the current file structure — add a button inside each workspace card that calls `window.location.assign(\`/workspaces/${workspace.id}/diagram\`)`.

**Step 3: Verify and commit**

```bash
git add src/components/settings/workspace-list.tsx
git commit -m "feat: add diagram navigation link to workspace list"
```

---

### Task 8: Update Manager prompt to know about Visualizer Bot

**Files:**
- Modify: `src/lib/bot-hq/templates.ts` (add Visualizer Bot role to manager prompts)

**Step 1: Read current templates.ts**

Read `src/lib/bot-hq/templates.ts` to understand the current prompt structure.

**Step 2: Add Visualizer Bot to the manager prompt**

Update the default manager prompt and re-init prompt to include:
- The Visualizer Bot role description
- When to spawn it (on startup if no diagrams exist, in parallel with SE Bot on task start, after task completion for updates)
- The diagram MCP tools it should use (`diagram_list`, `diagram_get`, `diagram_create`, `diagram_update`)
- The expected flowData JSON format (nodes with layer/label/description/files/position, edges with source/target/label/condition)

**Step 3: Commit**

```bash
git add src/lib/bot-hq/templates.ts
git commit -m "feat: update manager prompt with Visualizer Bot role"
```

---

### Task 9: Build and verify end-to-end

**Step 1: Build the project**

Run: `cd /Users/gregoryerrl/Projects/bot-hq && npm run build`
Expected: Build succeeds

**Step 2: Start dev server and verify pages load**

Run: `cd /Users/gregoryerrl/Projects/bot-hq && npm run dev`

Verify:
- `/workspaces` page loads
- `/workspaces/1/diagram` page loads (shows empty state)
- API routes work: `curl http://localhost:7890/api/diagrams?workspaceId=1`

**Step 3: Insert test diagram via API and verify rendering**

```bash
curl -X POST http://localhost:7890/api/diagrams \
  -H "Content-Type: application/json" \
  -d '{
    "workspaceId": 1,
    "title": "Test Flow",
    "flowData": "{\"nodes\":[{\"id\":\"1\",\"position\":{\"x\":0,\"y\":100},\"data\":{\"label\":\"User clicks button\",\"layer\":\"ux\",\"description\":\"User initiates the action\",\"files\":[]}},{\"id\":\"2\",\"position\":{\"x\":300,\"y\":100},\"data\":{\"label\":\"Send API request\",\"layer\":\"frontend\",\"description\":\"Frontend sends POST request\",\"files\":[{\"path\":\"src/api/client.ts\",\"lineStart\":10,\"lineEnd\":20}]}}],\"edges\":[{\"id\":\"e1-2\",\"source\":\"1\",\"target\":\"2\",\"label\":\"onClick\"}]}"
  }'
```

Verify:
- Flow card appears on `/workspaces/1/diagram`
- Clicking it opens the React Flow canvas
- Nodes are colored (blue for UX, green for frontend)
- Hover shows tooltip
- Click opens detail dialog
- Dragging nodes saves positions

**Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix: resolve any issues from end-to-end testing"
```
