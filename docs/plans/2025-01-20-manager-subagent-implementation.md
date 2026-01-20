# Manager + Subagent Architecture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace headless agent spawning with a persistent Claude Code manager that orchestrates subagents, using files as memory (Ralph + GSD techniques).

**Architecture:** Single Claude Code session spawns on server boot, receives UI commands, spawns subagents via Task tool with fresh 200k context each.

**Tech Stack:** Next.js, Drizzle ORM, SQLite, Claude Code CLI, node-pty for terminal

---

## Phase 1: Foundation

### Task 1.1: Create .bot-hq Initializer Library

**Files:**
- Create: `src/lib/bot-hq/index.ts`
- Create: `src/lib/bot-hq/templates.ts`

**Step 1: Create the initializer module**

```typescript
// src/lib/bot-hq/index.ts
import fs from "fs/promises";
import path from "path";
import { getDefaultManagerPrompt, getDefaultWorkspaceTemplate } from "./templates";

const BOT_HQ_ROOT = process.env.BOT_HQ_SCOPE || "/Users/gregoryerrl/Projects";
const BOT_HQ_DIR = path.join(BOT_HQ_ROOT, ".bot-hq");

export async function initializeBotHqStructure(): Promise<void> {
  // Create main .bot-hq directory
  await fs.mkdir(BOT_HQ_DIR, { recursive: true });
  await fs.mkdir(path.join(BOT_HQ_DIR, "workspaces"), { recursive: true });

  // Create MANAGER_PROMPT.md if it doesn't exist
  const managerPromptPath = path.join(BOT_HQ_DIR, "MANAGER_PROMPT.md");
  try {
    await fs.access(managerPromptPath);
  } catch {
    await fs.writeFile(managerPromptPath, getDefaultManagerPrompt());
  }

  // Create QUEUE.md if it doesn't exist
  const queuePath = path.join(BOT_HQ_DIR, "QUEUE.md");
  try {
    await fs.access(queuePath);
  } catch {
    await fs.writeFile(queuePath, "# Task Queue\n\nNo tasks currently running.\n");
  }
}

export async function initializeWorkspaceContext(workspaceName: string): Promise<void> {
  const workspaceDir = path.join(BOT_HQ_DIR, "workspaces", workspaceName);
  await fs.mkdir(workspaceDir, { recursive: true });
  await fs.mkdir(path.join(workspaceDir, "tasks"), { recursive: true });

  const workspaceMdPath = path.join(workspaceDir, "WORKSPACE.md");
  try {
    await fs.access(workspaceMdPath);
  } catch {
    await fs.writeFile(workspaceMdPath, getDefaultWorkspaceTemplate(workspaceName));
  }

  const stateMdPath = path.join(workspaceDir, "STATE.md");
  try {
    await fs.access(stateMdPath);
  } catch {
    await fs.writeFile(stateMdPath, "# Current State\n\nNo active state.\n");
  }
}

export async function getManagerPrompt(): Promise<string> {
  const promptPath = path.join(BOT_HQ_DIR, "MANAGER_PROMPT.md");
  try {
    return await fs.readFile(promptPath, "utf-8");
  } catch {
    return getDefaultManagerPrompt();
  }
}

export async function saveManagerPrompt(content: string): Promise<void> {
  const promptPath = path.join(BOT_HQ_DIR, "MANAGER_PROMPT.md");
  await fs.writeFile(promptPath, content);
}

export async function getWorkspaceContext(workspaceName: string): Promise<string> {
  const workspaceMdPath = path.join(BOT_HQ_DIR, "workspaces", workspaceName, "WORKSPACE.md");
  try {
    return await fs.readFile(workspaceMdPath, "utf-8");
  } catch {
    return "";
  }
}

export async function saveWorkspaceContext(workspaceName: string, content: string): Promise<void> {
  const workspaceDir = path.join(BOT_HQ_DIR, "workspaces", workspaceName);
  await fs.mkdir(workspaceDir, { recursive: true });
  await fs.writeFile(path.join(workspaceDir, "WORKSPACE.md"), content);
}

export async function getTaskProgress(workspaceName: string, taskId: number): Promise<string | null> {
  const progressPath = path.join(BOT_HQ_DIR, "workspaces", workspaceName, "tasks", String(taskId), "PROGRESS.md");
  try {
    return await fs.readFile(progressPath, "utf-8");
  } catch {
    return null;
  }
}

export async function cleanupTaskFiles(workspaceName: string, taskId: number): Promise<void> {
  const taskDir = path.join(BOT_HQ_DIR, "workspaces", workspaceName, "tasks", String(taskId));
  try {
    await fs.rm(taskDir, { recursive: true });
  } catch {
    // Directory may not exist
  }
}

export { BOT_HQ_DIR, BOT_HQ_ROOT };
```

**Step 2: Create templates module**

```typescript
// src/lib/bot-hq/templates.ts
export function getDefaultManagerPrompt(): string {
  return `# Bot-HQ Manager

You are the orchestration manager for bot-hq. You run as a persistent Claude Code session.

