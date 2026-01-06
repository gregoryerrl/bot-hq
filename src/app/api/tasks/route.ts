import { NextRequest, NextResponse } from "next/server";
import { db, tasks, workspaces } from "@/lib/db";
import { eq, desc } from "drizzle-orm";

export async function GET(request: NextRequest) {
  try {
    const workspaceId = request.nextUrl.searchParams.get("workspaceId");
    const state = request.nextUrl.searchParams.get("state");

    const query = db
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
