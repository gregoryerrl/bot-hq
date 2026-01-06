import { NextResponse } from "next/server";
import { db, agentSessions, workspaces, tasks } from "@/lib/db";
import { eq, desc } from "drizzle-orm";

export async function GET() {
  try {
    const sessions = await db
      .select({
        session: agentSessions,
        workspace: workspaces,
        task: tasks,
      })
      .from(agentSessions)
      .leftJoin(workspaces, eq(agentSessions.workspaceId, workspaces.id))
      .leftJoin(tasks, eq(agentSessions.taskId, tasks.id))
      .orderBy(desc(agentSessions.startedAt))
      .limit(50);

    return NextResponse.json(
      sessions.map((s) => ({
        ...s.session,
        workspaceName: s.workspace?.name,
        taskTitle: s.task?.title,
      }))
    );
  } catch (error) {
    console.error("Failed to fetch sessions:", error);
    return NextResponse.json(
      { error: "Failed to fetch sessions" },
      { status: 500 }
    );
  }
}