## Startup Tasks

On startup, perform these checks:
1. **Health check** - Verify all workspace paths exist and are valid git repos
2. **Cleanup** - Remove stale task files from .bot-hq/workspaces/*/tasks/
3. **Initialize** - Generate WORKSPACE.md for any workspace missing one
4. **Report** - Summarize what you found and fixed

## Awaiting Commands

After startup, wait for commands from the UI. When you receive a task command:

1. Read the task details from bot-hq
2. Read the workspace context from .bot-hq/workspaces/{name}/WORKSPACE.md
3. Read any previous progress from .bot-hq/workspaces/{name}/tasks/{id}/PROGRESS.md
4. Spawn a subagent with the Task tool to work on it
5. Monitor the subagent's progress
6. Handle completion, iteration, or escalation

## Subagent Spawning

When spawning a subagent, provide it with:
- Full workspace context (WORKSPACE.md)
- Current state (STATE.md)
- Previous progress if any (PROGRESS.md)
- Task description and success criteria
- Instructions to update PROGRESS.md on completion

## Iteration Loop

After a subagent completes:
- Read PROGRESS.md to check status
- If build passes and criteria met → task complete
- If same blocker 3x OR max iterations reached → escalate to needs_help
- Otherwise → spawn fresh subagent to continue

## Available Tools

You have access to bot-hq MCP tools:
- task_list, task_get, task_update
- workspace_list
- logs_get
- Read, Write, Edit, Glob, Grep, Bash

Stay lean. Delegate work to subagents. Don't accumulate context.
`;
}

export function getDefaultWorkspaceTemplate(workspaceName: string): string {
  return `# ${workspaceName}

## Overview

[Auto-generated workspace context. Edit this to add project-specific knowledge.]

## Architecture

[Describe the project architecture, key directories, patterns used.]

## Conventions

[List coding conventions, naming patterns, file organization rules.]

## Build & Test

[Document build commands, test commands, common tasks.]

## Known Issues

[Track known issues, gotchas, things to watch out for.]

## Recent Changes

[Track significant recent changes that affect how to work in this codebase.]
`;
}

export function getProgressTemplate(taskId: number, title: string): string {
  return `---
iteration: 1
max_iterations: 10
status: in_progress
blocker_hash: null
last_error: null
criteria_met: false
build_passes: false
---

# Task ${taskId}: ${title}

## Completed

(Nothing yet)

## Current Blocker

(None)

## Next Steps

- Start working on the task
`;
}
```

**Step 3: Run TypeScript compiler to verify**

```bash
cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit src/lib/bot-hq/index.ts src/lib/bot-hq/templates.ts
```

**Step 4: Commit**

```bash
git add src/lib/bot-hq/
git commit -m "feat: add .bot-hq structure initializer and templates"
```

---

### Task 1.2: Update Database Schema

**Files:**
- Modify: `src/lib/db/schema.ts`

**Step 1: Modify tasks table to add new fields and update states**

In `src/lib/db/schema.ts`, update the tasks table:

```typescript
// Tasks - update state enum and add new fields
export const tasks = sqliteTable("tasks", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id")
    .notNull()
    .references(() => workspaces.id),
  sourcePluginId: integer("source_plugin_id"),
  sourceRef: text("source_ref"),
  title: text("title").notNull(),
  description: text("description"),
  state: text("state", {
    enum: [
      "new",
      "queued",
      "in_progress",
      "needs_help",  // NEW: replaces stuck state
      "done",
      // REMOVED: "pending_review" - no longer needed
    ],
  })
    .notNull()
    .default("new"),
  priority: integer("priority").default(0),
  agentPlan: text("agent_plan"),
  branchName: text("branch_name"),
  // NEW FIELDS:
  completionCriteria: text("completion_criteria"),  // Task-specific success criteria
  iterationCount: integer("iteration_count").default(0),  // Current iteration
  maxIterations: integer("max_iterations"),  // Override global default
  feedback: text("feedback"),  // Human feedback on retry
  assignedAt: integer("assigned_at", { mode: "timestamp" }),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
}, (table) => [
  index("tasks_workspace_idx").on(table.workspaceId),
  index("tasks_state_idx").on(table.state),
]);
```

**Step 2: Remove approvals table export (comment out or delete)**

Comment out the entire `approvals` table definition and its type exports:

```typescript
// REMOVED: approvals table - replaced by git-native review
// export const approvals = sqliteTable("approvals", { ... });
```

**Step 3: Remove agentSessions table export (comment out or delete)**

Comment out the entire `agentSessions` table definition:

```typescript
// REMOVED: agentSessions table - single persistent manager session
// export const agentSessions = sqliteTable("agent_sessions", { ... });
```

**Step 4: Update type exports at bottom of file**

Remove or comment out:
```typescript
// export type Approval = typeof approvals.$inferSelect;
// export type NewApproval = typeof approvals.$inferInsert;
// export type AgentSession = typeof agentSessions.$inferSelect;
// export type NewAgentSession = typeof agentSessions.$inferInsert;
```

**Step 5: Generate migration**

```bash
cd /Users/gregoryerrl/Projects/bot-hq && npx drizzle-kit generate
```

**Step 6: Review and apply migration**

```bash
cd /Users/gregoryerrl/Projects/bot-hq && npx drizzle-kit push
```

**Step 7: Commit**

```bash
git add src/lib/db/schema.ts drizzle/
git commit -m "feat: update schema - add task iteration fields, remove approvals/agentSessions"
```

---

### Task 1.3: Add Manager Settings API

**Files:**
- Create: `src/app/api/manager-settings/route.ts`

**Step 1: Create the API route**

```typescript
// src/app/api/manager-settings/route.ts
import { NextResponse } from "next/server";
import { db, settings } from "@/lib/db";
import { eq } from "drizzle-orm";
import { getManagerPrompt, saveManagerPrompt } from "@/lib/bot-hq";

interface ManagerSettings {
  managerPrompt: string;
  maxIterations: number;
  stuckThreshold: number;
}

export async function GET() {
  try {
    // Get settings from database
    const maxIterationsRow = await db.query.settings.findFirst({
      where: eq(settings.key, "max_iterations"),
    });
    const stuckThresholdRow = await db.query.settings.findFirst({
      where: eq(settings.key, "stuck_threshold"),
    });

    // Get manager prompt from file
    const managerPrompt = await getManagerPrompt();

    const response: ManagerSettings = {
      managerPrompt,
      maxIterations: maxIterationsRow ? parseInt(maxIterationsRow.value) : 10,
      stuckThreshold: stuckThresholdRow ? parseInt(stuckThresholdRow.value) : 3,
    };

    return NextResponse.json(response);
  } catch (error) {
    console.error("Failed to get manager settings:", error);
    return NextResponse.json(
      { error: "Failed to get settings" },
      { status: 500 }
    );
  }
}

export async function PUT(request: Request) {
  try {
    const body: Partial<ManagerSettings> = await request.json();

    // Update manager prompt file
    if (body.managerPrompt !== undefined) {
      await saveManagerPrompt(body.managerPrompt);
    }

    // Update database settings
    if (body.maxIterations !== undefined) {
      await db
        .insert(settings)
        .values({
          key: "max_iterations",
          value: String(body.maxIterations),
        })
        .onConflictDoUpdate({
          target: settings.key,
          set: { value: String(body.maxIterations), updatedAt: new Date() },
        });
    }

    if (body.stuckThreshold !== undefined) {
      await db
        .insert(settings)
        .values({
          key: "stuck_threshold",
          value: String(body.stuckThreshold),
        })
        .onConflictDoUpdate({
          target: settings.key,
          set: { value: String(body.stuckThreshold), updatedAt: new Date() },
        });
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to update manager settings:", error);
    return NextResponse.json(
      { error: "Failed to update settings" },
      { status: 500 }
    );
  }
}
```

**Step 2: Verify the route compiles**

```bash
cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit src/app/api/manager-settings/route.ts
```

**Step 3: Commit**

```bash
git add src/app/api/manager-settings/
git commit -m "feat: add manager settings API for prompt and iteration config"
```

---

### Task 1.4: Add Workspace Context API

**Files:**
- Create: `src/app/api/workspaces/[id]/context/route.ts`

**Step 1: Create the API route**

```typescript
// src/app/api/workspaces/[id]/context/route.ts
import { NextResponse } from "next/server";
import { db, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";
import { getWorkspaceContext, saveWorkspaceContext, initializeWorkspaceContext } from "@/lib/bot-hq";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const workspaceId = parseInt(id);

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, workspaceId),
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    // Initialize if needed
    await initializeWorkspaceContext(workspace.name);

    const context = await getWorkspaceContext(workspace.name);

    return NextResponse.json({
      workspaceId,
      workspaceName: workspace.name,
      context,
    });
  } catch (error) {
    console.error("Failed to get workspace context:", error);
    return NextResponse.json(
      { error: "Failed to get workspace context" },
      { status: 500 }
    );
  }
}

export async function PUT(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const workspaceId = parseInt(id);
    const { context } = await request.json();

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, workspaceId),
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    await saveWorkspaceContext(workspace.name, context);

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to save workspace context:", error);
    return NextResponse.json(
      { error: "Failed to save workspace context" },
      { status: 500 }
    );
  }
}
```

**Step 2: Verify the route compiles**

```bash
cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit src/app/api/workspaces/[id]/context/route.ts
```

**Step 3: Commit**

```bash
git add src/app/api/workspaces/
git commit -m "feat: add workspace context API for WORKSPACE.md editing"
```

---

### Task 1.5: Add Manager Settings UI Component

**Files:**
- Create: `src/components/settings/manager-settings.tsx`

**Step 1: Create the component**

```typescript
// src/components/settings/manager-settings.tsx
"use client";

import { useState, useEffect } from "react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Save, RefreshCw, Loader2 } from "lucide-react";
import { useToast } from "@/hooks/use-toast";

interface ManagerSettings {
  managerPrompt: string;
  maxIterations: number;
  stuckThreshold: number;
}

export function ManagerSettings() {
  const [settings, setSettings] = useState<ManagerSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const { toast } = useToast();

  useEffect(() => {
    fetchSettings();
  }, []);

  const fetchSettings = async () => {
    try {
      setLoading(true);
      const response = await fetch("/api/manager-settings");
      if (!response.ok) throw new Error("Failed to fetch settings");
      const data = await response.json();
      setSettings(data);
    } catch (error) {
      toast({
        title: "Error",
        description: "Failed to load manager settings",
        variant: "destructive",
      });
    } finally {
      setLoading(false);
    }
  };

  const saveSettings = async () => {
    if (!settings) return;

    try {
      setSaving(true);
      const response = await fetch("/api/manager-settings", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(settings),
      });

      if (!response.ok) throw new Error("Failed to save settings");

      toast({
        title: "Saved",
        description: "Manager settings updated successfully",
      });
    } catch (error) {
      toast({
        title: "Error",
        description: "Failed to save settings",
        variant: "destructive",
      });
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        <span className="ml-2 text-muted-foreground">Loading settings...</span>
      </div>
    );
  }

  if (!settings) return null;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-lg font-semibold">Manager Configuration</h3>
          <p className="text-sm text-muted-foreground">
            Configure the persistent Claude Code manager session
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={fetchSettings}>
            <RefreshCw className="h-4 w-4 mr-2" />
            Refresh
          </Button>
          <Button size="sm" onClick={saveSettings} disabled={saving}>
            <Save className="h-4 w-4 mr-2" />
            {saving ? "Saving..." : "Save"}
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Iteration Settings</CardTitle>
          <CardDescription>Control how subagents iterate on tasks</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="maxIterations">Max Iterations</Label>
              <Input
                id="maxIterations"
                type="number"
                min={1}
                max={50}
                value={settings.maxIterations}
                onChange={(e) =>
                  setSettings({ ...settings, maxIterations: parseInt(e.target.value) || 10 })
                }
              />
              <p className="text-xs text-muted-foreground">
                Maximum attempts before escalating to needs_help
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="stuckThreshold">Stuck Threshold</Label>
              <Input
                id="stuckThreshold"
                type="number"
                min={1}
                max={10}
                value={settings.stuckThreshold}
                onChange={(e) =>
                  setSettings({ ...settings, stuckThreshold: parseInt(e.target.value) || 3 })
                }
              />
              <p className="text-xs text-muted-foreground">
                Same blocker N times triggers early escalation
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Manager Prompt</CardTitle>
          <CardDescription>
            Instructions given to the manager on startup (MANAGER_PROMPT.md)
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Textarea
            value={settings.managerPrompt}
            onChange={(e) => setSettings({ ...settings, managerPrompt: e.target.value })}
            className="min-h-[400px] font-mono text-sm"
            placeholder="Enter manager prompt..."
          />
        </CardContent>
      </Card>
    </div>
  );
}
```

**Step 2: Verify the component compiles**

```bash
cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit src/components/settings/manager-settings.tsx
```

**Step 3: Commit**

```bash
git add src/components/settings/manager-settings.tsx
git commit -m "feat: add manager settings UI component"
```

---

### Task 1.6: Add Manager Tab to Settings Page

**Files:**
- Modify: `src/app/settings/page.tsx`

**Step 1: Import the ManagerSettings component**

Add at top of file:
```typescript
import { ManagerSettings } from "@/components/settings/manager-settings";
```

**Step 2: Add Manager tab to TabsList**

Find the `<TabsList>` and add:
```typescript
<TabsTrigger value="manager" className="flex-1 sm:flex-initial">
  Manager
</TabsTrigger>
```

**Step 3: Add Manager TabsContent**

Add after the "claude" TabsContent:
```typescript
<TabsContent value="manager" className="space-y-6">
  <ManagerSettings />
</TabsContent>
```

**Step 4: Verify the page compiles**

```bash
cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit src/app/settings/page.tsx
```

**Step 5: Commit**

```bash
git add src/app/settings/page.tsx
git commit -m "feat: add Manager tab to settings page"
```

---

## Phase 2: Manager Session

### Task 2.1: Create Persistent Manager Spawner

**Files:**
- Create: `src/lib/manager/persistent-manager.ts`

**Step 1: Create the persistent manager module**

```typescript
// src/lib/manager/persistent-manager.ts
import { spawn, ChildProcess } from "child_process";
import { EventEmitter } from "events";
import { getManagerPrompt, initializeBotHqStructure, BOT_HQ_ROOT } from "@/lib/bot-hq";

class PersistentManager extends EventEmitter {
  private process: ChildProcess | null = null;
  private isRunning = false;
  private outputBuffer = "";

  async start(): Promise<void> {
    if (this.isRunning) {
      console.log("[Manager] Already running");
      return;
    }

    // Initialize .bot-hq structure
    await initializeBotHqStructure();

    // Get manager prompt
    const managerPrompt = await getManagerPrompt();

    console.log("[Manager] Starting persistent session...");

    this.process = spawn("claude", [
      "--dangerously-skip-permissions",
      "-p",
      "--output-format", "stream-json",
      "--mcp-config", "/Users/gregoryerrl/Projects/bot-hq/.mcp.json",
    ], {
      cwd: BOT_HQ_ROOT,
      env: { ...process.env },
    });

    this.isRunning = true;

    // Send startup prompt
    this.process.stdin?.write(managerPrompt);
    this.process.stdin?.write("\n\nPerform your startup tasks now.\n");
    // Don't end stdin - keep it open for commands

    this.process.stdout?.on("data", (data: Buffer) => {
      const text = data.toString();
      this.outputBuffer += text;
      this.emit("output", text);

      // Parse JSON lines
      const lines = this.outputBuffer.split("\n");
      this.outputBuffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const output = JSON.parse(line);
          this.emit("json", output);

          // Extract text for display
          if (output.type === "assistant" && output.message?.content) {
            for (const block of output.message.content) {
              if (block.type === "text") {
                this.emit("text", block.text);
              }
            }
          }
        } catch {
          // Non-JSON output
          this.emit("text", line);
        }
      }
    });

    this.process.stderr?.on("data", (data: Buffer) => {
      const text = data.toString();
      console.error("[Manager stderr]", text);
      this.emit("stderr", text);
    });

    this.process.on("error", (err) => {
      console.error("[Manager] Process error:", err);
      this.isRunning = false;
      this.emit("error", err);
    });

    this.process.on("exit", (code) => {
      console.log("[Manager] Process exited with code:", code);
      this.isRunning = false;
      this.emit("exit", code);
    });
  }

  sendCommand(command: string): void {
    if (!this.process || !this.isRunning) {
      console.error("[Manager] Cannot send command - not running");
      return;
    }

    this.process.stdin?.write(command + "\n");
  }

  stop(): void {
    if (this.process) {
      this.process.kill("SIGTERM");
      this.isRunning = false;
    }
  }

  getStatus(): { running: boolean; pid: number | null } {
    return {
      running: this.isRunning,
      pid: this.process?.pid || null,
    };
  }
}

// Singleton instance
let managerInstance: PersistentManager | null = null;

export function getManager(): PersistentManager {
  if (!managerInstance) {
    managerInstance = new PersistentManager();
  }
  return managerInstance;
}

export async function startManager(): Promise<void> {
  const manager = getManager();
  await manager.start();
}

export function stopManager(): void {
  if (managerInstance) {
    managerInstance.stop();
  }
}

export function sendManagerCommand(command: string): void {
  const manager = getManager();
  manager.sendCommand(command);
}

export function getManagerStatus(): { running: boolean; pid: number | null } {
  if (!managerInstance) {
    return { running: false, pid: null };
  }
  return managerInstance.getStatus();
}
```

**Step 2: Verify it compiles**

```bash
cd /Users/gregoryerrl/Projects/bot-hq && npx tsc --noEmit src/lib/manager/persistent-manager.ts
```

**Step 3: Commit**

```bash
git add src/lib/manager/
git commit -m "feat: add persistent manager spawner module"
```

---

### Task 2.2: Add Manager Startup to Instrumentation

**Files:**
- Modify: `src/instrumentation.ts`

**Step 1: Update instrumentation to start manager**

```typescript
// src/instrumentation.ts
export async function register() {
  if (process.env.NEXT_RUNTIME === "nodejs") {
    const { initializeAgentDocs } = await import("@/lib/agent-docs");
    await initializeAgentDocs();

    // Initialize plugins
    try {
      const { initializePlugins } = await import("@/lib/plugins");
      await initializePlugins();
      console.log("Plugins initialized");
    } catch (error) {
      console.error("Failed to initialize plugins:", error);
    }

    // Start persistent manager
    try {
      const { startManager } = await import("@/lib/manager/persistent-manager");
      await startManager();
      console.log("Manager session started");
    } catch (error) {
      console.error("Failed to start manager:", error);
    }
  }
}
```

**Step 2: Commit**

```bash
git add src/instrumentation.ts
git commit -m "feat: start persistent manager on server boot"
```

---

### Task 2.3: Create Manager Status API

**Files:**
- Create: `src/app/api/manager/status/route.ts`

**Step 1: Create the status endpoint**

```typescript
// src/app/api/manager/status/route.ts
import { NextResponse } from "next/server";
import { getManagerStatus } from "@/lib/manager/persistent-manager";

export async function GET() {
  try {
    const status = getManagerStatus();
    return NextResponse.json(status);
  } catch (error) {
    console.error("Failed to get manager status:", error);
    return NextResponse.json(
      { error: "Failed to get status", running: false, pid: null },
      { status: 500 }
    );
  }
}
```

**Step 2: Commit**

```bash
git add src/app/api/manager/status/
git commit -m "feat: add manager status API endpoint"
```

---

### Task 2.4: Create Manager Command API

**Files:**
- Create: `src/app/api/manager/command/route.ts`

**Step 1: Create the command endpoint**

```typescript
// src/app/api/manager/command/route.ts
import { NextResponse } from "next/server";
import { sendManagerCommand, getManagerStatus } from "@/lib/manager/persistent-manager";

export async function POST(request: Request) {
  try {
    const { command } = await request.json();

    if (!command || typeof command !== "string") {
      return NextResponse.json(
        { error: "Command is required" },
        { status: 400 }
      );
    }

    const status = getManagerStatus();
    if (!status.running) {
      return NextResponse.json(
        { error: "Manager is not running" },
        { status: 503 }
      );
    }

    sendManagerCommand(command);

    return NextResponse.json({ success: true, message: "Command sent" });
  } catch (error) {
    console.error("Failed to send command:", error);
    return NextResponse.json(
      { error: "Failed to send command" },
      { status: 500 }
    );
  }
}
```

**Step 2: Commit**

```bash
git add src/app/api/manager/command/
git commit -m "feat: add manager command API endpoint"
```

---

## Phase 3: Task Flow

### Task 3.1: Update Task Card to Send Manager Command

**Files:**
- Modify: `src/components/taskboard/task-card.tsx`

**Step 1: Update the onStartAgent handler**

Replace the Start button's onClick behavior. The parent component (TaskList) will need to be updated to send the command to the manager instead of starting a headless agent.

First, update the TaskCard component to use a different prop name for clarity:

```typescript
interface TaskCardProps {
  task: Task & { workspaceName?: string };
  onAssign: (taskId: number) => void;
  onStartTask: (taskId: number) => void;  // Renamed from onStartAgent
}

// ... in the JSX, update the button:
{task.state === "queued" && (
  <Button size="sm" onClick={() => onStartTask(task.id)}>
    <Play className="h-4 w-4 mr-1" />
    Start
  </Button>
)}
```

**Step 2: Commit**

```bash
git add src/components/taskboard/task-card.tsx
git commit -m "refactor: rename onStartAgent to onStartTask in TaskCard"
```

---

### Task 3.2: Update TaskList to Send Manager Commands

**Files:**
- Modify: `src/components/taskboard/task-list.tsx`

**Step 1: Read the current file first**

Read the file to understand the current implementation.

**Step 2: Update the startAgent function to send manager command**

Replace the fetch to `/api/agents/start` with a fetch to `/api/manager/command`:

```typescript
const handleStartTask = async (taskId: number) => {
  try {
    // First update task state to in_progress
    await fetch(`/api/tasks/${taskId}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ state: "in_progress" }),
    });

    // Send command to manager
    const response = await fetch("/api/manager/command", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        command: `Start working on task ${taskId}. Use the task_get tool to fetch the task details, then spawn a subagent to work on it.`,
      }),
    });

    if (!response.ok) {
      throw new Error("Failed to send command to manager");
    }

    toast({
      title: "Task Started",
      description: `Task ${taskId} sent to manager`,
    });

    fetchTasks();
  } catch (error) {
    console.error("Failed to start task:", error);
    toast({
      title: "Error",
      description: "Failed to start task",
      variant: "destructive",
    });
  }
};
```

**Step 3: Update the TaskCard usage to use the new prop name**

```typescript
<TaskCard
  key={task.id}
  task={task}
  onAssign={handleAssign}
  onStartTask={handleStartTask}  // Updated prop name
/>
```

**Step 4: Commit**

```bash
git add src/components/taskboard/task-list.tsx
git commit -m "feat: update TaskList to send commands to manager instead of spawning agents"
```

---

### Task 3.3: Add needs_help State UI

**Files:**
- Modify: `src/components/taskboard/task-card.tsx`

**Step 1: Add needs_help color and label**

Update the stateColors and stateLabels objects:

```typescript
const stateColors: Record<string, string> = {
  new: "bg-gray-500",
  queued: "bg-yellow-500",
  in_progress: "bg-orange-500",
  needs_help: "bg-red-500",  // NEW
  done: "bg-green-700",
};

const stateLabels: Record<string, string> = {
  new: "New",
  queued: "Queued",
  in_progress: "In Progress",
  needs_help: "Needs Help",  // NEW
  done: "Done",
};
```

**Step 2: Add Retry button for needs_help state**

```typescript
{task.state === "needs_help" && (
  <Button size="sm" variant="outline" onClick={() => onRetry?.(task.id)}>
    Retry
  </Button>
)}
```

**Step 3: Add onRetry to props**

```typescript
interface TaskCardProps {
  task: Task & { workspaceName?: string };
  onAssign: (taskId: number) => void;
  onStartTask: (taskId: number) => void;
  onRetry?: (taskId: number) => void;  // NEW
}
```

**Step 4: Commit**

```bash
git add src/components/taskboard/task-card.tsx
git commit -m "feat: add needs_help state UI with Retry button"
```

---

## Phase 4: Review Flow

### Task 4.1: Create Diff Review Component

**Files:**
- Create: `src/components/review/diff-review-card.tsx`

**Step 1: Create the component**

```typescript
// src/components/review/diff-review-card.tsx
"use client";

import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Check, X, RotateCcw, GitBranch, FileCode } from "lucide-react";
import { Task } from "@/lib/db/schema";

interface DiffFile {
  filename: string;
  additions: number;
  deletions: number;
}

interface DiffReviewCardProps {
  task: Task & { workspaceName?: string };
  diff: {
    branch: string;
    baseBranch: string;
    files: DiffFile[];
    totalAdditions: number;
    totalDeletions: number;
  };
  onAccept: (taskId: number) => void;
  onReject: (taskId: number) => void;
  onRetry: (taskId: number, feedback: string) => void;
}

export function DiffReviewCard({
  task,
  diff,
  onAccept,
  onReject,
  onRetry,
}: DiffReviewCardProps) {
  const [feedback, setFeedback] = useState("");
  const [showFeedback, setShowFeedback] = useState(false);

  const handleRetry = () => {
    if (feedback.trim()) {
      onRetry(task.id, feedback);
      setFeedback("");
      setShowFeedback(false);
    }
  };

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              <span>#{task.id}</span>
              <span>{task.title}</span>
            </CardTitle>
            <div className="flex items-center gap-2 mt-2 text-sm text-muted-foreground">
              <GitBranch className="h-4 w-4" />
              <code>{diff.branch}</code>
              <span>→</span>
              <code>{diff.baseBranch}</code>
            </div>
          </div>
          {task.workspaceName && (
            <Badge variant="outline">{task.workspaceName}</Badge>
          )}
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* File changes */}
        <div className="border rounded-lg p-3 space-y-2">
          <div className="flex items-center justify-between text-sm">
            <span className="font-medium">Changes</span>
            <div className="flex gap-2">
              <span className="text-green-600">+{diff.totalAdditions}</span>
              <span className="text-red-600">-{diff.totalDeletions}</span>
            </div>
          </div>
          <div className="space-y-1 max-h-48 overflow-y-auto">
            {diff.files.map((file, i) => (
              <div key={i} className="flex items-center justify-between text-xs">
                <div className="flex items-center gap-2 truncate">
                  <FileCode className="h-3 w-3 text-muted-foreground" />
                  <span className="truncate">{file.filename}</span>
                </div>
                <div className="flex gap-2 flex-shrink-0">
                  <span className="text-green-600">+{file.additions}</span>
                  <span className="text-red-600">-{file.deletions}</span>
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* Feedback input */}
        {showFeedback && (
          <div className="space-y-2">
            <Textarea
              placeholder="What should be changed?"
              value={feedback}
              onChange={(e) => setFeedback(e.target.value)}
              className="min-h-[100px]"
            />
            <div className="flex gap-2">
              <Button size="sm" onClick={handleRetry} disabled={!feedback.trim()}>
                Send Feedback
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => setShowFeedback(false)}
              >
                Cancel
              </Button>
            </div>
          </div>
        )}

        {/* Action buttons */}
        {!showFeedback && (
          <div className="flex gap-2">
            <Button
              className="flex-1"
              onClick={() => onAccept(task.id)}
            >
              <Check className="h-4 w-4 mr-2" />
              Accept
            </Button>
            <Button
              variant="outline"
              onClick={() => setShowFeedback(true)}
            >
              <RotateCcw className="h-4 w-4 mr-2" />
              Retry
            </Button>
            <Button
              variant="destructive"
              onClick={() => onReject(task.id)}
            >
              <X className="h-4 w-4 mr-2" />
              Reject & Remove
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
```

**Step 2: Commit**

```bash
git add src/components/review/
git commit -m "feat: add DiffReviewCard component for git-native review"
```

---

### Task 4.2: Create Review API Endpoints

**Files:**
- Create: `src/app/api/tasks/[id]/review/route.ts`

**Step 1: Create the review endpoint**

```typescript
// src/app/api/tasks/[id]/review/route.ts
import { NextResponse } from "next/server";
import { db, tasks, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";
import { execSync } from "child_process";
import { cleanupTaskFiles } from "@/lib/bot-hq";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const taskId = parseInt(id);

    const task = await db.query.tasks.findFirst({
      where: eq(tasks.id, taskId),
    });

    if (!task) {
      return NextResponse.json({ error: "Task not found" }, { status: 404 });
    }

    if (!task.branchName) {
      return NextResponse.json({ error: "No branch for this task" }, { status: 400 });
    }

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, task.workspaceId),
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    // Get diff info from git
    const baseBranch = "main";
    const diffNumstat = execSync(
      `git diff ${baseBranch}...${task.branchName} --numstat`,
      { cwd: workspace.repoPath, encoding: "utf-8" }
    );

    const files = diffNumstat
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const [additions, deletions, filename] = line.split("\t");
        return {
          filename,
          additions: parseInt(additions) || 0,
          deletions: parseInt(deletions) || 0,
        };
      });

    const totalAdditions = files.reduce((sum, f) => sum + f.additions, 0);
    const totalDeletions = files.reduce((sum, f) => sum + f.deletions, 0);

    return NextResponse.json({
      task,
      diff: {
        branch: task.branchName,
        baseBranch,
        files,
        totalAdditions,
        totalDeletions,
      },
    });
  } catch (error) {
    console.error("Failed to get review data:", error);
    return NextResponse.json(
      { error: "Failed to get review data" },
      { status: 500 }
    );
  }
}

