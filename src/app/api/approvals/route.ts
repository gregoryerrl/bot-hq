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
      .leftJoin(workspaces, eq(approvals.workspaceId, workspaces.id))
      .orderBy(desc(approvals.createdAt));

    const filtered = allApprovals.filter(
      (a) => a.approval.status === status
    );

    return NextResponse.json(
      filtered.map((a) => ({
        ...a.approval,
        taskTitle: a.task?.title,
        taskId: a.task?.id,
        workspaceName: a.workspace?.name,
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
