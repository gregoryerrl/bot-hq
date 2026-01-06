import { NextResponse } from "next/server";
import { db, authorizedDevices } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function GET() {
  try {
    const devices = await db
      .select()
      .from(authorizedDevices)
      .where(eq(authorizedDevices.isRevoked, false));
    return NextResponse.json(devices);
  } catch (error) {
    console.error("Failed to fetch devices:", error);
    return NextResponse.json(
      { error: "Failed to fetch devices" },
      { status: 500 }
    );
  }
}
