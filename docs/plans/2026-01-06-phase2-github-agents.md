# Phase 2: GitHub Sync & Agents Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable automatic GitHub issue sync, task assignment, Claude Code agent execution, and approval workflow.

**Architecture:** GitHub CLI (`gh`) fetches issues per workspace. Tasks stored in SQLite. Claude Code runs as subprocess per task with JSON output parsing. Approval queue gates external actions. SSE streams logs to UI.

**Tech Stack:** GitHub CLI, Claude Code CLI, Node.js child_process, Server-Sent Events, React

---

## Task 1: GitHub Integration Library

**Files:**
- Create: `src/lib/github/index.ts`
- Create: `src/lib/github/types.ts`

**Step 1: Create GitHub types**

Create `src/lib/github/types.ts`:

```typescript
export interface GitHubIssue {
  number: number;
  title: string;
  body: string;
  state: "open" | "closed";
  labels: string[];
  assignees: string[];
  createdAt: string;
  updatedAt: string;
  url: string;
}

export interface GitHubRepo {
  owner: string;
  name: string;
  fullName: string;
}

export function parseGitHubRemote(remote: string): GitHubRepo | null {
  // Handles: owner/repo, https://github.com/owner/repo, git@github.com:owner/repo
  const patterns = [
    /^([^/]+)\/([^/]+)$/,
    /github\.com\/([^/]+)\/([^/]+?)(?:\.git)?$/,
    /github\.com:([^/]+)\/([^/]+?)(?:\.git)?$/,
  ];

  for (const pattern of patterns) {
    const match = remote.match(pattern);
    if (match) {
      return {
        owner: match[1],
        name: match[2],
        fullName: `${match[1]}/${match[2]}`,
      };
    }
  }
  return null;
}
```

**Step 2: Create GitHub client**

Create `src/lib/github/index.ts`:

```typescript
import { exec } from "child_process";
import { promisify } from "util";
import { GitHubIssue, GitHubRepo, parseGitHubRemote } from "./types";

const execAsync = promisify(exec);

export async function checkGhAuth(): Promise<boolean> {
  try {
    await execAsync("gh auth status");
    return true;
  } catch {
    return false;
  }
}

export async function fetchIssues(
  repo: GitHubRepo,
  state: "open" | "closed" | "all" = "open"
): Promise<GitHubIssue[]> {
  try {
    const { stdout } = await execAsync(
      `gh issue list --repo ${repo.fullName} --state ${state} --json number,title,body,state,labels,assignees,createdAt,updatedAt,url --limit 100`
    );

    const issues = JSON.parse(stdout);
    return issues.map((issue: Record<string, unknown>) => ({
      number: issue.number,
      title: issue.title,
      body: issue.body || "",
      state: issue.state,
      labels: (issue.labels as Array<{ name: string }>)?.map((l) => l.name) || [],
      assignees: (issue.assignees as Array<{ login: string }>)?.map((a) => a.login) || [],
      createdAt: issue.createdAt,
      updatedAt: issue.updatedAt,
      url: issue.url,
    }));
  } catch (error) {
    console.error(`Failed to fetch issues for ${repo.fullName}:`, error);
    return [];
  }
}

export async function createLinkedBranch(
  repo: GitHubRepo,
  issueNumber: number,
  cwd: string
): Promise<string | null> {
  try {
    // gh issue develop creates a branch linked to the issue
    const { stdout } = await execAsync(
      `gh issue develop ${issueNumber} --repo ${repo.fullName} --checkout`,
      { cwd }
    );
    // Extract branch name from output
    const match = stdout.match(/Switched to.*branch '([^']+)'/);
    return match ? match[1] : `${issueNumber}-issue`;
  } catch (error) {
    console.error(`Failed to create linked branch:`, error);
    return null;
  }
}

export async function createDraftPR(
  repo: GitHubRepo,
  branch: string,
  title: string,
  body: string,
  cwd: string
): Promise<string | null> {
  try {
    const { stdout } = await execAsync(
      `gh pr create --repo ${repo.fullName} --head ${branch} --title "${title.replace(/"/g, '\\"')}" --body "${body.replace(/"/g, '\\"')}" --draft`,
      { cwd }
    );
    // Extract PR URL from output
    const match = stdout.match(/(https:\/\/github\.com\/[^\s]+)/);
    return match ? match[1] : null;
  } catch (error) {
    console.error(`Failed to create draft PR:`, error);
    return null;
  }
}

export { parseGitHubRemote, type GitHubIssue, type GitHubRepo };
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add GitHub integration library"
```

---

## Task 2: Issue Sync API

**Files:**
- Create: `src/app/api/sync/route.ts`
- Create: `src/lib/sync/index.ts`

**Step 1: Create sync logic**

Create `src/lib/sync/index.ts`:

```typescript
import { db, workspaces, tasks, logs } from "@/lib/db";
import { fetchIssues, parseGitHubRemote } from "@/lib/github";
import { eq, and } from "drizzle-orm";

