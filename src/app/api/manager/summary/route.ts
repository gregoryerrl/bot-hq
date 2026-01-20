import { NextResponse } from "next/server";
import { db, tasks, workspaces } from "@/lib/db";

export async function GET() {
  try {
    // Simple summary without the old manager
    const allTasks = await db.select().from(tasks);
    const allWorkspaces = await db.select().from(workspaces);

    const inProgress = allTasks.filter((t) => t.state === "in_progress").length;
    const pendingReview = allTasks.filter((t) => t.state === "pending_review").length;
    const done = allTasks.filter((t) => t.state === "done").length;

    const summary = `${allTasks.length} tasks | ${inProgress} in progress | ${pendingReview} pending review | ${done} done | ${allWorkspaces.length} workspaces`;

    return NextResponse.json({ summary });
  } catch (error) {
    console.error("Summary error:", error);
    return NextResponse.json(
      { error: "Failed to get summary" },
      { status: 500 }
    );
  }
}
