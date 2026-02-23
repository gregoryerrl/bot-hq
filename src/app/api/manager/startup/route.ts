import { NextResponse } from "next/server";
import { restartManager, getManagerStatus } from "@/lib/manager/persistent-manager";

export async function POST() {
  try {
    const status = getManagerStatus();
    if (!status.running) {
      return NextResponse.json(
        { error: "Manager is not running" },
        { status: 503 }
      );
    }

    // Fire and forget — restartManager handles /clear + full prompt injection
    restartManager();

    return NextResponse.json({ success: true, message: "Startup initiated — clearing and re-injecting full prompt" });
  } catch (error) {
    console.error("Failed to restart manager:", error);
    return NextResponse.json(
      { error: "Failed to restart manager" },
      { status: 500 }
    );
  }
}