export async function syncWorkspaceIssues(workspaceId: number): Promise<{
  added: number;
  updated: number;
  errors: string[];
}> {
  const result = { added: 0, updated: 0, errors: [] as string[] };

  // Get workspace
  const workspace = await db.query.workspaces.findFirst({
    where: eq(workspaces.id, workspaceId),
  });

  if (!workspace?.githubRemote) {
    result.errors.push("Workspace has no GitHub remote configured");
    return result;
  }

  const repo = parseGitHubRemote(workspace.githubRemote);
  if (!repo) {
    result.errors.push(`Invalid GitHub remote: ${workspace.githubRemote}`);
    return result;
  }

  // Fetch issues from GitHub
  const issues = await fetchIssues(repo, "open");

  for (const issue of issues) {
    // Check if task already exists
    const existing = await db.query.tasks.findFirst({
      where: and(
        eq(tasks.workspaceId, workspaceId),
        eq(tasks.githubIssueNumber, issue.number)
      ),
    });

    if (existing) {
      // Update existing task if not already in progress
      if (existing.state === "new") {
        await db
          .update(tasks)
          .set({
            title: issue.title,
            description: issue.body,
            updatedAt: new Date(),
          })
          .where(eq(tasks.id, existing.id));
        result.updated++;
      }
    } else {
      // Create new task
      await db.insert(tasks).values({
        workspaceId,
        githubIssueNumber: issue.number,
        title: issue.title,
        description: issue.body,
        state: "new",
        priority: issue.labels.includes("priority:high") ? 1 : 0,
      });
      result.added++;
    }
  }

  // Log sync result
  await db.insert(logs).values({
    workspaceId,
    type: "sync",
    message: `Synced ${result.added} new, ${result.updated} updated issues`,
    details: JSON.stringify({ repo: repo.fullName, issues: issues.length }),
  });

  return result;
}

export async function syncAllWorkspaces(): Promise<{
  workspaces: number;
  added: number;
  updated: number;
  errors: string[];
}> {
  const result = { workspaces: 0, added: 0, updated: 0, errors: [] as string[] };

  const allWorkspaces = await db.select().from(workspaces);

  for (const workspace of allWorkspaces) {
    if (!workspace.githubRemote) continue;

    result.workspaces++;
    const syncResult = await syncWorkspaceIssues(workspace.id);
    result.added += syncResult.added;
    result.updated += syncResult.updated;
    result.errors.push(...syncResult.errors);
  }

  return result;
}
```

**Step 2: Create sync API route**

Create `src/app/api/sync/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { syncAllWorkspaces, syncWorkspaceIssues } from "@/lib/sync";

export async function POST(request: NextRequest) {
  try {
    const body = await request.json().catch(() => ({}));
    const { workspaceId } = body;

    if (workspaceId) {
      const result = await syncWorkspaceIssues(workspaceId);
      return NextResponse.json(result);
    } else {
      const result = await syncAllWorkspaces();
      return NextResponse.json(result);
    }
  } catch (error) {
    console.error("Sync failed:", error);
    return NextResponse.json(
      { error: "Sync failed" },
      { status: 500 }
    );
  }
}
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add issue sync API"
```

---

## Task 3: Tasks API Routes

**Files:**
- Create: `src/app/api/tasks/route.ts`
- Create: `src/app/api/tasks/[id]/route.ts`
- Create: `src/app/api/tasks/[id]/assign/route.ts`

**Step 1: Create tasks list route**

Create `src/app/api/tasks/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, tasks, workspaces } from "@/lib/db";
import { eq, desc } from "drizzle-orm";

export async function GET(request: NextRequest) {
  try {
    const workspaceId = request.nextUrl.searchParams.get("workspaceId");
    const state = request.nextUrl.searchParams.get("state");

    let query = db
      .select({
        task: tasks,
        workspace: workspaces,
      })
      .from(tasks)
      .leftJoin(workspaces, eq(tasks.workspaceId, workspaces.id))
      .orderBy(desc(tasks.updatedAt));

    const allTasks = await query;

    // Filter in memory (simpler than dynamic where clauses)
    let filtered = allTasks;
    if (workspaceId) {
      filtered = filtered.filter(
        (t) => t.task.workspaceId === parseInt(workspaceId)
      );
    }
    if (state) {
      filtered = filtered.filter((t) => t.task.state === state);
    }

    return NextResponse.json(
      filtered.map((t) => ({
        ...t.task,
        workspaceName: t.workspace?.name,
      }))
    );
  } catch (error) {
    console.error("Failed to fetch tasks:", error);
    return NextResponse.json(
      { error: "Failed to fetch tasks" },
      { status: 500 }
    );
  }
}
```

**Step 2: Create single task route**

Create `src/app/api/tasks/[id]/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, tasks } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function GET(
  request: NextRequest,
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

    return NextResponse.json(task);
  } catch (error) {
    console.error("Failed to fetch task:", error);
    return NextResponse.json(
      { error: "Failed to fetch task" },
      { status: 500 }
    );
  }
}

