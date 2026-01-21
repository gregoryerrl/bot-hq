import { NextResponse } from "next/server";
import { db, tasks, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

export const dynamic = "force-dynamic";

export async function GET() {
  try {
    const awaitingTasks = await db
      .select({
        id: tasks.id,
        title: tasks.title,
        workspaceId: tasks.workspaceId,
        waitingQuestion: tasks.waitingQuestion,
        waitingContext: tasks.waitingContext,
        waitingSince: tasks.waitingSince,
      })
      .from(tasks)
      .where(eq(tasks.state, "awaiting_input"));

    // Enrich with workspace names
    const enriched = await Promise.all(
      awaitingTasks.map(async (task) => {
        const workspace = await db.query.workspaces.findFirst({
          where: eq(workspaces.id, task.workspaceId),
        });
        return {
          ...task,
          workspaceName: workspace?.name || "Unknown",
        };
      })
    );

    return NextResponse.json(enriched);
  } catch (error) {
    console.error("Failed to fetch awaiting tasks:", error);
    return NextResponse.json(
      { error: "Failed to fetch awaiting tasks" },
      { status: 500 }
    );
  }
}
