import { NextRequest, NextResponse } from "next/server";
import { db, pendingDevices } from "@/lib/db";
import { generatePairingCode } from "@/lib/auth";
import { eq } from "drizzle-orm";

export async function POST(request: NextRequest) {
  try {
    const { deviceId, userAgent } = await request.json();

    if (!deviceId) {
      return NextResponse.json({ error: "deviceId required" }, { status: 400 });
    }

    // Check if there's already a pending request for this device
    const existing = await db.query.pendingDevices.findFirst({
      where: eq(pendingDevices.pairingCode, deviceId),
    });

    if (existing && existing.expiresAt > new Date()) {
      // Return existing pairing code
      return NextResponse.json({ pairingCode: existing.pairingCode });
    }

    // Clean up expired pending devices
    const now = new Date();
    await db.delete(pendingDevices).where(eq(pendingDevices.expiresAt, now));

    // Generate new pairing code
    let pairingCode: string;
    let attempts = 0;
    do {
      pairingCode = generatePairingCode();
      const conflict = await db.query.pendingDevices.findFirst({
        where: eq(pendingDevices.pairingCode, pairingCode),
      });
      if (!conflict) break;
      attempts++;
    } while (attempts < 10);

    // Create pending device (expires in 10 minutes)
    const expiresAt = new Date(Date.now() + 10 * 60 * 1000);

    await db.insert(pendingDevices).values({
      pairingCode,
      deviceInfo: JSON.stringify({
        deviceId,
        userAgent,
        ip: request.headers.get("x-forwarded-for") || request.headers.get("x-real-ip") || "unknown",
        requestedAt: new Date().toISOString(),
      }),
      expiresAt,
    });

    return NextResponse.json({ pairingCode });
  } catch (error) {
    console.error("Failed to register device:", error);
    return NextResponse.json({ error: "Failed to register" }, { status: 500 });
  }
}
