import { NextRequest, NextResponse } from "next/server";
import { db, pendingDevices, authorizedDevices } from "@/lib/db";
import { eq } from "drizzle-orm";
import { generateToken, hashToken } from "@/lib/auth";

export async function POST(request: NextRequest) {
  try {
    const { pairingCode, deviceName } = await request.json();

    if (!pairingCode) {
      return NextResponse.json({ error: "pairingCode required" }, { status: 400 });
    }

    // Find pending device
    const pending = await db.query.pendingDevices.findFirst({
      where: eq(pendingDevices.pairingCode, pairingCode),
    });

    if (!pending) {
      return NextResponse.json({ error: "Invalid pairing code" }, { status: 404 });
    }

    if (pending.expiresAt < new Date()) {
      // Clean up expired
      await db.delete(pendingDevices).where(eq(pendingDevices.id, pending.id));
      return NextResponse.json({ error: "Pairing code expired" }, { status: 400 });
    }

    const deviceInfo = JSON.parse(pending.deviceInfo);

    // Generate auth token
    const token = generateToken();
    const tokenHash = hashToken(token);

    // Create authorized device
    await db.insert(authorizedDevices).values({
      deviceName: deviceName || `Device ${deviceInfo.deviceId.slice(0, 8)}`,
      deviceFingerprint: deviceInfo.deviceId,
      tokenHash,
    });

    // Remove from pending
    await db.delete(pendingDevices).where(eq(pendingDevices.id, pending.id));

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to approve device:", error);
    return NextResponse.json({ error: "Approval failed" }, { status: 500 });
  }
}
