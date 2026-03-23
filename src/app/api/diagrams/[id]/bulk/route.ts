import { NextRequest, NextResponse } from "next/server";
import { db, diagrams, diagramGroups, diagramNodes, diagramEdges } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const diagramId = Number(id);
  const body = await request.json();

  const groupTempIdMap = new Map<string, number>();
  const nodeTempIdMap = new Map<string, number>();

  let groupCount = 0;
  let nodeCount = 0;
  let edgeCount = 0;

  // 1. Insert groups first
  if (body.groups && Array.isArray(body.groups)) {
    for (const g of body.groups) {
      const [inserted] = await db
        .insert(diagramGroups)
        .values({
          diagramId,
          label: g.label,
          color: g.color ?? "#6b7280",
        })
        .returning();
      groupTempIdMap.set(g.tempId, inserted.id);
      groupCount++;
    }
  }

  // 2. Insert nodes, resolving groupTempId
  if (body.nodes && Array.isArray(body.nodes)) {
    for (const n of body.nodes) {
      const groupId = n.groupTempId ? groupTempIdMap.get(n.groupTempId) ?? null : null;
      const [inserted] = await db
        .insert(diagramNodes)
        .values({
          diagramId,
          label: n.label,
          nodeType: n.nodeType ?? "default",
          description: n.description ?? null,
          metadata: n.metadata ? JSON.stringify(n.metadata) : null,
          groupId,
          positionX: n.positionX ?? 0,
          positionY: n.positionY ?? 0,
        })
        .returning();
      nodeTempIdMap.set(n.tempId, inserted.id);
      nodeCount++;
    }
  }

  // 3. Insert edges, resolving sourceTempId and targetTempId
  if (body.edges && Array.isArray(body.edges)) {
    for (const e of body.edges) {
      const sourceNodeId = nodeTempIdMap.get(e.sourceTempId);
      const targetNodeId = nodeTempIdMap.get(e.targetTempId);
      if (sourceNodeId !== undefined && targetNodeId !== undefined) {
        await db
          .insert(diagramEdges)
          .values({
            diagramId,
            sourceNodeId,
            targetNodeId,
            label: e.label ?? null,
          });
        edgeCount++;
      }
    }
  }

  // Update diagram updatedAt
  await db
    .update(diagrams)
    .set({ updatedAt: new Date() })
    .where(eq(diagrams.id, diagramId));

  return NextResponse.json({ groups: groupCount, nodes: nodeCount, edges: edgeCount });
}
