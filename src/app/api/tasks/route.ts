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

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { workspaceId, title, description, state = "new" } = body;

    if (!workspaceId || !title) {
      return NextResponse.json(
        { error: "Missing required fields: workspaceId, title" },
        { status: 400 }
      );
    }

    const result = await db.insert(tasks).values({
      workspaceId,
      title,
      description: description || "",
      state,
      priority: 0,
      updatedAt: new Date(),
    }).returning();

    return NextResponse.json(result[0]);
  } catch (error) {
    console.error("Failed to create task:", error);
    return NextResponse.json(
      { error: "Failed to create task" },
      { status: 500 }
    );
  }
}
