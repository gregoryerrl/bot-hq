import { NextRequest, NextResponse } from "next/server";
import { db, tasks } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const taskId = Number(id);
  const body = await request.json();

  const existing = await db.query.tasks.findFirst({
    where: eq(tasks.id, taskId),
  });

  if (!existing) {
    return NextResponse.json({ error: "Task not found" }, { status: 404 });
  }

  const updates: Record<string, unknown> = { updatedAt: new Date() };
  if (body.parentTaskId !== undefined) updates.parentTaskId = body.parentTaskId;
  if (body.order !== undefined) updates.order = body.order;

  const [updated] = await db
    .update(tasks)
    .set(updates)
    .where(eq(tasks.id, taskId))
    .returning();

  return NextResponse.json(updated);
}