export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const body = await request.json();

    const updates: Partial<typeof tasks.$inferInsert> = {
      updatedAt: new Date(),
    };

    if (body.state !== undefined) updates.state = body.state;
    if (body.agentPlan !== undefined) updates.agentPlan = body.agentPlan;
    if (body.branchName !== undefined) updates.branchName = body.branchName;
    if (body.prUrl !== undefined) updates.prUrl = body.prUrl;
    if (body.priority !== undefined) updates.priority = body.priority;

    const result = await db
      .update(tasks)
      .set(updates)
      .where(eq(tasks.id, parseInt(id)))
      .returning();

    if (result.length === 0) {
      return NextResponse.json({ error: "Task not found" }, { status: 404 });
    }

    return NextResponse.json(result[0]);
  } catch (error) {
    console.error("Failed to update task:", error);
    return NextResponse.json(
      { error: "Failed to update task" },
      { status: 500 }
    );
  }
}
```

**Step 3: Create assign route**

Create `src/app/api/tasks/[id]/assign/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, tasks, logs } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(
  request: NextRequest,
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

    if (task.state !== "new") {
      return NextResponse.json(
        { error: "Task is already assigned or in progress" },
        { status: 400 }
      );
    }

    // Update task to queued
    const result = await db
      .update(tasks)
      .set({
        state: "queued",
        assignedAt: new Date(),
        updatedAt: new Date(),
      })
      .where(eq(tasks.id, parseInt(id)))
      .returning();

    // Log assignment
    await db.insert(logs).values({
      workspaceId: task.workspaceId,
      taskId: task.id,
      type: "agent",
      message: `Task #${task.githubIssueNumber || task.id} queued for agent`,
    });

    return NextResponse.json(result[0]);
  } catch (error) {
    console.error("Failed to assign task:", error);
    return NextResponse.json(
      { error: "Failed to assign task" },
      { status: 500 }
    );
  }
}
```

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: add tasks API routes"
```

---

## Task 4: Claude Code Agent Library

**Files:**
- Create: `src/lib/agents/claude-code.ts`
- Create: `src/lib/agents/types.ts`

**Step 1: Create agent types**

Create `src/lib/agents/types.ts`:

```typescript
export interface AgentMessage {
  type: "assistant" | "user" | "system" | "result";
  content: string;
  timestamp: Date;
}

export interface AgentOutput {
  type: "text" | "tool_use" | "tool_result" | "error";
  content: string;
  toolName?: string;
  toolInput?: Record<string, unknown>;
}

export interface AgentSession {
  id: number;
  workspaceId: number;
  taskId: number | null;
  pid: number | null;
  status: "running" | "idle" | "stopped" | "error";
  messages: AgentMessage[];
}

export type AgentEventHandler = (event: {
  type: "output" | "error" | "exit" | "approval_needed";
  data: unknown;
}) => void;
```

**Step 2: Create Claude Code wrapper**

Create `src/lib/agents/claude-code.ts`:

```typescript
import { spawn, ChildProcess } from "child_process";
import { AgentOutput, AgentEventHandler } from "./types";
import { db, agentSessions, logs, approvals, tasks } from "@/lib/db";
import { eq } from "drizzle-orm";

interface ClaudeCodeOptions {
  workspacePath: string;
  workspaceId: number;
  taskId: number;
  onOutput?: AgentEventHandler;
}

export class ClaudeCodeAgent {
  private process: ChildProcess | null = null;
  private sessionId: number | null = null;
  private options: ClaudeCodeOptions;
  private buffer: string = "";

  constructor(options: ClaudeCodeOptions) {
    this.options = options;
  }

  async start(prompt: string): Promise<void> {
    // Create session record
    const [session] = await db
      .insert(agentSessions)
      .values({
        workspaceId: this.options.workspaceId,
        taskId: this.options.taskId,
        status: "running",
        startedAt: new Date(),
        lastActivityAt: new Date(),
      })
      .returning();

    this.sessionId = session.id;

    // Update task state
    await db
      .update(tasks)
      .set({ state: "analyzing", updatedAt: new Date() })
      .where(eq(tasks.id, this.options.taskId));

    // Spawn Claude Code process
    this.process = spawn("claude", ["-p", prompt, "--output-format", "json"], {
      cwd: this.options.workspacePath,
      env: { ...process.env },
    });

    // Update session with PID
    await db
      .update(agentSessions)
      .set({ pid: this.process.pid })
      .where(eq(agentSessions.id, this.sessionId));

    this.process.stdout?.on("data", (data: Buffer) => {
      this.handleOutput(data.toString());
    });

    this.process.stderr?.on("data", (data: Buffer) => {
      this.handleError(data.toString());
    });

    this.process.on("exit", (code) => {
      this.handleExit(code);
    });
  }

  private async handleOutput(data: string): Promise<void> {
    this.buffer += data;

    // Try to parse complete JSON objects
    const lines = this.buffer.split("\n");
    this.buffer = lines.pop() || "";

    for (const line of lines) {
      if (!line.trim()) continue;

      try {
        const output: AgentOutput = JSON.parse(line);
        await this.processOutput(output);
      } catch {
        // Not JSON, treat as plain text
        await this.logMessage("agent", line);
      }
    }

    // Update last activity
    if (this.sessionId) {
      await db
        .update(agentSessions)
        .set({ lastActivityAt: new Date() })
        .where(eq(agentSessions.id, this.sessionId));
    }
  }

  private async processOutput(output: AgentOutput): Promise<void> {
    if (output.type === "tool_use") {
      // Check if this is a git push or other approval-required action
      if (this.requiresApproval(output)) {
        await this.requestApproval(output);
        return;
      }
    }

    await this.logMessage("agent", output.content || JSON.stringify(output));

    this.options.onOutput?.({
      type: "output",
      data: output,
    });
  }

  private requiresApproval(output: AgentOutput): boolean {
    const approvalCommands = ["git push", "gh pr create", "deploy", "npm publish"];
    const content = output.toolInput?.command as string || output.content;
    return approvalCommands.some((cmd) => content?.includes(cmd));
  }

  private async requestApproval(output: AgentOutput): Promise<void> {
    const command = (output.toolInput?.command as string) || output.content;

    // Create approval request
    await db.insert(approvals).values({
      taskId: this.options.taskId,
      type: command.includes("push") ? "git_push" : "external_command",
      command,
      reason: `Agent wants to run: ${command}`,
      status: "pending",
    });

    // Update task state
    await db
      .update(tasks)
      .set({ state: "plan_ready", updatedAt: new Date() })
      .where(eq(tasks.id, this.options.taskId));

    // Pause the agent (in real implementation, we'd need IPC)
    await this.logMessage("approval", `Approval needed for: ${command}`);

    this.options.onOutput?.({
      type: "approval_needed",
      data: { command },
    });
  }

  private async handleError(data: string): Promise<void> {
    await this.logMessage("error", data);
    this.options.onOutput?.({ type: "error", data });
  }

  private async handleExit(code: number | null): Promise<void> {
    if (this.sessionId) {
      await db
        .update(agentSessions)
        .set({ status: code === 0 ? "stopped" : "error" })
        .where(eq(agentSessions.id, this.sessionId));
    }

    // Update task state based on exit code
    if (code === 0) {
      await db
        .update(tasks)
        .set({ state: "pr_draft", updatedAt: new Date() })
        .where(eq(tasks.id, this.options.taskId));
    }

    this.options.onOutput?.({ type: "exit", data: { code } });
  }

  private async logMessage(
    type: "agent" | "error" | "approval",
    message: string
  ): Promise<void> {
    await db.insert(logs).values({
      workspaceId: this.options.workspaceId,
      taskId: this.options.taskId,
      type: type === "approval" ? "approval" : type === "error" ? "error" : "agent",
      message,
    });
  }

  async stop(): Promise<void> {
    if (this.process) {
      this.process.kill("SIGTERM");
      this.process = null;
    }

    if (this.sessionId) {
      await db
        .update(agentSessions)
        .set({ status: "stopped" })
        .where(eq(agentSessions.id, this.sessionId));
    }
  }

  async sendInput(input: string): Promise<void> {
    if (this.process?.stdin) {
      this.process.stdin.write(input + "\n");
    }
  }
}

export async function startAgentForTask(taskId: number): Promise<ClaudeCodeAgent | null> {
  const task = await db.query.tasks.findFirst({
    where: eq(tasks.id, taskId),
  });

  if (!task) return null;

  const workspace = await db.query.workspaces.findFirst({
    where: eq(tasks.workspaceId, task.workspaceId),
  });

  if (!workspace) return null;

  const prompt = `You are working on GitHub issue #${task.githubIssueNumber}: "${task.title}"

