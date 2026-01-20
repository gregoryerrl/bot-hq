import { NextResponse } from "next/server";
import { sendManagerCommand, getManagerStatus } from "@/lib/manager/persistent-manager";

export async function POST(request: Request) {
  try {
    const { command } = await request.json();

    if (!command || typeof command !== "string") {
      return NextResponse.json(
        { error: "Command is required" },
        { status: 400 }
      );
    }

    const status = getManagerStatus();
    if (!status.running) {
      return NextResponse.json(
        { error: "Manager is not running" },
        { status: 503 }
      );
    }

    sendManagerCommand(command);

    return NextResponse.json({ success: true, message: "Command sent" });
  } catch (error) {
    console.error("Failed to send command:", error);
    return NextResponse.json(
      { error: "Failed to send command" },
      { status: 500 }
    );
  }
}
