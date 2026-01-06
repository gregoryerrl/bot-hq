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
