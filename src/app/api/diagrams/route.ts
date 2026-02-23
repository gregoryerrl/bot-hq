import { NextResponse } from "next/server";
import { db, diagrams, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

// GET /api/diagrams?workspaceId=1
export async function GET(request: Request) {
  try {
    const { searchParams } = new URL(request.url);
    const workspaceId = searchParams.get("workspaceId");

    if (!workspaceId) {
      return NextResponse.json({ error: "workspaceId required" }, { status: 400 });
    }

    const diagramList = await db
      .select({
        id: diagrams.id,
        workspaceId: diagrams.workspaceId,
        title: diagrams.title,
        flowData: diagrams.flowData,
        createdAt: diagrams.createdAt,
        updatedAt: diagrams.updatedAt,
      })
      .from(diagrams)
      .where(eq(diagrams.workspaceId, parseInt(workspaceId)));

    return NextResponse.json(diagramList);
  } catch (error) {
    console.error("Failed to list diagrams:", error);
    return NextResponse.json({ error: "Failed to list diagrams" }, { status: 500 });
  }
}

// POST /api/diagrams
export async function POST(request: Request) {
  try {
    const { workspaceId, title, flowData } = await request.json();

    if (!workspaceId || !title || !flowData) {
      return NextResponse.json(
        { error: "workspaceId, title, and flowData required" },
        { status: 400 }
      );
    }

    // Validate flowData is valid JSON with nodes/edges
    try {
      const parsed = typeof flowData === "string" ? JSON.parse(flowData) : flowData;
      if (!parsed.nodes || !Array.isArray(parsed.nodes)) {
        return NextResponse.json(
          { error: "flowData must contain a nodes array" },
          { status: 400 }
        );
      }
    } catch {
      return NextResponse.json(
        { error: "flowData must be valid JSON" },
        { status: 400 }
      );
    }

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, workspaceId),
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    const [newDiagram] = await db
      .insert(diagrams)
      .values({
        workspaceId,
        title,
        flowData: typeof flowData === "string" ? flowData : JSON.stringify(flowData),
      })
      .returning();

    return NextResponse.json(newDiagram, { status: 201 });
  } catch (error) {
    console.error("Failed to create diagram:", error);
    return NextResponse.json({ error: "Failed to create diagram" }, { status: 500 });
  }
}