export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const taskId = parseInt(id);
    const { action, feedback } = await request.json();

    const task = await db.query.tasks.findFirst({
      where: eq(tasks.id, taskId),
    });

    if (!task) {
      return NextResponse.json({ error: "Task not found" }, { status: 404 });
    }

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, task.workspaceId),
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    switch (action) {
      case "accept": {
        // Push branch to remote
        if (task.branchName) {
          execSync(`git push -u origin ${task.branchName}`, {
            cwd: workspace.repoPath,
          });
        }

        // Update task state
        await db
          .update(tasks)
          .set({ state: "done", updatedAt: new Date() })
          .where(eq(tasks.id, taskId));

        // Cleanup task files
        await cleanupTaskFiles(workspace.name, taskId);

        return NextResponse.json({ success: true, action: "accepted" });
      }

      case "reject": {
        // Delete branch
        if (task.branchName) {
          execSync(`git checkout main`, { cwd: workspace.repoPath });
          execSync(`git branch -D ${task.branchName}`, { cwd: workspace.repoPath });
        }

        // Cleanup task files
        await cleanupTaskFiles(workspace.name, taskId);

        // Delete task
        await db.delete(tasks).where(eq(tasks.id, taskId));

        return NextResponse.json({ success: true, action: "rejected" });
      }

      case "retry": {
        // Update task with feedback and requeue
        await db
          .update(tasks)
          .set({
            state: "queued",
            feedback: feedback || null,
            iterationCount: (task.iterationCount || 0) + 1,
            updatedAt: new Date(),
          })
          .where(eq(tasks.id, taskId));

        return NextResponse.json({ success: true, action: "retry" });
      }

      default:
        return NextResponse.json({ error: "Invalid action" }, { status: 400 });
    }
  } catch (error) {
    console.error("Failed to process review action:", error);
    return NextResponse.json(
      { error: "Failed to process action" },
      { status: 500 }
    );
  }
}
```

**Step 2: Commit**

```bash
git add src/app/api/tasks/[id]/review/
git commit -m "feat: add review API for accept/reject/retry actions"
```

---

## Phase 5: Cleanup

### Task 5.1: Remove Old Agent Code

**Files:**
- Delete: `src/lib/agents/claude-code.ts`
- Delete: `src/app/api/agents/start/route.ts`
- Delete: `src/app/api/agents/stop/route.ts`
- Delete: `src/app/api/agents/sessions/route.ts`
- Delete: `src/app/api/approvals/` directory
- Delete: `src/mcp/tools/approvals.ts`
- Delete: `src/components/pending-board/` directory

**Step 1: Remove files**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
rm -f src/lib/agents/claude-code.ts
rm -rf src/app/api/agents/
rm -rf src/app/api/approvals/
rm -f src/mcp/tools/approvals.ts
rm -rf src/components/pending-board/
```

