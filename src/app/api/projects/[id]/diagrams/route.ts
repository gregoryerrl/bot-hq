import { NextRequest, NextResponse } from "next/server";
import { db, diagrams, projects } from "@/lib/db";
import { eq, desc } from "drizzle-orm";

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const projectId = Number(id);

  const results = await db
    .select()
    .from(diagrams)
    .where(eq(diagrams.projectId, projectId))
    .orderBy(desc(diagrams.updatedAt));

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

  const [diagram] = await db
    .insert(diagrams)
    .values({
      projectId,
      title: body.title.trim(),
      template: body.template ?? null,
    })
    .returning();

  return NextResponse.json(diagram, { status: 201 });
}
