import { NextRequest, NextResponse } from "next/server";
import { resetPromptToDefault } from "@/lib/prompts";

export async function POST(
  _request: NextRequest,
  { params }: { params: Promise<{ slug: string }> }
) {
  try {
    const { slug } = await params;
    const updated = await resetPromptToDefault(slug);
    return NextResponse.json(updated);
  } catch (error) {
    console.error("Failed to reset prompt:", error);
    return NextResponse.json(
      { error: "Failed to reset prompt" },
      { status: 500 }
    );
  }
}
