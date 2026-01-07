import { NextRequest, NextResponse } from "next/server";
import { exec } from "child_process";
import { promisify } from "util";
import { db, approvals, tasks, logs, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";
import { startAgentForTask } from "@/lib/agents/claude-code";

const execAsync = promisify(exec);

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const result = await db
      .select()
      .from(approvals)
      .where(eq(approvals.id, parseInt(id)))
      .limit(1);

    if (result.length === 0) {
      return NextResponse.json({ error: "Approval not found" }, { status: 404 });
    }

    return NextResponse.json(result[0]);
  } catch (error) {
    console.error("Failed to fetch approval:", error);
    return NextResponse.json({ error: "Failed to fetch approval" }, { status: 500 });
  }
}

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { action, instructions } = await request.json();

    if (!["approve", "reject", "request_changes"].includes(action)) {
      return NextResponse.json(
        { error: "Invalid action. Use 'approve', 'reject', or 'request_changes'" },
        { status: 400 }
      );
    }

    const approval = await db.query.approvals.findFirst({
      where: eq(approvals.id, parseInt(id)),
    });

    if (!approval) {
      return NextResponse.json({ error: "Approval not found" }, { status: 404 });
    }

    if (approval.status !== "pending") {
      return NextResponse.json({ error: "Approval already resolved" }, { status: 400 });
    }

    const task = await db.query.tasks.findFirst({
      where: eq(tasks.id, approval.taskId),
    });

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, approval.workspaceId),
    });

    if (!task || !workspace) {
      return NextResponse.json({ error: "Task or workspace not found" }, { status: 404 });
    }

    const repoPath = workspace.repoPath.replace("~", process.env.HOME || "");

    if (action === "approve") {
      // Push branch and create PR
      await handleApprove(approval, task, workspace, repoPath);
    } else if (action === "reject") {
      // Delete branch and reset task
      await handleReject(approval, task, repoPath);
    } else if (action === "request_changes") {
      // Save instructions and restart agent
      await handleRequestChanges(approval, task, instructions);
    }

    return NextResponse.json({ status: "success" });
  } catch (error) {
    console.error("Failed to process approval:", error);
    return NextResponse.json(
      { error: `Failed to process approval: ${error}` },
      { status: 500 }
    );
  }
}

async function handleApprove(
  approval: typeof approvals.$inferSelect,
  task: typeof tasks.$inferSelect,
  workspace: typeof workspaces.$inferSelect,
  repoPath: string
) {
  // Push branch to remote
  await execAsync(
    `git push -u origin ${approval.branchName}`,
    { cwd: repoPath }
  );

  // Create PR using GitHub CLI
  const prTitle = task.title;
  const prBody = `Closes #${task.githubIssueNumber || "N/A"}\n\n## Changes\n${
    approval.commitMessages
      ? JSON.parse(approval.commitMessages).map((m: string) => `- ${m}`).join("\n")
      : "No commit messages"
  }`;

  const { stdout: prOutput } = await execAsync(
    `gh pr create --title "${prTitle.replace(/"/g, '\\"')}" --body "${prBody.replace(/"/g, '\\"')}" --base ${approval.baseBranch} --head ${approval.branchName}`,
    { cwd: repoPath }
  );

  // Extract PR URL from output
  const prUrl = prOutput.trim();

  // Update approval status
  await db
    .update(approvals)
    .set({ status: "approved", resolvedAt: new Date() })
    .where(eq(approvals.id, approval.id));

  // Update task state and PR URL
  await db
    .update(tasks)
    .set({
      state: "pr_created",
      prUrl,
      updatedAt: new Date(),
    })
    .where(eq(tasks.id, task.id));

  // Log the action
  await db.insert(logs).values({
    workspaceId: workspace.id,
    taskId: task.id,
    type: "approval",
    message: `Draft PR approved. Created PR: ${prUrl}`,
  });
}

async function handleReject(
  approval: typeof approvals.$inferSelect,
  task: typeof tasks.$inferSelect,
  repoPath: string
) {
  // Switch to base branch
  await execAsync(
    `git checkout ${approval.baseBranch}`,
    { cwd: repoPath }
  );

  // Delete the feature branch
  await execAsync(
    `git branch -D ${approval.branchName}`,
    { cwd: repoPath }
  );

  // Update approval status
  await db
    .update(approvals)
    .set({ status: "rejected", resolvedAt: new Date() })
    .where(eq(approvals.id, approval.id));

  // Reset task state
  await db
    .update(tasks)
    .set({
      state: "new",
      branchName: null,
      updatedAt: new Date(),
    })
    .where(eq(tasks.id, task.id));

  // Log the action
  await db.insert(logs).values({
    workspaceId: approval.workspaceId,
    taskId: task.id,
    type: "approval",
    message: `Draft PR rejected. Branch ${approval.branchName} deleted.`,
  });
}

async function handleRequestChanges(
  approval: typeof approvals.$inferSelect,
  task: typeof tasks.$inferSelect,
  instructions: string
) {
  // Save the instructions
  await db
    .update(approvals)
    .set({ userInstructions: instructions })
    .where(eq(approvals.id, approval.id));

  // Delete this approval (agent will create a new one when done)
  await db
    .delete(approvals)
    .where(eq(approvals.id, approval.id));

  // Update task state back to in_progress
  await db
    .update(tasks)
    .set({
      state: "in_progress",
      updatedAt: new Date(),
    })
    .where(eq(tasks.id, task.id));

  // Log the action
  await db.insert(logs).values({
    workspaceId: approval.workspaceId,
    taskId: task.id,
    type: "approval",
    message: `Changes requested: ${instructions}`,
  });

  // Start a new agent with the instructions
  await startAgentForTask(task.id, instructions);
}