**Step 2: Update MCP server to remove approval tools**

Modify `src/mcp/server.ts` to remove approval tool registrations.

**Step 3: Commit**

```bash
git add -A
git commit -m "chore: remove old agent and approval code"
```

---

### Task 5.2: Update MCP Tools

**Files:**
- Modify: `src/mcp/tools/agents.ts`
- Modify: `src/mcp/server.ts`

**Step 1: Simplify agents.ts to only have status**

The agent tools should now just report on the manager status, not spawn agents.

**Step 2: Remove approval tool imports from server.ts**

**Step 3: Commit**

```bash
git add src/mcp/
git commit -m "refactor: update MCP tools - remove approvals, simplify agents"
```

---

### Task 5.3: Final Integration Test

**Step 1: Start the dev server**

```bash
cd /Users/gregoryerrl/Projects/bot-hq && npm run dev
```

**Step 2: Verify manager starts on boot**

Check console for "Manager session started" message.

**Step 3: Test creating a workspace and task**

1. Create a workspace via UI
2. Create a task for that workspace
3. Assign the task (new → queued)
4. Click Start (sends command to manager)
5. Verify manager receives command in terminal

**Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix: integration fixes for manager + subagent architecture"
```

---

## Summary

This plan transforms bot-hq from spawning multiple headless Claude processes to:

1. **Single persistent manager** - Spawns on server boot, lives in terminal tab
2. **Subagent orchestration** - Manager uses Task tool for fresh 200k context per task
3. **Files as memory** - .bot-hq/ structure with WORKSPACE.md, STATE.md, PROGRESS.md
4. **Git-native review** - Accept/Reject/Retry via diff UI, no approvals table
5. **UI-driven** - All settings editable in bot-hq, commands sent via API

Total tasks: ~15 bite-sized steps across 5 phases.
