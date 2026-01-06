import { NextRequest, NextResponse } from "next/server";
import { db, authorizedDevices } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    await db
      .update(authorizedDevices)
      .set({ isRevoked: true })
      .where(eq(authorizedDevices.id, parseInt(id)));

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to revoke device:", error);
    return NextResponse.json(
      { error: "Failed to revoke device" },
      { status: 500 }
    );
  }
}