${task.description || "No description provided."}

Please analyze this issue and create an implementation plan. After I approve the plan, implement the changes, run tests, and prepare for a pull request.

Important:
- Create a feature branch for this work
- Make small, focused commits
- Run tests before completing
- Do not push to remote until I approve`;

  const agent = new ClaudeCodeAgent({
    workspacePath: workspace.repoPath.replace("~", process.env.HOME || ""),
    workspaceId: workspace.id,
    taskId,
  });

  await agent.start(prompt);
  return agent;
}
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add Claude Code agent wrapper"
```

---

## Task 5: Agent Control API

**Files:**
- Create: `src/app/api/agents/start/route.ts`
- Create: `src/app/api/agents/stop/route.ts`
- Create: `src/app/api/agents/sessions/route.ts`

**Step 1: Create start agent route**

Create `src/app/api/agents/start/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { startAgentForTask } from "@/lib/agents/claude-code";

// Store active agents in memory (in production, use Redis or similar)
const activeAgents = new Map<number, ReturnType<typeof startAgentForTask>>();

export async function POST(request: NextRequest) {
  try {
    const { taskId } = await request.json();

    if (!taskId) {
      return NextResponse.json(
        { error: "taskId is required" },
        { status: 400 }
      );
    }

    // Check if agent already running for this task
    if (activeAgents.has(taskId)) {
      return NextResponse.json(
        { error: "Agent already running for this task" },
        { status: 400 }
      );
    }

    const agent = await startAgentForTask(taskId);
    if (!agent) {
      return NextResponse.json(
        { error: "Failed to start agent" },
        { status: 500 }
      );
    }

    activeAgents.set(taskId, Promise.resolve(agent));

    return NextResponse.json({ status: "started", taskId });
  } catch (error) {
    console.error("Failed to start agent:", error);
    return NextResponse.json(
      { error: "Failed to start agent" },
      { status: 500 }
    );
  }
}
```

**Step 2: Create stop agent route**

Create `src/app/api/agents/stop/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, agentSessions } from "@/lib/db";
import { eq, and } from "drizzle-orm";

export async function POST(request: NextRequest) {
  try {
    const { taskId } = await request.json();

    if (!taskId) {
      return NextResponse.json(
        { error: "taskId is required" },
        { status: 400 }
      );
    }

    // Find active session
    const session = await db.query.agentSessions.findFirst({
      where: and(
        eq(agentSessions.taskId, taskId),
        eq(agentSessions.status, "running")
      ),
    });

    if (!session) {
      return NextResponse.json(
        { error: "No active agent for this task" },
        { status: 404 }
      );
    }

    // Kill process if PID exists
    if (session.pid) {
      try {
        process.kill(session.pid, "SIGTERM");
      } catch {
        // Process may already be dead
      }
    }

    // Update session status
    await db
      .update(agentSessions)
      .set({ status: "stopped" })
      .where(eq(agentSessions.id, session.id));

    return NextResponse.json({ status: "stopped", taskId });
  } catch (error) {
    console.error("Failed to stop agent:", error);
    return NextResponse.json(
      { error: "Failed to stop agent" },
      { status: 500 }
    );
  }
}
```

