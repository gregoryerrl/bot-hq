import { NextResponse } from "next/server";
import { clearManager, getManagerStatus } from "@/lib/manager/persistent-manager";

export async function POST() {
  try {
    const status = getManagerStatus();
    if (!status.running) {
      return NextResponse.json(
        { error: "Manager is not running" },
        { status: 503 }
      );
    }

    // Fire and forget - clearManager handles the async /clear + re-init
    clearManager();

    return NextResponse.json({ success: true, message: "Clear initiated" });
  } catch (error) {
    console.error("Failed to clear manager:", error);
    return NextResponse.json(
      { error: "Failed to clear manager" },
      { status: 500 }
    );
  }
}
