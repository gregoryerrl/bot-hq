import { NextRequest, NextResponse } from "next/server";
import { ptyManager } from "@/lib/pty-manager";

// DELETE - Kill terminal session
export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;

    const success = ptyManager.killSession(id);
    if (!success) {
      return NextResponse.json(
        { error: "Session not found" },
        { status: 404 }
      );
    }

    return NextResponse.json({ message: "Session terminated" });
  } catch (error) {
    console.error("Failed to kill terminal session:", error);
    return NextResponse.json(
      { error: "Failed to kill session" },
      { status: 500 }
    );
  }
}

// POST - Send input to terminal
export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { input, resize } = await request.json();

    const session = ptyManager.getSession(id);
    if (!session) {
      return NextResponse.json(
        { error: "Session not found" },
        { status: 404 }
      );
    }

    // Handle resize
    if (resize) {
      ptyManager.resize(id, resize.cols, resize.rows);
      return NextResponse.json({ message: "Resized" });
    }

    // Handle input
    if (input !== undefined) {
      ptyManager.write(id, input);
      return NextResponse.json({ message: "Input sent" });
    }

    return NextResponse.json(
      { error: "No input or resize provided" },
      { status: 400 }
    );
  } catch (error) {
    console.error("Failed to send input to terminal:", error);
    return NextResponse.json(
      { error: "Failed to send input" },
      { status: 500 }
    );
  }
}
