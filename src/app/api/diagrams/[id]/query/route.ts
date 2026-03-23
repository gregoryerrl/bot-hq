import { NextRequest, NextResponse } from "next/server";
import { db, diagramNodes } from "@/lib/db";
import { eq, and, like, SQL } from "drizzle-orm";

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const diagramId = Number(id);
  const { searchParams } = request.nextUrl;

  const q = searchParams.get("q");
  const type = searchParams.get("type");
  const groupId = searchParams.get("groupId");

  const conditions: SQL[] = [eq(diagramNodes.diagramId, diagramId)];

  if (q) {
    const pattern = `%${q}%`;
    // Search across label, description, and metadata using OR via SQL template
    const { sql } = await import("drizzle-orm");
    conditions.push(
      sql`(${like(diagramNodes.label, pattern)} OR ${like(diagramNodes.description, pattern)} OR ${like(diagramNodes.metadata, pattern)})`
    );
  }

  if (type) {
    conditions.push(eq(diagramNodes.nodeType, type));
  }

  if (groupId) {
    conditions.push(eq(diagramNodes.groupId, Number(groupId)));
  }

  const results = await db
    .select()
    .from(diagramNodes)
    .where(and(...conditions));

  return NextResponse.json(results);
}
