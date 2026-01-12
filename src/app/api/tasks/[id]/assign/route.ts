import { NextRequest, NextResponse } from "next/server";
import { db, tasks, logs } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;

    const task = await db.query.tasks.findFirst({
      where: eq(tasks.id, parseInt(id)),
    });

    if (!task) {
      return NextResponse.json({ error: "Task not found" }, { status: 404 });
    }

    if (task.state !== "new") {
      return NextResponse.json(
        { error: "Task is already assigned or in progress" },
        { status: 400 }
      );
    }

    // Update task to queued
    const result = await db
      .update(tasks)
      .set({
        state: "queued",
        assignedAt: new Date(),
        updatedAt: new Date(),
      })
      .where(eq(tasks.id, parseInt(id)))
      .returning();

    // Log assignment
    await db.insert(logs).values({
      workspaceId: task.workspaceId,
      taskId: task.id,
      type: "agent",
      message: `Task #${task.id} queued for agent`,
    });

    return NextResponse.json(result[0]);
  } catch (error) {
    console.error("Failed to assign task:", error);
    return NextResponse.json(
      { error: "Failed to assign task" },
      { status: 500 }
    );
  }
}
