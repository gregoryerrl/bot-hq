import { NextRequest, NextResponse } from "next/server";
import { verifyDevice } from "@/lib/auth";

export async function POST(request: NextRequest) {
  const body = await request.json();
  const { fingerprint, token } = body;

  if (!fingerprint || !token) {
    return NextResponse.json(
      { error: "Missing credentials" },
      { status: 400 }
    );
  }

  const valid = await verifyDevice(fingerprint, token);

  if (!valid) {
    return NextResponse.json(
      { error: "Invalid or revoked credentials" },
      { status: 401 }
    );
  }

  return NextResponse.json({ valid: true });
}
