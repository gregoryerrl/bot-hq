import { spawn, ChildProcess, exec } from "child_process";
import { promisify } from "util";
import { AgentOutput, AgentEventHandler } from "./types";
import { db, agentSessions, logs, approvals, tasks, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

const execAsync = promisify(exec);

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
      .set({ state: "in_progress", updatedAt: new Date() })
      .where(eq(tasks.id, this.options.taskId));

    // Log startup
    await this.logMessage("agent", `Starting Claude Code agent for task #${this.options.taskId}`);
    await this.logMessage("agent", `Working directory: ${this.options.workspacePath}`);

    // Spawn Claude Code process in headless mode
    this.process = spawn("claude", [
      "-p",
      "--verbose",
      "--output-format", "stream-json",
      "--allowedTools", "Bash,Read,Write,Edit,Glob,Grep,LS,TodoWrite,Task,WebFetch,WebSearch,NotebookEdit",
    ], {
      cwd: this.options.workspacePath,
      env: { ...process.env },
    });

    // Write prompt to stdin then close it
    this.process.stdin?.write(prompt);
    this.process.stdin?.end();

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

    const lines = this.buffer.split("\n");
    this.buffer = lines.pop() || "";

    for (const line of lines) {
      if (!line.trim()) continue;

      try {
        const output = JSON.parse(line);

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
        if (line.trim()) {
          await this.logMessage("agent", line);
        }
      }
    }

    if (this.sessionId) {
      await db
        .update(agentSessions)
        .set({ lastActivityAt: new Date() })
        .where(eq(agentSessions.id, this.sessionId));
    }
  }

  private async processOutput(output: AgentOutput): Promise<void> {
    await this.logMessage("agent", output.content || JSON.stringify(output));
    this.options.onOutput?.({
      type: "output",
      data: output,
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

    if (code === 0) {
      await this.createPendingApproval();
    } else {
      await this.logMessage("error", `Task failed with exit code: ${code}`);
    }

    this.options.onOutput?.({ type: "exit", data: { code } });
  }

  private async createPendingApproval(): Promise<void> {
    try {
      // Get the current branch name
      const { stdout: branchOutput } = await execAsync(
        "git branch --show-current",
        { cwd: this.options.workspacePath }
      );
      const branchName = branchOutput.trim();

      // Check if we're on a feature branch (not main/master)
      if (branchName === "main" || branchName === "master" || !branchName) {
        await this.logMessage("agent", "No feature branch detected, skipping approval creation");
        return;
      }

      // Get the base branch
      const baseBranch = branchName === "main" ? "master" : "main";

      // Get commit messages
      const { stdout: logOutput } = await execAsync(
        `git log ${baseBranch}..${branchName} --format="%s" --reverse`,
        { cwd: this.options.workspacePath }
      );
      const commitMessages = logOutput.trim().split("\n").filter(Boolean);

      if (commitMessages.length === 0) {
        await this.logMessage("agent", "No commits on branch, skipping approval creation");
        return;
      }

      // Get diff summary
      const { stdout: numstat } = await execAsync(
        `git diff ${baseBranch}...${branchName} --numstat`,
        { cwd: this.options.workspacePath }
      );

      const statsLines = numstat.trim().split("\n").filter(Boolean);
      let totalAdditions = 0;
      let totalDeletions = 0;
      const files: { path: string; additions: number; deletions: number }[] = [];

      for (const line of statsLines) {
        const [addStr, delStr, path] = line.split("\t");
        const additions = addStr === "-" ? 0 : parseInt(addStr);
        const deletions = delStr === "-" ? 0 : parseInt(delStr);
        totalAdditions += additions;
        totalDeletions += deletions;
        files.push({ path, additions, deletions });
      }

      const diffSummary = JSON.stringify({
        files,
        totalAdditions,
        totalDeletions,
      });

      // Update task with branch name
      await db
        .update(tasks)
        .set({
          branchName,
          state: "pending_review",
          updatedAt: new Date()
        })
        .where(eq(tasks.id, this.options.taskId));

      // Create pending approval record
      await db.insert(approvals).values({
        taskId: this.options.taskId,
        workspaceId: this.options.workspaceId,
        branchName,
        baseBranch,
        commitMessages: JSON.stringify(commitMessages),
        diffSummary,
        status: "pending",
      });

      await this.logMessage("agent", `Pending approval created for branch: ${branchName}`);
      await this.logMessage("agent", `Files changed: ${files.length}, +${totalAdditions} -${totalDeletions}`);

    } catch (error) {
      await this.logMessage("error", `Failed to create pending approval: ${error}`);
    }
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

export async function startAgentForTask(
  taskId: number,
  userInstructions?: string
): Promise<ClaudeCodeAgent | null> {
  const task = await db.query.tasks.findFirst({
    where: eq(tasks.id, taskId),
  });

  if (!task) return null;

  const workspace = await db.query.workspaces.findFirst({
    where: eq(workspaces.id, task.workspaceId),
  });

  if (!workspace) return null;

  // Check if there's an existing branch to continue working on
  const existingBranch = task.branchName;

  let prompt: string;

  if (existingBranch && userInstructions) {
    // Continuing work with user feedback
    prompt = `You are continuing work on task: "${task.title}"

${task.description || "No description provided."}

You previously worked on this task and created branch: ${existingBranch}

The user has reviewed your work and requested changes:
${userInstructions}

Instructions:
1. Switch to the existing branch: git checkout ${existingBranch}
2. Address the user's feedback
3. Make commits as you work
4. Run the build (npm run build) before finishing
5. Do NOT push to remote - Bot-HQ will handle that after review`;
  } else {
    // Starting fresh
    prompt = `You are working on task: "${task.title}"

${task.description || "No description provided."}

Your task: Implement this feature completely.

Steps:
1. Create a feature branch: git checkout -b feature/task-${task.id}
2. Implement the required changes with small, focused commits
3. Run tests and fix any issues
4. Run the build (npm run build) before finishing

Important:
- Make commits as you work
- Do NOT push to remote or create PRs - Bot-HQ will handle that after you finish
- Work autonomously - complete the full implementation`;
  }

  const agent = new ClaudeCodeAgent({
    workspacePath: workspace.repoPath.replace("~", process.env.HOME || ""),
    workspaceId: workspace.id,
    taskId,
  });

  await agent.start(prompt);
  return agent;
}