**Step 3: Create sessions list route**

Create `src/app/api/agents/sessions/route.ts`:

```typescript
import { NextResponse } from "next/server";
import { db, agentSessions, workspaces, tasks } from "@/lib/db";
import { eq, desc } from "drizzle-orm";

export async function GET() {
  try {
    const sessions = await db
      .select({
        session: agentSessions,
        workspace: workspaces,
        task: tasks,
      })
      .from(agentSessions)
      .leftJoin(workspaces, eq(agentSessions.workspaceId, workspaces.id))
      .leftJoin(tasks, eq(agentSessions.taskId, tasks.id))
      .orderBy(desc(agentSessions.startedAt))
      .limit(50);

    return NextResponse.json(
      sessions.map((s) => ({
        ...s.session,
        workspaceName: s.workspace?.name,
        taskTitle: s.task?.title,
      }))
    );
  } catch (error) {
    console.error("Failed to fetch sessions:", error);
    return NextResponse.json(
      { error: "Failed to fetch sessions" },
      { status: 500 }
    );
  }
}
```

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: add agent control API routes"
```

---

## Task 6: Approvals API

**Files:**
- Create: `src/app/api/approvals/route.ts`
- Create: `src/app/api/approvals/[id]/route.ts`

**Step 1: Create approvals list route**

Create `src/app/api/approvals/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, approvals, tasks, workspaces } from "@/lib/db";
import { eq, desc } from "drizzle-orm";

export async function GET(request: NextRequest) {
  try {
    const status = request.nextUrl.searchParams.get("status") || "pending";

    const allApprovals = await db
      .select({
        approval: approvals,
        task: tasks,
        workspace: workspaces,
      })
      .from(approvals)
      .leftJoin(tasks, eq(approvals.taskId, tasks.id))
      .leftJoin(workspaces, eq(tasks.workspaceId, workspaces.id))
      .orderBy(desc(approvals.createdAt));

    const filtered = allApprovals.filter(
      (a) => a.approval.status === status
    );

    return NextResponse.json(
      filtered.map((a) => ({
        ...a.approval,
        taskTitle: a.task?.title,
        workspaceName: a.workspace?.name,
        githubIssueNumber: a.task?.githubIssueNumber,
      }))
    );
  } catch (error) {
    console.error("Failed to fetch approvals:", error);
    return NextResponse.json(
      { error: "Failed to fetch approvals" },
      { status: 500 }
    );
  }
}
```

**Step 2: Create approval action route**

Create `src/app/api/approvals/[id]/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, approvals, tasks, logs } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { action } = await request.json();

    if (!["approve", "reject"].includes(action)) {
      return NextResponse.json(
        { error: "Invalid action. Use 'approve' or 'reject'" },
        { status: 400 }
      );
    }

    const approval = await db.query.approvals.findFirst({
      where: eq(approvals.id, parseInt(id)),
    });

    if (!approval) {
      return NextResponse.json(
        { error: "Approval not found" },
        { status: 404 }
      );
    }

    if (approval.status !== "pending") {
      return NextResponse.json(
        { error: "Approval already resolved" },
        { status: 400 }
      );
    }

    // Update approval status
    const newStatus = action === "approve" ? "approved" : "rejected";
    await db
      .update(approvals)
      .set({
        status: newStatus,
        resolvedAt: new Date(),
      })
      .where(eq(approvals.id, parseInt(id)));

    // Update task state
    if (action === "approve") {
      await db
        .update(tasks)
        .set({ state: "in_progress", updatedAt: new Date() })
        .where(eq(tasks.id, approval.taskId));
    } else {
      await db
        .update(tasks)
        .set({ state: "queued", updatedAt: new Date() })
        .where(eq(tasks.id, approval.taskId));
    }

    // Log the action
    const task = await db.query.tasks.findFirst({
      where: eq(tasks.id, approval.taskId),
    });

    if (task) {
      await db.insert(logs).values({
        workspaceId: task.workspaceId,
        taskId: task.id,
        type: "approval",
        message: `${action === "approve" ? "Approved" : "Rejected"}: ${approval.command}`,
      });
    }

    return NextResponse.json({ status: newStatus });
  } catch (error) {
    console.error("Failed to process approval:", error);
    return NextResponse.json(
      { error: "Failed to process approval" },
      { status: 500 }
    );
  }
}
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add approvals API routes"
```

---

## Task 7: Logs API with SSE

**Files:**
- Create: `src/app/api/logs/route.ts`
- Create: `src/app/api/logs/stream/route.ts`

**Step 1: Create logs list route**

Create `src/app/api/logs/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, logs, workspaces, tasks } from "@/lib/db";
import { eq, desc } from "drizzle-orm";

