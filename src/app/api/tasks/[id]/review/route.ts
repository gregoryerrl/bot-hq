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
