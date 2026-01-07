import { NextRequest, NextResponse } from "next/server";
import { db, authorizedDevices } from "@/lib/db";
import { eq, and } from "drizzle-orm";
import { hashToken, generateToken } from "@/lib/auth";

export async function GET(request: NextRequest) {
  try {
    const { searchParams } = new URL(request.url);
    const deviceId = searchParams.get("deviceId");
    const code = searchParams.get("code");

    if (!deviceId || !code) {
      return NextResponse.json({ error: "Missing parameters" }, { status: 400 });
    }

    // Check if device has been authorized
    const device = await db.query.authorizedDevices.findFirst({
      where: and(
        eq(authorizedDevices.deviceFingerprint, deviceId),
        eq(authorizedDevices.isRevoked, false)
      ),
    });

    if (device) {
      // Device is authorized - generate a session token and set cookie
      const token = generateToken();
      const tokenHash = hashToken(token);

      // Update the device with the new token
      await db
        .update(authorizedDevices)
        .set({ tokenHash, lastSeenAt: new Date() })
        .where(eq(authorizedDevices.id, device.id));

      // Create response with cookies
      const response = NextResponse.json({ status: "approved" });

      // Set secure cookies
      response.cookies.set("device_token", token, {
        httpOnly: true,
        secure: process.env.NODE_ENV === "production",
        sameSite: "strict",
        maxAge: 60 * 60 * 24 * 365, // 1 year
        path: "/",
      });

      response.cookies.set("device_id", deviceId, {
        httpOnly: true,
        secure: process.env.NODE_ENV === "production",
        sameSite: "strict",
        maxAge: 60 * 60 * 24 * 365, // 1 year
        path: "/",
      });

      return response;
    }

    return NextResponse.json({ status: "pending" });
  } catch (error) {
    console.error("Poll error:", error);
    return NextResponse.json({ error: "Poll failed" }, { status: 500 });
  }
}