export async function GET(request: NextRequest) {
  try {
    const workspaceId = request.nextUrl.searchParams.get("workspaceId");
    const type = request.nextUrl.searchParams.get("type");
    const limit = parseInt(request.nextUrl.searchParams.get("limit") || "100");

    const allLogs = await db
      .select({
        log: logs,
        workspace: workspaces,
        task: tasks,
      })
      .from(logs)
      .leftJoin(workspaces, eq(logs.workspaceId, workspaces.id))
      .leftJoin(tasks, eq(logs.taskId, tasks.id))
      .orderBy(desc(logs.createdAt))
      .limit(limit);

    let filtered = allLogs;
    if (workspaceId) {
      filtered = filtered.filter(
        (l) => l.log.workspaceId === parseInt(workspaceId)
      );
    }
    if (type) {
      filtered = filtered.filter((l) => l.log.type === type);
    }

    return NextResponse.json(
      filtered.map((l) => ({
        ...l.log,
        workspaceName: l.workspace?.name,
        taskTitle: l.task?.title,
      }))
    );
  } catch (error) {
    console.error("Failed to fetch logs:", error);
    return NextResponse.json(
      { error: "Failed to fetch logs" },
      { status: 500 }
    );
  }
}
```

**Step 2: Create SSE stream route**

Create `src/app/api/logs/stream/route.ts`:

```typescript
import { NextRequest } from "next/server";
import { db, logs } from "@/lib/db";
import { desc, gt } from "drizzle-orm";

export const dynamic = "force-dynamic";

export async function GET(request: NextRequest) {
  const encoder = new TextEncoder();
  let lastId = 0;

  const stream = new ReadableStream({
    async start(controller) {
      // Send initial connection message
      controller.enqueue(
        encoder.encode(`data: ${JSON.stringify({ type: "connected" })}\n\n`)
      );

      // Poll for new logs every second
      const interval = setInterval(async () => {
        try {
          const newLogs = await db
            .select()
            .from(logs)
            .where(gt(logs.id, lastId))
            .orderBy(desc(logs.createdAt))
            .limit(20);

          if (newLogs.length > 0) {
            lastId = Math.max(...newLogs.map((l) => l.id));
            for (const log of newLogs.reverse()) {
              controller.enqueue(
                encoder.encode(`data: ${JSON.stringify(log)}\n\n`)
              );
            }
          }
        } catch (error) {
          console.error("SSE error:", error);
        }
      }, 1000);

      // Clean up on close
      request.signal.addEventListener("abort", () => {
        clearInterval(interval);
        controller.close();
      });
    },
  });

  return new Response(stream, {
    headers: {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    },
  });
}
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add logs API with SSE streaming"
```

---

## Task 8: Taskboard UI

**Files:**
- Modify: `src/app/page.tsx`
- Create: `src/components/taskboard/task-list.tsx`
- Create: `src/components/taskboard/task-card.tsx`
- Create: `src/components/taskboard/sync-button.tsx`

**Step 1: Create task card component**

Create `src/components/taskboard/task-card.tsx`:

```tsx
"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Play, ExternalLink } from "lucide-react";
import { Task } from "@/lib/db/schema";

interface TaskCardProps {
  task: Task & { workspaceName?: string };
  onAssign: (taskId: number) => void;
  onStartAgent: (taskId: number) => void;
}

const stateColors: Record<string, string> = {
  new: "bg-gray-500",
  queued: "bg-yellow-500",
  analyzing: "bg-blue-500",
  plan_ready: "bg-purple-500",
  in_progress: "bg-orange-500",
  pr_draft: "bg-green-500",
  done: "bg-green-700",
};

const stateLabels: Record<string, string> = {
  new: "New",
  queued: "Queued",
  analyzing: "Analyzing",
  plan_ready: "Plan Ready",
  in_progress: "In Progress",
  pr_draft: "PR Draft",
  done: "Done",
};

