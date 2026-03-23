import { NextRequest, NextResponse } from "next/server";
import { db, diagramEdges, diagrams } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const diagramId = Number(id);
  const body = await request.json();

  if (!body.sourceNodeId || !body.targetNodeId) {
    return NextResponse.json(
      { error: "sourceNodeId and targetNodeId are required" },
      { status: 400 }
    );
  }

  const [edge] = await db
    .insert(diagramEdges)
    .values({
      diagramId,
      sourceNodeId: body.sourceNodeId,
      targetNodeId: body.targetNodeId,
      label: body.label ?? null,
    })
    .returning();

  // Update diagram updatedAt
  await db
    .update(diagrams)
    .set({ updatedAt: new Date() })
    .where(eq(diagrams.id, diagramId));

  return NextResponse.json(edge, { status: 201 });
}
