import { NextRequest, NextResponse } from "next/server";
import { db, projects, tasks, diagrams } from "@/lib/db";
import { eq, sql } from "drizzle-orm";

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;

  const project = await db.query.projects.findFirst({
    where: eq(projects.id, Number(id)),
  });

  if (!project) {
    return NextResponse.json({ error: "Project not found" }, { status: 404 });
  }

  // Get task counts grouped by state
  const taskCountRows = await db
    .select({
      state: tasks.state,
      count: sql<number>`count(*)`,
    })
    .from(tasks)
    .where(eq(tasks.projectId, Number(id)))
    .groupBy(tasks.state);

  const taskCounts: Record<string, number> = {};
  for (const row of taskCountRows) {
    taskCounts[row.state] = row.count;
  }

  // Get diagram count
  const [diagramCountRow] = await db
    .select({ count: sql<number>`count(*)` })
    .from(diagrams)
    .where(eq(diagrams.projectId, Number(id)));

  return NextResponse.json({
    ...project,
    taskCounts,
    diagramCount: diagramCountRow.count,
  });
}

export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const body = await request.json();

  // Check project exists
  const existing = await db.query.projects.findFirst({
    where: eq(projects.id, Number(id)),
  });

  if (!existing) {
    return NextResponse.json({ error: "Project not found" }, { status: 404 });
  }

  const updates: Record<string, unknown> = { updatedAt: new Date() };
  if (body.name !== undefined) updates.name = body.name;
  if (body.description !== undefined) updates.description = body.description;
  if (body.repoPath !== undefined) updates.repoPath = body.repoPath;
  if (body.status !== undefined) updates.status = body.status;
  if (body.notes !== undefined) updates.notes = body.notes;

  const [updated] = await db
    .update(projects)
    .set(updates)
    .where(eq(projects.id, Number(id)))
    .returning();

  return NextResponse.json(updated);
}

export async function DELETE(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;

  await db.delete(projects).where(eq(projects.id, Number(id)));

  return NextResponse.json({ success: true });
}
