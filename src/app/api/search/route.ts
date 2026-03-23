import { NextRequest, NextResponse } from "next/server";
import { db, projects, tasks, diagramNodes } from "@/lib/db";
import { like, or } from "drizzle-orm";

export async function GET(request: NextRequest) {
  const q = request.nextUrl.searchParams.get("q");

  if (!q) {
    return NextResponse.json({ error: "q parameter is required" }, { status: 400 });
  }

  const pattern = `%${q}%`;

  const [matchedProjects, matchedTasks, matchedNodes] = await Promise.all([
    db
      .select()
      .from(projects)
      .where(
        or(
          like(projects.name, pattern),
          like(projects.description, pattern)
        )
      ),
    db
      .select()
      .from(tasks)
      .where(
        or(
          like(tasks.title, pattern),
          like(tasks.description, pattern)
        )
      ),
    db
      .select()
      .from(diagramNodes)
      .where(
        or(
          like(diagramNodes.label, pattern),
          like(diagramNodes.description, pattern),
          like(diagramNodes.metadata, pattern)
        )
      ),
  ]);

  return NextResponse.json({
    projects: matchedProjects,
    tasks: matchedTasks,
    diagramNodes: matchedNodes,
  });
}
