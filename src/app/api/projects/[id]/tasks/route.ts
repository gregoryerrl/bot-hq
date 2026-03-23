import { NextRequest, NextResponse } from "next/server";
import { db, tasks, projects } from "@/lib/db";
import { eq, and, isNull, asc, desc } from "drizzle-orm";

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const projectId = Number(id);
  const { searchParams } = request.nextUrl;

  const state = searchParams.get("state");
  const parent = searchParams.get("parent");

  const validStates = ["todo", "in_progress", "done", "blocked"] as const;
  const typedState = validStates.find((s) => s === state);

  const conditions = [eq(tasks.projectId, projectId)];

  if (typedState) {
    conditions.push(eq(tasks.state, typedState));
  }

  if (parent === "null") {
    conditions.push(isNull(tasks.parentTaskId));
  } else if (parent !== null) {
    const parentId = Number(parent);
    if (!isNaN(parentId)) {
      conditions.push(eq(tasks.parentTaskId, parentId));
    }
  }

  const results = await db
    .select()
    .from(tasks)
    .where(and(...conditions))
    .orderBy(asc(tasks.order), desc(tasks.createdAt));

  return NextResponse.json(results);
}

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const projectId = Number(id);
  const body = await request.json();

  if (!body.title || typeof body.title !== "string" || body.title.trim() === "") {
    return NextResponse.json({ error: "title is required" }, { status: 400 });
  }

  // Verify project exists
  const project = await db.query.projects.findFirst({
    where: eq(projects.id, projectId),
  });

  if (!project) {
    return NextResponse.json({ error: "Project not found" }, { status: 404 });
  }

  const [task] = await db
    .insert(tasks)
    .values({
      projectId,
      title: body.title.trim(),
      description: body.description ?? null,
      parentTaskId: body.parentTaskId ?? null,
      priority: body.priority ?? 0,
      tags: body.tags ? JSON.stringify(body.tags) : null,
      dueDate: body.dueDate ? new Date(body.dueDate) : null,
      state: body.state ?? "todo",
      order: body.order ?? 0,
    })
    .returning();

  return NextResponse.json(task, { status: 201 });
}
