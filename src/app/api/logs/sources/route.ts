import { NextResponse } from "next/server";
import { db, logs, agentSessions, tasks, workspaces } from "@/lib/db";
import { eq, desc, ne, and } from "drizzle-orm";

export const dynamic = "force-dynamic";

interface LogSource {
  id: string;
  type: "server" | "agent";
  name: string;
  status: "live" | "running";
  latestMessage: string | null;
  latestAt: string | null;
  sessionId?: number;
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

    // 2. Active agent sessions
    const activeSessions = await db
      .select({
        session: agentSessions,
        task: tasks,
        workspace: workspaces,
      })
      .from(agentSessions)
      .leftJoin(tasks, eq(agentSessions.taskId, tasks.id))
      .leftJoin(workspaces, eq(agentSessions.workspaceId, workspaces.id))
      .where(eq(agentSessions.status, "running"))
      .orderBy(desc(agentSessions.startedAt));

    // Get latest log for each active agent
    for (const { session, task, workspace } of activeSessions) {
      let latestAgentLog: typeof logs.$inferSelect[] = [];

      if (session.taskId !== null) {
        latestAgentLog = await db
          .select()
          .from(logs)
          .where(
            and(
              eq(logs.type, "agent"),
              eq(logs.taskId, session.taskId)
            )
          )
          .orderBy(desc(logs.createdAt))
          .limit(1);
      }

      sources.push({
        id: `agent-${session.id}`,
        type: "agent",
        name: `${workspace?.name || "Unknown"} agent`,
        status: "running",
        latestMessage: latestAgentLog[0]?.message || "Starting...",
        latestAt: latestAgentLog[0]?.createdAt?.toISOString() || session.startedAt?.toISOString() || null,
        sessionId: session.id,
        taskTitle: task?.title || undefined,
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
