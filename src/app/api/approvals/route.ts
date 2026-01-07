import { NextRequest, NextResponse } from "next/server";
import { db, approvals, tasks, workspaces } from "@/lib/db";
import { eq, desc } from "drizzle-orm";
import { NewApproval } from "@/lib/db/schema";

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

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { taskId, type, command, reason } = body;

    const newApproval: NewApproval = {
      taskId: taskId || null,
      type: type || "external_command",
      command,
      reason,
      status: "pending",
    };

    const result = await db
      .insert(approvals)
      .values(newApproval)
      .returning();

    return NextResponse.json(result[0], { status: 201 });
  } catch (error) {
    console.error("Failed to create approval:", error);
    return NextResponse.json(
      { error: "Failed to create approval" },
      { status: 500 }
    );
  }
}