export function TaskCard({ task, onAssign, onStartAgent }: TaskCardProps) {
  return (
    <Card className="p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            {task.githubIssueNumber && (
              <span className="text-sm text-muted-foreground">
                #{task.githubIssueNumber}
              </span>
            )}
            <Badge
              variant="secondary"
              className={`${stateColors[task.state]} text-white`}
            >
              {stateLabels[task.state]}
            </Badge>
            {task.workspaceName && (
              <Badge variant="outline">{task.workspaceName}</Badge>
            )}
          </div>
          <h3 className="font-medium truncate">{task.title}</h3>
          {task.description && (
            <p className="text-sm text-muted-foreground mt-1 line-clamp-2">
              {task.description}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          {task.state === "new" && (
            <Button size="sm" onClick={() => onAssign(task.id)}>
              Assign
            </Button>
          )}
          {task.state === "queued" && (
            <Button size="sm" onClick={() => onStartAgent(task.id)}>
              <Play className="h-4 w-4 mr-1" />
              Start
            </Button>
          )}
          {task.prUrl && (
            <Button size="sm" variant="outline" asChild>
              <a href={task.prUrl} target="_blank" rel="noopener noreferrer">
                <ExternalLink className="h-4 w-4" />
              </a>
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}
```

**Step 2: Create task list component**

Create `src/components/taskboard/task-list.tsx`:

```tsx
"use client";

import { useState, useEffect } from "react";
import { TaskCard } from "./task-card";
import { Task } from "@/lib/db/schema";

interface TaskListProps {
  workspaceFilter?: number;
  stateFilter?: string;
}

export function TaskList({ workspaceFilter, stateFilter }: TaskListProps) {
  const [tasks, setTasks] = useState<(Task & { workspaceName?: string })[]>([]);
  const [loading, setLoading] = useState(true);

  async function fetchTasks() {
    try {
      const params = new URLSearchParams();
      if (workspaceFilter) params.set("workspaceId", workspaceFilter.toString());
      if (stateFilter) params.set("state", stateFilter);

      const res = await fetch(`/api/tasks?${params}`);
      const data = await res.json();
      setTasks(data);
    } catch (error) {
      console.error("Failed to fetch tasks:", error);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    fetchTasks();
    const interval = setInterval(fetchTasks, 5000);
    return () => clearInterval(interval);
  }, [workspaceFilter, stateFilter]);

  async function handleAssign(taskId: number) {
    try {
      await fetch(`/api/tasks/${taskId}/assign`, { method: "POST" });
      fetchTasks();
    } catch (error) {
      console.error("Failed to assign task:", error);
    }
  }

  async function handleStartAgent(taskId: number) {
    try {
      await fetch("/api/agents/start", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ taskId }),
      });
      fetchTasks();
    } catch (error) {
      console.error("Failed to start agent:", error);
    }
  }

  if (loading) {
    return <div className="text-muted-foreground">Loading tasks...</div>;
  }

  if (tasks.length === 0) {
    return (
      <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
        No tasks found. Sync issues or add a workspace with GitHub configured.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {tasks.map((task) => (
        <TaskCard
          key={task.id}
          task={task}
          onAssign={handleAssign}
          onStartAgent={handleStartAgent}
        />
      ))}
    </div>
  );
}
```

**Step 3: Create sync button component**

Create `src/components/taskboard/sync-button.tsx`:

```tsx
"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { RefreshCw } from "lucide-react";

export function SyncButton() {
  const [syncing, setSyncing] = useState(false);

  async function handleSync() {
    setSyncing(true);
    try {
      const res = await fetch("/api/sync", { method: "POST" });
      const result = await res.json();
      console.log("Sync result:", result);
    } catch (error) {
      console.error("Sync failed:", error);
    } finally {
      setSyncing(false);
    }
  }

  return (
    <Button onClick={handleSync} disabled={syncing} size="sm">
      <RefreshCw className={`h-4 w-4 mr-2 ${syncing ? "animate-spin" : ""}`} />
      {syncing ? "Syncing..." : "Sync Issues"}
    </Button>
  );
}
```

**Step 4: Update Taskboard page**

Modify `src/app/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";
import { TaskList } from "@/components/taskboard/task-list";
import { SyncButton } from "@/components/taskboard/sync-button";

export default function TaskboardPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Taskboard"
        description="Manage issues across all repositories"
      />
      <div className="flex-1 p-6">
        <div className="flex items-center justify-between mb-6">
          <div className="text-sm text-muted-foreground">
            Issues synced from GitHub
          </div>
          <SyncButton />
        </div>
        <TaskList />
      </div>
    </div>
  );
}
```

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: add taskboard UI with task cards"
```

---

## Task 9: Pending Board UI

**Files:**
- Modify: `src/app/pending/page.tsx`
- Create: `src/components/pending-board/approval-list.tsx`
- Create: `src/components/pending-board/approval-card.tsx`

**Step 1: Create approval card component**

Create `src/components/pending-board/approval-card.tsx`:

```tsx
"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Check, X, Terminal } from "lucide-react";
import { Approval } from "@/lib/db/schema";

interface ApprovalCardProps {
  approval: Approval & {
    taskTitle?: string;
    workspaceName?: string;
    githubIssueNumber?: number;
  };
  onApprove: (id: number) => void;
  onReject: (id: number) => void;
}

const typeLabels: Record<string, string> = {
  git_push: "Git Push",
  external_command: "External Command",
  deploy: "Deploy",
};

export function ApprovalCard({
  approval,
  onApprove,
  onReject,
}: ApprovalCardProps) {
  return (
    <Card className="p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-2">
            <Badge variant="secondary">{typeLabels[approval.type]}</Badge>
            {approval.workspaceName && (
              <Badge variant="outline">{approval.workspaceName}</Badge>
            )}
            {approval.githubIssueNumber && (
              <span className="text-sm text-muted-foreground">
                Issue #{approval.githubIssueNumber}
              </span>
            )}
          </div>
          {approval.taskTitle && (
            <h3 className="font-medium mb-2">{approval.taskTitle}</h3>
          )}
          <div className="flex items-center gap-2 p-2 bg-muted rounded text-sm font-mono">
            <Terminal className="h-4 w-4 text-muted-foreground flex-shrink-0" />
            <code className="truncate">{approval.command}</code>
          </div>
          {approval.reason && (
            <p className="text-sm text-muted-foreground mt-2">
              {approval.reason}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            className="text-green-600 hover:text-green-700 hover:bg-green-50"
            onClick={() => onApprove(approval.id)}
          >
            <Check className="h-4 w-4" />
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="text-red-600 hover:text-red-700 hover:bg-red-50"
            onClick={() => onReject(approval.id)}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </Card>
  );
}
```

**Step 2: Create approval list component**

Create `src/components/pending-board/approval-list.tsx`:

```tsx
"use client";

import { useState, useEffect } from "react";
import { ApprovalCard } from "./approval-card";
import { Approval } from "@/lib/db/schema";

export function ApprovalList() {
  const [approvals, setApprovals] = useState<
    (Approval & {
      taskTitle?: string;
      workspaceName?: string;
      githubIssueNumber?: number;
    })[]
  >([]);
  const [loading, setLoading] = useState(true);

  async function fetchApprovals() {
    try {
      const res = await fetch("/api/approvals?status=pending");
      const data = await res.json();
      setApprovals(data);
    } catch (error) {
      console.error("Failed to fetch approvals:", error);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    fetchApprovals();
    const interval = setInterval(fetchApprovals, 3000);
    return () => clearInterval(interval);
  }, []);

  async function handleAction(id: number, action: "approve" | "reject") {
    try {
      await fetch(`/api/approvals/${id}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action }),
      });
      fetchApprovals();
    } catch (error) {
      console.error("Failed to process approval:", error);
    }
  }

  if (loading) {
    return <div className="text-muted-foreground">Loading approvals...</div>;
  }

  if (approvals.length === 0) {
    return (
      <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
        No pending approvals
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {approvals.map((approval) => (
        <ApprovalCard
          key={approval.id}
          approval={approval}
          onApprove={(id) => handleAction(id, "approve")}
          onReject={(id) => handleAction(id, "reject")}
        />
      ))}
    </div>
  );
}
```

**Step 3: Update Pending page**

Modify `src/app/pending/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";
import { ApprovalList } from "@/components/pending-board/approval-list";

