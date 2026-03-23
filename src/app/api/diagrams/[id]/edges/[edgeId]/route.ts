import { NextRequest, NextResponse } from "next/server";
import { db, diagramEdges, diagrams } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function DELETE(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string; edgeId: string }> }
) {
  const { id, edgeId } = await params;
  const diagramId = Number(id);
  const eId = Number(edgeId);

  await db.delete(diagramEdges).where(eq(diagramEdges.id, eId));

  // Update diagram updatedAt
  await db
    .update(diagrams)
    .set({ updatedAt: new Date() })
    .where(eq(diagrams.id, diagramId));

  return NextResponse.json({ success: true });
}
