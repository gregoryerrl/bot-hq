import { NextRequest, NextResponse } from "next/server";
import { db, diagramGroups, diagrams } from "@/lib/db";
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

  const [group] = await db
    .insert(diagramGroups)
    .values({
      diagramId,
      label: body.label.trim(),
      color: body.color ?? "#6b7280",
    })
    .returning();

  // Update diagram updatedAt
  await db
    .update(diagrams)
    .set({ updatedAt: new Date() })
    .where(eq(diagrams.id, diagramId));

  return NextResponse.json(group, { status: 201 });
}
