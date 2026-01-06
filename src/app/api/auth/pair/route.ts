import { NextRequest, NextResponse } from "next/server";
import {
  createPairingSession,
  getActivePairingCode,
  requestPairing,
  getPendingPairings,
  approvePairing,
  rejectPairing,
} from "@/lib/auth/pairing";
import { authorizeDevice } from "@/lib/auth";

// GET: Get current pairing code (for Mac display)
export async function GET(request: NextRequest) {
  const action = request.nextUrl.searchParams.get("action");

  if (action === "pending") {
    // Get pending pairing requests
    return NextResponse.json({ pending: getPendingPairings() });
  }

  // Get or create pairing code
  let code = getActivePairingCode();
  if (!code) {
    code = createPairingSession();
  }

  return NextResponse.json({ code });
}

// POST: Request pairing (from new device) or approve/reject (from Mac)
export async function POST(request: NextRequest) {
  const body = await request.json();
  const { action, code, deviceName, fingerprint } = body;

  if (action === "request") {
    // New device requesting pairing
    if (!code || !deviceName || !fingerprint) {
      return NextResponse.json(
        { error: "Missing required fields" },
        { status: 400 }
      );
    }

    const success = requestPairing(code, deviceName, fingerprint);
    if (!success) {
      return NextResponse.json(
        { error: "Invalid or expired code" },
        { status: 401 }
      );
    }

    return NextResponse.json({ status: "pending" });
  }

  if (action === "approve") {
    // Mac approving a device
    if (!fingerprint) {
      return NextResponse.json(
        { error: "Missing fingerprint" },
        { status: 400 }
      );
    }

    const pairing = approvePairing(fingerprint);
    if (!pairing) {
      return NextResponse.json(
        { error: "No pending pairing found" },
        { status: 404 }
      );
    }

    const token = await authorizeDevice(pairing.deviceName, pairing.fingerprint);
    return NextResponse.json({ status: "approved", token });
  }

  if (action === "reject") {
    if (!fingerprint) {
      return NextResponse.json(
        { error: "Missing fingerprint" },
        { status: 400 }
      );
    }

    rejectPairing(fingerprint);
    return NextResponse.json({ status: "rejected" });
  }

  return NextResponse.json({ error: "Invalid action" }, { status: 400 });
}