export default function PendingPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Pending Approvals"
        description="Actions waiting for your approval"
      />
      <div className="flex-1 p-6">
        <ApprovalList />
      </div>
    </div>
  );
}
```

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: add pending board UI with approval cards"
```

---

## Task 10: Logs UI with Live Stream

**Files:**
- Modify: `src/app/logs/page.tsx`
- Create: `src/components/log-viewer/log-list.tsx`
- Create: `src/components/log-viewer/log-entry.tsx`
- Create: `src/hooks/use-log-stream.ts`

**Step 1: Create log stream hook**

Create `src/hooks/use-log-stream.ts`:

```typescript
"use client";

import { useState, useEffect, useCallback } from "react";
import { Log } from "@/lib/db/schema";

export function useLogStream() {
  const [logs, setLogs] = useState<Log[]>([]);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    const eventSource = new EventSource("/api/logs/stream");

    eventSource.onopen = () => {
      setConnected(true);
    };

    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.type === "connected") {
          setConnected(true);
          return;
        }
        setLogs((prev) => [data, ...prev].slice(0, 200));
      } catch {
        // Ignore parse errors
      }
    };

    eventSource.onerror = () => {
      setConnected(false);
    };

    return () => {
      eventSource.close();
    };
  }, []);

  const clearLogs = useCallback(() => {
    setLogs([]);
  }, []);

  return { logs, connected, clearLogs };
}
```

**Step 2: Create log entry component**

Create `src/components/log-viewer/log-entry.tsx`:

```tsx
import { Badge } from "@/components/ui/badge";
import { Log } from "@/lib/db/schema";

interface LogEntryProps {
  log: Log & { workspaceName?: string; taskTitle?: string };
}

const typeColors: Record<string, string> = {
  agent: "bg-blue-500",
  test: "bg-purple-500",
  sync: "bg-green-500",
  approval: "bg-yellow-500",
  error: "bg-red-500",
  health: "bg-gray-500",
};

export function LogEntry({ log }: LogEntryProps) {
  const time = new Date(log.createdAt).toLocaleTimeString();

  return (
    <div className="flex items-start gap-3 py-2 border-b last:border-0">
      <span className="text-xs text-muted-foreground w-20 flex-shrink-0">
        {time}
      </span>
      <Badge
        variant="secondary"
        className={`${typeColors[log.type]} text-white text-xs`}
      >
        {log.type}
      </Badge>
      <span className="flex-1 text-sm">{log.message}</span>
    </div>
  );
}
```

**Step 3: Create log list component**

Create `src/components/log-viewer/log-list.tsx`:

```tsx
"use client";

import { useLogStream } from "@/hooks/use-log-stream";
import { LogEntry } from "./log-entry";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Trash2 } from "lucide-react";

export function LogList() {
  const { logs, connected, clearLogs } = useLogStream();

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between mb-4">
        <Badge variant={connected ? "default" : "destructive"}>
          {connected ? "Live" : "Disconnected"}
        </Badge>
        <Button variant="outline" size="sm" onClick={clearLogs}>
          <Trash2 className="h-4 w-4 mr-2" />
          Clear
        </Button>
      </div>
      <ScrollArea className="flex-1 border rounded-lg p-4">
        {logs.length === 0 ? (
          <div className="text-center text-muted-foreground py-8">
            Waiting for logs...
          </div>
        ) : (
          logs.map((log) => <LogEntry key={log.id} log={log} />)
        )}
      </ScrollArea>
    </div>
  );
}
```

**Step 4: Update Logs page**

Modify `src/app/logs/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";
import { LogList } from "@/components/log-viewer/log-list";

export default function LogsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header title="Logs" description="Real-time activity stream" />
      <div className="flex-1 p-6">
        <LogList />
      </div>
    </div>
  );
}
```

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: add logs UI with live streaming"
```

---

## Task 11: Final Verification

**Step 1: Build the project**

Run:
```bash
npm run build
```

Expected: Build succeeds with no errors

**Step 2: Run lint**

Run:
```bash
npm run lint
```

Expected: No lint errors

**Step 3: Test the app manually**

Start dev server:
```bash
npm run dev
```

Verify:
- Taskboard shows sync button
- Pending page loads
- Logs page shows live stream
- Settings still works

**Step 4: Final commit if needed**

```bash
git add -A && git commit -m "fix: address build issues"
```

---

## Summary

Phase 2 creates:
- GitHub integration library (fetch issues, create branches, create PRs)
- Issue sync system (manual sync via API)
- Tasks API with assignment workflow
- Claude Code agent wrapper (subprocess management)
- Agent control API (start/stop/list sessions)
- Approvals API (approve/reject workflow)
- Logs API with SSE streaming
- Taskboard UI with task cards and state badges
- Pending Board UI with approval actions
- Logs UI with live streaming

Total commits: ~11 focused commits
