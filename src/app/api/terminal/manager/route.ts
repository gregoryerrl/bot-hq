import { NextResponse } from "next/server";
import { ptyManager, MANAGER_SESSION_ID } from "@/lib/pty-manager";
import { getScopePath } from "@/lib/settings";

// GET - Get or create the manager session
export async function GET() {
  try {
    const scopePath = await getScopePath();

    // Ensure manager session exists
    ptyManager.ensureManagerSession(scopePath);

    // Get session info
    const session = ptyManager.getSession(MANAGER_SESSION_ID);

    if (!session) {
      return NextResponse.json(
        { error: "Failed to create manager session" },
        { status: 500 }
      );
    }

    return NextResponse.json({
      sessionId: MANAGER_SESSION_ID,
      exists: true,
      createdAt: session.createdAt,
      lastActivityAt: session.lastActivityAt,
      bufferSize: session.buffer.length,
    });
  } catch (error) {
    console.error("Failed to get/create manager session:", error);
    return NextResponse.json(
      { error: `Failed to get manager session: ${error}` },
      { status: 500 }
    );
  }
}
