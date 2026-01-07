import { NextResponse } from "next/server";
import { db, pendingDevices } from "@/lib/db";
import { gt } from "drizzle-orm";

export async function GET() {
  try {
    // Get all non-expired pending devices
    const pending = await db
      .select()
      .from(pendingDevices)
      .where(gt(pendingDevices.expiresAt, new Date()));

    // Parse device info and format response
    const devices = pending.map((p) => {
      const info = JSON.parse(p.deviceInfo);
      return {
        id: p.id,
        pairingCode: p.pairingCode,
        deviceId: info.deviceId,
        userAgent: info.userAgent,
        ip: info.ip,
        requestedAt: info.requestedAt,
        expiresAt: p.expiresAt,
      };
    });

    return NextResponse.json(devices);
  } catch (error) {
    console.error("Failed to get pending devices:", error);
    return NextResponse.json({ error: "Failed to fetch" }, { status: 500 });
  }
}
