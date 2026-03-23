import { NextRequest, NextResponse } from "next/server";
import { db, tasks, taskNotes, taskDependencies } from "@/lib/db";
import { eq, asc } from "drizzle-orm";

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const taskId = Number(id);

  const task = await db.query.tasks.findFirst({
    where: eq(tasks.id, taskId),
  });

  if (!task) {
    return NextResponse.json({ error: "Task not found" }, { status: 404 });
  }

  const subtasks = await db
    .select()
    .from(tasks)
    .where(eq(tasks.parentTaskId, taskId))
    .orderBy(asc(tasks.order));

  const notes = await db
    .select()
    .from(taskNotes)
    .where(eq(taskNotes.taskId, taskId))
    .orderBy(asc(taskNotes.createdAt));

  const dependencies = await db
    .select()
    .from(taskDependencies)
    .where(eq(taskDependencies.taskId, taskId));

  return NextResponse.json({
    ...task,
    subtasks,
    notes,
    dependencies,
  });
}

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
  if (body.title !== undefined) updates.title = body.title;
  if (body.description !== undefined) updates.description = body.description;
  if (body.state !== undefined) updates.state = body.state;
  if (body.priority !== undefined) updates.priority = body.priority;
  if (body.tags !== undefined) updates.tags = body.tags ? JSON.stringify(body.tags) : null;
  if (body.dueDate !== undefined) updates.dueDate = body.dueDate ? new Date(body.dueDate) : null;
  if (body.order !== undefined) updates.order = body.order;

  const [updated] = await db
    .update(tasks)
    .set(updates)
    .where(eq(tasks.id, taskId))
    .returning();

  return NextResponse.json(updated);
}

export async function DELETE(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const taskId = Number(id);

  await db.delete(tasks).where(eq(tasks.id, taskId));

  return NextResponse.json({ success: true });
}
