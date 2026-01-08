import { NextResponse } from "next/server";
import { listDocuments } from "@/lib/agent-docs";

export async function GET() {
  try {
    const files = await listDocuments();
    return NextResponse.json({ files });
  } catch (error) {
    console.error("Failed to list documents:", error);
    return NextResponse.json(
      { error: "Failed to list documents" },
      { status: 500 }
    );
  }
}
