import { NextRequest, NextResponse } from "next/server";
import { db, taskDependencies } from "@/lib/db";
import { eq, and } from "drizzle-orm";

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const taskId = Number(id);
  const body = await request.json();

  if (!body.dependsOnTaskId || typeof body.dependsOnTaskId !== "number") {
    return NextResponse.json({ error: "dependsOnTaskId is required" }, { status: 400 });
  }

  const [dep] = await db
    .insert(taskDependencies)
    .values({
      taskId,
      dependsOnTaskId: body.dependsOnTaskId,
    })
    .returning();

  return NextResponse.json(dep, { status: 201 });
}

export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const taskId = Number(id);
  const body = await request.json();

  if (!body.dependsOnTaskId || typeof body.dependsOnTaskId !== "number") {
    return NextResponse.json({ error: "dependsOnTaskId is required" }, { status: 400 });
  }

  await db
    .delete(taskDependencies)
    .where(
      and(
        eq(taskDependencies.taskId, taskId),
        eq(taskDependencies.dependsOnTaskId, body.dependsOnTaskId)
      )
    );

  return NextResponse.json({ success: true });
}
