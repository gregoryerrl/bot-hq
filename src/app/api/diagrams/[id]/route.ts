import { NextRequest, NextResponse } from "next/server";
import { db, diagrams, diagramNodes, diagramEdges, diagramGroups } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const diagramId = Number(id);

  const diagram = await db.query.diagrams.findFirst({
    where: eq(diagrams.id, diagramId),
  });

  if (!diagram) {
    return NextResponse.json({ error: "Diagram not found" }, { status: 404 });
  }

  const groups = await db
    .select()
    .from(diagramGroups)
    .where(eq(diagramGroups.diagramId, diagramId));

  const groupMap = new Map(groups.map((g) => [g.id, g]));

  const nodes = await db
    .select()
    .from(diagramNodes)
    .where(eq(diagramNodes.diagramId, diagramId));

  const edges = await db
    .select()
    .from(diagramEdges)
    .where(eq(diagramEdges.diagramId, diagramId));

  return NextResponse.json({
    ...diagram,
    nodes: nodes.map((n) => ({
      id: String(n.id),
      type: "flowNode",
      position: { x: n.positionX, y: n.positionY },
      data: {
        label: n.label,
        nodeType: n.nodeType,
        description: n.description,
        metadata: n.metadata ? JSON.parse(n.metadata) : null,
        groupId: n.groupId,
        groupColor: n.groupId ? groupMap.get(n.groupId)?.color ?? null : null,
      },
    })),
    edges: edges.map((e) => ({
      id: String(e.id),
      source: String(e.sourceNodeId),
      target: String(e.targetNodeId),
      ...(e.label ? { label: e.label } : {}),
    })),
    groups: groups.map((g) => ({
      id: g.id,
      label: g.label,
      color: g.color,
    })),
  });
}

export async function DELETE(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const diagramId = Number(id);

  await db.delete(diagrams).where(eq(diagrams.id, diagramId));

  return NextResponse.json({ success: true });
}
