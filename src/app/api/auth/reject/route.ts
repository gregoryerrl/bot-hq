import { NextRequest, NextResponse } from "next/server";
import { db, pendingDevices } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(request: NextRequest) {
  try {
    const { pairingCode } = await request.json();

    if (!pairingCode) {
      return NextResponse.json({ error: "pairingCode required" }, { status: 400 });
    }

    // Delete pending device
    await db.delete(pendingDevices).where(eq(pendingDevices.pairingCode, pairingCode));

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to reject device:", error);
    return NextResponse.json({ error: "Rejection failed" }, { status: 500 });
  }
}
