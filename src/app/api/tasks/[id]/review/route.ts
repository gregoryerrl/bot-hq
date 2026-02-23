import { NextResponse } from "next/server";
import { db, tasks, workspaces, logs } from "@/lib/db";
import { eq } from "drizzle-orm";
import { exec } from "child_process";
import { promisify } from "util";
import { cleanupTaskFiles, clearAllTaskContext } from "@/lib/bot-hq";
import { sendManagerCommand } from "@/lib/manager/persistent-manager";

const execAsync = promisify(exec);

// Workspace-level mutex to prevent concurrent branch-switching
const branchLocks = new Map<string, Promise<void>>();

async function withBranchLock<T>(repoPath: string, fn: () => Promise<T>): Promise<T> {
  while (branchLocks.has(repoPath)) {
    await branchLocks.get(repoPath);
  }

  let resolve: () => void;
  const lockPromise = new Promise<void>((r) => { resolve = r; });
  branchLocks.set(repoPath, lockPromise);

  try {
    return await fn();
  } finally {
    branchLocks.delete(repoPath);
    resolve!();
  }
}

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

    // Get uncommitted changes on the feature branch
    return await withBranchLock(workspace.repoPath, async () => {
      // Remember current branch
      const { stdout: currentBranch } = await execAsync(
        `git rev-parse --abbrev-ref HEAD`,
        { cwd: workspace.repoPath }
      );
      const originalBranch = currentBranch.trim();

      try {
        // Switch to feature branch
        await execAsync(`git checkout ${task.branchName}`, { cwd: workspace.repoPath });

        // Get uncommitted changes (staged + unstaged vs HEAD)
        const { stdout: numstat } = await execAsync(
          `git diff HEAD --numstat`,
          { cwd: workspace.repoPath }
        ).catch(() => ({ stdout: "" }));

        const { stdout: nameStatus } = await execAsync(
          `git diff HEAD --name-status`,
          { cwd: workspace.repoPath }
        ).catch(() => ({ stdout: "" }));

        const files = numstat
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

        // Also check for untracked files
        const { stdout: untrackedOutput } = await execAsync(
          `git ls-files --others --exclude-standard`,
          { cwd: workspace.repoPath }
        ).catch(() => ({ stdout: "" }));

        const untrackedFiles = untrackedOutput.trim().split("\n").filter(Boolean);
        for (const filename of untrackedFiles) {
          // Only add if not already in the diff
          if (!files.some(f => f.filename === filename)) {
            files.push({ filename, additions: 0, deletions: 0 });
          }
        }

        // Parse name-status for file status info
        const statusMap: Record<string, string> = {};
        nameStatus.trim().split("\n").filter(Boolean).forEach((line) => {
          const [status, ...pathParts] = line.split("\t");
          const filePath = pathParts[pathParts.length - 1];
          statusMap[filePath] = status;
        });

        const totalAdditions = files.reduce((sum, f) => sum + f.additions, 0);
        const totalDeletions = files.reduce((sum, f) => sum + f.deletions, 0);

        return NextResponse.json({
          task,
          diff: {
            branch: task.branchName,
            baseBranch: "main",
            files,
            totalAdditions,
            totalDeletions,
          },
        });
      } finally {
        // Always switch back to original branch
        await execAsync(`git checkout ${originalBranch}`, { cwd: workspace.repoPath }).catch(() => {});
      }
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
        if (!task.branchName) {
          return NextResponse.json({ error: "No branch to accept" }, { status: 400 });
        }

        await withBranchLock(workspace.repoPath, async () => {
          // Switch to feature branch
          await execAsync(`git checkout ${task.branchName}`, { cwd: workspace.repoPath });

          // Commit all changes
          await execAsync(`git add -A`, { cwd: workspace.repoPath });
          await execAsync(
            `git commit -m "feat(task-${taskId}): ${task.title}"`,
            { cwd: workspace.repoPath }
          );

          // Switch back to main
          await execAsync(`git checkout main`, { cwd: workspace.repoPath });
        });

        // Null out branchName to mark as reviewed/committed
        await db
          .update(tasks)
          .set({ branchName: null, updatedAt: new Date() })
          .where(eq(tasks.id, taskId));

        // Cleanup context files
        await clearAllTaskContext(workspace.name);

        return NextResponse.json({ success: true, action: "accepted" });
      }

      case "delete": {
        await withBranchLock(workspace.repoPath, async () => {
          if (task.branchName) {
            // Get current branch
            const { stdout: currentBranch } = await execAsync(
              `git rev-parse --abbrev-ref HEAD`,
              { cwd: workspace.repoPath }
            );

            // If on the feature branch, discard changes and switch to main
            if (currentBranch.trim() === task.branchName) {
              await execAsync(`git checkout -- .`, { cwd: workspace.repoPath }).catch(() => {});
              await execAsync(`git clean -fd`, { cwd: workspace.repoPath }).catch(() => {});
              await execAsync(`git checkout main`, { cwd: workspace.repoPath });
            }

            // Delete the branch
            await execAsync(`git branch -D ${task.branchName}`, { cwd: workspace.repoPath }).catch(() => {});
          }
        });

        // Cleanup context files
        await clearAllTaskContext(workspace.name);

        // Delete related logs first (FK constraint), then delete task
        await db.delete(logs).where(eq(logs.taskId, taskId));
        await db.delete(tasks).where(eq(tasks.id, taskId));

        return NextResponse.json({ success: true, action: "deleted" });
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

        // Send command to manager to pick up the requeued task
        sendManagerCommand(`TASK ${taskId}`);

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
