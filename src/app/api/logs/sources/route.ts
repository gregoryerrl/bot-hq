import { NextResponse } from "next/server";
import { db, logs, tasks, workspaces } from "@/lib/db";
import { eq, desc, ne, and } from "drizzle-orm";

export const dynamic = "force-dynamic";

interface LogSource {
  id: string;
  type: "server" | "manager";
  name: string;
  status: "live" | "running";
  latestMessage: string | null;
  latestAt: string | null;
  taskId?: number;
  taskTitle?: string;
  workspaceName?: string;
}

export async function GET() {
  try {
    const sources: LogSource[] = [];

    // 1. Server source - always present
    const latestServerLog = await db
      .select()
      .from(logs)
      .where(ne(logs.type, "agent"))
      .orderBy(desc(logs.createdAt))
      .limit(1);

    sources.push({
      id: "server",
      type: "server",
      name: "Server",
      status: "live",
      latestMessage: latestServerLog[0]?.message || null,
      latestAt: latestServerLog[0]?.createdAt?.toISOString() || null,
    });

    // 2. Manager source - persistent manager session
    sources.push({
      id: "manager",
      type: "manager",
      name: "Manager",
      status: "live",
      latestMessage: "Persistent manager architecture - logs will be available soon",
      latestAt: new Date().toISOString(),
    });

    // 3. Get active in_progress tasks as log sources
    const inProgressTasks = await db
      .select({
        task: tasks,
        workspace: workspaces,
      })
      .from(tasks)
      .leftJoin(workspaces, eq(tasks.workspaceId, workspaces.id))
      .where(eq(tasks.state, "in_progress"));

    for (const { task, workspace } of inProgressTasks) {
      const latestTaskLog = await db
        .select()
        .from(logs)
        .where(
          and(
            eq(logs.type, "agent"),
            eq(logs.taskId, task.id)
          )
        )
        .orderBy(desc(logs.createdAt))
        .limit(1);

      sources.push({
        id: `task-${task.id}`,
        type: "manager",
        name: `Task #${task.id}: ${task.title}`,
        status: "running",
        latestMessage: latestTaskLog[0]?.message || "In progress...",
        latestAt: latestTaskLog[0]?.createdAt?.toISOString() || task.updatedAt?.toISOString() || null,
        taskId: task.id,
        taskTitle: task.title,
        workspaceName: workspace?.name || undefined,
      });
    }

    return NextResponse.json(sources);
  } catch (error) {
    console.error("Failed to fetch log sources:", error);
    return NextResponse.json(
      { error: "Failed to fetch log sources" },
      { status: 500 }
    );
  }
}
