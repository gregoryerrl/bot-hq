import { NextRequest, NextResponse } from "next/server";
import { db, diagramNodes, diagramEdges, diagrams } from "@/lib/db";
import { eq, or } from "drizzle-orm";

export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string; nodeId: string }> }
) {
  const { id, nodeId } = await params;
  const diagramId = Number(id);
  const nId = Number(nodeId);
  const body = await request.json();

  const existing = await db.query.diagramNodes.findFirst({
    where: eq(diagramNodes.id, nId),
  });

  if (!existing) {
    return NextResponse.json({ error: "Node not found" }, { status: 404 });
  }

  const updates: Record<string, unknown> = {};
  if (body.label !== undefined) updates.label = body.label;
  if (body.description !== undefined) updates.description = body.description;
  if (body.nodeType !== undefined) updates.nodeType = body.nodeType;
  if (body.groupId !== undefined) updates.groupId = body.groupId;
  if (body.metadata !== undefined) updates.metadata = body.metadata ? JSON.stringify(body.metadata) : null;
  if (body.positionX !== undefined) updates.positionX = body.positionX;
  if (body.positionY !== undefined) updates.positionY = body.positionY;

  const [updated] = await db
    .update(diagramNodes)
    .set(updates)
    .where(eq(diagramNodes.id, nId))
    .returning();

  // Update diagram updatedAt
  await db
    .update(diagrams)
    .set({ updatedAt: new Date() })
    .where(eq(diagrams.id, diagramId));

  return NextResponse.json(updated);
}

export async function DELETE(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string; nodeId: string }> }
) {
  const { id, nodeId } = await params;
  const diagramId = Number(id);
  const nId = Number(nodeId);

  // Delete connected edges first
  await db
    .delete(diagramEdges)
    .where(
      or(
        eq(diagramEdges.sourceNodeId, nId),
        eq(diagramEdges.targetNodeId, nId)
      )
    );

  // Delete the node
  await db.delete(diagramNodes).where(eq(diagramNodes.id, nId));

  // Update diagram updatedAt
  await db
    .update(diagrams)
    .set({ updatedAt: new Date() })
    .where(eq(diagrams.id, diagramId));

  return NextResponse.json({ success: true });
}
