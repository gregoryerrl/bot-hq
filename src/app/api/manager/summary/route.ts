import { NextResponse } from "next/server";
import { getQuickSummary } from "@/lib/agents/manager";

export async function GET() {
  try {
    const summary = await getQuickSummary();
    return NextResponse.json({ summary });
  } catch (error) {
    console.error("Summary error:", error);
    return NextResponse.json(
      { error: "Failed to get summary" },
      { status: 500 }
    );
  }
}
