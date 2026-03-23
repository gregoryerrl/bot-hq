import { NextRequest, NextResponse } from "next/server";
import { db, diagramNodes, diagrams } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const diagramId = Number(id);
  const body = await request.json();

  if (!body.label || typeof body.label !== "string" || body.label.trim() === "") {
    return NextResponse.json({ error: "label is required" }, { status: 400 });
  }

  // Support position: { x, y } as alternative to positionX/positionY
  const positionX = body.position?.x ?? body.positionX ?? 0;
  const positionY = body.position?.y ?? body.positionY ?? 0;

  const [node] = await db
    .insert(diagramNodes)
    .values({
      diagramId,
      label: body.label.trim(),
      nodeType: body.nodeType ?? "default",
      description: body.description ?? null,
      metadata: body.metadata ? JSON.stringify(body.metadata) : null,
      groupId: body.groupId ?? null,
      positionX,
      positionY,
    })
    .returning();

  // Update diagram updatedAt
  await db
    .update(diagrams)
    .set({ updatedAt: new Date() })
    .where(eq(diagrams.id, diagramId));

  return NextResponse.json(node, { status: 201 });
}
