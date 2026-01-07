import { spawn, ChildProcess } from "child_process";
import { AgentOutput, AgentEventHandler } from "./types";
import { db, agentSessions, logs, approvals, tasks, workspaces } from "@/lib/db";
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

    // Log startup
    await this.logMessage("agent", `Starting Claude Code agent for task #${this.options.taskId}`);
    await this.logMessage("agent", `Working directory: ${this.options.workspacePath}`);

    // Spawn Claude Code process
    this.process = spawn("claude", ["-p", prompt, "--output-format", "stream-json"], {
      cwd: this.options.workspacePath,
      env: { ...process.env },
    });

    // Handle spawn errors
    this.process.on("error", async (err) => {
      await this.logMessage("error", `Failed to spawn claude: ${err.message}`);
      if (this.sessionId) {
        await db
          .update(agentSessions)
          .set({ status: "error" })
          .where(eq(agentSessions.id, this.sessionId));
      }
    });

    // Update session with PID
    await db
      .update(agentSessions)
      .set({ pid: this.process.pid })
      .where(eq(agentSessions.id, this.sessionId));

    await this.logMessage("agent", `Agent process started (PID: ${this.process.pid})`);

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

    // Try to parse complete JSON objects (stream-json outputs one JSON per line)
    const lines = this.buffer.split("\n");
    this.buffer = lines.pop() || "";

    for (const line of lines) {
      if (!line.trim()) continue;

      try {
        const output = JSON.parse(line);

        // Handle stream-json format - log the content for visibility
        if (output.type === "assistant" && output.message?.content) {
          const content = output.message.content;
          for (const block of content) {
            if (block.type === "text") {
              await this.logMessage("agent", block.text);
            } else if (block.type === "tool_use") {
              await this.logMessage("agent", `Tool: ${block.name} - ${JSON.stringify(block.input).slice(0, 200)}`);
            }
          }
        } else if (output.type === "result") {
          await this.logMessage("agent", `Result: ${output.result || "completed"}`);
        } else {
          await this.processOutput(output as AgentOutput);
        }
      } catch {
        // Not JSON, treat as plain text
        if (line.trim()) {
          await this.logMessage("agent", line);
        }
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
    await this.logMessage("agent", `Agent process exited with code: ${code}`);

    if (this.sessionId) {
      await db
        .update(agentSessions)
        .set({ status: code === 0 ? "stopped" : "error" })
        .where(eq(agentSessions.id, this.sessionId));
    }

    // Update task state based on exit code
    if (code === 0) {
      await this.logMessage("agent", "Task completed successfully");
      await db
        .update(tasks)
        .set({ state: "pr_draft", updatedAt: new Date() })
        .where(eq(tasks.id, this.options.taskId));
    } else {
      await this.logMessage("error", `Task failed with exit code: ${code}`);
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
    where: eq(workspaces.id, task.workspaceId),
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
