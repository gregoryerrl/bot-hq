import { NextRequest, NextResponse } from "next/server";
import { ptyManager } from "@/lib/pty-manager";
import { getScopePath } from "@/lib/settings";

// POST - Create new terminal session
export async function POST(request: NextRequest) {
  try {
    const body = await request.json().catch(() => ({}));
    const scopePath = await getScopePath();
    const cwd = body.cwd || scopePath;

    // Validate cwd is within scope
    if (!cwd.startsWith(scopePath)) {
      return NextResponse.json(
        { error: "Path is outside of scope" },
        { status: 400 }
      );
    }

    const sessionId = ptyManager.createSession(cwd);

    return NextResponse.json({
      sessionId,
      message: "Terminal session created",
    });
  } catch (error) {
    console.error("Failed to create terminal session:", error);
    return NextResponse.json(
      { error: `Failed to create session: ${error}` },
      { status: 500 }
    );
  }
}

// GET - List all active sessions
export async function GET() {
  try {
    const sessions = ptyManager.listSessions();
    return NextResponse.json({ sessions });
  } catch (error) {
    console.error("Failed to list terminal sessions:", error);
    return NextResponse.json(
      { error: "Failed to list sessions" },
      { status: 500 }
    );
  }
}
