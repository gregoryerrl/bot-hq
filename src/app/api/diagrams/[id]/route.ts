import { NextResponse } from "next/server";
import { db, diagrams } from "@/lib/db";
import { eq } from "drizzle-orm";

// GET /api/diagrams/[id]
export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const diagram = await db.query.diagrams.findFirst({
      where: eq(diagrams.id, parseInt(id)),
    });

    if (!diagram) {
      return NextResponse.json({ error: "Diagram not found" }, { status: 404 });
    }

    return NextResponse.json(diagram);
  } catch (error) {
    console.error("Failed to get diagram:", error);
    return NextResponse.json({ error: "Failed to get diagram" }, { status: 500 });
  }
}

// PUT /api/diagrams/[id]
export async function PUT(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const body = await request.json();

    const existing = await db.query.diagrams.findFirst({
      where: eq(diagrams.id, parseInt(id)),
    });

    if (!existing) {
      return NextResponse.json({ error: "Diagram not found" }, { status: 404 });
    }

    const updates: Record<string, unknown> = { updatedAt: new Date() };
    if (body.title) updates.title = body.title;
    if (body.flowData) {
      // Validate flowData is valid JSON with nodes array
      try {
        const parsed = typeof body.flowData === "string" ? JSON.parse(body.flowData) : body.flowData;
        if (!parsed.nodes || !Array.isArray(parsed.nodes)) {
          return NextResponse.json(
            { error: "flowData must contain a nodes array" },
            { status: 400 }
          );
        }
        updates.flowData = typeof body.flowData === "string"
          ? body.flowData
          : JSON.stringify(body.flowData);
      } catch {
        return NextResponse.json(
          { error: "flowData must be valid JSON" },
          { status: 400 }
        );
      }
    }

    await db.update(diagrams).set(updates).where(eq(diagrams.id, parseInt(id)));

    const updated = await db.query.diagrams.findFirst({
      where: eq(diagrams.id, parseInt(id)),
    });

    return NextResponse.json(updated);
  } catch (error) {
    console.error("Failed to update diagram:", error);
    return NextResponse.json({ error: "Failed to update diagram" }, { status: 500 });
  }
}

// DELETE /api/diagrams/[id]
export async function DELETE(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    await db.delete(diagrams).where(eq(diagrams.id, parseInt(id)));
    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to delete diagram:", error);
    return NextResponse.json({ error: "Failed to delete diagram" }, { status: 500 });
  }
}
