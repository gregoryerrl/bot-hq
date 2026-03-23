import { NextRequest, NextResponse } from "next/server";
import { db, projects } from "@/lib/db";
import { eq, desc } from "drizzle-orm";

export async function GET(request: NextRequest) {
  const { searchParams } = request.nextUrl;
  const status = searchParams.get("status");

  const validStatuses = ["active", "archived"] as const;
  const typedStatus = validStatuses.find((s) => s === status);

  let query = db.select().from(projects).orderBy(desc(projects.updatedAt));

  if (typedStatus) {
    query = query.where(eq(projects.status, typedStatus)) as typeof query;
  }

  const results = await query;
  return NextResponse.json(results);
}

export async function POST(request: NextRequest) {
  const body = await request.json();

  if (!body.name || typeof body.name !== "string" || body.name.trim() === "") {
    return NextResponse.json({ error: "name is required" }, { status: 400 });
  }

  const [project] = await db
    .insert(projects)
    .values({
      name: body.name.trim(),
      description: body.description ?? null,
      repoPath: body.repoPath ?? null,
      notes: body.notes ?? null,
    })
    .returning();

  return NextResponse.json(project, { status: 201 });
}
