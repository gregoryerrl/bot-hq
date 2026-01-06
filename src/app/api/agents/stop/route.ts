import { NextRequest, NextResponse } from "next/server";
import { db, agentSessions } from "@/lib/db";
import { eq, and } from "drizzle-orm";

export async function POST(request: NextRequest) {
  try {
    const { taskId } = await request.json();

    if (!taskId) {
      return NextResponse.json(
        { error: "taskId is required" },
        { status: 400 }
      );
    }

    // Find active session
    const session = await db.query.agentSessions.findFirst({
      where: and(
        eq(agentSessions.taskId, taskId),
        eq(agentSessions.status, "running")
      ),
    });

    if (!session) {
      return NextResponse.json(
        { error: "No active agent for this task" },
        { status: 404 }
      );
    }

    // Kill process if PID exists
    if (session.pid) {
      try {
        process.kill(session.pid, "SIGTERM");
      } catch {
        // Process may already be dead
      }
    }

    // Update session status
    await db
      .update(agentSessions)
      .set({ status: "stopped" })
      .where(eq(agentSessions.id, session.id));

    return NextResponse.json({ status: "stopped", taskId });
  } catch (error) {
    console.error("Failed to stop agent:", error);
    return NextResponse.json(
      { error: "Failed to stop agent" },
      { status: 500 }
    );
  }
}
