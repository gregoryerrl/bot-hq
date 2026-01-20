import { NextResponse } from "next/server";
import { getManagerStatus } from "@/lib/manager/persistent-manager";

export async function GET() {
  try {
    const status = getManagerStatus();
    return NextResponse.json(status);
  } catch (error) {
    console.error("Failed to get manager status:", error);
    return NextResponse.json(
      { error: "Failed to get status", running: false, pid: null },
      { status: 500 }
    );
  }
}
